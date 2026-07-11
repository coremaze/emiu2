//! The app's look: a quiet, dark shell so the tiny LCD and the character
//! art carry the color. Everything visual that more than one screen uses
//! lives here — palette, egui style overrides, character accents.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Shadow, Stroke, TextStyle, Visuals,
};

// ---- Palette ---------------------------------------------------------

/// Deepest background (behind everything).
pub const BG: Color32 = Color32::from_rgb(0x14, 0x14, 0x1a);
/// Panels and bars.
pub const PANEL: Color32 = Color32::from_rgb(0x1b, 0x1b, 0x22);
/// Cards and dialogs.
pub const CARD: Color32 = Color32::from_rgb(0x22, 0x22, 0x2b);
/// Hovered cards / inputs.
pub const CARD_HOVER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x36);
/// Hairlines between regions.
pub const OUTLINE: Color32 = Color32::from_rgb(0x33, 0x33, 0x40);

pub const TEXT: Color32 = Color32::from_rgb(0xe9, 0xe7, 0xe4);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x9c, 0x9a, 0xa8);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x6b, 0x69, 0x78);

/// The one brand accent: a warm Miuchiz orange.
pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x9d, 0x3a);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0xb5, 0x6f, 0x2a);

pub const GOOD: Color32 = Color32::from_rgb(0x7d, 0xd8, 0x8a);
pub const BAD: Color32 = Color32::from_rgb(0xf2, 0x6d, 0x6d);

/// The plastic shell of the drawn handheld.
pub const SHELL: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x3a);
pub const SHELL_EDGE: Color32 = Color32::from_rgb(0x41, 0x41, 0x52);
/// Buttons on the shell.
pub const BUTTON: Color32 = Color32::from_rgb(0x4a, 0x4a, 0x5c);
pub const BUTTON_HOVER: Color32 = Color32::from_rgb(0x59, 0x59, 0x6e);
pub const BUTTON_PRESSED: Color32 = ACCENT;
/// The dead screen border around the LCD glass.
pub const LCD_BEZEL: Color32 = Color32::from_rgb(0x0a, 0x0a, 0x0d);

/// Each character's accent, used for gallery placeholders and chips.
pub fn character_color(character: &str) -> Color32 {
    match character {
        "Spike" => Color32::from_rgb(0xff, 0x7a, 0x3c),
        "Inferno" => Color32::from_rgb(0xff, 0x52, 0x52),
        "Creeper" => Color32::from_rgb(0x6c, 0xd4, 0x4a),
        "Dash" => Color32::from_rgb(0x43, 0xa6, 0xff),
        "Roc" => Color32::from_rgb(0xff, 0xd2, 0x3e),
        "Cloe" => Color32::from_rgb(0xff, 0x6f, 0xb1),
        "Yasmin" => Color32::from_rgb(0xb1, 0x7a, 0xff),
        _ => ACCENT,
    }
}

// ---- Style -----------------------------------------------------------

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();

    style.text_styles = [
        (TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(14.5, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace)),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.window_margin = egui::Margin::same(16);

    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    visuals.window_fill = CARD;
    visuals.panel_fill = PANEL;
    visuals.extreme_bg_color = BG;
    visuals.faint_bg_color = CARD;
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.window_stroke = Stroke::new(1.0, OUTLINE);
    visuals.window_shadow = Shadow {
        offset: [0, 8],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    visuals.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(100),
    };

    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, OUTLINE);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(6);

    visuals.widgets.inactive.bg_fill = CARD;
    visuals.widgets.inactive.weak_bg_fill = CARD;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, OUTLINE);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(6);

    visuals.widgets.hovered.bg_fill = CARD_HOVER;
    visuals.widgets.hovered.weak_bg_fill = CARD_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_DIM);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(6);

    visuals.widgets.active.bg_fill = CARD_HOVER;
    visuals.widgets.active.weak_bg_fill = CARD_HOVER;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.5, TEXT);
    visuals.widgets.active.corner_radius = CornerRadius::same(6);

    visuals.widgets.open.bg_fill = CARD_HOVER;
    visuals.widgets.open.weak_bg_fill = CARD_HOVER;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, ACCENT_DIM);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.open.corner_radius = CornerRadius::same(6);

    visuals.selection.bg_fill = ACCENT_DIM.gamma_multiply(0.55);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;

    style.visuals = visuals;
    ctx.set_style_of(egui::Theme::Dark, style);
}

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
