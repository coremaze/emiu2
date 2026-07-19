//! Relay-server transport for the IR link: connects to an emiu2 relay,
//! receives an ephemeral friend code, and exchanges IR edges with
//! whichever peer it is paired with. Pairings can be formed and broken
//! at any time while the emulator runs, via [`RelayCommander`].
//!
//! Burst replay and rollback decisions live in
//! [`crate::ir_replay::ReplayEngine`], exactly as for the direct socket
//! transport; "connected" here means *paired* — edges only flow, and
//! snapshots are only worth taking, while a peer is on the other end.
//!
//! # Threading
//!
//! The transport and rollback handle share the engine through
//! `Rc<RefCell<..>>` on the emulator thread (a [`RelayIr`] must be
//! created on the thread that will use it). The connection thread and
//! the UI's [`RelayCommander`] communicate only through channels and
//! atomics, so the commander can live on any thread (the stdin console,
//! for one).

use crate::ir::{IrInterface, IrRollbackControl, RollbackDirective};
use crate::ir_replay::ReplayEngine;
use emiu2_netplay::{
    decode_edges, encode_edge, Decoder, FriendCode, Message, CLIENT_MAGIC, EDGE_RECORD_LEN,
    PROTOCOL_VERSION,
};
use std::cell::RefCell;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

const RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Backstop against unbounded growth if the emulator stops polling;
/// edges beyond this are dropped (such a link is long dead anyway).
const MAX_QUEUED_EDGES: usize = 100_000;

struct Shared {
    /// Bumped whenever the link identity changes (reconnect, pairing
    /// formed or dissolved) so stale edges can be recognized and
    /// discarded.
    generation: AtomicU64,
    /// Whether a peer is currently on the other end.
    paired: AtomicBool,
    /// Whether the relay itself is reachable.
    connected: AtomicBool,
    /// The friend code the relay assigned us, packed with
    /// [`pack_code`]; 0 while unknown.
    code: AtomicU64,
}

impl Shared {
    fn set_paired(&self, paired: bool) {
        let was = self.paired.swap(paired, Ordering::Relaxed);
        if was != paired {
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A friend code is six bytes from an alphabet that excludes 0, so it
/// packs losslessly into an atomic; 0 means "none yet". Also used by
/// [`crate::platform::link_ir`], which stores its code the same way.
pub(crate) fn pack_code(code: FriendCode) -> u64 {
    let bytes = code.as_bytes();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], 0, 0,
    ])
}

pub(crate) fn unpack_code(packed: u64) -> Option<FriendCode> {
    if packed == 0 {
        return None;
    }
    let bytes = packed.to_le_bytes();
    FriendCode::from_wire([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]])
}

/// One received edge, tagged with the link generation it arrived on.
struct TaggedEdge {
    generation: u64,
    sender_ns: u64,
    level: bool,
}

/// User-initiated actions, from the UI to the network thread.
pub enum RelayCommand {
    Join(FriendCode),
    Leave,
}

/// One outgoing wire message: the edge records it carries. Live edges
/// travel alone; a completed burst's atomic copy travels as one item so
/// it reaches the peer in a single message (see `crate::ir_replay` on
/// atomic resend).
type OutgoingRecords = Vec<(u64, bool)>;

pub struct RelayIr {
    outgoing: mpsc::Sender<OutgoingRecords>,
    incoming: mpsc::Receiver<TaggedEdge>,
    commands: mpsc::Sender<RelayCommand>,
    shared: Arc<Shared>,
    engine: Rc<RefCell<ReplayEngine>>,
    /// The generation the engine's state belongs to.
    seen_generation: u64,
}

impl RelayIr {
    /// Starts connecting to `addr` (a `host:port` string) in the
    /// background, retrying until it succeeds.
    pub fn connect(addr: impl Into<String>) -> Self {
        let addr = addr.into();
        let (outgoing, outgoing_rx) = mpsc::channel();
        let (incoming_tx, incoming) = mpsc::sync_channel(MAX_QUEUED_EDGES);
        let (commands, commands_rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            generation: AtomicU64::new(0),
            paired: AtomicBool::new(false),
            connected: AtomicBool::new(false),
            code: AtomicU64::new(0),
        });

        let thread_shared = shared.clone();
        std::thread::spawn(move || {
            connection_loop(addr, outgoing_rx, incoming_tx, commands_rx, thread_shared)
        });

        Self {
            outgoing,
            incoming,
            commands,
            shared,
            engine: Rc::new(RefCell::new(ReplayEngine::new())),
            seen_generation: 0,
        }
    }

    pub fn paired(&self) -> bool {
        self.shared.paired.load(Ordering::Relaxed)
    }

    /// The control handle for the user interface. Send-safe: it talks
    /// to the connection thread through a channel and atomics only.
    pub fn commander(&self) -> RelayCommander {
        RelayCommander {
            commands: self.commands.clone(),
            shared: self.shared.clone(),
        }
    }

    /// The run loop's handle for rollback coordination (same thread as
    /// the emulator).
    pub fn rollback_handle(&self) -> RelayIrRollback {
        RelayIrRollback {
            engine: self.engine.clone(),
            outgoing: self.outgoing.clone(),
            shared: self.shared.clone(),
        }
    }
}

impl IrInterface for RelayIr {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64) {
        self.engine.borrow_mut().set_clock_rate(emulated_clock_rate);
    }

    fn set_carrier(&mut self, cycle: u64, carrier: bool) {
        if !self.paired() {
            return;
        }
        let mut engine = self.engine.borrow_mut();
        let wire_ns = engine.outgoing_wire_ns(cycle, carrier);
        // If this edge started a new burst, the previous burst's atomic
        // copy goes out first, keeping the wire in stream order.
        while let Some(burst) = engine.take_burst_resend(cycle) {
            let _ = self.outgoing.send(burst);
        }
        let _ = self.outgoing.send(vec![(wire_ns, carrier)]);
    }

    fn carrier_detected(&mut self, cycle: u64) -> bool {
        let mut engine = self.engine.borrow_mut();

        // Reconnects and pairing changes invalidate all link state,
        // whether or not edges have arrived yet.
        let current = self.shared.generation.load(Ordering::Relaxed);
        if self.seen_generation != current {
            self.seen_generation = current;
            engine.reset();
        }

        while let Ok(edge) = self.incoming.try_recv() {
            if edge.generation != current {
                continue; // leftover from a previous connection or pairing
            }
            engine.push_incoming(cycle, edge.sender_ns, edge.level);
        }
        engine.current_level(cycle)
    }

    fn set_receiver_power(&mut self, cycle: u64, powered: bool) {
        let paired = self.paired();
        self.engine
            .borrow_mut()
            .receiver_power_changed(cycle, powered, paired);
    }
}

/// The user-facing side: issue join/leave and read status, from any
/// thread.
pub struct RelayCommander {
    commands: mpsc::Sender<RelayCommand>,
    shared: Arc<Shared>,
}

impl RelayCommander {
    pub fn join(&self, code: FriendCode) {
        let _ = self.commands.send(RelayCommand::Join(code));
    }

    pub fn leave(&self) {
        let _ = self.commands.send(RelayCommand::Leave);
    }

    pub fn code(&self) -> Option<FriendCode> {
        unpack_code(self.shared.code.load(Ordering::Relaxed))
    }

    pub fn connected(&self) -> bool {
        self.shared.connected.load(Ordering::Relaxed)
    }

    pub fn paired(&self) -> bool {
        self.shared.paired.load(Ordering::Relaxed)
    }
}

/// The [`IrRollbackControl`] endpoint of a [`RelayIr`].
pub struct RelayIrRollback {
    engine: Rc<RefCell<ReplayEngine>>,
    outgoing: mpsc::Sender<OutgoingRecords>,
    shared: Arc<Shared>,
}

impl RelayIrRollback {
    /// Sends any completed burst's atomic copy; unpaired copies are
    /// discarded (the engine resets on the next pairing anyway).
    fn pump_resends(&self, engine: &mut ReplayEngine, now_cycle: u64) {
        while let Some(burst) = engine.take_burst_resend(now_cycle) {
            if self.shared.paired.load(Ordering::Relaxed) {
                let _ = self.outgoing.send(burst);
            }
        }
    }
}

impl IrRollbackControl for RelayIrRollback {
    fn poll(&mut self, now_cycle: u64) -> RollbackDirective {
        let mut engine = self.engine.borrow_mut();
        let directive = engine.poll(now_cycle);
        self.pump_resends(&mut engine, now_cycle);
        directive
    }

    fn snapshot_taken(&mut self, cycle: u64) {
        self.engine.borrow_mut().snapshot_taken(cycle);
    }

    fn rolled_back(&mut self, restored_cycle: u64, abandoned_cycle: u64) {
        let mut engine = self.engine.borrow_mut();
        let close = engine.rolled_back(restored_cycle, abandoned_cycle);
        if let Some(close_ns) = close {
            if self.shared.paired.load(Ordering::Relaxed) {
                let _ = self.outgoing.send(vec![(close_ns, false)]);
            }
        }
        self.pump_resends(&mut engine, restored_cycle);
    }
}

fn connection_loop(
    addr: String,
    outgoing: mpsc::Receiver<OutgoingRecords>,
    incoming: mpsc::SyncSender<TaggedEdge>,
    commands: mpsc::Receiver<RelayCommand>,
    shared: Arc<Shared>,
) {
    loop {
        let stream = match TcpStream::connect(addr.as_str()) {
            Ok(stream) => stream,
            Err(_) => {
                // The relay may not be reachable yet; retry quietly.
                std::thread::sleep(RETRY_INTERVAL);
                continue;
            }
        };

        if let Err(why) = run_connection(stream, &outgoing, &incoming, &commands, &shared) {
            eprintln!("IR relay: connection lost: {why}");
        }
        shared.connected.store(false, Ordering::Relaxed);
        shared.set_paired(false);
        shared.code.store(0, Ordering::Relaxed);
        std::thread::sleep(RETRY_INTERVAL);
    }
}

fn run_connection(
    mut stream: TcpStream,
    outgoing: &mpsc::Receiver<OutgoingRecords>,
    incoming: &mpsc::SyncSender<TaggedEdge>,
    commands: &mpsc::Receiver<RelayCommand>,
    shared: &Shared,
) -> std::io::Result<()> {
    stream.set_nodelay(true).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.set_read_timeout(Some(Duration::from_millis(1)))?;

    stream.write_all(&CLIENT_MAGIC)?;
    stream.write_all(
        &Message::Hello {
            version: PROTOCOL_VERSION,
        }
        .encode(),
    )?;

    // Anything queued while unconnected is stale.
    while outgoing.try_recv().is_ok() {}

    let mut decoder = Decoder::new();
    let mut chunk = [0u8; 4096];
    loop {
        // Outgoing records; each channel item becomes one IrData
        // message, so a burst's atomic copy stays in one piece.
        loop {
            match outgoing.try_recv() {
                Ok(edges) => {
                    if !shared.paired.load(Ordering::Relaxed) {
                        continue;
                    }
                    let mut records = Vec::with_capacity(edges.len() * EDGE_RECORD_LEN);
                    for (ns, level) in edges {
                        records.extend_from_slice(&encode_edge(ns, level));
                    }
                    stream.write_all(&Message::IrData { records }.encode())?;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }

        // User commands.
        loop {
            match commands.try_recv() {
                Ok(RelayCommand::Join(code)) => {
                    stream.write_all(&Message::Join { code }.encode())?;
                }
                Ok(RelayCommand::Leave) => {
                    stream.write_all(&Message::Leave.encode())?;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }

        // Incoming bytes; the 1ms timeout paces the loop.
        match stream.read(&mut chunk) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "relay disconnected",
                ))
            }
            Ok(n) => {
                decoder.push(&chunk[..n]);
                while let Some(message) = decoder
                    .try_next()
                    .map_err(|why| std::io::Error::new(ErrorKind::InvalidData, why))?
                {
                    handle_message(message, &mut stream, incoming, shared)?;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
            Err(e) => return Err(e),
        }
    }
}

fn handle_message(
    message: Message,
    stream: &mut TcpStream,
    incoming: &mpsc::SyncSender<TaggedEdge>,
    shared: &Shared,
) -> std::io::Result<()> {
    match message {
        Message::Welcome { code, .. } => {
            shared.code.store(pack_code(code), Ordering::Relaxed);
            shared.connected.store(true, Ordering::Relaxed);
            eprintln!("IR relay: connected. Your friend code is {code}");
            eprintln!("IR relay: type \"join <code>\" to connect to a friend");
        }
        Message::Paired => {
            shared.set_paired(true);
            eprintln!("IR relay: paired with a peer");
        }
        Message::PeerLeft => {
            shared.set_paired(false);
            eprintln!("IR relay: the peer left");
        }
        Message::IrData { records } => {
            if shared.paired.load(Ordering::Relaxed) {
                let generation = shared.generation.load(Ordering::Relaxed);
                let edges = decode_edges(&records)
                    .map_err(|why| std::io::Error::new(ErrorKind::InvalidData, why))?;
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
                        Err(mpsc::TrySendError::Disconnected(_)) => {
                            return Err(std::io::Error::new(
                                ErrorKind::ConnectionAborted,
                                "emulator gone",
                            ))
                        }
                    }
                }
            }
        }
        Message::Ping => {
            stream.write_all(&Message::Pong.encode())?;
        }
        Message::Pong => {}
        Message::Error { code: _, message } => {
            eprintln!("IR relay: {message}");
        }
        // Client-to-server messages arriving here would be a server bug.
        Message::Hello { .. } | Message::Join { .. } | Message::Leave => {}
    }
    Ok(())
}
