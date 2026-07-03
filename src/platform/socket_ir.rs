//! TCP transport for the IR link, connecting two emulators on the same
//! machine or across a network.
//!
//! The wire protocol is a stream of envelope edges: 9-byte records of
//! `(u64 LE sender time in emulated nanoseconds, u8 carrier level)`,
//! after an 8-byte magic handshake in each direction.
//!
//! Burst replay, jitter handling and the rollback decision logic all
//! live in [`ReplayEngine`]; this file is the socket shell: a background
//! connection thread, the channels between it and the emulator thread,
//! and reconnect handling.
//!
//! # Threading
//!
//! Everything except the connection thread runs on the emulator thread:
//! the transport (called from inside the machine) and the rollback
//! handle (called by the run loop) share the engine through
//! `Rc<RefCell<..>>`, which also means a [`SocketIr`] must be created on
//! the thread that will use it. The connection thread communicates only
//! through channels and atomics.

use crate::ir::{IrInterface, IrRollbackControl, RollbackDirective};
use crate::ir_replay::ReplayEngine;
use emiu2_netplay::{decode_edges, encode_edge, EDGE_RECORD_LEN};
use std::cell::RefCell;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

const MAGIC: [u8; 8] = *b"EMIU2IR\x01";

/// How long a reconnect waits after a failed attempt or lost connection.
const RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Backstop against unbounded growth if the emulator stops polling;
/// edges beyond this are dropped (such a link is long dead anyway).
const MAX_QUEUED_EDGES: usize = 100_000;

struct Shared {
    /// Bumped on every new connection so edges from a dead connection
    /// can be recognized and discarded.
    generation: AtomicU64,
    connected: AtomicBool,
}

/// One received edge, tagged with the connection it arrived on.
struct TaggedEdge {
    generation: u64,
    sender_ns: u64,
    level: bool,
}

enum Endpoint {
    Listen(TcpListener),
    Connect(String),
}

pub struct SocketIr {
    outgoing: mpsc::Sender<(u64, bool)>,
    incoming: mpsc::Receiver<TaggedEdge>,
    shared: Arc<Shared>,
    local_addr: Option<SocketAddr>,
    engine: Rc<RefCell<ReplayEngine>>,
    /// The generation the engine's state belongs to.
    seen_generation: u64,
}

impl SocketIr {
    /// Binds immediately (so address errors surface here) and accepts
    /// peers in the background, one at a time.
    pub fn listen(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        Ok(Self::from_listener(TcpListener::bind(addr)?))
    }

    /// Accepts peers on an already-bound listener; useful when the
    /// listener is bound on a different thread than the emulator runs on
    /// (a `SocketIr` itself must live on the emulator thread).
    pub fn from_listener(listener: TcpListener) -> Self {
        let local_addr = listener.local_addr().ok();
        Self::start(Endpoint::Listen(listener), local_addr)
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
            engine: self.engine.clone(),
            outgoing: self.outgoing.clone(),
            shared: self.shared.clone(),
        }
    }

    fn start(endpoint: Endpoint, local_addr: Option<SocketAddr>) -> Self {
        let (outgoing, outgoing_rx) = mpsc::channel();
        let (incoming_tx, incoming) = mpsc::sync_channel(MAX_QUEUED_EDGES);
        let shared = Arc::new(Shared {
            generation: AtomicU64::new(0),
            connected: AtomicBool::new(false),
        });

        let thread_shared = shared.clone();
        std::thread::spawn(move || {
            connection_loop(endpoint, outgoing_rx, incoming_tx, thread_shared)
        });

        Self {
            outgoing,
            incoming,
            shared,
            local_addr,
            engine: Rc::new(RefCell::new(ReplayEngine::new())),
            seen_generation: 0,
        }
    }
}

impl IrInterface for SocketIr {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64) {
        self.engine.borrow_mut().set_clock_rate(emulated_clock_rate);
    }

    fn set_carrier(&mut self, cycle: u64, carrier: bool) {
        // Without a peer the edge just goes out into the void, and queuing
        // it would only replay it, stale, after a connection arrives.
        if !self.connected() {
            return;
        }
        let wire_ns = self.engine.borrow_mut().outgoing_wire_ns(cycle, carrier);
        let _ = self.outgoing.send((wire_ns, carrier));
    }

    fn carrier_detected(&mut self, cycle: u64) -> bool {
        let mut engine = self.engine.borrow_mut();

        // A new connection invalidates all link state, whether or not it
        // has produced edges yet.
        let current = self.shared.generation.load(Ordering::Relaxed);
        if self.seen_generation != current {
            self.seen_generation = current;
            engine.reset();
        }

        while let Ok(edge) = self.incoming.try_recv() {
            if edge.generation != current {
                continue; // leftover from a dead connection
            }
            engine.push_incoming(cycle, edge.sender_ns, edge.level);
        }
        engine.current_level(cycle)
    }

    fn set_receiver_power(&mut self, cycle: u64, powered: bool) {
        let connected = self.connected();
        self.engine
            .borrow_mut()
            .receiver_power_changed(cycle, powered, connected);
    }
}

/// The [`IrRollbackControl`] endpoint of a [`SocketIr`], held by the
/// platform's run loop (same thread as the emulator).
pub struct SocketIrRollback {
    engine: Rc<RefCell<ReplayEngine>>,
    outgoing: mpsc::Sender<(u64, bool)>,
    shared: Arc<Shared>,
}

impl IrRollbackControl for SocketIrRollback {
    fn poll(&mut self, now_cycle: u64) -> RollbackDirective {
        self.engine.borrow_mut().poll(now_cycle)
    }

    fn snapshot_taken(&mut self, cycle: u64) {
        self.engine.borrow_mut().snapshot_taken(cycle);
    }

    fn rolled_back(&mut self, restored_cycle: u64, abandoned_cycle: u64) {
        let close = self
            .engine
            .borrow_mut()
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

fn connection_loop(
    endpoint: Endpoint,
    outgoing: mpsc::Receiver<(u64, bool)>,
    incoming: mpsc::SyncSender<TaggedEdge>,
    shared: Arc<Shared>,
) {
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

        let after = match run_connection(stream, &outgoing, &incoming, &shared) {
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
    incoming: &mpsc::SyncSender<TaggedEdge>,
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

    // The new generation marks anything already queued as stale.
    let generation = shared.generation.fetch_add(1, Ordering::Relaxed) + 1;
    shared.connected.store(true, Ordering::Relaxed);

    // The 1ms read timeout paces the loop: sending waits at most 1ms,
    // well within the replay latency budget.
    stream.set_read_timeout(Some(Duration::from_millis(1)))?;

    let mut inbuf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        loop {
            match outgoing.try_recv() {
                Ok((ns, level)) => stream.write_all(&encode_edge(ns, level))?,
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
                // Decode the complete records; a partial one stays
                // buffered until the rest of it arrives.
                let complete = inbuf.len() - inbuf.len() % EDGE_RECORD_LEN;
                let edges = decode_edges(&inbuf[..complete])
                    .map_err(|why| std::io::Error::new(ErrorKind::InvalidData, why))?;
                inbuf.drain(..complete);
                for (sender_ns, level) in edges {
                    match incoming.try_send(TaggedEdge {
                        generation,
                        sender_ns,
                        level,
                    }) {
                        Ok(()) => {}
                        // The emulator has stopped draining; drop edges
                        // rather than block the network thread.
                        Err(mpsc::TrySendError::Full(_)) => {}
                        Err(mpsc::TrySendError::Disconnected(_)) => return Ok(After::Shutdown),
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
            Err(e) => return Err(e),
        }
    }
}
