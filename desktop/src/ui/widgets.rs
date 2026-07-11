//! Custom-drawn pieces shared by the screens: the device's physical
//! buttons and the LCD glass. egui's stock widgets are for the chrome;
//! the drawn handheld needs its own look.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui,
};

use crate::theme;

/// How a device button is drawn.
pub enum ButtonFace {
    /// A round button with a text glyph (Action, screen corners).
    Circle,
    /// A rounded rectangle (D-pad arms, Menu, Power, Mute).
    Rounded(f32),
}

/// One press-and-hold button of the drawn handheld. Returns `(response,
/// mouse_held)`; the button lights up when either the mouse holds it or
/// `key_held` says its mapped key is down, so the on-screen device always
/// mirrors what the machine feels.
pub fn device_button(
    ui: &mut Ui,
    rect: Rect,
    id_salt: &str,
    face: ButtonFace,
    glyph: &str,
    glyph_size: f32,
    key_held: bool,
) -> (Response, bool) {
    let response = ui.interact(rect, ui.id().with(id_salt), Sense::drag());
    let mouse_held = response.is_pointer_button_down_on();
    let pressed = mouse_held || key_held;

    // A pressed button sits 1px lower: cheap, convincing travel.
    let draw_rect = if pressed {
        rect.translate(egui::vec2(0.0, 1.0))
    } else {
        rect
    };

    let fill = if pressed {
        theme::BUTTON_PRESSED
    } else if response.hovered() {
        theme::BUTTON_HOVER
    } else {
        theme::BUTTON
    };
    let glyph_color = if pressed {
        Color32::from_rgb(0x1d, 0x12, 0x05)
    } else {
        theme::TEXT
    };
    let edge = if pressed {
        Stroke::new(1.0, theme::ACCENT_DIM)
    } else {
        Stroke::new(1.0, theme::SHELL_EDGE)
    };

    let painter = ui.painter();
    match face {
        ButtonFace::Circle => {
            let radius = draw_rect.width().min(draw_rect.height()) / 2.0;
            // Resting shadow, hidden while pressed.
            if !pressed {
                painter.circle_filled(
                    draw_rect.center() + egui::vec2(0.0, 1.5),
                    radius,
                    Color32::from_black_alpha(90),
                );
            }
            painter.circle(draw_rect.center(), radius, fill, edge);
            // Unlabeled buttons (the screen-corner ones) get a molded dot
            // so they read as pressable.
            if glyph.is_empty() {
                painter.circle_filled(
                    draw_rect.center(),
                    radius * 0.35,
                    if pressed {
                        Color32::from_black_alpha(60)
                    } else {
                        Color32::from_white_alpha(22)
                    },
                );
            }
        }
        ButtonFace::Rounded(radius) => {
            if !pressed {
                painter.rect_filled(
                    draw_rect.translate(egui::vec2(0.0, 1.5)),
                    CornerRadius::same(radius as u8),
                    Color32::from_black_alpha(90),
                );
            }
            painter.rect(
                draw_rect,
                CornerRadius::same(radius as u8),
                fill,
                edge,
                StrokeKind::Inside,
            );
        }
    }
    if !glyph.is_empty() {
        painter.text(
            draw_rect.center(),
            Align2::CENTER_CENTER,
            glyph,
            FontId::proportional(glyph_size),
            glyph_color,
        );
    }

    (response, mouse_held)
}

/// The LCD glass: bezel, panel, and the current frame with crisp pixels.
pub fn lcd(ui: &mut Ui, glass: Rect, texture: &egui::TextureHandle) {
    let bezel = glass.expand(12.0);
    let painter = ui.painter();
    painter.rect(
        bezel,
        CornerRadius::same(8),
        theme::LCD_BEZEL,
        Stroke::new(1.0, Color32::from_white_alpha(10)),
        StrokeKind::Inside,
    );
    painter.image(
        texture.id(),
        glass,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// A small status chip for the menu bar ("USB", "IR"...).
pub fn status_chip(ui: &mut Ui, label: &str, on: bool, on_color: Color32) -> Response {
    let (color, text_color) = if on {
        (on_color, Color32::from_rgb(0x14, 0x14, 0x1a))
    } else {
        (theme::CARD_HOVER, theme::TEXT_FAINT)
    };
    let text = egui::RichText::new(label)
        .color(text_color)
        .size(11.0)
        .strong();
    ui.add(
        egui::Button::new(text)
            .fill(color)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(9))
            .min_size(egui::vec2(0.0, 18.0)),
    )
}
