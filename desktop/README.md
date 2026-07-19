## Emiu2 Desktop

Players should start here: run `emiu2-desktop` (or `cargo run -r -p emiu2-desktop`), pick a character, and play. Every character's 1.09.03 firmware is built in (custom OTP/flash images remain available under the new-save dialog's advanced options).

- **Saves** live under the shared Miuchiz Reborn data directory, one folder per save, and are primarily snapshots: the running machine is saved every 15 seconds and when the app closes, so resuming continues exactly where you left off. Each save also keeps a current flash dump (`flash.bin`, usable by USB tools) and a thumbnail for the gallery.
- **USB** works exactly like `emiu2-dev`'s: each session publishes a discovery endpoint, and the cable (plugged in by default; toggle it in the Device menu, remembered in `config.toml`) is what host tools see. *Device → Restart to PC connection* boots straight into the handheld's "Please Connect to PC" mode.
- **Friends** wraps the IR relay: it shows your friend code, joins a friend's, and pairs, like holding two real ones face to face.
- **Controls** are remappable, and the on-screen buttons respond to both the mouse and the mapped keys. F11 shows just the device screen, fullscreen; scaling never stretches, and integer scaling (on by default) keeps the pixels sharp. *View → Screen size* snaps the window to exact whole-pixel scales.
