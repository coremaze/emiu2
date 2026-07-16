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
    /// Resize the window so the LCD lands exactly on this integer scale.
    SetWindowScale(u32),
    OpenControls,
    OpenFriends,
}

// The drawn handheld's fixed chrome, shared by the layout and by the
// window-size presets so a preset always lands the LCD on a whole scale.
const SIDE_W: f32 = 150.0;
const TOP_H: f32 = 26.0;
const BOTTOM_H: f32 = 66.0;
const BEZEL: f32 = 12.0;
const OUTER_PAD: f32 = 14.0;
/// The menu bar's height (fill + margins + text), for the presets.
const MENU_BAR_H: f32 = 30.0;

/// The window inner size that gives the LCD exactly `scale`x pixels.
fn window_size_for_scale(scale: u32) -> egui::Vec2 {
    let glass = egui::vec2(
        LCD_WIDTH as f32 * scale as f32,
        LCD_HEIGHT as f32 * scale as f32,
    );
    egui::vec2(
        glass.x + 2.0 * (BEZEL + SIDE_W + OUTER_PAD),
        glass.y + 2.0 * (BEZEL + OUTER_PAD) + TOP_H + BOTTOM_H + MENU_BAR_H,
    )
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
        fullscreen_lcd(root, session, &app.bindings, app.config.video.integer_scaling);
        return;
    }

    let full = root.max_rect();
    crate::ui::backdrop::paint(root, full);

    let mut actions: Vec<MenuAction> = Vec::new();
    menu_bar(root, app, &mut actions);

    // Re-borrow: `menu_bar` needed `app` itself.
    let Some(session) = &mut app.session else {
        return;
    };
    egui::CentralPanel::default_margins()
        .frame(egui::Frame::new().fill(Color32::TRANSPARENT))
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
            MenuAction::SetWindowScale(scale) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                    window_size_for_scale(scale),
                ));
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
                .stroke(Stroke::new(1.0, theme::OUTLINE))
                .inner_margin(egui::Margin::symmetric(8, 5)),
        )
        .show(root, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                if ui.button("‹ Saves").on_hover_text("Save and return to your saves").clicked() {
                    actions.push(MenuAction::BackToLibrary);
                }

                ui.menu_button("Device", |ui| {
                    if ui
                        .add(egui::Button::new("Save now").shortcut_text("Ctrl+S"))
                        .clicked()
                    {
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
                        .add(
                            egui::Button::new("Fullscreen screen")
                                .shortcut_text("F11"),
                        )
                        .on_hover_text("Show only the device screen")
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
                    ui.separator();
                    ui.menu_button("Screen size", |ui| {
                        for scale in 3..=8u32 {
                            if ui
                                .button(format!("{scale}×  ({}×{})",
                                    LCD_WIDTH as u32 * scale,
                                    LCD_HEIGHT as u32 * scale))
                                .clicked()
                            {
                                actions.push(MenuAction::SetWindowScale(scale));
                            }
                        }
                    });
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

fn fullscreen_lcd(
    root: &mut egui::Ui,
    session: &mut PlaySession,
    bindings: &Bindings,
    integer: bool,
) {
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
            if session.looks_asleep() {
                asleep_hint(ui, glass, bindings);
            }

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

/// The device sleeps like the real one; without this, resuming a sleeping
/// save is indistinguishable from a hang.
fn asleep_hint(ui: &egui::Ui, glass: Rect, bindings: &Bindings) {
    let key = bindings
        .key_for(MiuchizGpio::Power)
        .map(|k| format!(" ({})", k.name()))
        .unwrap_or_default();
    ui.painter().text(
        glass.center(),
        Align2::CENTER_CENTER,
        format!("zZz   asleep — press POWER{key} to wake"),
        FontId::proportional(13.0),
        Color32::from_white_alpha(90),
    );
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

    // The shell: a frosted translucent white slab, lifted off the ice with
    // a cool drop shadow and a faint icy halo. Just translucent enough that
    // the prismatic shafts ghost through the plastic.
    let cr = CornerRadius::same(30);
    ui.painter()
        .add(egui::Shape::from(theme::fx::shell().as_shape(shell, cr)));
    ui.painter()
        .add(egui::Shape::from(theme::fx::glow().as_shape(shell, cr)));
    ui.painter().add(egui::Shape::mesh(theme::rounded_vgrad_mesh(
        shell,
        cr,
        theme::with_alpha(Color32::WHITE, 214),
        theme::with_alpha(Color32::from_rgb(0xdb, 0xec, 0xf8), 182),
    )));
    // A faint ground-glass dither over the slab.
    theme::dither(ui.painter(), shell, 110, 2.0, 26);
    // Chrome hairline rim, with a white inner hairline: polished edge work.
    ui.painter().rect_stroke(
        shell,
        cr,
        Stroke::new(1.2, theme::SHELL_EDGE),
        StrokeKind::Inside,
    );
    ui.painter().rect_stroke(
        shell.shrink(2.0),
        CornerRadius::same(28),
        Stroke::new(1.0, theme::with_alpha(Color32::WHITE, 170)),
        StrokeKind::Inside,
    );
    // The bright inner top-edge highlight that sells the frosted glass.
    theme::frost_top_edge(ui.painter(), shell, cr, 240);

    // The brand, printed above the screen like on the real shell.
    ui.painter().text(
        egui::pos2(shell.center().x, shell.top() + TOP_H / 2.0 + 6.0),
        Align2::CENTER_CENTER,
        "miuchiz",
        FontId::new(13.0, theme::display_family()),
        theme::TEXT_DIM,
    );

    widgets::lcd(ui, glass, &session.texture);
    if session.looks_asleep() {
        asleep_hint(ui, glass, bindings);
    }

    // D-pad, centered on the left side.
    let dpad_center = egui::pos2(bezel_rect.left() - SIDE_W / 2.0 - 8.0, glass.center().y);
    let arm = egui::vec2(34.0, 34.0);
    let reach = 34.0;
    // A recessed well molded into the white plastic behind the D-pad arms.
    ui.painter().circle_filled(
        dpad_center,
        54.0,
        theme::with_alpha(Color32::from_rgb(0x93, 0xb6, 0xd0), 70),
    );
    ui.painter().circle_stroke(
        dpad_center,
        54.0,
        Stroke::new(1.0, theme::SHELL_EDGE),
    );
    ui.painter()
        .circle_filled(dpad_center, 24.0, theme::BUTTON);
    ui.painter().circle_stroke(
        dpad_center,
        24.0,
        Stroke::new(1.0, theme::SHELL_EDGE),
    );

    let right_x = bezel_rect.right() + SIDE_W / 2.0 + 8.0;
    // A recessed molded ring around the Action button, like the rim on the
    // real shell.
    ui.painter().circle(
        egui::pos2(right_x, glass.center().y + 16.0),
        36.0,
        theme::with_alpha(Color32::from_rgb(0x93, 0xb6, 0xd0), 60),
        Stroke::new(1.0, theme::SHELL_EDGE),
    );

    let specs = [
        ButtonSpec {
            gpio: MiuchizGpio::Up,
            label: "D-pad up",
            rect: Rect::from_center_size(dpad_center - egui::vec2(0.0, reach), arm),
            face: ButtonFace::Rounded(8.0),
            glyph: "⏶",
            glyph_size: 15.0,
        },
        ButtonSpec {
            gpio: MiuchizGpio::Down,
            label: "D-pad down",
            rect: Rect::from_center_size(dpad_center + egui::vec2(0.0, reach), arm),
            face: ButtonFace::Rounded(8.0),
            glyph: "⏷",
            glyph_size: 15.0,
        },
        ButtonSpec {
            gpio: MiuchizGpio::Left,
            label: "D-pad left",
            rect: Rect::from_center_size(dpad_center - egui::vec2(reach, 0.0), arm),
            face: ButtonFace::Rounded(8.0),
            glyph: "⏴",
            glyph_size: 15.0,
        },
        ButtonSpec {
            gpio: MiuchizGpio::Right,
            label: "D-pad right",
            rect: Rect::from_center_size(dpad_center + egui::vec2(reach, 0.0), arm),
            face: ButtonFace::Rounded(8.0),
            glyph: "⏵",
            glyph_size: 15.0,
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
