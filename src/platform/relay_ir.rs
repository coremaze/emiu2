//! Relay-server transport for the IR link: connects to an emiu2 relay,
//! receives an ephemeral friend code, and exchanges IR edges with
//! whichever peer it is paired with. Pairings can be formed and broken
//! at any time while the emulator runs, via [`RelayCommander`].
//!
//! Burst replay and rollback decisions live in
//! [`crate::ir_replay::ReplayEngine`], exactly as for the direct socket
//! transport; "connected" here means *paired* — edges only flow, and
//! snapshots are only worth taking, while a peer is on the other end.

use crate::ir::{IrInterface, IrRollbackControl, RollbackDirective};
use crate::ir_replay::ReplayEngine;
use emiu2_netplay::{Decoder, FriendCode, Message, CLIENT_MAGIC, PROTOCOL_VERSION};
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const RECORD_LEN: usize = 9;
const RETRY_INTERVAL: Duration = Duration::from_secs(2);
const MAX_QUEUED_EDGES: usize = 100_000;

struct Shared {
    /// Received edges: (sender time in ns, carrier level).
    edges: Mutex<VecDeque<(u64, bool)>>,
    /// Bumped whenever the link identity changes (reconnect, pairing
    /// formed or dissolved) so replay state can be reset.
    generation: AtomicU64,
    /// Whether a peer is currently on the other end.
    paired: AtomicBool,
    /// Whether the relay itself is reachable.
    connected: AtomicBool,
    /// The friend code the relay assigned us, once known.
    code: Mutex<Option<FriendCode>>,
}

/// User-initiated actions, from the UI to the network thread.
pub enum RelayCommand {
    Join(FriendCode),
    Leave,
}

struct Link {
    engine: ReplayEngine,
    seen_generation: u64,
}

pub struct RelayIr {
    outgoing: mpsc::Sender<(u64, bool)>,
    commands: mpsc::Sender<RelayCommand>,
    shared: Arc<Shared>,
    link: Arc<Mutex<Link>>,
}

impl RelayIr {
    /// Starts connecting to `addr` (a `host:port` string) in the
    /// background, retrying until it succeeds.
    pub fn connect(addr: impl Into<String>) -> Self {
        let addr = addr.into();
        let (outgoing, outgoing_rx) = mpsc::channel();
        let (commands, commands_rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            edges: Mutex::new(VecDeque::new()),
            generation: AtomicU64::new(0),
            paired: AtomicBool::new(false),
            connected: AtomicBool::new(false),
            code: Mutex::new(None),
        });

        let thread_shared = shared.clone();
        std::thread::spawn(move || connection_loop(addr, outgoing_rx, commands_rx, thread_shared));

        Self {
            outgoing,
            commands,
            shared,
            link: Arc::new(Mutex::new(Link {
                engine: ReplayEngine::new(),
                seen_generation: 0,
            })),
        }
    }

    pub fn paired(&self) -> bool {
        self.shared.paired.load(Ordering::Relaxed)
    }

    /// The control handle for the user interface.
    pub fn commander(&self) -> RelayCommander {
        RelayCommander {
            commands: self.commands.clone(),
            shared: self.shared.clone(),
        }
    }

    /// The run loop's handle for rollback coordination.
    pub fn rollback_handle(&self) -> RelayIrRollback {
        RelayIrRollback {
            link: self.link.clone(),
            outgoing: self.outgoing.clone(),
            shared: self.shared.clone(),
        }
    }
}

impl IrInterface for RelayIr {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64) {
        self.link
            .lock()
            .unwrap()
            .engine
            .set_clock_rate(emulated_clock_rate);
    }

    fn set_carrier(&mut self, cycle: u64, carrier: bool) {
        if !self.paired() {
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
        let generation = self.shared.generation.load(Ordering::Relaxed);
        if generation != link.seen_generation {
            link.seen_generation = generation;
            link.engine.reset();
        }
        {
            let mut edges = self.shared.edges.lock().unwrap();
            while let Some((ns, level)) = edges.pop_front() {
                link.engine.push_incoming(cycle, ns, level);
            }
        }
        link.engine.current_level(cycle)
    }

    fn set_receiver_power(&mut self, cycle: u64, powered: bool) {
        let paired = self.paired();
        self.link
            .lock()
            .unwrap()
            .engine
            .receiver_power_changed(cycle, powered, paired);
    }
}

/// The user-facing side: issue join/leave and read status. Cheap to
/// clone-ish (all handles are shared).
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
        *self.shared.code.lock().unwrap()
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
    link: Arc<Mutex<Link>>,
    outgoing: mpsc::Sender<(u64, bool)>,
    shared: Arc<Shared>,
}

impl IrRollbackControl for RelayIrRollback {
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
            if self.shared.paired.load(Ordering::Relaxed) {
                let _ = self.outgoing.send((close_ns, false));
            }
        }
    }
}

fn connection_loop(
    addr: String,
    outgoing: mpsc::Receiver<(u64, bool)>,
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

        if let Err(why) = run_connection(stream, &outgoing, &commands, &shared) {
            eprintln!("IR relay: connection lost: {why}");
        }
        shared.connected.store(false, Ordering::Relaxed);
        set_paired(&shared, false);
        *shared.code.lock().unwrap() = None;
        std::thread::sleep(RETRY_INTERVAL);
    }
}

fn set_paired(shared: &Shared, paired: bool) {
    let was = shared.paired.swap(paired, Ordering::Relaxed);
    if was != paired {
        shared.generation.fetch_add(1, Ordering::Relaxed);
        shared.edges.lock().unwrap().clear();
    }
}

fn run_connection(
    mut stream: TcpStream,
    outgoing: &mpsc::Receiver<(u64, bool)>,
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
        // Outgoing edges, each as one IrData record.
        loop {
            match outgoing.try_recv() {
                Ok((ns, level)) => {
                    if !shared.paired.load(Ordering::Relaxed) {
                        continue;
                    }
                    let mut records = Vec::with_capacity(RECORD_LEN);
                    records.extend_from_slice(&ns.to_le_bytes());
                    records.push(level as u8);
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
                    .next()
                    .map_err(|why| std::io::Error::new(ErrorKind::InvalidData, why))?
                {
                    handle_message(message, &mut stream, shared)?;
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
    shared: &Shared,
) -> std::io::Result<()> {
    match message {
        Message::Welcome { code, .. } => {
            *shared.code.lock().unwrap() = Some(code);
            shared.connected.store(true, Ordering::Relaxed);
            eprintln!("IR relay: connected. Your friend code is {code}");
            eprintln!("IR relay: type \"join <code>\" to connect to a friend");
        }
        Message::Paired => {
            set_paired(shared, true);
            eprintln!("IR relay: paired with a peer");
        }
        Message::PeerLeft => {
            set_paired(shared, false);
            eprintln!("IR relay: the peer left");
        }
        Message::IrData { records } => {
            if shared.paired.load(Ordering::Relaxed) {
                let mut edges = shared.edges.lock().unwrap();
                for record in records.chunks_exact(RECORD_LEN) {
                    let ns = u64::from_le_bytes(record[..8].try_into().unwrap());
                    let level = record[8] != 0;
                    if edges.len() >= MAX_QUEUED_EDGES {
                        edges.pop_front();
                    }
                    edges.push_back((ns, level));
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
