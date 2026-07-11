//! The save library: the screen the app opens on. A first launch guides
//! straight into picking a character (every character ships with the
//! recommended 1.09.03 firmware built in); afterwards it's a gallery of
//! saves with live thumbnails, most recent first.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, TextureHandle,
};

use crate::app::{DesktopApp, Dialog, ToastKind};
use crate::firmware;
use crate::saves::{self, SaveSlot};
use crate::theme;

/// The new-save form, shown as a hero on first launch and as a modal later.
pub struct NewSaveState {
    pub character: Option<&'static str>,
    pub name: String,
    /// Set once the player types their own name; stops us from overwriting
    /// it when they switch characters.
    pub name_edited: bool,
    pub advanced: bool,
    pub version: &'static str,
    pub custom_otp: String,
    pub custom_flash: String,
}

impl Default for NewSaveState {
    fn default() -> Self {
        Self {
            character: None,
            name: String::new(),
            name_edited: false,
            advanced: false,
            version: firmware::RECOMMENDED_VERSION,
            custom_otp: String::new(),
            custom_flash: String::new(),
        }
    }
}

pub enum FormAction {
    None,
    Create,
    Cancel,
}

enum CardAction {
    None,
    Play,
    Rename,
    ShowFiles,
    Delete,
}

pub fn show(app: &mut DesktopApp, root: &mut egui::Ui) {
    let ctx = root.ctx().clone();
    top_bar(app, root);

    let first_run = app.saves.is_empty();

    egui::CentralPanel::default_margins()
        .frame(egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin::same(24)))
        .show(root, |ui| {
            if first_run {
                hero_new_save(app, &ctx, ui);
            } else {
                gallery(app, &ctx, ui);
            }
        });
}

fn top_bar(app: &mut DesktopApp, root: &mut egui::Ui) {
    egui::Panel::top("library_bar")
        .frame(
            egui::Frame::new()
                .fill(theme::PANEL)
                .inner_margin(egui::Margin::symmetric(16, 10)),
        )
        .show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("EMIU2")
                        .color(theme::ACCENT)
                        .strong()
                        .size(17.0),
                );
                ui.label(
                    egui::RichText::new("DESKTOP")
                        .color(theme::TEXT_DIM)
                        .size(17.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Controls").clicked() {
                        app.dialog = Dialog::Controls(crate::ui::dialogs::RemapState::default());
                    }
                });
            });
        });
}

/// First launch: no saves yet, so the character picker IS the screen.
fn hero_new_save(app: &mut DesktopApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    // The state lives in the dialog slot even though it's drawn inline.
    if !matches!(app.dialog, Dialog::NewSave(_)) {
        app.dialog = Dialog::NewSave(NewSaveState::default());
    }

    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.08);
        ui.label(
            egui::RichText::new("Pick your Miuchiz")
                .heading()
                .color(theme::TEXT)
                .size(28.0),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Choose a character to start. Your game saves itself while you play.",
            )
            .color(theme::TEXT_DIM),
        );
        ui.add_space(20.0);
    });

    let mut action = FormAction::None;
    let Dialog::NewSave(state) = &mut app.dialog else {
        return;
    };
    ui.vertical_centered(|ui| {
        ui.set_max_width(560.0);
        action = new_save_form(ui, state, false);
    });
    apply_form_action(app, ctx, action);
}

fn gallery(app: &mut DesktopApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let now = saves::unix_now();
    let mut play: Option<SaveSlot> = None;
    let mut open_dialog: Option<Dialog> = None;
    let mut show_files: Option<std::path::PathBuf> = None;
    let mut want_new_save = false;

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Your saves")
                    .heading()
                    .color(theme::TEXT),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(egui::RichText::new("＋  New save").color(theme::ACCENT))
                    .clicked()
                {
                    want_new_save = true;
                }
            });
        });
        ui.add_space(12.0);

        // Make sure thumbnails are loaded (once per refresh).
        let ids: Vec<String> = app.saves.iter().map(|s| s.id.clone()).collect();
        for slot in &app.saves {
            if !app.thumbs.contains_key(&slot.id) {
                let texture = load_thumb(ctx, slot);
                app.thumbs.insert(slot.id.clone(), texture);
            }
        }
        app.thumbs.retain(|id, _| ids.contains(id));

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(16.0, 16.0);
            for slot in &app.saves {
                let thumb = app.thumbs.get(&slot.id).and_then(|t| t.as_ref());
                match save_card(ui, slot, thumb, now) {
                    CardAction::None => {}
                    CardAction::Play => play = Some(slot.clone()),
                    CardAction::Rename => {
                        open_dialog = Some(Dialog::Rename {
                            save_id: slot.id.clone(),
                            name: slot.meta.name.clone(),
                        });
                    }
                    CardAction::ShowFiles => show_files = Some(slot.dir.clone()),
                    CardAction::Delete => {
                        open_dialog = Some(Dialog::ConfirmDelete {
                            save_id: slot.id.clone(),
                        });
                    }
                }
            }
            if new_save_tile(ui) {
                want_new_save = true;
            }
        });
    });

    if want_new_save {
        open_dialog = Some(Dialog::NewSave(NewSaveState::default()));
    }
    if let Some(dialog) = open_dialog {
        app.dialog = dialog;
    }
    if let Some(dir) = show_files {
        open_in_file_manager(app, &dir);
    }
    if let Some(slot) = play {
        app.start_session(ctx, slot);
    }
}

const CARD_W: f32 = 236.0;
const THUMB_W: f32 = 212.0;
const THUMB_H: f32 = 145.0; // 212 * 67/98 ≈ 145: the LCD's own shape

fn save_card(
    ui: &mut egui::Ui,
    slot: &SaveSlot,
    thumb: Option<&TextureHandle>,
    now: u64,
) -> CardAction {
    let mut action = CardAction::None;
    let card_h = THUMB_H + 64.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(CARD_W, card_h), Sense::click());

    let hovered = response.hovered();
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(10),
        if hovered { theme::CARD_HOVER } else { theme::CARD },
        Stroke::new(1.0, if hovered { theme::ACCENT_DIM } else { theme::OUTLINE }),
        StrokeKind::Inside,
    );

    // Thumbnail (or a character-colored placeholder).
    let thumb_rect = Rect::from_min_size(
        rect.min + egui::vec2(12.0, 12.0),
        egui::vec2(THUMB_W, THUMB_H),
    );
    match thumb {
        Some(texture) => {
            painter.rect_filled(thumb_rect, CornerRadius::same(6), Color32::BLACK);
            painter.image(
                texture.id(),
                thumb_rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            let color = theme::character_color(&slot.meta.character);
            painter.rect_filled(
                thumb_rect,
                CornerRadius::same(6),
                Color32::from_rgb(
                    (color.r() as f32 * 0.25) as u8,
                    (color.g() as f32 * 0.25) as u8,
                    (color.b() as f32 * 0.25) as u8,
                ),
            );
            let initial = slot.meta.character.chars().next().unwrap_or('?');
            painter.text(
                thumb_rect.center(),
                Align2::CENTER_CENTER,
                initial,
                FontId::proportional(44.0),
                color,
            );
        }
    }

    // Hovering suggests playing.
    if hovered {
        painter.rect_filled(
            thumb_rect,
            CornerRadius::same(6),
            Color32::from_black_alpha(90),
        );
        painter.text(
            thumb_rect.center(),
            Align2::CENTER_CENTER,
            "▶",
            FontId::proportional(30.0),
            Color32::from_white_alpha(230),
        );
    }

    // Name and details.
    let text_x = rect.min.x + 14.0;
    painter.text(
        egui::pos2(text_x, thumb_rect.bottom() + 10.0),
        Align2::LEFT_TOP,
        &slot.meta.name,
        FontId::proportional(15.0),
        theme::TEXT,
    );
    let detail = format!(
        "{} · {} · {}",
        slot.meta.character,
        theme::playtime(slot.meta.play_seconds),
        theme::ago(now, slot.meta.last_played_unix),
    );
    painter.text(
        egui::pos2(text_x, thumb_rect.bottom() + 31.0),
        Align2::LEFT_TOP,
        detail,
        FontId::proportional(11.5),
        theme::TEXT_DIM,
    );

    // The ⋯ button, quiet until the card is hovered.
    let more_rect = Rect::from_center_size(
        egui::pos2(rect.right() - 20.0, thumb_rect.bottom() + 26.0),
        egui::vec2(24.0, 24.0),
    );
    let more = ui.interact(more_rect, response.id.with("more"), Sense::click());
    if hovered || more.hovered() {
        ui.painter().text(
            more_rect.center(),
            Align2::CENTER_CENTER,
            "⋯",
            FontId::proportional(18.0),
            if more.hovered() { theme::TEXT } else { theme::TEXT_DIM },
        );
    }

    let menu_contents = |ui: &mut egui::Ui, action: &mut CardAction| {
        if ui.button("Rename…").clicked() {
            *action = CardAction::Rename;
        }
        if ui.button("Show files").clicked() {
            *action = CardAction::ShowFiles;
        }
        ui.separator();
        if ui
            .button(egui::RichText::new("Delete…").color(theme::BAD))
            .clicked()
        {
            *action = CardAction::Delete;
        }
    };
    egui::Popup::menu(&more).show(|ui| menu_contents(ui, &mut action));
    response.context_menu(|ui| menu_contents(ui, &mut action));

    if response.clicked() && matches!(action, CardAction::None) {
        action = CardAction::Play;
    }
    action
}

/// The trailing "start another save" tile.
fn new_save_tile(ui: &mut egui::Ui) -> bool {
    let card_h = THUMB_H + 64.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(CARD_W, card_h), Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(10),
        if hovered {
            theme::CARD
        } else {
            Color32::TRANSPARENT
        },
        Stroke::new(1.0, if hovered { theme::ACCENT_DIM } else { theme::OUTLINE }),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center() - egui::vec2(0.0, 12.0),
        Align2::CENTER_CENTER,
        "＋",
        FontId::proportional(30.0),
        if hovered { theme::ACCENT } else { theme::TEXT_FAINT },
    );
    painter.text(
        rect.center() + egui::vec2(0.0, 18.0),
        Align2::CENTER_CENTER,
        "New save",
        FontId::proportional(13.0),
        if hovered { theme::TEXT } else { theme::TEXT_DIM },
    );
    response.clicked()
}

fn load_thumb(ctx: &egui::Context, slot: &SaveSlot) -> Option<TextureHandle> {
    let path = slot.thumb_path()?;
    let bytes = std::fs::read(path).ok()?;
    let image = image::load_from_memory(&bytes).ok()?.to_rgb8();
    let size = [image.width() as usize, image.height() as usize];
    let color_image = egui::ColorImage::from_rgb(size, image.as_raw());
    Some(ctx.load_texture(
        format!("thumb-{}", slot.id),
        color_image,
        egui::TextureOptions::NEAREST,
    ))
}

/// The shared new-save form (hero and modal). Returns what the player chose.
pub fn new_save_form(
    ui: &mut egui::Ui,
    state: &mut NewSaveState,
    show_cancel: bool,
) -> FormAction {
    let mut action = FormAction::None;

    // Character picker: a row of colored tiles.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
        for &character in firmware::CHARACTERS {
            if character_tile(ui, character, state.character == Some(character)) {
                state.character = Some(character);
                if !state.name_edited {
                    state.name = character.to_owned();
                }
                if firmware::find(character, state.version).is_none() {
                    state.version = firmware::RECOMMENDED_VERSION;
                }
            }
        }
    });

    ui.add_space(14.0);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Name").color(theme::TEXT_DIM));
        let edit = egui::TextEdit::singleline(&mut state.name)
            .hint_text("Name this save")
            .desired_width(220.0);
        if ui.add(edit).changed() {
            state.name_edited = !state.name.trim().is_empty();
        }
    });

    ui.add_space(16.0);

    let has_custom_flash = !state.custom_flash.trim().is_empty();
    let ready = (state.character.is_some() || has_custom_flash)
        && !state.name.trim().is_empty();

    ui.horizontal(|ui| {
        let create = egui::Button::new(
            egui::RichText::new("Create & Play")
                .color(Color32::from_rgb(0x1d, 0x12, 0x05))
                .strong(),
        )
        .fill(theme::ACCENT)
        .min_size(egui::vec2(150.0, 34.0));
        if ui.add_enabled(ready, create).clicked() {
            action = FormAction::Create;
        }
        if show_cancel && ui.button("Cancel").clicked() {
            action = FormAction::Cancel;
        }
    });

    if !ready {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(if state.character.is_none() {
                "Pick a character above."
            } else {
                "Give the save a name."
            })
            .small()
            .color(theme::TEXT_FAINT),
        );
    }

    // Deliberately quiet: nothing here matters to a player. Firmware
    // version choice and custom image files live behind this link.
    ui.add_space(18.0);
    let advanced_label = egui::RichText::new(if state.advanced {
        "Hide advanced options"
    } else {
        "Advanced options"
    })
    .small()
    .color(theme::TEXT_FAINT);
    if ui
        .add(egui::Label::new(advanced_label).sense(Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        state.advanced = !state.advanced;
    }

    if state.advanced {
        ui.add_space(6.0);
        egui::Frame::new()
            .fill(theme::BG)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                if let Some(character) = state.character {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Firmware")
                                .small()
                                .color(theme::TEXT_DIM),
                        );
                        egui::ComboBox::from_id_salt("fw_version")
                            .selected_text(state.version)
                            .show_ui(ui, |ui| {
                                for version in firmware::versions_for(character) {
                                    let label = if version == firmware::RECOMMENDED_VERSION {
                                        format!("{version} (recommended)")
                                    } else {
                                        version.to_owned()
                                    };
                                    ui.selectable_value(&mut state.version, version, label);
                                }
                            });
                    });
                    ui.add_space(6.0);
                }
                ui.label(
                    egui::RichText::new("Custom images (developers)")
                        .small()
                        .color(theme::TEXT_DIM),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut state.custom_otp)
                        .hint_text("OTP image path (blank = built-in)")
                        .desired_width(f32::INFINITY),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut state.custom_flash)
                        .hint_text("Flash image path (blank = built-in firmware)")
                        .desired_width(f32::INFINITY),
                );
            });
    }

    action
}

fn character_tile(ui: &mut egui::Ui, character: &str, selected: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(72.0, 78.0), Sense::click());
    let hovered = response.hovered();
    let color = theme::character_color(character);
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(10),
        if selected {
            theme::CARD_HOVER
        } else if hovered {
            theme::CARD
        } else {
            Color32::TRANSPARENT
        },
        Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected {
                color
            } else if hovered {
                theme::ACCENT_DIM
            } else {
                theme::OUTLINE
            },
        ),
        StrokeKind::Inside,
    );
    // The character "portrait": a filled disc with the initial.
    let disc_center = rect.center() - egui::vec2(0.0, 12.0);
    painter.circle_filled(disc_center, 17.0, color.gamma_multiply(0.28));
    painter.circle(disc_center, 17.0, Color32::TRANSPARENT, Stroke::new(1.5, color));
    painter.text(
        disc_center,
        Align2::CENTER_CENTER,
        character.chars().next().unwrap_or('?'),
        FontId::proportional(16.0),
        color,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 12.0),
        Align2::CENTER_CENTER,
        character,
        FontId::proportional(11.5),
        if selected { theme::TEXT } else { theme::TEXT_DIM },
    );
    response.clicked()
}

/// Executes the new-save form's outcome (used by both hero and modal).
pub fn apply_form_action(app: &mut DesktopApp, ctx: &egui::Context, action: FormAction) {
    match action {
        FormAction::None => {}
        FormAction::Cancel => app.dialog = Dialog::None,
        FormAction::Create => {
            let Dialog::NewSave(state) =
                std::mem::replace(&mut app.dialog, Dialog::None)
            else {
                return;
            };
            let read_custom = |path: &str| -> Result<Option<Vec<u8>>, String> {
                let path = path.trim();
                if path.is_empty() {
                    return Ok(None);
                }
                std::fs::read(path)
                    .map(Some)
                    .map_err(|why| format!("Could not read {path}: {why}"))
            };
            let custom_otp = match read_custom(&state.custom_otp) {
                Ok(data) => data,
                Err(why) => {
                    app.toast(ToastKind::Error, why);
                    app.dialog = Dialog::NewSave(state);
                    return;
                }
            };
            let custom_flash = match read_custom(&state.custom_flash) {
                Ok(data) => data,
                Err(why) => {
                    app.toast(ToastKind::Error, why);
                    app.dialog = Dialog::NewSave(state);
                    return;
                }
            };
            let character = state.character.unwrap_or("Custom");
            let slot = app.create_save(
                state.name.trim(),
                character,
                Some(state.version),
                custom_otp,
                custom_flash,
            );
            if let Some(slot) = slot {
                app.start_session(ctx, slot);
            } else {
                app.dialog = Dialog::NewSave(state);
            }
        }
    }
}

fn open_in_file_manager(app: &mut DesktopApp, dir: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";

    if let Err(why) = std::process::Command::new(program).arg(dir).spawn() {
        app.toast(
            ToastKind::Error,
            format!("Could not open the file manager: {why}"),
        );
    }
}
