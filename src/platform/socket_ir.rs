//! TCP transport for the IR link, connecting two emulators on the same
//! machine or across a network.
//!
//! The wire protocol is a stream of envelope edges: 9-byte records of
//! `(u64 LE sender time in emulated nanoseconds, u8 carrier level)`,
//! after an 8-byte magic handshake in each direction.
//!
//! Burst replay, jitter handling and the rollback decision logic all
//! live in [`ReplayEngine`]; this file is the socket shell: a background
//! connection thread, the queues between it and the emulator thread, and
//! reconnect handling.

use crate::ir::{IrInterface, IrRollbackControl, RollbackDirective};
use crate::ir_replay::ReplayEngine;
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAGIC: [u8; 8] = *b"EMIU2IR\x01";
const RECORD_LEN: usize = 9;

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

/// The replay engine plus the connection generation it has seen. Shared
/// between the transport (driven from inside the machine) and the
/// rollback handle (driven by the run loop); both run on the emulator
/// thread, so the mutex is never contended.
struct Link {
    engine: ReplayEngine,
    seen_generation: u64,
}

impl Link {
    /// Resets the engine when a new connection replaced the old one.
    fn sync_generation(&mut self, shared: &Shared) {
        let generation = shared.generation.load(Ordering::Relaxed);
        if generation != self.seen_generation {
            self.seen_generation = generation;
            self.engine.reset();
        }
    }
}

pub struct SocketIr {
    outgoing: mpsc::Sender<(u64, bool)>,
    shared: Arc<Shared>,
    local_addr: Option<SocketAddr>,
    link: Arc<Mutex<Link>>,
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
            link: self.link.clone(),
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
            link: Arc::new(Mutex::new(Link {
                engine: ReplayEngine::new(),
                seen_generation: 0,
            })),
        }
    }
}

impl IrInterface for SocketIr {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64) {
        self.link
            .lock()
            .unwrap()
            .engine
            .set_clock_rate(emulated_clock_rate);
    }

    fn set_carrier(&mut self, cycle: u64, carrier: bool) {
        // Without a peer the edge just goes out into the void, and queuing
        // it would only replay it, stale, after a connection arrives.
        if !self.connected() {
            return;
        }
        let wire_ns = self
            .link
            .lock()
            .unwrap()
            .engine
            .outgoing_wire_ns(cycle, carrier);
        let _ = self.outgoing.send((wire_ns, carrier));
    }

    fn carrier_detected(&mut self, cycle: u64) -> bool {
        let mut link = self.link.lock().unwrap();
        link.sync_generation(&self.shared);
        {
            let mut edges = self.shared.edges.lock().unwrap();
            while let Some((ns, level)) = edges.pop_front() {
                link.engine.push_incoming(cycle, ns, level);
            }
        }
        link.engine.current_level(cycle)
    }

    fn set_receiver_power(&mut self, cycle: u64, powered: bool) {
        let connected = self.connected();
        self.link
            .lock()
            .unwrap()
            .engine
            .receiver_power_changed(cycle, powered, connected);
    }
}

/// The [`IrRollbackControl`] endpoint of a [`SocketIr`], held by the
/// platform's run loop.
pub struct SocketIrRollback {
    link: Arc<Mutex<Link>>,
    outgoing: mpsc::Sender<(u64, bool)>,
    shared: Arc<Shared>,
}

impl IrRollbackControl for SocketIrRollback {
    fn poll(&mut self, now_cycle: u64) -> RollbackDirective {
        self.link.lock().unwrap().engine.poll(now_cycle)
    }

    fn snapshot_taken(&mut self, cycle: u64) {
        self.link.lock().unwrap().engine.snapshot_taken(cycle);
    }

    fn rolled_back(&mut self, restored_cycle: u64, abandoned_cycle: u64) {
        let close = self
            .link
            .lock()
            .unwrap()
            .engine
            .rolled_back(restored_cycle, abandoned_cycle);
        if let Some(close_ns) = close {
            if self.shared.connected.load(Ordering::Relaxed) {
                let _ = self.outgoing.send((close_ns, false));
            }
        }
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
