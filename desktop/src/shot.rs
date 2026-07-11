//! A hidden development harness: `--shot out.png` runs the app for a few
//! frames, captures the window with egui's screenshot viewport command,
//! writes the PNG, and exits. `--shot-ui <state>` forces a UI state first
//! so dialogs can be captured unattended. Used for automated visual review;
//! harmless (and undocumented) for players.

use std::path::PathBuf;

use eframe::egui;

use crate::app::Dialog;

pub struct ShotState {
    path: PathBuf,
    /// Frames to let the app settle (textures, first LCD frames) before
    /// asking for the capture.
    frames_left: u32,
    requested: bool,
    ui_state: Option<String>,
}

impl ShotState {
    pub fn new(path: PathBuf, frames: u32, ui_state: Option<String>) -> Self {
        Self {
            path,
            frames_left: frames,
            requested: false,
            ui_state,
        }
    }

    /// Applies `--shot-ui` by opening the corresponding dialog.
    pub fn force_ui_state(&self, dialog: &mut Dialog) {
        match self.ui_state.as_deref() {
            Some("new-save") => *dialog = Dialog::NewSave(Default::default()),
            Some("controls") => *dialog = Dialog::Controls(Default::default()),
            Some("friends") => *dialog = Dialog::Friends(Default::default()),
            Some("reset-confirm") => {
                *dialog = Dialog::ConfirmReset {
                    connect_mode: false,
                }
            }
            _ => {}
        }
    }

    /// Advances the harness one frame. Returns true when finished.
    pub fn step(&mut self, ctx: &egui::Context) -> bool {
        // A capture that has arrived ends the run.
        let image = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let size = [image.size[0] as u32, image.size[1] as u32];
            let pixels: Vec<u8> = image
                .pixels
                .iter()
                .flat_map(|p| [p.r(), p.g(), p.b(), p.a()])
                .collect();
            let result = image::RgbaImage::from_raw(size[0], size[1], pixels)
                .ok_or_else(|| "bad screenshot buffer".to_owned())
                .and_then(|img| img.save(&self.path).map_err(|why| why.to_string()));
            match result {
                Ok(()) => println!("shot: wrote {}", self.path.display()),
                Err(why) => eprintln!("shot: failed: {why}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return true;
        }

        ctx.request_repaint();
        if self.frames_left > 0 {
            self.frames_left -= 1;
        } else if !self.requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.requested = true;
        }
        false
    }
}
