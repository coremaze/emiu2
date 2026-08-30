//! The app's dialogs: new save (modal form), friends (IR pairing),
//! controls remapping, and the small confirmations. Exactly one dialog is
//! open at a time (see [`Dialog`]); while any is open, device input is
//! suppressed by the app shell.

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, Id, Key, RichText, Stroke};
use emiu2::platform::ir_discovery::{self, LocalPeer};
use emiu2::platform::link_ir::DialState;
use emiu2_netplay::FriendCode;

use crate::app::{DesktopApp, Dialog, ToastKind};
use crate::config::IrMode;
use crate::theme;
use crate::ui::library;

#[derive(Default)]
pub struct RemapState {
    /// The binding row waiting for a key press.
    pub capture: Option<usize>,
}

/// How often the Friends dialog rescans for other windows while open.
const SCAN_INTERVAL: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub struct FriendsState {
    pub code_entry: String,
    /// The relay address being edited, when the field is open.
    pub relay_edit: Option<String>,
    /// The latest nearby scan, and when it ran.
    peers: Vec<LocalPeer>,
    last_scan: Option<Instant>,
    /// Who the player last dialed, for the status text.
    link_target: Option<String>,
}

pub fn show(app: &mut DesktopApp, ctx: &egui::Context) {
    // Take the dialog out for the frame so its UI can borrow `app` freely.
    let dialog = std::mem::replace(&mut app.dialog, Dialog::None);
    match dialog {
        Dialog::None => {}
        Dialog::NewSave(state) => new_save(app, ctx, state),
        Dialog::Controls(state) => controls(app, ctx, state),
        Dialog::Friends(state) => friends(app, ctx, state),
        Dialog::ConfirmReset { connect_mode } => confirm_reset(app, ctx, connect_mode),
        Dialog::ConfirmDelete { save_id } => confirm_delete(app, ctx, save_id),
        Dialog::Rename { save_id, name } => rename(app, ctx, save_id, name),
    }
}

fn new_save(app: &mut DesktopApp, ctx: &egui::Context, mut state: library::NewSaveState) {
    // On a fresh library the form is drawn inline by the library screen,
    // not as a modal.
    if app.saves.is_empty() {
        app.dialog = Dialog::NewSave(state);
        return;
    }
    let modal = egui::Modal::new(Id::new("new_save")).show(ctx, |ui| {
        // Keep a clear gap of backdrop all around, even on a small window;
        // when the form is taller than that fits, it scrolls.
        const GAP: f32 = 16.0;
        let max = ctx.content_rect().shrink(GAP).size()
            - egui::Frame::popup(ui.style()).total_margin().sum();
        // Wide enough that all seven characters sit on one row, but never
        // wider than the window allows.
        ui.set_width(580.0_f32.min(max.x));
        ui.set_max_height(max.y);

        ui.heading("New save");
        ui.add_space(2.0);
        ui.label(RichText::new("Pick a character; the game is built in.").color(theme::TEXT_DIM));
        ui.add_space(12.0);

        // A solid, always-there scrollbar so a cramped window still shows
        // there is more form below.
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        theme::scrollbar_fills(ui);
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.reset_style();
                library::new_save_form(ui, &mut state, true)
            })
            .inner
    });
    let close = modal.should_close();
    let action = modal.inner;
    app.dialog = Dialog::NewSave(state);
    if close {
        app.dialog = Dialog::None;
    } else {
        library::apply_form_action(app, ctx, action);
    }
}

fn controls(app: &mut DesktopApp, ctx: &egui::Context, mut state: RemapState) {
    // While a row is capturing, feed it the next key press. Escape cancels
    // the capture (and is consumed before the modal sees it).
    if let Some(index) = state.capture {
        let pressed = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Key {
                    key, pressed: true, ..
                } => Some(*key),
                _ => None,
            })
        });
        if let Some(key) = pressed {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key));
            if key != Key::Escape {
                app.bindings.assign(index, key);
                app.save_config();
            }
            state.capture = None;
        }
    }

    let capturing = state.capture.is_some();
    let modal = egui::Modal::new(Id::new("controls")).show(ctx, |ui| {
        // Fit inside the window with a clear gap of backdrop all around;
        // when the window is too short for the list, the bindings scroll
        // between the fixed header and footer.
        const GAP: f32 = 16.0;
        let max = ctx.content_rect().shrink(GAP).size()
            - egui::Frame::popup(ui.style()).total_margin().sum();
        ui.set_width(400.0_f32.min(max.x));
        ui.set_max_height(max.y);

        ui.heading("Controls");
        ui.add_space(2.0);
        ui.label(
            RichText::new("Click a binding, then press the key you want. On-screen buttons always work with the mouse too.")
                .color(theme::TEXT_DIM),
        );
        ui.add_space(12.0);

        let mut done = false;
        // Keep room below the list for the spacer + footer buttons.
        let footer = 12.0 + 26.0 + ui.spacing().item_spacing.y;
        // A solid, always-there scrollbar: on a cramped window it is the
        // only hint that the rest of the bindings are below.
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        theme::scrollbar_fills(ui);
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height((ui.available_height() - footer).max(48.0))
            .show(ui, |ui| {
                ui.reset_style();
                egui::Grid::new("bindings")
                    .num_columns(3)
                    .spacing(egui::vec2(14.0, 7.0))
                    .show(ui, |ui| {
                        for index in 0..app.bindings.len() {
                            let binding = app.bindings.get(index).clone();
                            ui.label(RichText::new(binding.label).color(theme::TEXT));

                            let capturing_this = state.capture == Some(index);
                            let key_text = if capturing_this {
                                RichText::new("Press a key…")
                                    .color(theme::ACCENT)
                                    .italics()
                            } else {
                                match binding.key {
                                    Some(key) => RichText::new(key.name()).strong(),
                                    None => RichText::new("unbound").color(theme::TEXT_FAINT),
                                }
                            };
                            let button = egui::Button::new(key_text)
                                .min_size(egui::vec2(120.0, 24.0))
                                .stroke(if capturing_this {
                                    Stroke::new(1.0, theme::ACCENT)
                                } else {
                                    Stroke::new(1.0, theme::OUTLINE)
                                });
                            if ui.add(button).clicked() {
                                state.capture = Some(index);
                            }

                            let clear = ui
                                .add_enabled(
                                    binding.key.is_some(),
                                    egui::Button::new(RichText::new("×").size(11.0)).frame(false),
                                )
                                .on_hover_text("Unbind");
                            if clear.clicked() {
                                app.bindings.clear(index);
                                app.save_config();
                            }
                            ui.end_row();
                        }
                    });
            });
        ui.reset_style();

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Done").clicked() {
                done = true;
            }
            if ui
                .button(RichText::new("Reset to defaults").color(theme::TEXT_DIM))
                .clicked()
            {
                app.bindings = crate::controls::Bindings::default();
                app.save_config();
            }
        });
        done
    });

    let done = modal.inner;
    // Escape during capture cancels the capture, not the dialog (the
    // capture handler consumed it above).
    if done || (modal.should_close() && !capturing) {
        app.dialog = Dialog::None;
    } else {
        app.dialog = Dialog::Controls(state);
    }
}

fn friends(app: &mut DesktopApp, ctx: &egui::Context, mut state: FriendsState) {
    let modal = egui::Modal::new(Id::new("friends")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.heading("Friends");
        ui.add_space(2.0);
        ui.label(
            RichText::new(
                "Play and trade over IR, with another window on this \
                 computer, or online.",
            )
            .color(theme::TEXT_DIM),
        );
        ui.add_space(10.0);

        // The mode tabs. Switching applies to the running link right away.
        //
        // Built as always-framed selectable buttons: egui sizes a resting
        // unselected selectable_label without the stroke compensation the
        // stroked states get, so its pill would grow 2px on hover and
        // shift the row (the theme's 1px inactive hairline arms this).
        let mode = app.config.ir.mode;
        let tab = |ui: &mut egui::Ui, selected: bool, text: &str| {
            // Transparent at rest so the hairline reads alone; selected
            // keeps the theme's selection fill (an explicit fill would
            // otherwise override it too).
            let fill = if selected {
                ui.visuals().selection.bg_fill
            } else {
                Color32::TRANSPARENT
            };
            ui.add(
                egui::Button::selectable(selected, text)
                    .frame_when_inactive(true)
                    .fill(fill),
            )
            .clicked()
        };
        ui.horizontal(|ui| {
            if tab(ui, mode == IrMode::Local, "This computer") && mode != IrMode::Local {
                app.config.ir.mode = IrMode::Local;
                app.save_config();
                if let Some(link) = &app.link {
                    link.set_local();
                }
            }
            if tab(ui, mode == IrMode::Online, "Online") && mode != IrMode::Online {
                app.config.ir.mode = IrMode::Online;
                app.save_config();
                // Always hand the mode to the transport, even with an
                // empty relay: it must leave local discovery (stop
                // advertising and accepting nearby links) and idle, which
                // is exactly the "no relay server set" state the tab
                // shows. Otherwise it stays discoverable behind the UI.
                if let Some(link) = &app.link {
                    link.set_online(app.config.ir.relay.trim());
                }
            }
        });
        ui.add_space(10.0);

        match app.config.ir.mode {
            IrMode::Local => local_tab(app, ui, &mut state),
            IrMode::Online => online_tab(app, ctx, ui, &mut state),
        }

        if app.session.is_some() {
            ui.add_space(12.0);
            ui.label(
                RichText::new(
                    "Once linked, take both Miuchiz to their wireless/IR \
                     feature to play or trade, just like holding two real \
                     ones face to face.",
                )
                .small()
                .color(theme::TEXT_FAINT),
            );
        }
    });

    if modal.should_close() {
        app.dialog = Dialog::None;
    } else {
        app.dialog = Dialog::Friends(state);
    }
}

/// The status line's colored dot plus text.
fn status_line(ui: &mut egui::Ui, dot: Color32, label: impl Into<egui::WidgetText>) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, dot);
        ui.label(label);
    });
}

/// The local ("This computer") side of the Friends dialog: discovery of
/// other running Emiu2 windows and one-click linking.
fn local_tab(app: &mut DesktopApp, ui: &mut egui::Ui, state: &mut FriendsState) {
    if app.session.is_none() {
        ui.label("Start playing a save to link up.");
        return;
    }
    let Some(link) = &app.link else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("Setting up…").color(theme::TEXT_DIM));
        });
        return;
    };

    if link.linked() {
        let with = state.link_target.as_deref().unwrap_or("a nearby player");
        status_line(ui, theme::GOOD, format!("Linked with {with}"));
        ui.add_space(8.0);
        if ui.button("Unlink").clicked() {
            link.disconnect_peer();
            state.link_target = None;
        }
        return;
    }

    match link.dial() {
        DialState::Dialing => {
            let with = state.link_target.as_deref().unwrap_or("the other window");
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Linking with {with}…"));
            });
        }
        dial => {
            status_line(
                ui,
                theme::ACCENT,
                "Visible to other Emiu2 windows on this computer",
            );
            if dial == DialState::Failed {
                let with = state.link_target.as_deref().unwrap_or("the other window");
                ui.label(
                    RichText::new(format!(
                        "Couldn't link with {with}. It may have closed, \
                         or already be linked with someone else."
                    ))
                    .small()
                    .color(theme::BAD),
                );
            }
        }
    }
    ui.add_space(10.0);

    // The scan, refreshed while the dialog is open (the app's heartbeat
    // repaint keeps this ticking).
    if state
        .last_scan
        .is_none_or(|at| at.elapsed() >= SCAN_INTERVAL)
    {
        state.peers = ir_discovery::scan();
        state.last_scan = Some(Instant::now());
    }

    if state.peers.is_empty() {
        ui.label(
            RichText::new(
                "No other window found. Open Emiu2 again, start a save, \
                 and it will appear here.",
            )
            .color(theme::TEXT_FAINT),
        );
        return;
    }

    ui.label(RichText::new("On this computer").color(theme::TEXT_DIM));
    ui.add_space(4.0);
    let mut dial: Option<LocalPeer> = None;
    egui::Frame::new()
        .fill(theme::BG)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (index, peer) in state.peers.iter().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                // A fixed-height row (an unconstrained layout would take
                // the dialog's whole remaining height): the button goes
                // in first so the name centers against it, then the name
                // takes the remaining width and elides rather than
                // stretch the row.
                let row = egui::vec2(ui.available_width(), 26.0);
                ui.allocate_ui_with_layout(
                    row,
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.button("Link").clicked() {
                            dial = Some(peer.clone());
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(RichText::new(&peer.name).strong()).truncate());
                        });
                    },
                );
            }
        });
    if let Some(peer) = dial {
        link.connect_peer(peer.addr());
        state.link_target = Some(peer.name);
    }
}

/// The online side of the Friends dialog: the relay, friend codes, and
/// the tucked-away relay address editor.
fn online_tab(
    app: &mut DesktopApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    state: &mut FriendsState,
) {
    if app.session.is_none() {
        ui.label("Start playing a save to go online.");
    } else if app.config.ir.relay.trim().is_empty() {
        ui.label(
            "No relay server is set. Enter the address of an emiu2 relay \
             to get a friend code.",
        );
    } else {
        match &app.link {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Setting up…").color(theme::TEXT_DIM));
                });
            }
            Some(link) => {
                let (dot, label) = if link.linked() {
                    (theme::GOOD, "Paired with a friend".to_owned())
                } else if link.relay_connected() {
                    (theme::GOOD, "Connected".to_owned())
                } else {
                    (
                        theme::ACCENT,
                        format!("Connecting to {}…", app.config.ir.relay),
                    )
                };
                status_line(ui, dot, label);
                ui.add_space(8.0);

                // Your code, big and copyable.
                if let Some(code) = link.code() {
                    egui::Frame::new()
                        .fill(theme::BG)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new("YOUR CODE")
                                            .size(10.0)
                                            .color(theme::TEXT_FAINT),
                                    );
                                    ui.label(
                                        RichText::new(code.to_string())
                                            .monospace()
                                            .size(26.0)
                                            .color(theme::ACCENT),
                                    );
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("Copy").clicked() {
                                            ctx.copy_text(code.to_string());
                                        }
                                    },
                                );
                            });
                        });
                    ui.add_space(10.0);
                }

                if link.linked() {
                    if ui.button("Leave the pairing").clicked() {
                        link.leave();
                    }
                } else if link.relay_connected() {
                    ui.label(RichText::new("Join a friend").color(theme::TEXT_DIM));
                    ui.horizontal(|ui| {
                        let edit = egui::TextEdit::singleline(&mut state.code_entry)
                            .hint_text("their code")
                            .font(egui::TextStyle::Monospace)
                            .char_limit(6)
                            .desired_width(110.0);
                        ui.add(edit);
                        state.code_entry = state.code_entry.to_uppercase();
                        let code = FriendCode::parse(state.code_entry.trim());
                        if ui
                            .add_enabled(code.is_some(), egui::Button::new("Join"))
                            .clicked()
                        {
                            if let Some(code) = code {
                                link.join(code);
                                state.code_entry.clear();
                            }
                        }
                    });
                }
            }
        }
    }

    // The relay address, tucked away: most people never change it.
    ui.add_space(12.0);
    match &mut state.relay_edit {
        None => {
            let label = RichText::new(format!(
                "Relay: {}",
                if app.config.ir.relay.trim().is_empty() {
                    "none"
                } else {
                    app.config.ir.relay.as_str()
                }
            ))
            .small()
            .color(theme::TEXT_FAINT);
            if ui
                .add(egui::Label::new(label).sense(egui::Sense::click()))
                .on_hover_text("Click to change the relay server")
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                state.relay_edit = Some(app.config.ir.relay.clone());
            }
        }
        Some(edit) => {
            let mut saved = false;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Relay").small().color(theme::TEXT_DIM));
                ui.add(
                    egui::TextEdit::singleline(edit)
                        .hint_text("host:port")
                        .desired_width(180.0),
                );
                if ui.button("Save").clicked() {
                    app.config.ir.relay = edit.trim().to_owned();
                    app.save_config();
                    // A running online link moves to the new relay now,
                    // and a cleared relay disconnects it — otherwise it
                    // would keep the old connection and a live friend code
                    // while the UI claims no relay is set.
                    if let Some(link) = &app.link {
                        link.set_online(app.config.ir.relay.clone());
                    }
                    app.toasts.push(crate::app::Toast {
                        text: "Relay saved".to_owned(),
                        kind: ToastKind::Info,
                        born: Instant::now(),
                    });
                    saved = true;
                }
            });
            if saved {
                state.relay_edit = None;
            }
        }
    }
}

fn confirm_reset(app: &mut DesktopApp, ctx: &egui::Context, connect_mode: bool) {
    let (title, body, confirm) = if connect_mode {
        (
            "Restart to PC connection?",
            "The device reboots straight into its \"Please Connect to PC\" mode \
             so USB tools can manage it. Anything not saved on the device is lost, \
             and the USB cable will be plugged in.",
            "Restart",
        )
    } else {
        (
            "Restart the device?",
            "This is like pulling the batteries: the device reboots from its \
             flash memory, and anything it hasn't saved itself is lost.",
            "Restart",
        )
    };
    match confirm_modal(ctx, "confirm_reset", title, body, confirm) {
        Some(true) => {
            app.restart_session(ctx, connect_mode);
            app.dialog = Dialog::None;
        }
        Some(false) => app.dialog = Dialog::None,
        None => app.dialog = Dialog::ConfirmReset { connect_mode },
    }
}

fn confirm_delete(app: &mut DesktopApp, ctx: &egui::Context, save_id: String) {
    let Some(slot) = app.library.get(&save_id) else {
        app.dialog = Dialog::None;
        return;
    };
    let body = format!(
        "Delete \"{}\"? The character, all progress, and the save's files \
         are gone for good.",
        slot.meta.name
    );
    match confirm_modal(ctx, "confirm_delete", "Delete this save?", &body, "Delete") {
        Some(true) => {
            if let Err(why) = app.library.delete(&slot) {
                app.toast(
                    ToastKind::Error,
                    format!("Could not delete the save: {why}"),
                );
            }
            app.refresh_saves();
            app.dialog = Dialog::None;
        }
        Some(false) => app.dialog = Dialog::None,
        None => app.dialog = Dialog::ConfirmDelete { save_id },
    }
}

fn rename(app: &mut DesktopApp, ctx: &egui::Context, save_id: String, mut name: String) {
    let mut outcome: Option<bool> = None;
    let modal = egui::Modal::new(Id::new("rename")).show(ctx, |ui| {
        ui.set_width(320.0);
        ui.heading("Rename save");
        ui.add_space(10.0);
        let edit = ui.add(
            egui::TextEdit::singleline(&mut name)
                .char_limit(crate::saves::NAME_MAX_CHARS)
                .desired_width(f32::INFINITY),
        );
        if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            outcome = Some(true);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let ready = !name.trim().is_empty();
            if ui
                .add_enabled(ready, egui::Button::new(RichText::new("Rename").strong()))
                .clicked()
            {
                outcome = Some(true);
            }
            if ui.button("Cancel").clicked() {
                outcome = Some(false);
            }
        });
    });
    if modal.should_close() {
        outcome = Some(false);
    }
    match outcome {
        Some(true) => {
            if let Some(mut slot) = app.library.get(&save_id) {
                slot.meta.name = name.trim().to_owned();
                if let Err(why) = slot.write_meta() {
                    app.toast(ToastKind::Error, format!("Could not rename: {why}"));
                }
                app.refresh_saves();
            }
            app.dialog = Dialog::None;
        }
        Some(false) => app.dialog = Dialog::None,
        None => app.dialog = Dialog::Rename { save_id, name },
    }
}

/// A small confirmation modal. `Some(true)` = confirmed, `Some(false)` =
/// cancelled, `None` = still open.
fn confirm_modal(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    body: &str,
    confirm_label: &str,
) -> Option<bool> {
    let mut outcome: Option<bool> = None;
    let modal = egui::Modal::new(Id::new(id.to_owned())).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.heading(title);
        ui.add_space(8.0);
        ui.label(RichText::new(body).color(theme::TEXT_DIM));
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            let danger =
                egui::Button::new(RichText::new(confirm_label).color(Color32::WHITE).strong())
                    .fill(theme::BAD);
            if ui.add(danger).clicked() {
                outcome = Some(true);
            }
            if ui.button("Cancel").clicked() {
                outcome = Some(false);
            }
        });
    });
    if outcome.is_none() && modal.should_close() {
        outcome = Some(false);
    }
    outcome
}
