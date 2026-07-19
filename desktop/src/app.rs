//! The application shell: which screen is showing, the running session (if
//! any), dialogs, toasts, and the plumbing between the UI and the emulator
//! thread. The screens themselves are drawn by [`crate::ui`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use eframe::egui::{self, TextureHandle};
use emiu2::platform::link_ir::{LinkCommander, LinkMode};

use crate::config::{Config, IrMode};
use crate::controls::Bindings;
use crate::emu::{self, EmuCmd, EmuEvent, EmuSession, SessionSpec};
use crate::saves::{self, Library, SaveSlot};
use crate::{theme, ui};

pub enum View {
    Library,
    Playing,
}

/// At most one dialog is open at a time; while one is, device input is
/// suppressed so typing a save name can't steer the game.
pub enum Dialog {
    None,
    NewSave(ui::library::NewSaveState),
    Controls(ui::dialogs::RemapState),
    Friends(ui::dialogs::FriendsState),
    ConfirmReset { connect_mode: bool },
    ConfirmDelete { save_id: String },
    Rename { save_id: String, name: String },
}

pub enum ToastKind {
    Info,
    Error,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub born: Instant,
}

const TOAST_LIFETIME: Duration = Duration::from_secs(5);

/// A save being played: the emulator session plus its UI-side state.
pub struct PlaySession {
    pub emu: EmuSession,
    pub slot: SaveSlot,
    pub texture: TextureHandle,
    pub frame_seq: u64,
    rgb_scratch: Vec<u8>,
    pub started: Instant,
    pub base_play_seconds: u64,
    /// When the last save hit disk (drives the "Saved" indicator).
    pub last_saved: Option<Instant>,
    pub fullscreen: bool,
    /// Buttons held with the mouse this frame, filled in during drawing.
    pub click_mask: u16,
    /// Since when the LCD has shown nothing but black (device asleep, or
    /// powered-off screen). `None` while the panel shows anything.
    blank_since: Option<Instant>,
}

impl PlaySession {
    pub fn play_seconds(&self) -> u64 {
        self.base_play_seconds + self.started.elapsed().as_secs()
    }

    /// Uploads the newest LCD frame into the texture if one arrived.
    pub fn refresh_texture(&mut self) {
        if self.emu.frame.seq() == self.frame_seq {
            return;
        }
        self.frame_seq = self.emu.frame.read_rgb(&mut self.rgb_scratch);
        if emu::frame_is_blank(&self.rgb_scratch) {
            self.blank_since.get_or_insert_with(Instant::now);
        } else {
            self.blank_since = None;
        }
        let image =
            egui::ColorImage::from_rgb([emu::LCD_WIDTH, emu::LCD_HEIGHT], &self.rgb_scratch);
        self.texture.set(image, egui::TextureOptions::NEAREST);
    }

    /// True when the panel has been black long enough that it's the
    /// device sleeping, not a scene transition. Resuming a sleeping save
    /// would otherwise look broken.
    pub fn looks_asleep(&self) -> bool {
        self.blank_since
            .is_some_and(|since| since.elapsed() > std::time::Duration::from_secs(3))
    }
}

pub struct DesktopApp {
    pub config: Config,
    pub config_path: PathBuf,
    pub library: Library,
    pub saves: Vec<SaveSlot>,
    /// Gallery thumbnails by save id, dropped on every library refresh.
    pub thumbs: HashMap<String, Option<TextureHandle>>,
    pub bindings: Bindings,
    pub view: View,
    pub session: Option<PlaySession>,
    /// IR link control, once the session's transport reports in.
    pub link: Option<LinkCommander>,
    pub dialog: Dialog,
    pub toasts: Vec<Toast>,
    pub shot: Option<crate::shot::ShotState>,
}

impl DesktopApp {
    pub fn new(cc: &eframe::CreationContext<'_>, options: crate::StartupOptions) -> Self {
        theme::apply(&cc.egui_ctx);
        // The wordmark's SVG logo needs the svg image loader.
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let config_path = Config::path();
        let config = Config::load(&config_path);
        let mut bindings = Bindings::default();
        bindings.apply_config(&config.controls);

        let library = Library::open();
        let saves = library.list();

        let mut app = Self {
            config,
            config_path,
            library,
            saves,
            thumbs: HashMap::new(),
            bindings,
            view: View::Library,
            session: None,
            link: None,
            dialog: Dialog::None,
            toasts: Vec::new(),
            shot: options.shot,
        };

        if let Some(play) = options.play {
            app.play_by_name(&cc.egui_ctx, &play);
        }
        if let Some(shot) = &app.shot {
            shot.force_ui_state(&mut app.dialog);
            // Only the in-app layout: requesting native fullscreen during
            // startup stalls some compositors, and it's the layout the
            // harness is checking.
            if shot.ui_state() == Some("fullscreen") {
                if let Some(session) = &mut app.session {
                    session.fullscreen = true;
                }
            }
        }
        app
    }

    pub fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        let text = text.into();
        if matches!(kind, ToastKind::Error) {
            eprintln!("{text}");
        }
        self.toasts.push(Toast {
            text,
            kind,
            born: Instant::now(),
        });
    }

    pub fn save_config(&mut self) {
        self.config.controls = self.bindings.to_config();
        self.config.save(&self.config_path);
    }

    pub fn refresh_saves(&mut self) {
        self.saves = self.library.list();
        self.thumbs.clear();
    }

    /// Starts playing a save slot; on failure stays in the library.
    pub fn start_session(&mut self, ctx: &egui::Context, slot: SaveSlot) {
        self.boot_session(ctx, slot, false);
    }

    /// The shared boot path. `connect_mode` holds Left+Menu through early
    /// boot so the device starts in "Please Connect to PC".
    fn boot_session(&mut self, ctx: &egui::Context, slot: SaveSlot, connect_mode: bool) {
        let read = |file: &str| std::fs::read(slot.path(file));
        let otp = match read(saves::OTP_FILE) {
            Ok(data) => data,
            Err(why) => {
                self.toast(
                    ToastKind::Error,
                    format!("Could not read the save's OTP: {why}"),
                );
                return;
            }
        };
        let flash = match read(saves::FLASH_FILE) {
            Ok(data) => data,
            Err(why) => {
                self.toast(
                    ToastKind::Error,
                    format!("Could not read the save's flash image: {why}"),
                );
                return;
            }
        };
        let snapshot = slot
            .has_snapshot
            .then(|| read(saves::SNAPSHOT_FILE).ok())
            .flatten();

        // Online mode needs a relay address; without one (or in local
        // mode) the session links locally, which needs no configuration.
        let link = {
            let relay = self.config.ir.relay.trim();
            match self.config.ir.mode {
                IrMode::Online if !relay.is_empty() => LinkMode::Online {
                    relay: relay.to_owned(),
                },
                _ => LinkMode::Local,
            }
        };

        let spec = SessionSpec {
            save_dir: slot.dir.clone(),
            identity: slot.meta.name.clone(),
            otp,
            flash,
            snapshot,
            usb_plugged: self.config.usb.plugged,
            link,
            connect_mode,
        };

        let emu = EmuSession::start(spec, ctx.clone());
        let texture = ctx.load_texture(
            "lcd",
            egui::ColorImage::filled([emu::LCD_WIDTH, emu::LCD_HEIGHT], egui::Color32::BLACK),
            egui::TextureOptions::NEAREST,
        );

        let base_play_seconds = slot.meta.play_seconds;
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "{} — Emiu2 Desktop",
            slot.meta.name
        )));

        self.session = Some(PlaySession {
            emu,
            slot,
            texture,
            frame_seq: 0,
            rgb_scratch: Vec::new(),
            started: Instant::now(),
            base_play_seconds,
            last_saved: None,
            fullscreen: false,
            click_mask: 0,
            blank_since: None,
        });
        self.link = None;
        self.view = View::Playing;
    }

    /// Starts a save by display name; creates it (recommended firmware)
    /// when the name matches a character and no save exists yet. Used by
    /// the `--play` flag.
    fn play_by_name(&mut self, ctx: &egui::Context, name: &str) {
        let existing = self
            .saves
            .iter()
            .find(|slot| slot.meta.name.eq_ignore_ascii_case(name))
            .cloned();
        let slot = match existing {
            Some(slot) => Some(slot),
            None => {
                let character = crate::firmware::CHARACTERS
                    .iter()
                    .find(|c| c.eq_ignore_ascii_case(name));
                match character {
                    Some(character) => self.create_save(name, character, None, None, None),
                    None => {
                        self.toast(
                            ToastKind::Error,
                            format!("No save or character named {name:?}"),
                        );
                        None
                    }
                }
            }
        };
        if let Some(slot) = slot {
            self.start_session(ctx, slot);
        }
    }

    /// Creates a save from bundled firmware (or explicit image bytes) and
    /// returns it. Reports problems as toasts.
    pub fn create_save(
        &mut self,
        name: &str,
        character: &str,
        version: Option<&str>,
        custom_otp: Option<Vec<u8>>,
        custom_flash: Option<Vec<u8>>,
    ) -> Option<SaveSlot> {
        let version = version.unwrap_or(crate::firmware::RECOMMENDED_VERSION);
        let flash_owned;
        let flash: &[u8] = match custom_flash {
            Some(data) => {
                flash_owned = data;
                &flash_owned
            }
            None => match crate::firmware::find(character, version) {
                Some(fw) => fw.data,
                None => {
                    self.toast(
                        ToastKind::Error,
                        format!("No bundled firmware for {character} {version}"),
                    );
                    return None;
                }
            },
        };
        let otp_owned;
        let otp: &[u8] = match custom_otp {
            Some(data) => {
                otp_owned = data;
                &otp_owned
            }
            None => crate::firmware::OTP,
        };

        match self.library.create(name, character, version, otp, flash) {
            Ok(slot) => {
                self.refresh_saves();
                Some(slot)
            }
            Err(why) => {
                self.toast(
                    ToastKind::Error,
                    format!("Could not create the save: {why}"),
                );
                None
            }
        }
    }

    /// Stops the running session, blocking through its final save. Safe
    /// with no session running.
    fn shutdown_session(&mut self) {
        let Some(mut session) = self.session.take() else {
            return;
        };
        session.slot.meta.play_seconds = session.play_seconds();
        session.slot.meta.last_played_unix = saves::unix_now();
        if let Err(why) = session.slot.write_meta() {
            self.toast(
                ToastKind::Error,
                format!("Could not update the save: {why}"),
            );
        }
        session.emu.shutdown();
        self.link = None;
    }

    /// A battery pull: saves and stops the running session, drops the RAM
    /// snapshot so only what the game wrote to flash survives, and boots
    /// the same save again cold from that flash.
    pub fn restart_session(&mut self, ctx: &egui::Context, connect_mode: bool) {
        let Some(session) = &self.session else {
            return;
        };
        let fullscreen = session.fullscreen;
        let save_id = session.slot.id.clone();
        let snapshot_path = session.slot.path(saves::SNAPSHOT_FILE);
        if connect_mode {
            // Connect mode exists to talk to a host; imply the cable.
            self.config.usb.plugged = true;
            self.save_config();
        }
        self.shutdown_session();
        // The final save just wrote a snapshot; a pulled battery keeps no
        // RAM, so drop it — the reboot (and any later resume) must be a
        // cold boot from flash.
        if let Err(why) = std::fs::remove_file(&snapshot_path) {
            if why.kind() != std::io::ErrorKind::NotFound {
                self.toast(
                    ToastKind::Error,
                    format!("Could not drop the old snapshot: {why}"),
                );
            }
        }
        self.refresh_saves();
        let Some(slot) = self.library.get(&save_id) else {
            self.toast(
                ToastKind::Error,
                "The save disappeared during the restart".to_owned(),
            );
            self.view = View::Library;
            return;
        };
        self.boot_session(ctx, slot, connect_mode);
        if let Some(session) = &mut self.session {
            session.fullscreen = fullscreen;
        }
    }

    /// Stops the running session (final save included) and returns to the
    /// library.
    pub fn close_session(&mut self, ctx: &egui::Context) {
        if self.session.as_ref().is_some_and(|s| s.fullscreen) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        self.shutdown_session();
        self.view = View::Library;
        ctx.send_viewport_cmd(egui::ViewportCommand::Title("Emiu2 Desktop".to_owned()));
        self.refresh_saves();
    }

    fn poll_session_events(&mut self) {
        let Some(session) = &mut self.session else {
            return;
        };
        let mut errors = Vec::new();
        while let Ok(event) = session.emu.events.try_recv() {
            match event {
                EmuEvent::Saved => {
                    session.last_saved = Some(Instant::now());
                    session.slot.meta.play_seconds = session.play_seconds();
                    session.slot.meta.last_played_unix = saves::unix_now();
                    session.slot.has_snapshot = true;
                    if let Err(why) = session.slot.write_meta() {
                        errors.push(format!("Could not update the save: {why}"));
                    }
                }
                EmuEvent::Link(commander) => self.link = Some(commander),
                EmuEvent::Error(message) => errors.push(message),
            }
        }
        for error in errors {
            self.toast(ToastKind::Error, error);
        }
    }

    /// The device input for this frame: mapped keys held (unless the UI
    /// wants the keyboard) plus buttons held with the mouse.
    fn push_device_input(&mut self, ctx: &egui::Context) {
        let Some(session) = &self.session else {
            return;
        };
        let keys_allowed = matches!(self.dialog, Dialog::None) && !ctx.egui_wants_keyboard_input();
        let key_mask = if keys_allowed {
            ctx.input(|i| self.bindings.mask_from_keys(&i.keys_down))
        } else {
            0
        };
        session
            .emu
            .buttons
            .store(key_mask | session.click_mask, Ordering::Relaxed);
    }

    fn global_shortcuts(&mut self, ctx: &egui::Context) {
        if !matches!(self.view, View::Playing) || !matches!(self.dialog, Dialog::None) {
            return;
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (f11, esc, save_now) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::F11),
                i.key_pressed(egui::Key::Escape),
                i.modifiers.command && i.key_pressed(egui::Key::S),
            )
        });
        if f11 {
            self.set_fullscreen(ctx, self.session.as_ref().is_some_and(|s| !s.fullscreen));
        }
        if esc {
            self.set_fullscreen(ctx, false);
        }
        if save_now {
            if let Some(session) = &self.session {
                session.emu.send(EmuCmd::SaveNow);
            }
        }
    }

    pub fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        if let Some(session) = &mut self.session {
            if session.fullscreen != on {
                session.fullscreen = on;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
            }
        }
    }

    fn show_toasts(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|t| t.born.elapsed() < TOAST_LIFETIME);
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for toast in &self.toasts {
                    let (icon, tint) = match toast.kind {
                        ToastKind::Info => ("•", theme::ACCENT),
                        ToastKind::Error => ("!", theme::BAD),
                    };
                    egui::Frame::new()
                        .fill(theme::CARD)
                        .stroke(egui::Stroke::new(1.0, theme::OUTLINE))
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .shadow(egui::Shadow {
                            offset: [0, 4],
                            blur: 16,
                            spread: 0,
                            color: egui::Color32::from_black_alpha(100),
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(icon).color(tint).strong());
                                ui.label(&toast.text);
                            });
                        });
                }
            });
    }
}

impl eframe::App for DesktopApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        self.poll_session_events();

        // A crashed emulator thread closes its event channel; fall back to
        // the library rather than showing a frozen device.
        if matches!(self.view, View::Playing) {
            let dead = self
                .session
                .as_ref()
                .is_none_or(|session| session.emu.is_finished());
            if dead {
                self.toast(ToastKind::Error, "The emulator stopped unexpectedly");
                self.close_session(&ctx);
            }
        }

        self.global_shortcuts(&ctx);

        match self.view {
            View::Library => ui::library::show(self, root),
            View::Playing => ui::player::show(self, root),
        }

        ui::dialogs::show(self, &ctx);
        self.show_toasts(&ctx);
        self.push_device_input(&ctx);

        if let Some(shot) = &mut self.shot {
            if shot.step(&ctx) {
                self.shot = None;
            }
        }

        // Heartbeat repaint: frames already wake the UI, but toasts, the
        // saved-indicator fade, and a sleeping device (black screen, no
        // frames) still need the clock to advance.
        ctx.request_repaint_after(Duration::from_millis(150));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // The window is closing: capture the running machine exactly as it
        // is (the session's shutdown snapshots before stopping).
        self.shutdown_session();
        self.save_config();
    }
}
