//! Emiu2 Desktop: the player-facing Miuchiz emulator frontend.
//!
//! No arguments needed — firmware is built in and saves manage themselves.
//! The few flags that exist are development tools (see [`StartupOptions`]).

mod app;
mod config;
mod controls;
mod emu;
mod firmware;
mod saves;
mod shot;
mod theme;
mod ui;

/// Hidden flags for development and testing:
/// `--play <name>`      jump straight into the save (or character) `<name>`
/// `--shot <path>`      screenshot the window after settling, then exit
/// `--shot-frames <n>`  frames to settle before the shot (default 45)
/// `--shot-ui <state>`  force a UI state first: new-save | controls |
///                      friends | reset-confirm | fullscreen
/// `--shot-size <WxH>`  open the window at this inner size
pub struct StartupOptions {
    pub play: Option<String>,
    pub shot: Option<shot::ShotState>,
    pub window_size: Option<[f32; 2]>,
}

fn parse_args() -> Result<StartupOptions, String> {
    let mut play = None;
    let mut shot_path: Option<std::path::PathBuf> = None;
    let mut shot_frames: u32 = 45;
    let mut shot_ui: Option<String> = None;
    let mut window_size = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--play" => play = Some(value("--play")?),
            "--shot" => shot_path = Some(value("--shot")?.into()),
            "--shot-frames" => {
                shot_frames = value("--shot-frames")?
                    .parse()
                    .map_err(|_| "--shot-frames needs a number".to_owned())?;
            }
            "--shot-ui" => shot_ui = Some(value("--shot-ui")?),
            "--shot-size" => {
                let text = value("--shot-size")?;
                let (w, h) = text
                    .split_once('x')
                    .ok_or_else(|| "--shot-size needs WxH".to_owned())?;
                window_size = Some([
                    w.parse()
                        .map_err(|_| "--shot-size needs numbers".to_owned())?,
                    h.parse()
                        .map_err(|_| "--shot-size needs numbers".to_owned())?,
                ]);
            }
            other => return Err(format!("Unknown argument {other:?}")),
        }
    }

    Ok(StartupOptions {
        play,
        shot: shot_path.map(|path| shot::ShotState::new(path, shot_frames, shot_ui)),
        window_size,
    })
}

fn main() {
    let options = match parse_args() {
        Ok(options) => options,
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(2);
        }
    };

    // The floor is the smallest View > Screen size preset, so every preset
    // stays reachable by resizing. An explicit --shot-size may be smaller
    // still; let it win so low-resolution layouts can be exercised.
    let floor = ui::player::window_size_for_scale(ui::player::MIN_SCALE);
    let size = options.window_size.unwrap_or([960.0, 640.0]);
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Emiu2 Desktop")
            .with_app_id("emiu2-desktop")
            .with_inner_size(size)
            .with_min_inner_size([size[0].min(floor.x), size[1].min(floor.y)]),
        ..Default::default()
    };

    let result = eframe::run_native(
        "Emiu2 Desktop",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::DesktopApp::new(cc, options)))),
    );
    if let Err(why) = result {
        eprintln!("Could not start Emiu2 Desktop: {why}");
        std::process::exit(1);
    }
}
