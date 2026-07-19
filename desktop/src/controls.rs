//! Keyboard bindings for the device's physical buttons.
//!
//! The emulator thread consumes button state as a bitmask (one bit per
//! [`MiuchizGpio`] button, see [`gpio_bit`]); the UI builds that mask each
//! frame from whichever keys are held plus whichever on-screen buttons are
//! being clicked. Bindings are remappable and persist in the config file by
//! egui key name.

use std::collections::HashSet;

use eframe::egui::Key;
use emiu2::miuchiz::{MiuchizButtonStates, MiuchizGpio};

/// The bit each button occupies in the shared button mask.
pub fn gpio_bit(gpio: MiuchizGpio) -> u16 {
    1 << match gpio {
        MiuchizGpio::Up => 0,
        MiuchizGpio::Down => 1,
        MiuchizGpio::Left => 2,
        MiuchizGpio::Right => 3,
        MiuchizGpio::Power => 4,
        MiuchizGpio::Menu => 5,
        MiuchizGpio::UpsideUp => 6,
        MiuchizGpio::UpsideDown => 7,
        MiuchizGpio::ScreenTopLeft => 8,
        MiuchizGpio::ScreenTopRight => 9,
        MiuchizGpio::ScreenBottomLeft => 10,
        MiuchizGpio::ScreenBottomRight => 11,
        MiuchizGpio::Action => 12,
        MiuchizGpio::Mute => 13,
    }
}

/// Expands a button mask back into per-button states (the emulator side).
pub fn mask_to_states(mask: u16) -> MiuchizButtonStates {
    let mut states = MiuchizButtonStates::default();
    for gpio in ALL_GPIO {
        if mask & gpio_bit(gpio) != 0 {
            states.set(gpio, true);
        }
    }
    states
}

const ALL_GPIO: [MiuchizGpio; 14] = [
    MiuchizGpio::Up,
    MiuchizGpio::Down,
    MiuchizGpio::Left,
    MiuchizGpio::Right,
    MiuchizGpio::Power,
    MiuchizGpio::Menu,
    MiuchizGpio::UpsideUp,
    MiuchizGpio::UpsideDown,
    MiuchizGpio::ScreenTopLeft,
    MiuchizGpio::ScreenTopRight,
    MiuchizGpio::ScreenBottomLeft,
    MiuchizGpio::ScreenBottomRight,
    MiuchizGpio::Action,
    MiuchizGpio::Mute,
];

/// One remappable control: a device button and the key currently bound to it.
#[derive(Clone)]
pub struct Binding {
    pub gpio: MiuchizGpio,
    /// The player-facing button name.
    pub label: &'static str,
    /// The config-file key (stable, unlike the label).
    pub slug: &'static str,
    pub key: Option<Key>,
}

/// The full keyboard map, in the order the controls dialog lists them.
#[derive(Clone)]
pub struct Bindings(Vec<Binding>);

impl Default for Bindings {
    fn default() -> Self {
        let bind = |gpio, label, slug, key| Binding {
            gpio,
            label,
            slug,
            key: Some(key),
        };
        Self(vec![
            bind(MiuchizGpio::Up, "D-pad up", "up", Key::ArrowUp),
            bind(MiuchizGpio::Down, "D-pad down", "down", Key::ArrowDown),
            bind(MiuchizGpio::Left, "D-pad left", "left", Key::ArrowLeft),
            bind(MiuchizGpio::Right, "D-pad right", "right", Key::ArrowRight),
            bind(MiuchizGpio::Action, "Action", "action", Key::Space),
            bind(MiuchizGpio::Menu, "Menu", "menu", Key::Enter),
            // The four soft buttons hugging the screen corners; Q/E/A/D
            // mirror their positions on a QWERTY board.
            bind(
                MiuchizGpio::ScreenTopLeft,
                "Screen top-left",
                "screen_top_left",
                Key::Q,
            ),
            bind(
                MiuchizGpio::ScreenTopRight,
                "Screen top-right",
                "screen_top_right",
                Key::E,
            ),
            bind(
                MiuchizGpio::ScreenBottomLeft,
                "Screen bottom-left",
                "screen_bottom_left",
                Key::A,
            ),
            bind(
                MiuchizGpio::ScreenBottomRight,
                "Screen bottom-right",
                "screen_bottom_right",
                Key::D,
            ),
            bind(MiuchizGpio::Power, "Power", "power", Key::P),
            bind(MiuchizGpio::Mute, "Mute", "mute", Key::M),
        ])
    }
}

impl Bindings {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn get(&self, index: usize) -> &Binding {
        &self.0[index]
    }

    /// Binds `key` to the control at `index`, unbinding it anywhere else
    /// (one key drives one button).
    pub fn assign(&mut self, index: usize, key: Key) {
        for binding in &mut self.0 {
            if binding.key == Some(key) {
                binding.key = None;
            }
        }
        self.0[index].key = Some(key);
    }

    pub fn clear(&mut self, index: usize) {
        self.0[index].key = None;
    }

    /// The key currently bound to a button, for UI hints.
    pub fn key_for(&self, gpio: MiuchizGpio) -> Option<Key> {
        self.0.iter().find(|b| b.gpio == gpio).and_then(|b| b.key)
    }

    /// The button-mask contribution of the currently held keys.
    pub fn mask_from_keys(&self, keys_down: &HashSet<Key>) -> u16 {
        let mut mask = 0;
        for binding in &self.0 {
            if let Some(key) = binding.key {
                if keys_down.contains(&key) {
                    mask |= gpio_bit(binding.gpio);
                }
            }
        }
        mask
    }

    /// Serializes to `slug = "KeyName"` pairs for the config file.
    pub fn to_config(&self) -> std::collections::BTreeMap<String, String> {
        self.0
            .iter()
            .map(|b| {
                let name = b.key.map(|k| k.name().to_owned()).unwrap_or_default();
                (b.slug.to_owned(), name)
            })
            .collect()
    }

    /// Applies config-file pairs over the defaults. Unknown slugs and key
    /// names are ignored; an empty name means explicitly unbound.
    pub fn apply_config(&mut self, map: &std::collections::BTreeMap<String, String>) {
        for (slug, name) in map {
            let Some(binding) = self.0.iter_mut().find(|b| b.slug == slug) else {
                continue;
            };
            binding.key = if name.is_empty() {
                None
            } else {
                match Key::from_name(name) {
                    Some(key) => Some(key),
                    None => continue, // unknown key name: keep the default
                }
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_round_trips_through_states() {
        let mask = gpio_bit(MiuchizGpio::Up) | gpio_bit(MiuchizGpio::Action);
        let states = mask_to_states(mask);
        assert!(states.up && states.action);
        assert!(!states.down && !states.menu);
    }

    #[test]
    fn assigning_a_key_steals_it_from_other_bindings() {
        let mut bindings = Bindings::default();
        // Give the Action key (Space) to Power.
        let power_index = bindings
            .0
            .iter()
            .position(|b| b.gpio == MiuchizGpio::Power)
            .unwrap();
        bindings.assign(power_index, Key::Space);
        assert_eq!(bindings.key_for(MiuchizGpio::Power), Some(Key::Space));
        assert_eq!(bindings.key_for(MiuchizGpio::Action), None);
    }

    #[test]
    fn config_round_trip_preserves_bindings() {
        let mut bindings = Bindings::default();
        bindings.assign(0, Key::W);
        let map = bindings.to_config();
        let mut restored = Bindings::default();
        restored.apply_config(&map);
        assert_eq!(restored.key_for(MiuchizGpio::Up), Some(Key::W));
    }
}
