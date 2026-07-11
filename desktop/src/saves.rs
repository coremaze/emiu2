//! The save library: one directory per save under the app's data dir
//! (miuchiz-reborn-paths).
//!
//! A save is primarily a **snapshot** (`state.snapshot`, the complete
//! machine) — that's what resuming loads, so play continues exactly where it
//! left off even though the firmware itself only persists to flash when the
//! device sleeps. Alongside it the save keeps the material a snapshot is
//! built from and the things other tools want:
//!
//! - `save.toml`      metadata (name, character, timestamps)
//! - `otp.bin`        the boot ROM the save was created with
//! - `flash.bin`      the latest flash dump — exportable, USB-tool friendly
//! - `state.snapshot` the latest autosnapshot
//! - `thumb.png`      the last interesting (non-black) LCD frame
//!
//! The emulator session rewrites `state.snapshot`/`flash.bin`/`thumb.png`
//! (see [`crate::emu`]); the UI owns `save.toml`. All writes go through
//! [`write_atomic`] so a crash never leaves a truncated file.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const META_FILE: &str = "save.toml";
pub const OTP_FILE: &str = "otp.bin";
pub const FLASH_FILE: &str = "flash.bin";
pub const SNAPSHOT_FILE: &str = "state.snapshot";
pub const THUMB_FILE: &str = "thumb.png";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SaveMeta {
    pub name: String,
    pub character: String,
    pub firmware_version: String,
    pub created_unix: u64,
    pub last_played_unix: u64,
    pub play_seconds: u64,
}

impl Default for SaveMeta {
    fn default() -> Self {
        Self {
            name: "Miuchiz".to_owned(),
            character: String::new(),
            firmware_version: String::new(),
            created_unix: 0,
            last_played_unix: 0,
            play_seconds: 0,
        }
    }
}

/// A save as listed in the library.
#[derive(Clone)]
pub struct SaveSlot {
    /// The directory name; stable for the save's lifetime.
    pub id: String,
    pub dir: PathBuf,
    pub meta: SaveMeta,
    /// False only before the first autosnapshot (a brand-new save boots
    /// from flash instead).
    pub has_snapshot: bool,
}

impl SaveSlot {
    pub fn path(&self, file: &str) -> PathBuf {
        self.dir.join(file)
    }

    pub fn thumb_path(&self) -> Option<PathBuf> {
        let path = self.path(THUMB_FILE);
        path.exists().then_some(path)
    }

    pub fn write_meta(&self) -> io::Result<()> {
        let text = toml::to_string_pretty(&self.meta)
            .map_err(|why| io::Error::new(io::ErrorKind::InvalidData, why))?;
        write_atomic(&self.path(META_FILE), text.as_bytes())
    }
}

pub struct Library {
    root: PathBuf,
}

impl Library {
    /// Opens the default library under the shared storage policy.
    pub fn open() -> Self {
        let root = miuchiz_reborn_paths::AppDirs::new("emiu2-desktop")
            .data_dir()
            .join("saves");
        Self { root }
    }

    #[cfg(test)]
    pub fn open_at(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every save, most recently played first.
    pub fn list(&self) -> Vec<SaveSlot> {
        let mut slots = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return slots;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            let Some(slot) = load_slot(&dir) else {
                continue;
            };
            slots.push(slot);
        }
        slots.sort_by_key(|slot| {
            std::cmp::Reverse((slot.meta.last_played_unix, slot.meta.created_unix))
        });
        slots
    }

    pub fn get(&self, id: &str) -> Option<SaveSlot> {
        load_slot(&self.root.join(id))
    }

    /// Creates a save from an OTP and flash image. The images are copied in;
    /// the first snapshot arrives once the save has been played.
    pub fn create(
        &self,
        name: &str,
        character: &str,
        firmware_version: &str,
        otp: &[u8],
        flash: &[u8],
    ) -> io::Result<SaveSlot> {
        let now = unix_now();
        let id = unique_id(&self.root, name, now);
        let dir = self.root.join(&id);
        std::fs::create_dir_all(&dir)?;

        let slot = SaveSlot {
            id,
            dir,
            meta: SaveMeta {
                name: name.to_owned(),
                character: character.to_owned(),
                firmware_version: firmware_version.to_owned(),
                created_unix: now,
                last_played_unix: now,
                play_seconds: 0,
            },
            has_snapshot: false,
        };
        write_atomic(&slot.path(OTP_FILE), otp)?;
        write_atomic(&slot.path(FLASH_FILE), flash)?;
        slot.write_meta()?;
        Ok(slot)
    }

    /// Permanently deletes a save and everything in it.
    pub fn delete(&self, slot: &SaveSlot) -> io::Result<()> {
        // Refuse to remove anything that doesn't look like one of ours.
        if !slot.dir.starts_with(&self.root) || !slot.path(META_FILE).exists() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a save directory",
            ));
        }
        std::fs::remove_dir_all(&slot.dir)
    }
}

fn load_slot(dir: &Path) -> Option<SaveSlot> {
    let meta_text = std::fs::read_to_string(dir.join(META_FILE)).ok()?;
    let meta: SaveMeta = toml::from_str(&meta_text).ok()?;
    Some(SaveSlot {
        id: dir.file_name()?.to_string_lossy().into_owned(),
        dir: dir.to_path_buf(),
        meta,
        has_snapshot: dir.join(SNAPSHOT_FILE).exists(),
    })
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A filesystem-safe directory name from the save's display name, made
/// unique with the creation time (and a counter, should two saves be
/// created the same second).
fn unique_id(root: &Path, name: &str, now: u64) -> String {
    let slug: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .to_lowercase();
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "save" } else { slug };
    let base = format!("{slug}-{now}");
    let mut id = base.clone();
    let mut counter = 1;
    while root.join(&id).exists() {
        id = format!("{base}-{counter}");
        counter += 1;
    }
    id
}

/// Writes via a temporary sibling then renames over the target, so readers
/// (and crashes) never observe a half-written file.
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_library(name: &str) -> Library {
        let root = std::env::temp_dir()
            .join(format!("emiu2-desktop-test-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        Library::open_at(root)
    }

    #[test]
    fn create_list_delete_round_trip() {
        let library = temp_library("crud");
        let slot = library
            .create("Spike!", "Spike", "1.09.03", &[1, 2], &[3, 4])
            .unwrap();
        assert!(slot.id.starts_with("spike-"));
        assert!(!slot.has_snapshot);

        let listed = library.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].meta.character, "Spike");
        assert_eq!(std::fs::read(listed[0].path(FLASH_FILE)).unwrap(), [3, 4]);

        library.delete(&listed[0]).unwrap();
        assert!(library.list().is_empty());
        std::fs::remove_dir_all(library.root()).ok();
    }

    #[test]
    fn same_second_names_stay_unique() {
        let library = temp_library("unique");
        let a = library.create("Pet", "Cloe", "1.09.03", &[], &[]).unwrap();
        let b = library.create("Pet", "Cloe", "1.09.03", &[], &[]).unwrap();
        assert_ne!(a.id, b.id);
        std::fs::remove_dir_all(library.root()).ok();
    }

    #[test]
    fn stray_directories_are_not_saves() {
        let library = temp_library("stray");
        std::fs::create_dir_all(library.root().join("not-a-save")).unwrap();
        assert!(library.list().is_empty());
        let stray = SaveSlot {
            id: "not-a-save".into(),
            dir: library.root().join("not-a-save"),
            meta: SaveMeta::default(),
            has_snapshot: false,
        };
        assert!(library.delete(&stray).is_err());
        std::fs::remove_dir_all(library.root()).ok();
    }
}
