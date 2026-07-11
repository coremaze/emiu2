//! The playing screen: a drawn handheld around the live LCD, plus the menu
//! bar. Button arrangement mirrors the dev frontend (and the real device):
//! D-pad left, Menu/Action right, four soft buttons hugging the screen
//! corners, Power and Mute below.

use std::collections::HashSet;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Key, Rect, Stroke, StrokeKind};
use emiu2::miuchiz::MiuchizGpio;

use crate::app::{DesktopApp, Dialog, PlaySession};
use crate::controls::{gpio_bit, Bindings};
use crate::emu::{EmuCmd, LCD_HEIGHT, LCD_WIDTH};
use crate::theme;
use crate::ui::widgets::{self, ButtonFace};

/// What the menu bar asked for; applied after the panels are drawn so the
/// menu closures don't fight the rest of the app for borrows.
enum MenuAction {
    BackToLibrary,
    SaveNow,
    ConfirmReset { connect_mode: bool },
    ToggleUsb,
    ToggleFullscreen,
    ToggleIntegerScaling,
    OpenControls,
    OpenFriends,
}

pub fn show(app: &mut DesktopApp, root: &mut egui::Ui) {
    let ctx = root.ctx().clone();
    let Some(session) = &mut app.session else {
        return;
    };
    session.refresh_texture();
    session.click_mask = 0;

    // Keys light the drawn buttons only when the game actually receives
    // them (no dialog open, no text field focused).
    let keys_down: Option<HashSet<Key>> = if matches!(app.dialog, Dialog::None)
        && !ctx.egui_wants_keyboard_input()
    {
        Some(ctx.input(|i| i.keys_down.clone()))
    } else {
        None
    };

    if session.fullscreen {
        fullscreen_lcd(root, session, app.config.video.integer_scaling);
        return;
    }

    let mut actions: Vec<MenuAction> = Vec::new();
    menu_bar(root, app, &mut actions);

    // Re-borrow: `menu_bar` needed `app` itself.
    let Some(session) = &mut app.session else {
        return;
    };
    egui::CentralPanel::default_margins()
        .frame(egui::Frame::new().fill(theme::BG))
        .show(root, |ui| {
            device_panel(
                ui,
                session,
                &app.bindings,
                keys_down.as_ref(),
                app.config.video.integer_scaling,
            );
        });

    for action in actions {
        match action {
            MenuAction::BackToLibrary => app.close_session(&ctx),
            MenuAction::SaveNow => {
                if let Some(session) = &app.session {
                    session.emu.send(EmuCmd::SaveNow);
                }
            }
            MenuAction::ConfirmReset { connect_mode } => {
                app.dialog = Dialog::ConfirmReset { connect_mode };
            }
            MenuAction::ToggleUsb => {
                if let Some(session) = &app.session {
                    let plugged = !session.emu.cable.plugged();
                    session.emu.cable.set_plugged(plugged);
                    app.config.usb.plugged = plugged;
                    app.save_config();
                }
            }
            MenuAction::ToggleFullscreen => {
                let on = app.session.as_ref().is_some_and(|s| !s.fullscreen);
                app.set_fullscreen(&ctx, on);
            }
            MenuAction::ToggleIntegerScaling => {
                app.config.video.integer_scaling = !app.config.video.integer_scaling;
                app.save_config();
            }
            MenuAction::OpenControls => {
                app.dialog = Dialog::Controls(crate::ui::dialogs::RemapState::default());
            }
            MenuAction::OpenFriends => {
                app.dialog = Dialog::Friends(crate::ui::dialogs::FriendsState::default());
            }
        }
    }
}

fn menu_bar(root: &mut egui::Ui, app: &mut DesktopApp, actions: &mut Vec<MenuAction>) {
    let Some(session) = &app.session else {
        return;
    };
    let usb_plugged = session.emu.cable.plugged();
    let ir_paired = app.relay.as_ref().is_some_and(|r| r.paired());
    let saved_recently = session
        .last_saved
        .is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(2500));
    let save_name = session.slot.meta.name.clone();
    let character_color = theme::character_color(&session.slot.meta.character);

    egui::Panel::top("menu_bar")
        .frame(
            egui::Frame::new()
                .fill(theme::PANEL)
                .inner_margin(egui::Margin::symmetric(8, 5)),
        )
        .show(root, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                if ui.button("‹ Saves").on_hover_text("Save and return to your saves").clicked() {
                    actions.push(MenuAction::BackToLibrary);
                }

                ui.menu_button("Device", |ui| {
                    if ui.button("Save now").clicked() {
                        actions.push(MenuAction::SaveNow);
                    }
                    ui.separator();
                    if ui
                        .checkbox(&mut { usb_plugged }, "USB cable plugged in")
                        .on_hover_text("Host tools can only talk to the device while plugged")
                        .clicked()
                    {
                        actions.push(MenuAction::ToggleUsb);
                    }
                    ui.separator();
                    if ui.button("Restart device…").clicked() {
                        actions.push(MenuAction::ConfirmReset {
                            connect_mode: false,
                        });
                    }
                    if ui
                        .button("Restart to PC connection…")
                        .on_hover_text(
                            "Boot straight into \"Please Connect to PC\" mode \
                             so USB tools can manage the device",
                        )
                        .clicked()
                    {
                        actions.push(MenuAction::ConfirmReset { connect_mode: true });
                    }
                });

                ui.menu_button("View", |ui| {
                    if ui
                        .checkbox(&mut { false }, "Fullscreen screen")
                        .on_hover_text("F11 · shows only the device screen")
                        .clicked()
                    {
                        actions.push(MenuAction::ToggleFullscreen);
                    }
                    if ui
                        .checkbox(
                            &mut { app.config.video.integer_scaling },
                            "Integer scaling",
                        )
                        .on_hover_text("Scale the screen only by whole pixels, keeping it razor sharp")
                        .clicked()
                    {
                        actions.push(MenuAction::ToggleIntegerScaling);
                    }
                });

                if ui.button("Controls").clicked() {
                    actions.push(MenuAction::OpenControls);
                }
                if ui.button("Friends").clicked() {
                    actions.push(MenuAction::OpenFriends);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::status_chip(ui, "USB", usb_plugged, theme::GOOD)
                        .on_hover_text(if usb_plugged {
                            "USB cable plugged in — click to unplug"
                        } else {
                            "USB cable unplugged — click to plug in"
                        })
                        .clicked()
                    {
                        actions.push(MenuAction::ToggleUsb);
                    }
                    if widgets::status_chip(ui, "IR", ir_paired, theme::GOOD)
                        .on_hover_text(if ir_paired {
                            "Paired with a friend"
                        } else {
                            "Not paired — click to open Friends"
                        })
                        .clicked()
                    {
                        actions.push(MenuAction::OpenFriends);
                    }
                    if saved_recently {
                        ui.label(
                            egui::RichText::new("Saved ✓")
                                .color(theme::GOOD)
                                .size(12.0),
                        );
                    }

                    // The save's name, pushed to the far left of this
                    // right-to-left region so it reads as a title.
                    ui.with_layout(
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.add_space(12.0);
                            let (dot, _) = ui.allocate_exact_size(
                                egui::vec2(8.0, 8.0),
                                egui::Sense::hover(),
                            );
                            ui.painter()
                                .circle_filled(dot.center(), 4.0, character_color);
                            ui.label(
                                egui::RichText::new(&save_name).color(theme::TEXT_DIM),
                            );
                        },
                    );
                });
            });
        });
}

/// Picks the LCD scale for the space available: whole multiples when
/// integer scaling is on, otherwise the largest aspect-true fit. Never
/// stretches or distorts.
fn lcd_scale(avail_w: f32, avail_h: f32, integer: bool) -> f32 {
    let fit = (avail_w / LCD_WIDTH as f32).min(avail_h / LCD_HEIGHT as f32);
    if integer {
        fit.floor().max(1.0)
    } else {
        fit.max(0.5)
    }
}

fn fullscreen_lcd(root: &mut egui::Ui, session: &mut PlaySession, integer: bool) {
    egui::CentralPanel::default_margins()
        .frame(egui::Frame::new().fill(Color32::BLACK))
        .show(root, |ui| {
            let avail = ui.available_rect_before_wrap();
            let scale = lcd_scale(avail.width(), avail.height(), integer);
            let size = egui::vec2(LCD_WIDTH as f32 * scale, LCD_HEIGHT as f32 * scale);
            let glass = Rect::from_center_size(avail.center(), size);
            ui.painter().image(
                session.texture.id(),
                glass,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );

            // A quiet exit hint while the pointer is moving.
            if ui.input(|i| i.pointer.is_moving()) {
                ui.painter().text(
                    egui::pos2(avail.center().x, avail.bottom() - 24.0),
                    Align2::CENTER_BOTTOM,
                    "Esc to leave fullscreen",
                    FontId::proportional(13.0),
                    Color32::from_white_alpha(140),
                );
            }
        });
}

/// One drawn button: interaction, key mirroring, click-mask contribution.
struct ButtonSpec {
    gpio: MiuchizGpio,
    label: &'static str,
    rect: Rect,
    face: ButtonFace,
    glyph: &'static str,
    glyph_size: f32,
}

fn device_panel(
    ui: &mut egui::Ui,
    session: &mut PlaySession,
    bindings: &Bindings,
    keys_down: Option<&HashSet<Key>>,
    integer_scaling: bool,
) {
    const SIDE_W: f32 = 150.0;
    const TOP_H: f32 = 22.0;
    const BOTTOM_H: f32 = 66.0;
    const BEZEL: f32 = 12.0;
    const OUTER_PAD: f32 = 14.0;

    let avail = ui.available_rect_before_wrap();
    let scale = lcd_scale(
        avail.width() - 2.0 * (SIDE_W + BEZEL + OUTER_PAD),
        avail.height() - TOP_H - BOTTOM_H - 2.0 * (BEZEL + OUTER_PAD),
        integer_scaling,
    );
    let glass_size = egui::vec2(LCD_WIDTH as f32 * scale, LCD_HEIGHT as f32 * scale);

    let shell_size = egui::vec2(
        glass_size.x + 2.0 * (BEZEL + SIDE_W),
        glass_size.y + 2.0 * BEZEL + TOP_H + BOTTOM_H,
    );
    let shell = Rect::from_center_size(avail.center(), shell_size);
    let glass = Rect::from_center_size(
        egui::pos2(
            shell.center().x,
            shell.top() + TOP_H + BEZEL + glass_size.y / 2.0,
        ),
        glass_size,
    );
    let bezel_rect = glass.expand(BEZEL);

    // The shell: one rounded slab of plastic.
    ui.painter().rect(
        shell,
        CornerRadius::same(26),
        theme::SHELL,
        Stroke::new(1.5, theme::SHELL_EDGE),
        StrokeKind::Inside,
    );

    widgets::lcd(ui, glass, &session.texture);

    // D-pad, centered on the left side.
    let dpad_center = egui::pos2(bezel_rect.left() - 84.0, glass.center().y);
    let arm = egui::vec2(30.0, 30.0);
    let reach = 31.0;
    // The hub behind the arms.
    ui.painter()
        .circle_filled(dpad_center, 24.0, theme::BUTTON);

    let right_x = bezel_rect.right() + 84.0;
    let specs = [
        ButtonSpec {
            gpio: MiuchizGpio::Up,
            label: "D-pad up",
            rect: Rect::from_center_size(dpad_center - egui::vec2(0.0, reach), arm),
            face: ButtonFace::Rounded(8.0),
            glyph: "⏶",
            glyph_size: 13.0,
        },
        ButtonSpec {
            gpio: MiuchizGpio::Down,
            label: "D-pad down",
            rect: Rect::from_center_size(dpad_center + egui::vec2(0.0, reach), arm),
            face: ButtonFace::Rounded(8.0),
            glyph: "⏷",
            glyph_size: 13.0,
        },
        ButtonSpec {
            gpio: MiuchizGpio::Left,
            label: "D-pad left",
            rect: Rect::from_center_size(dpad_center - egui::vec2(reach, 0.0), arm),
            face: ButtonFace::Rounded(8.0),
            glyph: "⏴",
            glyph_size: 13.0,
        },
        ButtonSpec {
            gpio: MiuchizGpio::Right,
            label: "D-pad right",
            rect: Rect::from_center_size(dpad_center + egui::vec2(reach, 0.0), arm),
            face: ButtonFace::Rounded(8.0),
            glyph: "⏵",
            glyph_size: 13.0,
        },
        // The four soft buttons that hug the screen corners.
        ButtonSpec {
            gpio: MiuchizGpio::ScreenTopLeft,
            label: "Screen top-left",
            rect: Rect::from_center_size(
                egui::pos2(bezel_rect.left() - 17.0, glass.top() + 9.0),
                egui::vec2(22.0, 22.0),
            ),
            face: ButtonFace::Circle,
            glyph: "",
            glyph_size: 10.0,
        },
        ButtonSpec {
            gpio: MiuchizGpio::ScreenBottomLeft,
            label: "Screen bottom-left",
            rect: Rect::from_center_size(
                egui::pos2(bezel_rect.left() - 17.0, glass.bottom() - 9.0),
                egui::vec2(22.0, 22.0),
            ),
            face: ButtonFace::Circle,
            glyph: "",
            glyph_size: 10.0,
        },
        ButtonSpec {
            gpio: MiuchizGpio::ScreenTopRight,
            label: "Screen top-right",
            rect: Rect::from_center_size(
                egui::pos2(bezel_rect.right() + 17.0, glass.top() + 9.0),
                egui::vec2(22.0, 22.0),
            ),
            face: ButtonFace::Circle,
            glyph: "",
            glyph_size: 10.0,
        },
        ButtonSpec {
            gpio: MiuchizGpio::ScreenBottomRight,
            label: "Screen bottom-right",
            rect: Rect::from_center_size(
                egui::pos2(bezel_rect.right() + 17.0, glass.bottom() - 9.0),
                egui::vec2(22.0, 22.0),
            ),
            face: ButtonFace::Circle,
            glyph: "",
            glyph_size: 10.0,
        },
        ButtonSpec {
            gpio: MiuchizGpio::Menu,
            label: "Menu",
            rect: Rect::from_center_size(
                egui::pos2(right_x, glass.top() + 13.0),
                egui::vec2(58.0, 26.0),
            ),
            face: ButtonFace::Rounded(13.0),
            glyph: "MENU",
            glyph_size: 10.5,
        },
        ButtonSpec {
            gpio: MiuchizGpio::Action,
            label: "Action",
            rect: Rect::from_center_size(
                egui::pos2(right_x, glass.center().y + 16.0),
                egui::vec2(54.0, 54.0),
            ),
            face: ButtonFace::Circle,
            glyph: "A",
            glyph_size: 17.0,
        },
        ButtonSpec {
            gpio: MiuchizGpio::Power,
            label: "Power",
            rect: Rect::from_center_size(
                egui::pos2(glass.center().x - 55.0, bezel_rect.bottom() + BOTTOM_H / 2.0),
                egui::vec2(72.0, 26.0),
            ),
            face: ButtonFace::Rounded(13.0),
            glyph: "POWER",
            glyph_size: 10.5,
        },
        ButtonSpec {
            gpio: MiuchizGpio::Mute,
            label: "Mute",
            rect: Rect::from_center_size(
                egui::pos2(glass.center().x + 55.0, bezel_rect.bottom() + BOTTOM_H / 2.0),
                egui::vec2(72.0, 26.0),
            ),
            face: ButtonFace::Rounded(13.0),
            glyph: "MUTE",
            glyph_size: 10.5,
        },
    ];

    for spec in specs {
        let key = bindings.key_for(spec.gpio);
        let key_held = match (keys_down, key) {
            (Some(down), Some(key)) => down.contains(&key),
            _ => false,
        };
        let (response, mouse_held) = widgets::device_button(
            ui,
            spec.rect,
            spec.label,
            spec.face,
            spec.glyph,
            spec.glyph_size,
            key_held,
        );
        if mouse_held {
            session.click_mask |= gpio_bit(spec.gpio);
        }
        let hint = match key {
            Some(key) => format!("{} — key {}", spec.label, key.name()),
            None => format!("{} — no key bound", spec.label),
        };
        response.on_hover_text(hint);
    }
}
