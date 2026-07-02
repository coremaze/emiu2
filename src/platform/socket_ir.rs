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

use crate::ir::IrInterface;
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
/// It also delays every frame, and the firmware is impatient: after
/// transmitting, it listens for a reply for only ~98ms before retrying
/// (during which it cannot hear). A reply spends this budget twice (once
/// per direction) plus ~44ms on the air and in processing, so much beyond
/// ~25ms here makes handshakes miss the window forever.
const REPLAY_LATENCY_NS: u64 = 10_000_000;

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

pub struct SocketIr {
    clock_rate: u64,
    outgoing: mpsc::Sender<(u64, bool)>,
    shared: Arc<Shared>,
    local_addr: Option<SocketAddr>,

    // Replay state, touched only from the emulator thread.
    seen_generation: u64,
    level: bool,
    play_queue: VecDeque<(u64, bool)>,
    last_sender_ns: Option<u64>,
    /// (sender ns, local cycle) correspondence for the current burst.
    anchor: Option<(u64, u64)>,
    last_scheduled_cycle: u64,
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
            clock_rate: DEFAULT_CLOCK_RATE,
            outgoing,
            shared,
            local_addr,
            seen_generation: 0,
            level: false,
            play_queue: VecDeque::new(),
            last_sender_ns: None,
            anchor: None,
            last_scheduled_cycle: 0,
        }
    }

    fn cycles_to_ns(&self, cycles: u64) -> u64 {
        (cycles as u128 * 1_000_000_000 / self.clock_rate as u128) as u64
    }

    fn ns_to_cycles(&self, ns: u64) -> u64 {
        (ns as u128 * self.clock_rate as u128 / 1_000_000_000) as u64
    }

    /// Moves received edges onto the local replay queue, anchoring each
    /// burst `REPLAY_LATENCY_NS` into the local future.
    fn drain_incoming(&mut self, cycle: u64) {
        let generation = self.shared.generation.load(Ordering::Relaxed);
        if generation != self.seen_generation {
            self.seen_generation = generation;
            self.play_queue.clear();
            self.level = false;
            self.last_sender_ns = None;
            self.anchor = None;
        }

        let mut edges = self.shared.edges.lock().unwrap();
        while let Some((ns, level)) = edges.pop_front() {
            let new_burst = match self.last_sender_ns {
                None => true,
                Some(previous) => ns.saturating_sub(previous) > BURST_GAP_NS,
            };
            self.last_sender_ns = Some(ns);

            if new_burst {
                self.anchor = Some((ns, cycle + self.ns_to_cycles(REPLAY_LATENCY_NS)));
            }
            let (anchor_ns, anchor_cycle) = self.anchor.unwrap();

            let mut at = anchor_cycle + self.ns_to_cycles(ns.saturating_sub(anchor_ns));
            if at <= self.last_scheduled_cycle {
                at = self.last_scheduled_cycle + 1;
            }
            self.last_scheduled_cycle = at;
            self.play_queue.push_back((at, level));
        }
    }
}

impl IrInterface for SocketIr {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64) {
        if emulated_clock_rate > 0 {
            self.clock_rate = emulated_clock_rate;
        }
    }

    fn set_carrier(&mut self, cycle: u64, carrier: bool) {
        // Without a peer the edge just goes out into the void, and queuing
        // it would only replay it, stale, after a connection arrives.
        if !self.connected() {
            return;
        }
        let _ = self.outgoing.send((self.cycles_to_ns(cycle), carrier));
    }

    fn carrier_detected(&mut self, cycle: u64) -> bool {
        self.drain_incoming(cycle);
        while let Some(&(at, level)) = self.play_queue.front() {
            if at > cycle {
                break;
            }
            self.level = level;
            self.play_queue.pop_front();
        }
        self.level
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
