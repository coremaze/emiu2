//! Custom-drawn pieces shared by the screens: the device's physical
//! buttons and the LCD glass. egui's stock widgets are for the chrome;
//! the drawn handheld needs its own look.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui,
};

use crate::theme;

enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// The D-pad arm glyphs are painted as triangles (the display font has no
/// arrow glyphs); everything else is drawn as text.
fn arrow_dir(glyph: &str) -> Option<Dir> {
    match glyph {
        "⏶" => Some(Dir::Up),
        "⏷" => Some(Dir::Down),
        "⏴" => Some(Dir::Left),
        "⏵" => Some(Dir::Right),
        _ => None,
    }
}

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

    let base = if pressed {
        theme::BUTTON_PRESSED
    } else if response.hovered() {
        theme::BUTTON_HOVER
    } else {
        theme::BUTTON
    };
    let top_c = base.gamma_multiply(1.16);
    let bot_c = base.gamma_multiply(0.84);
    let glyph_color = if pressed {
        theme::ON_ACCENT
    } else {
        theme::TEXT
    };
    let edge = if pressed {
        Stroke::new(1.2, theme::with_alpha(theme::ACCENT, 0xf0))
    } else {
        Stroke::new(1.0, theme::SHELL_EDGE)
    };
    let corner = match face {
        ButtonFace::Circle => {
            CornerRadius::same((draw_rect.width().min(draw_rect.height()) / 2.0) as u8)
        }
        ButtonFace::Rounded(radius) => CornerRadius::same(radius as u8),
    };

    let painter = ui.painter();
    // A pressed pill lights up: a cyan bloom under the frosted glass.
    if pressed {
        painter.add(egui::Shape::mesh(theme::radial_mesh(
            draw_rect.center(),
            draw_rect.width() * 0.95,
            draw_rect.height() * 0.98,
            theme::ACCENT,
            120,
        )));
    }
    // Frosted pill body: contact shadow, translucent gradient, rim, top sheen.
    if !pressed {
        painter.add(egui::Shape::from(
            theme::fx::button().as_shape(draw_rect, corner),
        ));
    }
    painter.add(egui::Shape::mesh(theme::rounded_vgrad_mesh(
        draw_rect, corner, top_c, bot_c,
    )));
    painter.rect_stroke(draw_rect, corner, edge, StrokeKind::Inside);
    theme::gloss_cap(painter, draw_rect, if pressed { 55 } else { 90 });
    // Unlabeled buttons (the screen-corner ones) get a molded dot.
    if matches!(face, ButtonFace::Circle) && glyph.is_empty() {
        let radius = draw_rect.width().min(draw_rect.height()) / 2.0;
        painter.circle_filled(
            draw_rect.center(),
            radius * 0.34,
            if pressed {
                theme::with_alpha(Color32::WHITE, 120)
            } else {
                theme::with_alpha(theme::ACCENT, 70)
            },
        );
    }
    // D-pad arrows aren't in the display font — paint them as triangles.
    // Everything else (A, MENU, POWER, MUTE) is drawn in the display face.
    if let Some(dir) = arrow_dir(glyph) {
        let c = draw_rect.center();
        let s = glyph_size * 0.62;
        let tri = match dir {
            Dir::Up => [
                c + egui::vec2(-s, s * 0.55),
                c + egui::vec2(s, s * 0.55),
                c + egui::vec2(0.0, -s * 0.8),
            ],
            Dir::Down => [
                c + egui::vec2(-s, -s * 0.55),
                c + egui::vec2(s, -s * 0.55),
                c + egui::vec2(0.0, s * 0.8),
            ],
            Dir::Left => [
                c + egui::vec2(s * 0.55, -s),
                c + egui::vec2(s * 0.55, s),
                c + egui::vec2(-s * 0.8, 0.0),
            ],
            Dir::Right => [
                c + egui::vec2(-s * 0.55, -s),
                c + egui::vec2(-s * 0.55, s),
                c + egui::vec2(s * 0.8, 0.0),
            ],
        };
        painter.add(egui::Shape::convex_polygon(
            tri.to_vec(),
            glyph_color,
            Stroke::NONE,
        ));
    } else if !glyph.is_empty() {
        painter.text(
            draw_rect.center(),
            Align2::CENTER_CENTER,
            glyph,
            FontId::new(glyph_size, theme::display_family()),
            glyph_color,
        );
    }

    (response, mouse_held)
}

/// The LCD glass: a chrome-rimmed dark bezel set into the frosted white
/// shell, the frame with crisp pixels, and a sheen across the glass.
pub fn lcd(ui: &mut Ui, glass: Rect, texture: &egui::TextureHandle) {
    let bezel = glass.expand(12.0);
    let painter = ui.painter();
    let cr = CornerRadius::same(10);
    // The screen's light spills onto the ice-white shell: a soft glacier
    // halo hugging the bezel.
    painter.add(egui::Shape::mesh(theme::radial_mesh(
        glass.center(),
        glass.width() * 0.74,
        glass.height() * 0.92,
        theme::ACCENT,
        56,
    )));
    // A polished chrome lip around the dark glass.
    painter.rect_stroke(
        bezel.expand(1.5),
        CornerRadius::same(11),
        Stroke::new(1.5, theme::SHELL_EDGE),
        StrokeKind::Inside,
    );
    painter.add(egui::Shape::mesh(theme::rounded_vgrad_mesh(
        bezel,
        cr,
        Color32::from_rgb(0x2b, 0x3c, 0x4a),
        Color32::from_rgb(0x0c, 0x16, 0x1f),
    )));
    painter.rect_stroke(
        bezel,
        cr,
        Stroke::new(1.0, theme::with_alpha(Color32::WHITE, 60)),
        StrokeKind::Inside,
    );
    painter.image(
        texture.id(),
        glass,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    // A subtle glass sheen over the top strip (kept faint so gameplay reads).
    let glare = Rect::from_min_max(
        glass.left_top(),
        egui::pos2(glass.right(), glass.top() + glass.height() * 0.16),
    );
    painter.add(egui::Shape::mesh(theme::vgrad_mesh(
        glare,
        theme::with_alpha(Color32::WHITE, 26),
        theme::with_alpha(Color32::WHITE, 0),
    )));
}

/// A small status chip for the menu bar ("USB", "IR"...).
pub fn status_chip(ui: &mut Ui, label: &str, on: bool, on_color: Color32) -> Response {
    let (fill, stroke, text_color) = if on {
        // Transparent 1px, not NONE: the frame stroke width feeds the
        // button's size, and the resting chrome is drawn around a 1px
        // stroke. NONE would shrink the lit chip by 2px in each axis.
        (
            on_color,
            Stroke::new(1.0, Color32::TRANSPARENT),
            theme::ON_ACCENT,
        )
    } else {
        (
            theme::with_alpha(theme::FROST_HILITE, 150),
            Stroke::new(1.0, theme::OUTLINE),
            theme::TEXT_DIM,
        )
    };
    let text = egui::RichText::new(label)
        .font(FontId::new(10.5, theme::mono_family()))
        .color(text_color);
    ui.add(
        egui::Button::new(text)
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(9))
            .min_size(egui::vec2(0.0, 19.0)),
    )
}
