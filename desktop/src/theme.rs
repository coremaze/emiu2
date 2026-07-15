//! Skin: "Frost" — frosted acrylic over a drifting aurora (Fluent / visionOS).
//! A slow cyan/violet/teal aurora (see [`crate::ui::backdrop`]) glows over a
//! dark-cool base; the chrome is translucent frosted-acrylic glass — a faint
//! near-white veil with a bright inner top-edge highlight and a light rim —
//! floating over it, and the handheld reads as a frosted glass slab the LCD
//! shines through. Everything visual that more than one screen uses lives
//! here: the font stack, the palette, egui style, paint helpers, and the
//! per-character accents.

use std::sync::Arc;

use eframe::egui::epaint::Vertex;
use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, FontTweak, Mesh,
    Rect, RichText, Shadow, Stroke, TextStyle, Visuals,
};

// ---- Palette ---------------------------------------------------------
// Frost reads dark and moody: the aurora backdrop is deep and cool, and the
// chrome is frosted-acrylic glass — a faint near-white veil over the aurora,
// carried by a bright top-edge highlight and a light rim, not a bright fill.
// (Theme::Dark, but the panels read as luminous frosted glass.)

// The translucent frosted fills are written premultiplied (const-friendly);
// each is the near-white tint above, carried at a low alpha over the aurora.

/// Fallback base behind the backdrop — the dark-cool floor.
pub const BG: Color32 = Color32::from_rgb(0x0d, 0x14, 0x20);
/// Bars — a translucent frosted-acrylic veil (≈ #ddecff @ 0x22).
pub const PANEL: Color32 = Color32::from_rgba_premultiplied(0x1d, 0x1f, 0x22, 0x22);
/// Cards and dialogs — frosted acrylic (≈ #e4f0ff @ 0x20).
pub const CARD: Color32 = Color32::from_rgba_premultiplied(0x1d, 0x1e, 0x20, 0x20);
/// Hovered cards / inputs — the veil thickens (≈ #f0f8ff @ 0x4a).
pub const CARD_HOVER: Color32 = Color32::from_rgba_premultiplied(0x46, 0x48, 0x4a, 0x4a);
/// Hairlines and rims — a light frosted edge (≈ #cbe4ff @ 0x3a).
pub const OUTLINE: Color32 = Color32::from_rgba_premultiplied(0x2e, 0x34, 0x3a, 0x3a);

pub const TEXT: Color32 = Color32::from_rgb(0xea, 0xf2, 0xfb);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0xa4, 0xb7, 0xcb);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x71, 0x84, 0x9a);

/// Icy cyan — the aurora's brightest thread.
pub const ACCENT: Color32 = Color32::from_rgb(0x6f, 0xd8, 0xff);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x4a, 0xba, 0xe8);
/// A warm point of light against the cool — wordmark dot, small highlights.
pub const GOLD: Color32 = Color32::from_rgb(0xff, 0xd6, 0x7a);

pub const GOOD: Color32 = Color32::from_rgb(0x49, 0xda, 0x9c);
pub const BAD: Color32 = Color32::from_rgb(0xff, 0x6b, 0x74);

/// The frosted glass rim of the drawn handheld (≈ #d6ecff @ 0x66).
pub const SHELL_EDGE: Color32 = Color32::from_rgba_premultiplied(0x56, 0x5e, 0x66, 0x66);
/// Frosted pill buttons on the shell (≈ #e6f2ff @ 0x3c / #f2f9ff @ 0x5e).
pub const BUTTON: Color32 = Color32::from_rgba_premultiplied(0x36, 0x39, 0x3c, 0x3c);
pub const BUTTON_HOVER: Color32 = Color32::from_rgba_premultiplied(0x59, 0x5c, 0x5e, 0x5e);
pub const BUTTON_PRESSED: Color32 = ACCENT;
/// The dark screen border around the LCD glass.
pub const LCD_BEZEL: Color32 = Color32::from_rgb(0x0a, 0x11, 0x1d);

/// The bright inner top-edge highlight that lights a frosted panel's lip.
pub const FROST_HILITE: Color32 = Color32::from_rgb(0xf4, 0xfb, 0xff);
/// Deep ink for text sitting on a bright (icy-cyan) accent fill.
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x05, 0x1a, 0x2a);

/// Each character's accent, used for gallery placeholders and chips.
pub fn character_color(character: &str) -> Color32 {
    match character {
        "Spike" => Color32::from_rgb(0xf2, 0xa5, 0x1c),
        "Inferno" => Color32::from_rgb(0xef, 0x4b, 0x2e),
        "Creeper" => Color32::from_rgb(0x35, 0xa5, 0x3e),
        "Dash" => Color32::from_rgb(0x3a, 0x8f, 0xe8),
        "Roc" => Color32::from_rgb(0xf0, 0xc0, 0x1e),
        "Cloe" => Color32::from_rgb(0xec, 0x40, 0xa8),
        "Yasmin" => Color32::from_rgb(0x8c, 0x55, 0xf5),
        _ => ACCENT,
    }
}

// ---- Fonts -----------------------------------------------------------

const FONT_DISPLAY: &str = "display_nunito";
const FONT_MONO: &str = "jetbrains_mono";

pub fn display_family() -> FontFamily {
    FontFamily::Name(Arc::from(FONT_DISPLAY))
}
pub fn mono_family() -> FontFamily {
    FontFamily::Name(Arc::from(FONT_MONO))
}

/// Nunito ExtraBold at `size`, optionally uppercased with tracking — the
/// wordmark / caps-label treatment shared across the family.
pub fn display(size: f32, text: impl Into<String>) -> RichText {
    RichText::new(text).font(FontId::new(size, display_family()))
}
pub fn caps(size: f32, text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .font(FontId::new(size, display_family()))
        .extra_letter_spacing(0.6)
}
/// JetBrains Mono at `size` — dates, codes, status read-outs.
pub fn mono(size: f32, text: impl Into<String>) -> RichText {
    RichText::new(text).font(FontId::new(size, mono_family()))
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "inter".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "nunito".into(),
        Arc::new(
            FontData::from_static(include_bytes!("../assets/fonts/Nunito-wght.ttf")).tweak(
                FontTweak {
                    coords: egui::epaint::text::VariationCoords::new([(b"wght", 800.0f32)]),
                    ..Default::default()
                },
            ),
        ),
    );
    fonts.font_data.insert(
        "jetbrains_mono".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/JetBrainsMono-Regular.ttf"
        ))),
    );

    // Inter is the default proportional body; Nunito leads the display family.
    let prop = fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .expect("egui default proportional family");
    prop.insert(0, "inter".into());

    fonts.families.insert(
        display_family(),
        vec!["nunito".into(), "inter".into(), "NotoEmoji-Regular".into()],
    );
    fonts.families.insert(
        mono_family(),
        vec![
            "jetbrains_mono".into(),
            "inter".into(),
            "NotoEmoji-Regular".into(),
        ],
    );
    // Keep the default monospace pointing at JetBrains too (friend codes).
    if let Some(m) = fonts.families.get_mut(&FontFamily::Monospace) {
        m.insert(0, "jetbrains_mono".into());
    }
    ctx.set_fonts(fonts);
}

// ---- Style -----------------------------------------------------------

pub fn apply(ctx: &egui::Context) {
    setup_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();

    style.text_styles = [
        (TextStyle::Heading, FontId::new(23.0, display_family())),
        (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(14.5, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.5, mono_family())),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(10.0, 9.0);
    style.spacing.button_padding = egui::vec2(13.0, 6.0);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.window_margin = egui::Margin::same(18);

    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    // Modals sit over a dimmed backdrop: a substantial dark-cool frosted pane
    // so near-white body text always reads, with a light rim from window_stroke.
    visuals.window_fill = Color32::from_rgba_unmultiplied(0x1b, 0x28, 0x3a, 0xe2);
    visuals.panel_fill = Color32::TRANSPARENT;
    // Inset fields read as recessed frosted glass, darker than the panel.
    visuals.extreme_bg_color = Color32::from_rgba_unmultiplied(0x08, 0x10, 0x1c, 0xc0);
    visuals.faint_bg_color = CARD;
    visuals.window_corner_radius = CornerRadius::same(16);
    visuals.window_stroke = Stroke::new(1.0, OUTLINE);
    visuals.window_shadow = fx::shell();
    visuals.popup_shadow = fx::soft_card();

    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, OUTLINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    w.noninteractive.corner_radius = CornerRadius::same(10);

    w.inactive.bg_fill = CARD;
    w.inactive.weak_bg_fill = CARD;
    w.inactive.bg_stroke = Stroke::new(1.0, OUTLINE);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = CornerRadius::same(11);

    w.hovered.bg_fill = CARD_HOVER;
    w.hovered.weak_bg_fill = CARD_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, with_alpha(ACCENT, 0xb0));
    w.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    w.hovered.corner_radius = CornerRadius::same(11);

    w.active.bg_fill = CARD_HOVER;
    w.active.weak_bg_fill = CARD_HOVER;
    w.active.bg_stroke = Stroke::new(1.5, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5, ACCENT);
    w.active.corner_radius = CornerRadius::same(11);

    w.open.bg_fill = CARD_HOVER;
    w.open.weak_bg_fill = CARD_HOVER;
    w.open.bg_stroke = Stroke::new(1.0, with_alpha(ACCENT, 0xb0));
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    w.open.corner_radius = CornerRadius::same(11);

    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.32);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT_DIM;

    style.visuals = visuals;
    ctx.set_style_of(egui::Theme::Dark, style);
}

// ---- Paint helpers (shared with the backdrop, cards, and the device) --

pub fn with_alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        ch(a.r(), b.r()),
        ch(a.g(), b.g()),
        ch(a.b(), b.b()),
        ch(a.a(), b.a()),
    )
}

/// A rectangle filled top→bottom with a vertical gradient (square corners).
pub fn vgrad_mesh(rect: Rect, top: Color32, bot: Color32) -> Mesh {
    let mut m = Mesh::default();
    m.vertices.push(Vertex::untextured(rect.left_top(), top));
    m.vertices.push(Vertex::untextured(rect.right_top(), top));
    m.vertices.push(Vertex::untextured(rect.right_bottom(), bot));
    m.vertices.push(Vertex::untextured(rect.left_bottom(), bot));
    m.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    m
}

/// Paint a vertical gradient into `rect`.
pub fn vgrad(painter: &egui::Painter, rect: Rect, top: Color32, bot: Color32) {
    painter.add(egui::Shape::mesh(vgrad_mesh(rect, top, bot)));
}

/// A rounded-rect vertical gradient mesh (corners follow the outline), used
/// for gel buttons and glossy fills. Interpolates in premultiplied space so a
/// fade to transparent stays hue-stable.
pub fn rounded_vgrad_mesh(rect: Rect, cr: CornerRadius, top: Color32, bot: Color32) -> Mesh {
    use eframe::egui::epaint::{tessellator::path, CornerRadiusF32, Pos2};
    let mut outline: Vec<Pos2> = Vec::new();
    path::rounded_rectangle(&mut outline, rect, CornerRadiusF32::from(cr));
    let mut mesh = Mesh::default();
    let n = outline.len();
    if n < 3 {
        return mesh;
    }
    let y0 = rect.top();
    let h = rect.height().max(1.0e-3);
    let color_at = |y: f32| lerp_color(top, bot, (y - y0) / h);
    for p in &outline {
        mesh.vertices.push(Vertex::untextured(*p, color_at(p.y)));
    }
    let center = rect.center();
    mesh.vertices
        .push(Vertex::untextured(center, color_at(center.y)));
    let cidx = n as u32;
    for i in 0..n {
        let i1 = ((i + 1) % n) as u32;
        mesh.indices.extend_from_slice(&[cidx, i1, i as u32]);
    }
    mesh
}

/// Glossy white sheen over the top half of a rounded shape (Web-2.0 cap).
pub fn gloss_cap(painter: &egui::Painter, rect: Rect, strength: u8) {
    let cap = Rect::from_min_max(
        rect.left_top(),
        egui::pos2(rect.right(), rect.top() + rect.height() * 0.52),
    );
    let cr = CornerRadius {
        nw: 12,
        ne: 12,
        sw: 0,
        se: 0,
    };
    painter.add(egui::Shape::mesh(rounded_vgrad_mesh(
        cap,
        cr,
        with_alpha(Color32::WHITE, strength),
        Color32::TRANSPARENT,
    )));
}

// ---- Frosted acrylic -------------------------------------------------

/// A soft radial (elliptical) glow mesh: bright at the center, fading to
/// transparent at the rim. The aurora and the device's luminous glows are
/// built from these — a triangle fan gives a smooth, shader-free falloff.
pub fn radial_mesh(center: egui::Pos2, rx: f32, ry: f32, color: Color32, center_alpha: u8) -> Mesh {
    let mut m = Mesh::default();
    const SEG: usize = 56;
    m.vertices
        .push(Vertex::untextured(center, with_alpha(color, center_alpha)));
    let edge = with_alpha(color, 0);
    for i in 0..=SEG {
        let a = i as f32 / SEG as f32 * std::f32::consts::TAU;
        m.vertices.push(Vertex::untextured(
            center + egui::vec2(a.cos() * rx, a.sin() * ry),
            edge,
        ));
    }
    for i in 1..=SEG {
        m.indices.extend_from_slice(&[0, i as u32, (i + 1) as u32]);
    }
    m
}

/// The bright 1px inner highlight that rides the top lip of a frosted panel —
/// the single detail that most sells "acrylic glass lit from above".
pub fn frost_top_edge(painter: &egui::Painter, rect: Rect, cr: CornerRadius, alpha: u8) {
    let inset = cr.nw.max(cr.ne) as f32 * 0.72 + 1.5;
    let y = rect.top() + 1.0;
    painter.line_segment(
        [
            egui::pos2(rect.left() + inset, y),
            egui::pos2(rect.right() - inset, y),
        ],
        Stroke::new(1.0, with_alpha(FROST_HILITE, alpha)),
    );
}

/// Paint a frosted-acrylic panel: a translucent veil, a bright inner top-edge
/// highlight, and a light outer rim. The luminous aurora shows through.
pub fn acrylic(painter: &egui::Painter, rect: Rect, cr: CornerRadius, fill: Color32, rim: Color32) {
    painter.rect_filled(rect, cr, fill);
    frost_top_edge(painter, rect, cr, 150);
    painter.rect_stroke(rect, cr, Stroke::new(1.0, rim), egui::StrokeKind::Inside);
}

/// A deterministic 0..1 hash so frosted dither stays put across frames.
fn frost_hash(n: f32) -> f32 {
    (n.sin() * 43758.5453).fract().abs()
}

/// A faint fine dither: tiny low-alpha specks that suggest a frosted, ground
/// texture on a large acrylic surface. Cheap; keep `count` modest.
pub fn dither(painter: &egui::Painter, rect: Rect, count: u32, seed: f32, alpha: u8) {
    let c = with_alpha(FROST_HILITE, alpha);
    for i in 0..count {
        let fi = i as f32 + seed * 97.0;
        let x = rect.left() + frost_hash(fi * 1.7 + 0.3) * rect.width();
        let y = rect.top() + frost_hash(fi * 2.31 + 4.0) * rect.height();
        painter.rect_filled(
            Rect::from_min_size(egui::pos2(x, y), egui::vec2(1.0, 1.0)),
            CornerRadius::ZERO,
            c,
        );
    }
}

// ---- Drop shadows / glows --------------------------------------------

pub mod fx {
    use super::{Color32, Shadow};

    /// Deep, cool drop shadow that lifts the device off the aurora.
    pub fn shell() -> Shadow {
        Shadow {
            offset: [0, 16],
            blur: 48,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(2, 6, 12, 155),
        }
    }
    /// Softer grounding shadow under a floating frosted card.
    pub fn soft_card() -> Shadow {
        Shadow {
            offset: [0, 9],
            blur: 28,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(3, 8, 14, 120),
        }
    }
    /// Small contact shadow under a raised button.
    pub fn button() -> Shadow {
        Shadow {
            offset: [0, 2],
            blur: 7,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(0, 3, 8, 95),
        }
    }
    /// A luminous icy-cyan halo around hero acrylic (the device slab).
    pub fn glow() -> Shadow {
        Shadow {
            offset: [0, 0],
            blur: 38,
            spread: 3,
            color: Color32::from_rgba_unmultiplied(0x6f, 0xd8, 0xff, 46),
        }
    }
}

// ---- Formatting ------------------------------------------------------

/// "3m ago", "2h ago", ... for the gallery cards.
pub fn ago(now_unix: u64, then_unix: u64) -> String {
    if then_unix == 0 || then_unix > now_unix {
        return "just now".to_owned();
    }
    let secs = now_unix - then_unix;
    match secs {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        86_400..=2_591_999 => format!("{}d ago", secs / 86_400),
        _ => format!("{}mo ago", secs / 2_592_000),
    }
}

/// "4h 32m" style playtime.
pub fn playtime(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        "<1m".to_owned()
    }
}
