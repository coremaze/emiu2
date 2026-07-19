//! Same-machine discovery for the IR link: each emulator in local link
//! mode advertises its loopback TCP listener as a small file in a shared
//! runtime directory, and scans that directory to find the others.
//!
//! The directory follows the shared Miuchiz Reborn storage-location
//! policy (miuchiz-reborn-paths), like the USB discovery endpoints, so
//! every instance agrees on it without configuration and
//! `MIUCHIZ_REBORN_HOME` reroots everything at once. `EMIU2_IR_DIR` is a
//! narrower, higher-priority override of just this directory.
//!
//! An advert is `key=value` lines (`port`, `pid`, `name`). Liveness is
//! the file's mtime: the advertiser rewrites its file every
//! [`REFRESH_INTERVAL`], and scanners ignore files that stopped
//! refreshing. Probing the advertised port would instead consume the
//! listener's accept and disturb a forming link. A clean exit removes
//! the file; a crash leaves it to age out (and be pruned).

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// How often an advert's mtime is refreshed.
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Adverts older than this are considered dead by scanners. Generous
/// against clock skew and stalled machines; a wrongly-listed peer only
/// costs a failed link attempt.
const STALE_AFTER: Duration = Duration::from_secs(20);

/// Adverts older than this are deleted by the next advertiser to start.
const PRUNE_AFTER: Duration = Duration::from_secs(60);

/// The directory adverts live in.
pub fn advert_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("EMIU2_IR_DIR") {
        return PathBuf::from(dir);
    }
    miuchiz_reborn_paths::AppDirs::new("emiu2")
        .runtime_dir()
        .join("ir")
}

/// This emulator's advert file. Dropping it withdraws the advert.
pub struct Advert {
    path: PathBuf,
    content: String,
    written: Instant,
}

impl Advert {
    /// Advertises a listener on `127.0.0.1:port` in the default
    /// [`advert_dir`].
    pub fn create(name: &str, port: u16) -> io::Result<Self> {
        Self::create_in(&advert_dir(), name, port)
    }

    pub fn create_in(dir: &Path, name: &str, port: u16) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        prune_stale(dir);
        let pid = std::process::id();
        let path = dir.join(format!("{pid}.advert"));
        // The format is line-based; a name can hold anything else.
        let name = name.replace(['\r', '\n'], " ");
        let content = format!("port={port}\npid={pid}\nname={name}\n");
        std::fs::write(&path, &content)?;
        Ok(Self {
            path,
            content,
            written: Instant::now(),
        })
    }

    /// Keeps the advert alive; call regularly (any pace faster than
    /// [`STALE_AFTER`]). Rewrites the file once per [`REFRESH_INTERVAL`]
    /// so its mtime shows the emulator is still here.
    pub fn refresh(&mut self) {
        if self.written.elapsed() < REFRESH_INTERVAL {
            return;
        }
        std::fs::write(&self.path, &self.content).ok();
        self.written = Instant::now();
    }
}

impl Drop for Advert {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}

/// Another emulator's advertised IR listener.
#[derive(Debug, Clone)]
pub struct LocalPeer {
    pub name: String,
    pub pid: u32,
    pub port: u16,
}

impl LocalPeer {
    pub fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

/// The live adverts of other processes in the default [`advert_dir`],
/// sorted by name.
pub fn scan() -> Vec<LocalPeer> {
    scan_in(&advert_dir())
}

pub fn scan_in(dir: &Path) -> Vec<LocalPeer> {
    let own_pid = std::process::id();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut peers: Vec<LocalPeer> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("advert") {
                return None;
            }
            if age(&path)? > STALE_AFTER {
                return None;
            }
            let peer = parse(&std::fs::read_to_string(&path).ok()?)?;
            (peer.pid != own_pid).then_some(peer)
        })
        .collect();
    peers.sort_by(|a, b| a.name.cmp(&b.name).then(a.pid.cmp(&b.pid)));
    peers
}

fn parse(text: &str) -> Option<LocalPeer> {
    let mut port = None;
    let mut pid = None;
    let mut name = None;
    for line in text.lines() {
        let (key, value) = line.split_once('=')?;
        match key {
            "port" => port = Some(value.parse().ok()?),
            "pid" => pid = Some(value.parse().ok()?),
            "name" => name = Some(value.to_owned()),
            // Unknown keys are fine: a newer emulator may say more.
            _ => {}
        }
    }
    Some(LocalPeer {
        name: name?,
        pid: pid?,
        port: port?,
    })
}

fn age(path: &Path) -> Option<Duration> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    // A future mtime (the clock stepped backwards under NTP or a resume
    // from suspend) means the file was just written, not that it is dead.
    // Treating `duration_since`'s error as "unknown age" would let a scan
    // hide a live advert and let `prune_stale` delete it, so clamp to now.
    Some(
        SystemTime::now()
            .duration_since(modified)
            .unwrap_or(Duration::ZERO),
    )
}

/// Removes adverts whose emulator crashed without cleaning up.
fn prune_stale(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("advert") {
            continue;
        }
        if age(&path).is_none_or(|age| age > PRUNE_AFTER) {
            std::fs::remove_file(&path).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advert_round_trips_through_scan() {
        let dir = std::env::temp_dir().join(format!("emiu2-ir-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let advert = Advert::create_in(&dir, "Roc = the first", 43210).unwrap();

        // Our own advert is filtered out by pid; forge another pid's.
        let text = std::fs::read_to_string(&advert.path).unwrap();
        let forged = text.replace(
            &format!("pid={}", std::process::id()),
            &format!("pid={}", u32::MAX),
        );
        std::fs::write(dir.join("4294967295.advert"), forged).unwrap();

        let peers = scan_in(&dir);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].name, "Roc = the first");
        assert_eq!(peers[0].port, 43210);
        assert_eq!(peers[0].addr(), "127.0.0.1:43210");

        drop(advert);
        assert_eq!(scan_in(&dir).len(), 1, "dropping only removes our own");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
