//! The "Frost" backdrop: a slow aurora over a dark-cool base. Several large
//! soft radial glows in cyan, violet, and teal overlap and drift, their
//! centers animated off the egui clock, building a deep, atmospheric wash
//! behind the frosted-acrylic chrome. Painter-only (no shader), so it stays
//! calm and smooth — no bokeh, no bright sky.

use eframe::egui::{self, Color32, Rect};

use crate::theme;

const BASE_TOP: Color32 = Color32::from_rgb(0x0c, 0x14, 0x21);
const BASE_BOT: Color32 = Color32::from_rgb(0x08, 0x0d, 0x16);

const CYAN: Color32 = Color32::from_rgb(0x35, 0xe0, 0xff);
const VIOLET: Color32 = Color32::from_rgb(0x8a, 0x6c, 0xff);
const TEAL: Color32 = Color32::from_rgb(0x3f, 0xd0, 0xc0);

/// One drifting aurora glow: a base position (fraction of the rect), an
/// elliptical radius (fraction), a color and peak alpha, and a slow drift.
struct Blob {
    x: f32,
    y: f32,
    rx: f32,
    ry: f32,
    color: Color32,
    alpha: u8,
    /// Drift: amplitude (fraction), angular speed, and phase per axis.
    ax: f32,
    ay: f32,
    sx: f32,
    sy: f32,
    px: f32,
    py: f32,
}

const BLOBS: &[Blob] = &[
    // A wide cyan curtain, upper-left.
    Blob { x: 0.26, y: 0.28, rx: 0.62, ry: 0.58, color: CYAN, alpha: 64,
           ax: 0.06, ay: 0.05, sx: 0.031, sy: 0.024, px: 0.0, py: 1.7 },
    // A tall violet drape on the right.
    Blob { x: 0.80, y: 0.44, rx: 0.52, ry: 0.70, color: VIOLET, alpha: 58,
           ax: 0.07, ay: 0.06, sx: 0.026, sy: 0.037, px: 2.1, py: 0.6 },
    // Teal pooling low across the floor.
    Blob { x: 0.50, y: 0.90, rx: 0.72, ry: 0.44, color: TEAL, alpha: 52,
           ax: 0.08, ay: 0.04, sx: 0.022, sy: 0.030, px: 1.0, py: 3.0 },
    // A cyan swell, lower-left.
    Blob { x: 0.14, y: 0.74, rx: 0.44, ry: 0.46, color: CYAN, alpha: 46,
           ax: 0.06, ay: 0.06, sx: 0.035, sy: 0.028, px: 3.4, py: 1.2 },
    // A faint violet bloom, upper-right.
    Blob { x: 0.68, y: 0.12, rx: 0.46, ry: 0.40, color: VIOLET, alpha: 40,
           ax: 0.05, ay: 0.05, sx: 0.029, sy: 0.033, px: 0.7, py: 2.4 },
    // A brighter, livelier cyan thread near the middle.
    Blob { x: 0.44, y: 0.50, rx: 0.30, ry: 0.34, color: CYAN, alpha: 44,
           ax: 0.10, ay: 0.08, sx: 0.045, sy: 0.052, px: 1.9, py: 0.2 },
];

/// Paint the aurora into `rect`. Call once, first thing, so the chrome draws
/// over it. Requests a repaint to keep the drift smooth.
pub fn paint(ui: &mut egui::Ui, rect: Rect) {
    let t = ui.input(|i| i.time) as f32;
    ui.ctx().request_repaint();
    let p = ui.painter().with_clip_rect(rect);

    // Dark-cool base, a touch deeper at the floor.
    theme::vgrad(&p, rect, BASE_TOP, BASE_BOT);

    let w = rect.width();
    let h = rect.height();
    for b in BLOBS {
        let cx = rect.left() + (b.x + (t * b.sx + b.px).sin() * b.ax) * w;
        let cy = rect.top() + (b.y + (t * b.sy + b.py).cos() * b.ay) * h;
        p.add(egui::Shape::mesh(theme::radial_mesh(
            egui::pos2(cx, cy),
            b.rx * w,
            b.ry * h,
            b.color,
            b.alpha,
        )));
    }

    // A gentle vignette: darken the very top and bottom lips so the aurora
    // feels held in a deep frame rather than running off the edges.
    let fade = (h * 0.16).min(120.0);
    p.add(egui::Shape::mesh(theme::vgrad_mesh(
        Rect::from_min_max(rect.left_top(), egui::pos2(rect.right(), rect.top() + fade)),
        theme::with_alpha(BASE_BOT, 150),
        theme::with_alpha(BASE_BOT, 0),
    )));
    p.add(egui::Shape::mesh(theme::vgrad_mesh(
        Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - fade), rect.right_bottom()),
        theme::with_alpha(BASE_BOT, 0),
        theme::with_alpha(BASE_BOT, 170),
    )));
}
