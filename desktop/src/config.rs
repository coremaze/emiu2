//! Settings, persisted as TOML in the app's config directory under the
//! shared Miuchiz Reborn storage policy. Every field has a default so a
//! missing or partial file (or one from a newer version) still loads.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub usb: UsbConfig,
    pub ir: IrConfig,
    pub video: VideoConfig,
    /// Device button slug -> egui key name (empty = unbound). See
    /// [`crate::controls::Bindings`].
    pub controls: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UsbConfig {
    /// Whether the emulated USB cable starts plugged in. Kept plugged by
    /// default so host tools can always see the device.
    pub plugged: bool,
}

impl Default for UsbConfig {
    fn default() -> Self {
        Self { plugged: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IrConfig {
    /// The IR relay (`host:port`) used for friend-code play. Cleared, the
    /// Friends feature explains how to set one.
    pub relay: String,
}

impl Default for IrConfig {
    fn default() -> Self {
        Self {
            relay: "emiu2.miuchiz.com:5885".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoConfig {
    /// Draw the LCD only at whole-number multiples of its 98x67 pixels.
    /// The panel is so small that fractional scaling visibly ripples it.
    pub integer_scaling: bool,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            integer_scaling: true,
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        miuchiz_reborn_paths::AppDirs::new("emiu2-desktop")
            .config_dir()
            .join("config.toml")
    }

    /// Loads the config, falling back to defaults for anything missing,
    /// malformed, or absent entirely.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|why| {
                eprintln!("Config file {path:?} is malformed ({why}); using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) {
        let Ok(text) = toml::to_string_pretty(self) else {
            return;
        };
        if let Some(dir) = path.parent() {
            if let Err(why) = std::fs::create_dir_all(dir) {
                eprintln!("Could not create config directory {dir:?}: {why}");
                return;
            }
        }
        if let Err(why) = crate::saves::write_atomic(path, text.as_bytes()) {
            eprintln!("Could not save config to {path:?}: {why}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_survive_a_partial_file() {
        let config: Config = toml::from_str("[usb]\nplugged = false\n").unwrap();
        assert!(!config.usb.plugged);
        assert!(config.video.integer_scaling);
        assert!(!config.ir.relay.is_empty());
    }

    #[test]
    fn round_trips_through_toml() {
        let mut config = Config::default();
        config.usb.plugged = false;
        config
            .controls
            .insert("up".to_owned(), "W".to_owned());
        let text = toml::to_string_pretty(&config).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert!(!back.usb.plugged);
        assert_eq!(back.controls["up"], "W");
    }
}
