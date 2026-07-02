//! TCP transport for the IR link, connecting two emulators on the same
//! machine or across a network.
//!
//! The wire protocol is a stream of envelope edges: 9-byte records of
//! `(u64 LE sender time in emulated nanoseconds, u8 carrier level)`,
//! after an 8-byte magic handshake in each direction.
//!
//! Network jitter would corrupt a frame if edges were replayed as they
//! arrived: within a frame the firmware free-runs on its 8192Hz tick and
//! tolerates only a fraction of a bit time (~1ms) of skew. Edges are
//! therefore replayed per burst: the first edge after a gap anchors the
//! burst at `now + REPLAY_LATENCY_NS` on the local timeline, and the rest
//! of the burst keeps its exact sender-relative spacing. Only the idle
//! gaps between frames stretch, which the protocol does not care about.
//!
//! # Rollback
//!
//! The firmware listens for a reply for only ~98ms after transmitting
//! (and cannot hear while retransmitting), so on links slower than
//! ~30ms round trip, replies always arrive after the window and the
//! handshake livelocks. The transport recovers by cooperating with the
//! platform's run loop through [`IrRollbackControl`]: a machine snapshot
//! is taken each time the receiver powers on, and when a frame arrives
//! that cannot be heard live — a new burst while the receiver is off, or
//! the receiver powering off with a burst still replaying — the machine
//! is rolled back to that snapshot and the frame is replayed early in
//! the restored listen window.
//!
//! Everything transmitted streams to the wire immediately; a rollback
//! never retracts wire history. Whatever the abandoned timeline sent was
//! by construction a retransmission or beacon (the timeout path is the
//! only one that transmits after a silent window), which the firmware's
//! own loss recovery already tolerates — the same duplicates arise on
//! real hardware when two blind half-duplex devices cross transmissions.
//! Wire timestamps stay monotonic across rollbacks via a sender-side
//! offset, so the peer never needs to know one happened.

use crate::ir::{IrInterface, IrRollbackControl, RollbackDirective};
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAGIC: [u8; 8] = *b"EMIU2IR\x01";
const RECORD_LEN: usize = 9;

/// A gap this long in the sender's timeline separates bursts (frames).
/// The longest in-frame carrier-off run is one bit time, ~1ms.
const BURST_GAP_NS: u64 = 3_000_000;

/// How far into the local future the start of a burst is scheduled. This
/// is the jitter budget: edges arriving up to this much later than the
/// burst's first edge still replay with exact relative timing.
///
/// It also delays every frame. Live delivery works while two of these
/// plus the network round trip fit into the firmware's ~98ms listen
/// window; beyond that, rollback takes over.
const REPLAY_LATENCY_NS: u64 = 10_000_000;

/// A snapshot older than this cannot be rolled back to: past a couple of
/// seconds the peer's own retransmissions are the better recovery.
const SNAPSHOT_MAX_AGE_NS: u64 = 2_000_000_000;

/// Placeholder until `set_clock_rate` is called.
const DEFAULT_CLOCK_RATE: u64 = 16_000_000;

/// How long a reconnect waits after a failed attempt or lost connection.
const RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Backstop against unbounded growth if the emulator stops polling.
const MAX_QUEUED_EDGES: usize = 100_000;

struct Shared {
    /// Received edges: (sender time in ns, carrier level).
    edges: Mutex<VecDeque<(u64, bool)>>,
    /// Bumped on every new connection so replay state can be reset.
    generation: AtomicU64,
    connected: AtomicBool,
}

enum Endpoint {
    Listen(TcpListener),
    Connect(String),
}

/// Replay and rollback state. Shared between the transport (driven from
/// inside the machine) and the rollback handle (driven by the run loop);
/// both run on the emulator thread, so the mutex is never contended.
struct ReplayState {
    clock_rate: u64,
    seen_generation: u64,

    /// The receiver's current demodulated level.
    level: bool,
    /// Undelivered edges: (local delivery cycle, sender ns, level).
    play_queue: VecDeque<(u64, u64, bool)>,
    last_sender_ns: Option<u64>,
    /// (sender ns, local cycle) correspondence for the current burst.
    anchor: Option<(u64, u64)>,
    last_scheduled_cycle: u64,

    /// Already-delivered edges of the burst currently replaying, in
    /// sender time, retained so an interrupted burst can be replayed
    /// whole after a rollback.
    delivered_burst: Vec<(u64, bool)>,

    /// Receiver power as reported by the board (PB6).
    rx_powered: bool,
    /// Edges (sender ns, level) waiting to be delivered into a restored
    /// or freshly opened listen window instead of live.
    held: Vec<(u64, bool)>,
    /// While set, drained edges accumulate into `held`.
    holding: bool,

    want_snapshot: bool,
    armed_snapshot_cycle: Option<u64>,
    rollback_requested: bool,

    /// Added to outgoing timestamps; grows by the rewound span on each
    /// rollback so wire time never runs backwards.
    wire_offset_ns: u64,
    last_sent_level: bool,
}

impl ReplayState {
    fn new() -> Self {
        Self {
            clock_rate: DEFAULT_CLOCK_RATE,
            seen_generation: 0,
            level: false,
            play_queue: VecDeque::new(),
            last_sender_ns: None,
            anchor: None,
            last_scheduled_cycle: 0,
            delivered_burst: Vec::new(),
            // PB6 idles high until the firmware drives it.
            rx_powered: true,
            held: Vec::new(),
            holding: false,
            want_snapshot: false,
            armed_snapshot_cycle: None,
            rollback_requested: false,
            wire_offset_ns: 0,
            last_sent_level: false,
        }
    }

    fn cycles_to_ns(&self, cycles: u64) -> u64 {
        (cycles as u128 * 1_000_000_000 / self.clock_rate as u128) as u64
    }

    fn ns_to_cycles(&self, ns: u64) -> u64 {
        (ns as u128 * self.clock_rate as u128 / 1_000_000_000) as u64
    }

    fn snapshot_is_fresh(&self, now_cycle: u64) -> bool {
        match self.armed_snapshot_cycle {
            Some(at) => now_cycle.saturating_sub(at) < self.ns_to_cycles(SNAPSHOT_MAX_AGE_NS),
            None => false,
        }
    }

    /// Moves received edges into the replay queue (or the held buffer,
    /// when they can only be heard through a rollback).
    fn drain_incoming(&mut self, cycle: u64, shared: &Shared) {
        let generation = shared.generation.load(Ordering::Relaxed);
        if generation != self.seen_generation {
            self.seen_generation = generation;
            self.play_queue.clear();
            self.level = false;
            self.last_sender_ns = None;
            self.anchor = None;
            self.delivered_burst.clear();
            self.held.clear();
            self.holding = false;
            self.rollback_requested = false;
            self.armed_snapshot_cycle = None;
        }

        let mut edges = shared.edges.lock().unwrap();
        while let Some((ns, level)) = edges.pop_front() {
            let new_burst = match self.last_sender_ns {
                None => true,
                Some(previous) => ns.saturating_sub(previous) > BURST_GAP_NS,
            };
            self.last_sender_ns = Some(ns);

            if new_burst && !self.holding {
                self.delivered_burst.clear();
                if !self.rx_powered && self.snapshot_is_fresh(cycle) {
                    // The receiver cannot hear this frame; hold it and
                    // ask for a rollback into the last listen window.
                    self.holding = true;
                    self.held.clear();
                    self.rollback_requested = true;
                } else {
                    self.anchor = Some((ns, cycle + self.ns_to_cycles(REPLAY_LATENCY_NS)));
                }
            }

            if self.holding {
                if self.held.len() < MAX_QUEUED_EDGES {
                    self.held.push((ns, level));
                }
                continue;
            }

            let (anchor_ns, anchor_cycle) = self.anchor.unwrap();
            let mut at = anchor_cycle + self.ns_to_cycles(ns.saturating_sub(anchor_ns));
            if at <= self.last_scheduled_cycle {
                at = self.last_scheduled_cycle + 1;
            }
            self.last_scheduled_cycle = at;
            self.play_queue.push_back((at, ns, level));
        }
    }

    /// Delivers due edges and returns the receiver's current level.
    fn deliver(&mut self, cycle: u64) -> bool {
        while let Some(&(at, ns, level)) = self.play_queue.front() {
            if at > cycle {
                break;
            }
            self.level = level;
            if self.delivered_burst.len() < MAX_QUEUED_EDGES {
                self.delivered_burst.push((ns, level));
            }
            self.play_queue.pop_front();
        }
        self.level
    }

    fn receiver_power_changed(&mut self, cycle: u64, powered: bool, connected: bool) {
        if powered == self.rx_powered {
            return;
        }
        self.rx_powered = powered;

        if powered {
            // A listen window opened: this is the snapshot point. Any
            // held frame no longer needs a rollback — it can be heard
            // live in this window.
            if connected {
                self.want_snapshot = true;
            }
            if self.holding {
                self.deliver_held(cycle);
            }
        } else if !self.play_queue.is_empty() && self.snapshot_is_fresh(cycle) {
            // The window closed with a frame still replaying: the
            // firmware heard at most a truncated prefix. Retain the
            // whole burst and ask to replay it into the reopened
            // window.
            self.holding = true;
            self.held.clear();
            self.held.append(&mut self.delivered_burst);
            while let Some((_, ns, level)) = self.play_queue.pop_front() {
                if self.held.len() < MAX_QUEUED_EDGES {
                    self.held.push((ns, level));
                }
            }
            self.level = false;
            self.rollback_requested = true;
        }
    }

    /// Schedules the held edges to replay starting `REPLAY_LATENCY_NS`
    /// after `start_cycle`, dropping whatever else was scheduled (the
    /// sender's retransmissions cover anything lost).
    fn deliver_held(&mut self, start_cycle: u64) {
        self.play_queue.clear();
        self.level = false;
        self.delivered_burst.clear();
        self.last_scheduled_cycle = start_cycle;

        let held = std::mem::take(&mut self.held);
        if let Some(&(first_ns, _)) = held.first() {
            let anchor_cycle = start_cycle + self.ns_to_cycles(REPLAY_LATENCY_NS);
            self.anchor = Some((first_ns, anchor_cycle));
            for (ns, level) in held {
                let mut at = anchor_cycle + self.ns_to_cycles(ns.saturating_sub(first_ns));
                if at <= self.last_scheduled_cycle {
                    at = self.last_scheduled_cycle + 1;
                }
                self.last_scheduled_cycle = at;
                self.play_queue.push_back((at, ns, level));
            }
        }

        self.holding = false;
        self.rollback_requested = false;
    }
}

pub struct SocketIr {
    outgoing: mpsc::Sender<(u64, bool)>,
    shared: Arc<Shared>,
    local_addr: Option<SocketAddr>,
    replay: Arc<Mutex<ReplayState>>,
}

impl SocketIr {
    /// Binds immediately (so address errors surface here) and accepts
    /// peers in the background, one at a time.
    pub fn listen(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let local_addr = listener.local_addr().ok();
        Ok(Self::start(Endpoint::Listen(listener), local_addr))
    }

    /// Connects to a listening peer, retrying in the background until it
    /// succeeds.
    pub fn connect(addr: impl Into<String>) -> Self {
        Self::start(Endpoint::Connect(addr.into()), None)
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    pub fn connected(&self) -> bool {
        self.shared.connected.load(Ordering::Relaxed)
    }

    /// The run loop's handle for rollback coordination. High-latency
    /// links only work when the platform drives this.
    pub fn rollback_handle(&self) -> SocketIrRollback {
        SocketIrRollback {
            replay: self.replay.clone(),
            outgoing: self.outgoing.clone(),
            shared: self.shared.clone(),
        }
    }

    fn start(endpoint: Endpoint, local_addr: Option<SocketAddr>) -> Self {
        let (outgoing, outgoing_rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            edges: Mutex::new(VecDeque::new()),
            generation: AtomicU64::new(0),
            connected: AtomicBool::new(false),
        });

        let thread_shared = shared.clone();
        std::thread::spawn(move || connection_loop(endpoint, outgoing_rx, thread_shared));

        Self {
            outgoing,
            shared,
            local_addr,
            replay: Arc::new(Mutex::new(ReplayState::new())),
        }
    }
}

impl IrInterface for SocketIr {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64) {
        if emulated_clock_rate > 0 {
            self.replay.lock().unwrap().clock_rate = emulated_clock_rate;
        }
    }

    fn set_carrier(&mut self, cycle: u64, carrier: bool) {
        // Without a peer the edge just goes out into the void, and queuing
        // it would only replay it, stale, after a connection arrives.
        if !self.connected() {
            return;
        }
        let mut replay = self.replay.lock().unwrap();
        replay.last_sent_level = carrier;
        let wire_ns = replay.cycles_to_ns(cycle) + replay.wire_offset_ns;
        drop(replay);
        let _ = self.outgoing.send((wire_ns, carrier));
    }

    fn carrier_detected(&mut self, cycle: u64) -> bool {
        let mut replay = self.replay.lock().unwrap();
        replay.drain_incoming(cycle, &self.shared);
        replay.deliver(cycle)
    }

    fn set_receiver_power(&mut self, cycle: u64, powered: bool) {
        let connected = self.connected();
        self.replay
            .lock()
            .unwrap()
            .receiver_power_changed(cycle, powered, connected);
    }
}

/// The [`IrRollbackControl`] endpoint of a [`SocketIr`], held by the
/// platform's run loop.
pub struct SocketIrRollback {
    replay: Arc<Mutex<ReplayState>>,
    outgoing: mpsc::Sender<(u64, bool)>,
    shared: Arc<Shared>,
}

impl IrRollbackControl for SocketIrRollback {
    fn poll(&mut self, now_cycle: u64) -> RollbackDirective {
        let mut replay = self.replay.lock().unwrap();

        if replay.rollback_requested {
            if replay.snapshot_is_fresh(now_cycle) {
                // One rollback per snapshot: re-arming requires a new
                // receiver-on snapshot. `holding` stays set so edges of
                // the held burst that are still arriving keep
                // accumulating until `rolled_back` schedules them.
                replay.armed_snapshot_cycle = None;
                replay.rollback_requested = false;
                return RollbackDirective::RollBack;
            }
            // Too stale to rewind; let the frame play (unheard) and rely
            // on the peer's retransmissions.
            replay.deliver_held(now_cycle);
        }

        if replay.want_snapshot {
            replay.want_snapshot = false;
            return RollbackDirective::TakeSnapshot;
        }

        RollbackDirective::Continue
    }

    fn snapshot_taken(&mut self, cycle: u64) {
        self.replay.lock().unwrap().armed_snapshot_cycle = Some(cycle);
    }

    fn rolled_back(&mut self, restored_cycle: u64, abandoned_cycle: u64) {
        let mut replay = self.replay.lock().unwrap();

        // Wire time must not run backwards: absorb the rewound span into
        // the outgoing offset. If the abandoned timeline left the carrier
        // on (a rollback mid-retransmission), close the dangling frame;
        // the peer sees a truncated burst and discards it on checksum.
        let rewound = abandoned_cycle.saturating_sub(restored_cycle);
        let off_ns = replay.cycles_to_ns(abandoned_cycle) + replay.wire_offset_ns;
        replay.wire_offset_ns += replay.cycles_to_ns(rewound);
        if replay.last_sent_level && self.shared.connected.load(Ordering::Relaxed) {
            replay.last_sent_level = false;
            let _ = self.outgoing.send((off_ns, false));
        }

        // Snapshots are only taken while the receiver is on.
        replay.rx_powered = true;
        replay.deliver_held(restored_cycle);
    }
}

/// Whether the connection loop should keep running after a connection
/// ended.
enum After {
    Reconnect,
    Shutdown,
}

fn connection_loop(endpoint: Endpoint, outgoing: mpsc::Receiver<(u64, bool)>, shared: Arc<Shared>) {
    loop {
        let stream = match &endpoint {
            Endpoint::Listen(listener) => match listener.accept() {
                Ok((stream, peer)) => {
                    eprintln!("IR: peer connected from {peer}");
                    stream
                }
                Err(why) => {
                    eprintln!("IR: accept failed: {why}");
                    std::thread::sleep(RETRY_INTERVAL);
                    continue;
                }
            },
            Endpoint::Connect(addr) => match TcpStream::connect(addr.as_str()) {
                Ok(stream) => {
                    eprintln!("IR: connected to {addr}");
                    stream
                }
                Err(_) => {
                    // The peer may simply not be up yet; retry quietly.
                    std::thread::sleep(RETRY_INTERVAL);
                    continue;
                }
            },
        };

        let after = match run_connection(stream, &outgoing, &shared) {
            Ok(after) => after,
            Err(why) => {
                eprintln!("IR: connection lost: {why}");
                After::Reconnect
            }
        };
        shared.connected.store(false, Ordering::Relaxed);

        match after {
            After::Reconnect => std::thread::sleep(RETRY_INTERVAL),
            After::Shutdown => return,
        }
    }
}

fn run_connection(
    mut stream: TcpStream,
    outgoing: &mpsc::Receiver<(u64, bool)>,
    shared: &Shared,
) -> std::io::Result<After> {
    stream.set_nodelay(true).ok();

    // Handshake, with timeouts so a bad peer cannot hang us.
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(&MAGIC)?;
    let mut peer_magic = [0u8; 8];
    stream.read_exact(&mut peer_magic)?;
    if peer_magic != MAGIC {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "peer is not an emiu2 IR endpoint",
        ));
    }

    // Edges transmitted while unconnected belong to frames that are
    // already lost; replaying them late would only produce garbage.
    while outgoing.try_recv().is_ok() {}
    shared.edges.lock().unwrap().clear();
    shared.generation.fetch_add(1, Ordering::Relaxed);
    shared.connected.store(true, Ordering::Relaxed);

    // The 1ms read timeout paces the loop: sending waits at most 1ms,
    // well within the replay latency budget.
    stream.set_read_timeout(Some(Duration::from_millis(1)))?;

    let mut inbuf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        loop {
            match outgoing.try_recv() {
                Ok((ns, level)) => {
                    let mut record = [0u8; RECORD_LEN];
                    record[..8].copy_from_slice(&ns.to_le_bytes());
                    record[8] = level as u8;
                    stream.write_all(&record)?;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(After::Shutdown),
            }
        }

        match stream.read(&mut chunk) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "peer disconnected",
                ))
            }
            Ok(n) => {
                inbuf.extend_from_slice(&chunk[..n]);
                let mut edges = shared.edges.lock().unwrap();
                let mut consumed = 0;
                while inbuf.len() - consumed >= RECORD_LEN {
                    let record = &inbuf[consumed..consumed + RECORD_LEN];
                    let ns = u64::from_le_bytes(record[..8].try_into().unwrap());
                    let level = match record[8] {
                        0 => false,
                        1 => true,
                        _ => {
                            return Err(std::io::Error::new(
                                ErrorKind::InvalidData,
                                "malformed IR record",
                            ))
                        }
                    };
                    if edges.len() >= MAX_QUEUED_EDGES {
                        edges.pop_front();
                    }
                    edges.push_back((ns, level));
                    consumed += RECORD_LEN;
                }
                inbuf.drain(..consumed);
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
            Err(e) => return Err(e),
        }
    }
}
