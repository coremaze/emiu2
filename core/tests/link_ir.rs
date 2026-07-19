//! Integration test of the switchable [`LinkIr`] transport: local
//! advertising and direct linking, online pairing through an in-process
//! relay, and switching between the two modes at runtime.
//!
//! Everything here drives the transport through its commander, so no
//! firmware images are needed (the machine-level IR behavior over
//! sockets and the relay is covered by `ir_integration.rs`).

use emiu2::platform::link_ir::{DialState, LinkIr, LinkMode, LinkModeKind};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// `EMIU2_IR_DIR` is process-global, so the tests that set it must not
/// overlap. Held for each test's duration; poison is ignored since the
/// guarded state is only the environment.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn advert_port(path: &Path) -> u16 {
    let text = std::fs::read_to_string(path).expect("could not read the advert");
    text.lines()
        .find_map(|line| line.strip_prefix("port="))
        .expect("advert has no port")
        .parse()
        .expect("advert port is not a number")
}

/// One test covers the whole local/online/switch lifecycle, since spinning
/// up transports and pairing them is the expensive part.
#[test]
fn links_locally_pairs_online_and_switches_modes() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("emiu2-link-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::env::set_var("EMIU2_IR_DIR", &dir);

    // The responder comes up local and advertises. Its port is read from
    // the advert file directly: a scan would filter out our own pid, and
    // both ends of this test live in one process.
    let responder = LinkIr::start(LinkMode::Local, "Responder".to_owned());
    let responder_cmd = responder.commander();
    let advert_path = dir.join(format!("{}.advert", std::process::id()));
    wait_for("the responder's advert", || advert_path.exists());
    let port = advert_port(&advert_path);

    let requester = LinkIr::start(LinkMode::Local, "Requester".to_owned());
    let requester_cmd = requester.commander();
    assert_eq!(requester_cmd.mode(), LinkModeKind::Local);

    // Dial the advertised listener; both sides link up.
    requester_cmd.connect_peer(format!("127.0.0.1:{port}"));
    wait_for("the local link", || {
        requester_cmd.linked() && responder_cmd.linked()
    });
    assert_eq!(requester_cmd.dial(), DialState::Idle);
    // Linked emulators withdraw their adverts (no third window can join).
    wait_for("the adverts to withdraw", || !advert_path.exists());

    // Unlink: both fall back to being discoverable.
    requester_cmd.disconnect_peer();
    wait_for("the unlink", || {
        !requester_cmd.linked() && !responder_cmd.linked()
    });
    wait_for("the advert to return", || advert_path.exists());

    // A dial to a dead port fails and says so.
    let dead = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let dead_port = dead.local_addr().unwrap().port();
    drop(dead);
    requester_cmd.connect_peer(format!("127.0.0.1:{dead_port}"));
    wait_for("the dial to fail", || {
        requester_cmd.dial() == DialState::Failed
    });

    // Both switch online to an in-process relay and pair by friend code.
    let relay_addr = emiu2_relay::RelayServer::bind(("127.0.0.1", 0))
        .expect("could not bind relay")
        .spawn();
    let relay = format!("127.0.0.1:{}", relay_addr.port());
    requester_cmd.set_online(&relay);
    responder_cmd.set_online(&relay);
    wait_for("both friend codes", || {
        requester_cmd.code().is_some() && responder_cmd.code().is_some()
    });
    assert_eq!(requester_cmd.mode(), LinkModeKind::Online);
    assert!(requester_cmd.relay_connected());
    requester_cmd.join(responder_cmd.code().unwrap());
    wait_for("the online pairing", || {
        requester_cmd.linked() && responder_cmd.linked()
    });

    // Back to local: the pairing dissolves (the peer sees it too) and
    // discovery resumes.
    requester_cmd.set_local();
    wait_for("the mode switch", || {
        requester_cmd.mode() == LinkModeKind::Local
    });
    wait_for("the pairing to dissolve", || {
        !requester_cmd.linked() && !responder_cmd.linked()
    });
    assert!(requester_cmd.code().is_none());
    wait_for("the advert to return", || advert_path.exists());

    // Dropping the transports withdraws the advert and stops the threads.
    drop(requester);
    drop(responder);
    wait_for("the advert to be withdrawn", || !advert_path.exists());
    std::env::remove_var("EMIU2_IR_DIR");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Switching to online with an empty relay must leave local discovery —
/// withdraw the advert and idle disconnected — rather than silently stay
/// discoverable while the UI reports "no relay server set". One transport,
/// so the advert file is unambiguously this one's.
#[test]
fn online_with_empty_relay_leaves_local_discovery() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("emiu2-link-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::env::set_var("EMIU2_IR_DIR", &dir);

    let link = LinkIr::start(LinkMode::Local, "Lonely".to_owned());
    let cmd = link.commander();
    let advert_path = dir.join(format!("{}.advert", std::process::id()));
    wait_for("the advert", || advert_path.exists());

    cmd.set_online("");
    wait_for("the advert to withdraw", || !advert_path.exists());
    assert_eq!(cmd.mode(), LinkModeKind::Online);
    assert!(!cmd.relay_connected(), "an empty relay must not connect");

    // Back to local resumes discovery.
    cmd.set_local();
    wait_for("discovery to resume", || advert_path.exists());

    drop(link);
    std::env::remove_var("EMIU2_IR_DIR");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Dropping a transport whose connection thread is mid-dial to an
/// unreachable host must not block for the full connect timeout: the
/// shutdown flag aborts the connect promptly.
#[test]
fn drop_does_not_block_on_a_stuck_dial() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("emiu2-link-drop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::env::set_var("EMIU2_IR_DIR", &dir);

    let link = LinkIr::start(LinkMode::Local, "Dialer".to_owned());
    let cmd = link.commander();

    // 203.0.113.0/24 (TEST-NET-3) is reserved and non-routable, so the
    // connect blocks until it times out rather than being refused.
    cmd.connect_peer("203.0.113.1:5885");
    wait_for("the dial to start", || cmd.dial() == DialState::Dialing);

    let start = Instant::now();
    drop(link);
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "drop blocked {:?} on a stuck dial (DIAL_TIMEOUT is 3s)",
        start.elapsed()
    );

    std::env::remove_var("EMIU2_IR_DIR");
    let _ = std::fs::remove_dir_all(&dir);
}
