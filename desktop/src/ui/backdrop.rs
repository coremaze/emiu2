//! The "Prism" backdrop: glacier light. A pale ice-blue/white field is
//! crossed by broad diagonal prismatic light shafts that slowly sweep and
//! overlap; where two shafts cross, a faint rainbow dispersion bloom opens
//! (spectral rose/gold/sky glows split along the crossing). Painter-only
//! (no shader) — meshes with vertex-alpha falloff keep it smooth and calm.

use eframe::egui::epaint::Vertex;
use eframe::egui::{self, Color32, Mesh, Rect};

use crate::theme;

const BASE_TOP: Color32 = Color32::from_rgb(0xdf, 0xef, 0xf9);
const BASE_MID: Color32 = Color32::from_rgb(0xc8, 0xe2, 0xf4);
const BASE_BOT: Color32 = Color32::from_rgb(0xa8, 0xcd, 0xea);

const SHAFT_WHITE: Color32 = Color32::WHITE;
const SHAFT_ICE: Color32 = Color32::from_rgb(0xe2, 0xf6, 0xff);

// The dispersion spectrum (kept pastel, and deliberately violet-free):
// a warm rose, a pale gold, a mint, and a sky blue, split along the shaft.
const SPECTRUM: [(Color32, f32, u8); 4] = [
    (Color32::from_rgb(0xff, 0x96, 0x86), -1.0, 42),
    (Color32::from_rgb(0xff, 0xdd, 0x78), -0.33, 38),
    (Color32::from_rgb(0x7f, 0xe6, 0xb8), 0.33, 38),
    (Color32::from_rgb(0x58, 0xb8, 0xff), 1.0, 46),
];

/// One prismatic light shaft: a broad diagonal band, brightest at its core,
/// fading to nothing at both edges. `x` is the anchor of the core where it
/// meets the top edge (fraction of width), `slope` is dx per dy going down,
/// and the whole shaft sweeps sideways slowly around its anchor.
struct Shaft {
    x: f32,
    slope: f32,
    /// Half-width of the band at the top edge (fraction of width).
    half_w: f32,
    color: Color32,
    alpha: u8,
    /// Sweep: amplitude (fraction of width), angular speed, phase.
    amp: f32,
    speed: f32,
    phase: f32,
}

const SHAFTS: &[Shaft] = &[
    // The hero shaft: broad, bright, leaning hard from the upper light.
    Shaft { x: 0.16, slope: 0.62, half_w: 0.16, color: SHAFT_WHITE, alpha: 196,
            amp: 0.070, speed: 0.051, phase: 0.0 },
    // A wide, softer ice-tinted companion further right.
    Shaft { x: 0.52, slope: 0.34, half_w: 0.21, color: SHAFT_ICE, alpha: 142,
            amp: 0.095, speed: 0.037, phase: 2.1 },
    // A steep bright sliver near the right edge.
    Shaft { x: 0.86, slope: 0.78, half_w: 0.10, color: SHAFT_WHITE, alpha: 170,
            amp: 0.060, speed: 0.066, phase: 4.0 },
    // One fainter counter-leaning shaft, so crossings actually happen.
    Shaft { x: 0.66, slope: -0.30, half_w: 0.13, color: SHAFT_ICE, alpha: 108,
            amp: 0.110, speed: 0.044, phase: 1.2 },
];

/// Where the shaft's core crosses the top edge right now, in pixels.
fn shaft_core_x(s: &Shaft, t: f32, rect: &Rect) -> f32 {
    rect.left() + (s.x + (t * s.speed + s.phase).sin() * s.amp) * rect.width()
}

/// A diagonal band with a smooth alpha bump across its width: transparent
/// edge → bright core → transparent edge, as vertex columns spanning the
/// rect top-to-bottom (the clip rect trims the overhang).
fn shaft_mesh(rect: &Rect, core_x: f32, slope: f32, half_w: f32, color: Color32, alpha: u8) -> Mesh {
    const STOPS: [(f32, f32); 5] = [(-1.0, 0.0), (-0.30, 0.48), (0.0, 1.0), (0.30, 0.48), (1.0, 0.0)];
    let h = rect.height();
    let mut m = Mesh::default();
    for (off, k) in STOPS {
        let c = theme::with_alpha(color, (alpha as f32 * k) as u8);
        let xt = core_x + off * half_w;
        m.vertices.push(Vertex::untextured(egui::pos2(xt, rect.top()), c));
        m.vertices
            .push(Vertex::untextured(egui::pos2(xt + slope * h, rect.bottom()), c));
    }
    for i in 0..STOPS.len() as u32 - 1 {
        let b = i * 2;
        m.indices.extend_from_slice(&[b, b + 2, b + 1, b + 2, b + 3, b + 1]);
    }
    m
}

/// 0 at `lo` and `hi`, easing smoothly up to 1 from `band` inside them.
/// Anything positioned by the sweeping shafts must fade through this at
/// its validity edges — a hard cutoff pops in and out as it drifts.
fn edge_fade(v: f32, lo: f32, hi: f32, band: f32) -> f32 {
    let ease = |t: f32| {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    ease((v - lo) / band) * ease((hi - v) / band)
}

/// Paint the glacier into `rect`. Call once, first thing, so the chrome
/// draws over it. Requests a repaint to keep the sweep smooth.
pub fn paint(ui: &mut egui::Ui, rect: Rect) {
    let t = ui.input(|i| i.time) as f32;
    ui.ctx().request_repaint();
    let p = ui.painter().with_clip_rect(rect);

    // Pale ice base: near-white light at the top, deeper glacier blue below.
    let mid_y = rect.top() + rect.height() * 0.42;
    theme::vgrad(&p, Rect::from_min_max(rect.left_top(), egui::pos2(rect.right(), mid_y)),
                 BASE_TOP, BASE_MID);
    theme::vgrad(&p, Rect::from_min_max(egui::pos2(rect.left(), mid_y), rect.right_bottom()),
                 BASE_MID, BASE_BOT);

    let w = rect.width();
    let h = rect.height();

    // A soft breathing sun bloom, upper-left — the light source the shafts
    // pour from.
    let breathe = 1.0 + 0.06 * (t * 0.23).sin();
    p.add(egui::Shape::mesh(theme::radial_mesh(
        egui::pos2(rect.left() + 0.16 * w, rect.top() + 0.04 * h),
        0.40 * w * breathe,
        0.26 * h * breathe,
        Color32::WHITE,
        112,
    )));

    // The prismatic shafts, slowly sweeping. The hero shaft (index 0) also
    // carries a faint spectral fringe on each edge — light splitting as it
    // leaves the prism: rose off the left lip, sky off the right.
    let cores: Vec<f32> = SHAFTS.iter().map(|s| shaft_core_x(s, t, &rect)).collect();
    for (i, (s, &core)) in SHAFTS.iter().zip(&cores).enumerate() {
        p.add(egui::Shape::mesh(shaft_mesh(
            &rect, core, s.slope, s.half_w * w, s.color, s.alpha,
        )));
        if i == 0 {
            let hw = s.half_w * w;
            p.add(egui::Shape::mesh(shaft_mesh(
                &rect, core - hw * 0.92, s.slope, hw * 0.30,
                Color32::from_rgb(0xff, 0xa8, 0x96), 30,
            )));
            p.add(egui::Shape::mesh(shaft_mesh(
                &rect, core + hw * 0.92, s.slope, hw * 0.30,
                Color32::from_rgb(0x66, 0xc2, 0xff), 34,
            )));
        }
    }

    // Rainbow dispersion where shafts cross: find each pair's crossing point
    // and open a faint spectral bloom there, its colors split along the
    // brighter shaft's direction. The crossing drifts (fast — core distance
    // over a small slope difference) as the shafts sweep, so the bloom
    // fades through its validity edges instead of popping.
    for i in 0..SHAFTS.len() {
        for j in (i + 1)..SHAFTS.len() {
            let (si, sj) = (&SHAFTS[i], &SHAFTS[j]);
            let dslope = si.slope - sj.slope;
            if dslope.abs() < 0.08 {
                continue; // near-parallel (static): no crossing worth marking
            }
            let dy = (cores[j] - cores[i]) / dslope;
            let cx = cores[i] + si.slope * dy;
            let cy = rect.top() + dy;
            let envelope = edge_fade(dy, 0.06 * h, 0.94 * h, 0.12 * h)
                * edge_fade(cx - rect.left(), 0.03 * w, 0.97 * w, 0.08 * w);
            if envelope <= 0.0 {
                continue;
            }
            // Split the spectrum along the steeper shaft's normal.
            let steep = if si.slope.abs() > sj.slope.abs() { si } else { sj };
            let norm = egui::vec2(1.0, -steep.slope).normalized();
            let spread = (si.half_w.min(sj.half_w)) * w * 0.80;
            for (color, offset, alpha) in SPECTRUM {
                p.add(egui::Shape::mesh(theme::radial_mesh(
                    egui::pos2(cx, cy) + norm * (offset * spread),
                    spread * 2.3,
                    spread * 2.9,
                    color,
                    (alpha as f32 * envelope) as u8,
                )));
            }
        }
    }

    // Ground the floor: the ice deepens toward the bottom lip.
    let fade = (h * 0.18).min(130.0);
    p.add(egui::Shape::mesh(theme::vgrad_mesh(
        Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - fade), rect.right_bottom()),
        theme::with_alpha(Color32::from_rgb(0x8e, 0xba, 0xdd), 0),
        theme::with_alpha(Color32::from_rgb(0x8e, 0xba, 0xdd), 120),
    )));
}
