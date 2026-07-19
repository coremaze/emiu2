//! The player-facing IR transport: one long-lived transport that links
//! either **locally** with another emulator on this machine, found
//! through [`crate::platform::ir_discovery`] and connected directly over
//! loopback TCP or **online**, through an emiu2 relay with friend
//! codes. The mode can be switched at any time while the emulator runs,
//! via [`LinkCommander`], without touching the machine.
//!
//! Both modes speak wire formats that already exist: local links use the
//! direct socket protocol (the [`crate::platform::socket_ir`] magic then
//! bare 9-byte edge records), online links the relay protocol from
//! `emiu2_netplay`. Burst replay, jitter handling and rollback decisions
//! live in [`ReplayEngine`], exactly as for the fixed transports;
//! "linked" means a peer is on the other end, whichever way they got
//! there.
//!
//! In local mode the transport listens on an ephemeral loopback port and
//! advertises it, so every running emulator is both discoverable and
//! able to dial whichever peer the player picks. One link at a time:
//! while linked, the advert is withdrawn and extra connections are
//! refused.
//!
//! # Threading
//!
//! The transport and rollback handle share the engine through
//! `Rc<RefCell<..>>` on the emulator thread (a [`LinkIr`] must be
//! created on the thread that will use it). The connection thread and
//! the UI's [`LinkCommander`] communicate only through channels and
//! atomics, so the commander can live on any thread.

use crate::ir::{IrInterface, IrRollbackControl, RollbackDirective};
use crate::ir_replay::ReplayEngine;
use crate::platform::ir_discovery::Advert;
use crate::platform::relay_ir::{pack_code, unpack_code};
use crate::platform::socket_ir::MAGIC;
use emiu2_netplay::{
    decode_edges, encode_edge, Decoder, FriendCode, Message, CLIENT_MAGIC, EDGE_RECORD_LEN,
    PROTOCOL_VERSION,
};
use std::cell::RefCell;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Loop pacing while nothing is linked (a linked loop is paced by the
/// stream's 1ms read timeout instead).
const IDLE_POLL: Duration = Duration::from_millis(10);

/// Both directions of the direct handshake, so a bad peer cannot hang us.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

/// A discovered peer should be up (its advert is fresh); a dial that
/// takes longer than this is not going to work.
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);

const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a failed relay connection waits before the next attempt.
const RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Backstop against unbounded growth if the emulator stops polling;
/// edges beyond this are dropped (such a link is long dead anyway).
const MAX_QUEUED_EDGES: usize = 100_000;

/// Where the link starts (and where a [`LinkCommander`] can move it).
#[derive(Debug, Clone)]
pub enum LinkMode {
    /// Discoverable on this machine; links form by dialing a discovered
    /// peer (or being dialed).
    Local,
    /// Connected to an emiu2 relay; links form by friend code.
    Online { relay: String },
}

/// The mode as a plain answer for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkModeKind {
    Local,
    Online,
}

/// Where the latest user-initiated local dial stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialState {
    Idle,
    Dialing,
    Failed,
}

const MODE_LOCAL: u8 = 0;
const MODE_ONLINE: u8 = 1;

const DIAL_IDLE: u8 = 0;
const DIAL_BUSY: u8 = 1;
const DIAL_FAILED: u8 = 2;

struct Shared {
    /// Bumped whenever the link identity changes (a link formed or
    /// dissolved, in either mode) so stale edges can be recognized and
    /// discarded.
    generation: AtomicU64,
    /// Whether a peer is currently on the other end.
    linked: AtomicBool,
    /// Whether the relay itself is reachable (online mode only).
    relay_connected: AtomicBool,
    /// The friend code the relay assigned us, packed with
    /// [`pack_code`]; 0 while unknown.
    code: AtomicU64,
    mode: AtomicU8,
    dial: AtomicU8,
    /// Set when the transport is being dropped, so a connect in progress
    /// (which the `Shutdown` command cannot interrupt) bails promptly
    /// instead of blocking the join for its full timeout.
    shutdown: AtomicBool,
}

impl Shared {
    fn set_linked(&self, linked: bool) {
        let was = self.linked.swap(linked, Ordering::Relaxed);
        if was != linked {
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// One received edge, tagged with the link generation it arrived on.
struct TaggedEdge {
    generation: u64,
    sender_ns: u64,
    level: bool,
}

/// User-initiated actions, from the UI to the connection thread.
enum LinkCommand {
    GoLocal,
    GoOnline(String),
    /// Dial a discovered peer (local mode).
    ConnectPeer(String),
    /// Drop the current local link but stay discoverable.
    DisconnectPeer,
    Join(FriendCode),
    Leave,
    /// Stop the connection thread ([`LinkIr`] is being dropped). Channel
    /// disconnection alone would also stop it, but only eventually. The
    /// advert must be withdrawn before the process can exit.
    Shutdown,
}

/// One outgoing wire item: the edge records it carries. Live edges
/// travel alone; a completed burst's atomic copy travels as one item so
/// it reaches the peer in one piece (see `crate::ir_replay` on atomic
/// resend).
type OutgoingRecords = Vec<(u64, bool)>;

pub struct LinkIr {
    outgoing: mpsc::Sender<OutgoingRecords>,
    incoming: mpsc::Receiver<TaggedEdge>,
    commands: mpsc::Sender<LinkCommand>,
    shared: Arc<Shared>,
    engine: Rc<RefCell<ReplayEngine>>,
    /// The generation the engine's state belongs to.
    seen_generation: u64,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Stopping the thread synchronously (rather than letting it notice the
/// closed channels on its own) guarantees the advert file is gone before
/// a cleanly-exiting process ends.
impl Drop for LinkIr {
    fn drop(&mut self) {
        // The flag reaches a thread blocked mid-connect (which the command
        // cannot); the command reaches it anywhere else.
        self.shared.shutdown.store(true, Ordering::Relaxed);
        let _ = self.commands.send(LinkCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

impl LinkIr {
    /// Starts the connection thread in `mode`. `identity` is the name
    /// this emulator advertises to others on this machine (the save's
    /// name).
    pub fn start(mode: LinkMode, identity: String) -> Self {
        let (outgoing, outgoing_rx) = mpsc::channel();
        let (incoming_tx, incoming) = mpsc::sync_channel(MAX_QUEUED_EDGES);
        let (commands, commands_rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            generation: AtomicU64::new(0),
            linked: AtomicBool::new(false),
            relay_connected: AtomicBool::new(false),
            code: AtomicU64::new(0),
            mode: AtomicU8::new(match mode {
                LinkMode::Local => MODE_LOCAL,
                LinkMode::Online { .. } => MODE_ONLINE,
            }),
            dial: AtomicU8::new(DIAL_IDLE),
            shutdown: AtomicBool::new(false),
        });

        let thread_shared = shared.clone();
        let thread = std::thread::spawn(move || {
            link_loop(
                mode,
                identity,
                outgoing_rx,
                incoming_tx,
                commands_rx,
                thread_shared,
            )
        });

        Self {
            outgoing,
            incoming,
            commands,
            shared,
            engine: Rc::new(RefCell::new(ReplayEngine::new())),
            seen_generation: 0,
            thread: Some(thread),
        }
    }

    pub fn linked(&self) -> bool {
        self.shared.linked.load(Ordering::Relaxed)
    }

    /// The control handle for the user interface. Send-safe: it talks
    /// to the connection thread through a channel and atomics only.
    pub fn commander(&self) -> LinkCommander {
        LinkCommander {
            commands: self.commands.clone(),
            shared: self.shared.clone(),
        }
    }

    /// The run loop's handle for rollback coordination (same thread as
    /// the emulator).
    pub fn rollback_handle(&self) -> LinkIrRollback {
        LinkIrRollback {
            engine: self.engine.clone(),
            outgoing: self.outgoing.clone(),
            shared: self.shared.clone(),
        }
    }
}

impl IrInterface for LinkIr {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64) {
        self.engine.borrow_mut().set_clock_rate(emulated_clock_rate);
    }

    fn set_carrier(&mut self, cycle: u64, carrier: bool) {
        if !self.linked() {
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

        // Any change of link identity invalidates all link state,
        // whether or not edges have arrived yet.
        let current = self.shared.generation.load(Ordering::Relaxed);
        if self.seen_generation != current {
            self.seen_generation = current;
            engine.reset();
        }

        while let Ok(edge) = self.incoming.try_recv() {
            if edge.generation != current {
                continue; // leftover from a previous link
            }
            engine.push_incoming(cycle, edge.sender_ns, edge.level);
        }
        engine.current_level(cycle)
    }

    fn set_receiver_power(&mut self, cycle: u64, powered: bool) {
        let linked = self.linked();
        self.engine
            .borrow_mut()
            .receiver_power_changed(cycle, powered, linked);
    }
}

/// The user-facing side: switch modes, dial peers, join friends, read
/// status from any thread.
pub struct LinkCommander {
    commands: mpsc::Sender<LinkCommand>,
    shared: Arc<Shared>,
}

impl LinkCommander {
    pub fn set_local(&self) {
        let _ = self.commands.send(LinkCommand::GoLocal);
    }

    /// Switches to (or re-targets) the relay at `relay`.
    pub fn set_online(&self, relay: impl Into<String>) {
        let _ = self.commands.send(LinkCommand::GoOnline(relay.into()));
    }

    pub fn connect_peer(&self, addr: impl Into<String>) {
        let _ = self.commands.send(LinkCommand::ConnectPeer(addr.into()));
    }

    pub fn disconnect_peer(&self) {
        let _ = self.commands.send(LinkCommand::DisconnectPeer);
    }

    pub fn join(&self, code: FriendCode) {
        let _ = self.commands.send(LinkCommand::Join(code));
    }

    pub fn leave(&self) {
        let _ = self.commands.send(LinkCommand::Leave);
    }

    pub fn mode(&self) -> LinkModeKind {
        match self.shared.mode.load(Ordering::Relaxed) {
            MODE_ONLINE => LinkModeKind::Online,
            _ => LinkModeKind::Local,
        }
    }

    /// Whether a peer is on the other end (paired, in online terms).
    pub fn linked(&self) -> bool {
        self.shared.linked.load(Ordering::Relaxed)
    }

    /// Whether the relay itself is reachable (online mode).
    pub fn relay_connected(&self) -> bool {
        self.shared.relay_connected.load(Ordering::Relaxed)
    }

    pub fn code(&self) -> Option<FriendCode> {
        unpack_code(self.shared.code.load(Ordering::Relaxed))
    }

    pub fn dial(&self) -> DialState {
        match self.shared.dial.load(Ordering::Relaxed) {
            DIAL_BUSY => DialState::Dialing,
            DIAL_FAILED => DialState::Failed,
            _ => DialState::Idle,
        }
    }
}

/// The [`IrRollbackControl`] endpoint of a [`LinkIr`].
pub struct LinkIrRollback {
    engine: Rc<RefCell<ReplayEngine>>,
    outgoing: mpsc::Sender<OutgoingRecords>,
    shared: Arc<Shared>,
}

impl LinkIrRollback {
    /// Sends any completed burst's atomic copy; unlinked copies are
    /// discarded (the engine resets on the next link anyway).
    fn pump_resends(&self, engine: &mut ReplayEngine, now_cycle: u64) {
        while let Some(burst) = engine.take_burst_resend(now_cycle) {
            if self.shared.linked.load(Ordering::Relaxed) {
                let _ = self.outgoing.send(burst);
            }
        }
    }
}

impl IrRollbackControl for LinkIrRollback {
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
            if self.shared.linked.load(Ordering::Relaxed) {
                let _ = self.outgoing.send(vec![(close_ns, false)]);
            }
        }
        self.pump_resends(&mut engine, restored_cycle);
    }
}

// ---------------------------------------------------------------------
// The connection thread.

fn link_loop(
    mut mode: LinkMode,
    identity: String,
    outgoing: mpsc::Receiver<OutgoingRecords>,
    incoming: mpsc::SyncSender<TaggedEdge>,
    commands: mpsc::Receiver<LinkCommand>,
    shared: Arc<Shared>,
) {
    loop {
        let next = match mode {
            LinkMode::Local => local_session(&identity, &outgoing, &incoming, &commands, &shared),
            LinkMode::Online { relay } => {
                online_session(relay, &outgoing, &incoming, &commands, &shared)
            }
        };
        shared.set_linked(false);
        match next {
            Some(next_mode) => mode = next_mode,
            None => return, // the emulator is gone
        }
    }
}

/// A live direct link and its per-connection state.
struct ActiveLink {
    stream: TcpStream,
    generation: u64,
    /// Partial edge record awaiting the rest of its bytes.
    inbuf: Vec<u8>,
}

/// What one pump pass over an active link concluded.
enum Pump {
    Ok,
    Lost(std::io::Error),
    Shutdown,
}

/// Local mode: listen + advertise + dial, until a mode switch (`Some`)
/// or emulator shutdown (`None`).
fn local_session(
    identity: &str,
    outgoing: &mpsc::Receiver<OutgoingRecords>,
    incoming: &mpsc::SyncSender<TaggedEdge>,
    commands: &mpsc::Receiver<LinkCommand>,
    shared: &Shared,
) -> Option<LinkMode> {
    shared.mode.store(MODE_LOCAL, Ordering::Relaxed);
    shared.relay_connected.store(false, Ordering::Relaxed);
    shared.code.store(0, Ordering::Relaxed);
    shared.dial.store(DIAL_IDLE, Ordering::Relaxed);

    // The listener other emulators dial. Without one (loopback bind
    // refused, rare) this emulator can still dial out.
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| {
            listener.set_nonblocking(true)?;
            Ok(listener)
        })
        .map_err(|why| eprintln!("IR link: could not listen for nearby players: {why}"))
        .ok();
    let port = listener
        .as_ref()
        .and_then(|l| l.local_addr().ok())
        .map(|addr| addr.port());

    let mut advert: Option<Advert> = None;
    let mut advert_broken = false;
    let mut active: Option<ActiveLink> = None;

    loop {
        // Advertise exactly while a new link could actually be accepted.
        if active.is_some() {
            advert = None;
        } else if let Some(advert) = &mut advert {
            advert.refresh();
        } else if let (Some(port), false) = (port, advert_broken) {
            match Advert::create(identity, port) {
                Ok(created) => advert = Some(created),
                Err(why) => {
                    eprintln!("IR link: could not advertise to nearby players: {why}");
                    advert_broken = true;
                }
            }
        }

        loop {
            match commands.try_recv() {
                Ok(LinkCommand::GoOnline(relay)) => return Some(LinkMode::Online { relay }),
                Ok(LinkCommand::GoLocal) => {}
                Ok(LinkCommand::ConnectPeer(addr)) => {
                    if active.take().is_some() {
                        shared.set_linked(false);
                    }
                    shared.dial.store(DIAL_BUSY, Ordering::Relaxed);
                    match dial(&addr, shared) {
                        Ok(stream) => {
                            eprintln!("IR link: linked with {addr}");
                            active = Some(establish(stream, outgoing, shared));
                        }
                        // A dial cut short by shutdown is not a failure to
                        // surface: the next command drain returns.
                        Err(why) if why.kind() == ErrorKind::Interrupted => {}
                        Err(why) => {
                            eprintln!("IR link: could not reach {addr}: {why}");
                            shared.dial.store(DIAL_FAILED, Ordering::Relaxed);
                        }
                    }
                }
                Ok(LinkCommand::DisconnectPeer) => {
                    if active.take().is_some() {
                        shared.set_linked(false);
                        eprintln!("IR link: unlinked");
                    }
                    shared.dial.store(DIAL_IDLE, Ordering::Relaxed);
                }
                // Online-mode commands; nothing to do here.
                Ok(LinkCommand::Join(_)) | Ok(LinkCommand::Leave) => {}
                Ok(LinkCommand::Shutdown) => return None,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return None,
            }
        }

        if let Some(listener) = &listener {
            match listener.accept() {
                Ok((stream, peer)) => {
                    if active.is_some() {
                        // One link at a time. Dropping the socket without
                        // a handshake makes the caller's dial fail fast.
                        drop(stream);
                    } else {
                        match prepare(stream) {
                            Ok(stream) => {
                                eprintln!("IR link: linked with a nearby player ({peer})");
                                active = Some(establish(stream, outgoing, shared));
                            }
                            Err(why) => {
                                eprintln!("IR link: a nearby connection failed: {why}")
                            }
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(why) => eprintln!("IR link: accept failed: {why}"),
            }
        }

        let pumped = match &mut active {
            // The stream's 1ms read timeout paces the linked loop.
            Some(link) => pump(link, outgoing, incoming),
            None => {
                // Edges transmitted while unlinked belong to frames that
                // are already lost; also notices the emulator going away.
                loop {
                    match outgoing.try_recv() {
                        Ok(_) => {}
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => return None,
                    }
                }
                std::thread::sleep(IDLE_POLL);
                Pump::Ok
            }
        };
        match pumped {
            Pump::Ok => {}
            Pump::Lost(why) => {
                eprintln!("IR link: the link dropped: {why}");
                active = None;
                shared.set_linked(false);
            }
            Pump::Shutdown => return None,
        }
    }
}

/// How long a single `connect_timeout` attempt waits before the loop
/// rechecks the shutdown flag. Bounds how long a drop can block on a
/// connect to an unreachable host.
const CONNECT_STEP: Duration = Duration::from_millis(250);

/// Connects with an overall `budget`, rechecking `shared.shutdown`
/// between short attempts so a drop is not stuck for the full timeout.
/// A firewalled/suspended host times out each step and retries; a
/// refused or unresolvable address fails fast, as before.
fn connect_interruptible(
    addr: &std::net::SocketAddr,
    budget: Duration,
    shared: &Shared,
) -> std::io::Result<TcpStream> {
    let start = Instant::now();
    loop {
        if shared.shutdown.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(ErrorKind::Interrupted, "shutting down"));
        }
        let remaining = budget.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return Err(std::io::Error::new(
                ErrorKind::TimedOut,
                "connect timed out",
            ));
        }
        match TcpStream::connect_timeout(addr, CONNECT_STEP.min(remaining)) {
            Ok(stream) => return Ok(stream),
            // This attempt's step elapsed; loop to recheck shutdown.
            Err(e) if e.kind() == ErrorKind::TimedOut => {}
            Err(e) => return Err(e),
        }
    }
}

/// Dials a peer's advertised listener and runs the direct handshake.
fn dial(addr: &str, shared: &Shared) -> std::io::Result<TcpStream> {
    let target = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::new(ErrorKind::NotFound, "no address"))?;
    prepare(connect_interruptible(&target, DIAL_TIMEOUT, shared)?)
}

/// The direct-protocol handshake (both sides run it), leaving the stream
/// ready for the linked loop.
fn prepare(mut stream: TcpStream) -> std::io::Result<TcpStream> {
    // An accepted stream can inherit the listener's non-blocking mode on
    // some platforms.
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true).ok();
    stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    stream.write_all(&MAGIC)?;
    let mut peer_magic = [0u8; 8];
    stream.read_exact(&mut peer_magic)?;
    if peer_magic != MAGIC {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "peer is not an emiu2 IR endpoint",
        ));
    }
    // The 1ms read timeout paces the linked loop: sending waits at most
    // 1ms, well within the replay latency budget.
    stream.set_read_timeout(Some(Duration::from_millis(1)))?;
    Ok(stream)
}

/// A handshaken stream becomes the active link.
fn establish(
    stream: TcpStream,
    outgoing: &mpsc::Receiver<OutgoingRecords>,
    shared: &Shared,
) -> ActiveLink {
    // Anything queued while unlinked is stale.
    while outgoing.try_recv().is_ok() {}
    shared.dial.store(DIAL_IDLE, Ordering::Relaxed);
    shared.set_linked(true);
    ActiveLink {
        stream,
        generation: shared.generation.load(Ordering::Relaxed),
        inbuf: Vec::new(),
    }
}

/// One pass over the active link: flush outgoing records, read once.
fn pump(
    link: &mut ActiveLink,
    outgoing: &mpsc::Receiver<OutgoingRecords>,
    incoming: &mpsc::SyncSender<TaggedEdge>,
) -> Pump {
    // Each channel item goes out in one write. The bare-records wire has
    // no framing, so TCP may still split a burst copy; the peer's engine
    // reassembles a split copy before replaying it.
    loop {
        match outgoing.try_recv() {
            Ok(edges) => {
                let mut records = Vec::with_capacity(edges.len() * EDGE_RECORD_LEN);
                for (ns, level) in edges {
                    records.extend_from_slice(&encode_edge(ns, level));
                }
                if let Err(why) = link.stream.write_all(&records) {
                    return Pump::Lost(why);
                }
            }
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => return Pump::Shutdown,
        }
    }

    let mut chunk = [0u8; 1024];
    match link.stream.read(&mut chunk) {
        Ok(0) => Pump::Lost(std::io::Error::new(
            ErrorKind::UnexpectedEof,
            "peer disconnected",
        )),
        Ok(n) => {
            link.inbuf.extend_from_slice(&chunk[..n]);
            // Decode the complete records; a partial one stays buffered
            // until the rest of it arrives.
            let complete = link.inbuf.len() - link.inbuf.len() % EDGE_RECORD_LEN;
            let edges = match decode_edges(&link.inbuf[..complete]) {
                Ok(edges) => edges,
                Err(why) => return Pump::Lost(std::io::Error::new(ErrorKind::InvalidData, why)),
            };
            link.inbuf.drain(..complete);
            for (sender_ns, level) in edges {
                match incoming.try_send(TaggedEdge {
                    generation: link.generation,
                    sender_ns,
                    level,
                }) {
                    Ok(()) => {}
                    // The emulator has stopped draining; drop edges
                    // rather than block the network thread.
                    Err(mpsc::TrySendError::Full(_)) => {}
                    Err(mpsc::TrySendError::Disconnected(_)) => return Pump::Shutdown,
                }
            }
            Pump::Ok
        }
        Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => Pump::Ok,
        Err(e) => Pump::Lost(e),
    }
}

/// How a relay connection ended.
enum OnlineEnd {
    /// The connection dropped; reconnect to the same relay.
    Lost(std::io::Error),
    Mode(LinkMode),
    Shutdown,
}

/// Online mode: keep a relay connection up, until a mode switch (`Some`)
/// or emulator shutdown (`None`).
fn online_session(
    mut relay: String,
    outgoing: &mpsc::Receiver<OutgoingRecords>,
    incoming: &mpsc::SyncSender<TaggedEdge>,
    commands: &mpsc::Receiver<LinkCommand>,
    shared: &Shared,
) -> Option<LinkMode> {
    shared.mode.store(MODE_ONLINE, Ordering::Relaxed);
    shared.dial.store(DIAL_IDLE, Ordering::Relaxed);

    // Persist across reconnects: without this a connection that drops
    // immediately (a relay that accepts then closes) would be redialed
    // with no delay, in a tight loop. A `Join` typed during a reconnect
    // blip is held here and sent once the next connection is up, instead
    // of being dropped (the UI clears the code entry on click).
    let mut last_attempt: Option<Instant> = None;
    let mut pending_join: Option<FriendCode> = None;

    loop {
        // Between connections, stay responsive to mode switches instead
        // of blocking on the connect.
        let stream = loop {
            loop {
                match outgoing.try_recv() {
                    Ok(_) => {} // stale edges from before the drop
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return None,
                }
            }
            loop {
                match commands.try_recv() {
                    Ok(LinkCommand::GoLocal) => return Some(LinkMode::Local),
                    Ok(LinkCommand::GoOnline(addr)) => {
                        // A user-picked relay should be tried at once.
                        relay = addr;
                        last_attempt = None;
                    }
                    Ok(LinkCommand::Join(code)) => pending_join = Some(code),
                    Ok(LinkCommand::Leave) => pending_join = None,
                    Ok(LinkCommand::Shutdown) => return None,
                    // Local-mode commands; nothing to do here.
                    Ok(LinkCommand::ConnectPeer(_)) | Ok(LinkCommand::DisconnectPeer) => {}
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return None,
                }
            }
            // An empty relay is the "no relay server set" state: idle
            // disconnected rather than churning on an unresolvable name.
            if !relay.trim().is_empty()
                && last_attempt.is_none_or(|at| at.elapsed() >= RETRY_INTERVAL)
            {
                last_attempt = Some(Instant::now());
                // The relay may not be reachable yet; retry quietly.
                match relay_connect(&relay, shared) {
                    Ok(stream) => break stream,
                    Err(why) if why.kind() == ErrorKind::Interrupted => return None,
                    Err(_) => {}
                }
            }
            std::thread::sleep(IDLE_POLL);
        };

        let end = run_relay(
            stream,
            &relay,
            pending_join.take(),
            outgoing,
            incoming,
            commands,
            shared,
        )
        .unwrap_or_else(OnlineEnd::Lost);
        shared.relay_connected.store(false, Ordering::Relaxed);
        shared.set_linked(false);
        shared.code.store(0, Ordering::Relaxed);
        match end {
            OnlineEnd::Lost(why) => eprintln!("IR relay: connection lost: {why}"),
            OnlineEnd::Mode(mode) => return Some(mode),
            OnlineEnd::Shutdown => return None,
        }
    }
}

/// Connects to a relay `host:port`, trying each resolved address with a
/// timeout so an unreachable relay cannot stall mode switches for long,
/// and rechecking the shutdown flag between attempts.
fn relay_connect(relay: &str, shared: &Shared) -> std::io::Result<TcpStream> {
    let mut last_err = None;
    for addr in relay.to_socket_addrs()? {
        match connect_interruptible(&addr, RELAY_CONNECT_TIMEOUT, shared) {
            Ok(stream) => return Ok(stream),
            Err(why) if why.kind() == ErrorKind::Interrupted => return Err(why),
            Err(why) => last_err = Some(why),
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::new(ErrorKind::NotFound, "no address")))
}

fn run_relay(
    mut stream: TcpStream,
    relay: &str,
    pending_join: Option<FriendCode>,
    outgoing: &mpsc::Receiver<OutgoingRecords>,
    incoming: &mpsc::SyncSender<TaggedEdge>,
    commands: &mpsc::Receiver<LinkCommand>,
    shared: &Shared,
) -> std::io::Result<OnlineEnd> {
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

    // A join requested during the reconnect gap goes out now, so a blip
    // at the moment the user clicked does not swallow the pairing.
    if let Some(code) = pending_join {
        stream.write_all(&Message::Join { code }.encode())?;
    }

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
                    if !shared.linked.load(Ordering::Relaxed) {
                        continue;
                    }
                    let mut records = Vec::with_capacity(edges.len() * EDGE_RECORD_LEN);
                    for (ns, level) in edges {
                        records.extend_from_slice(&encode_edge(ns, level));
                    }
                    stream.write_all(&Message::IrData { records }.encode())?;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(OnlineEnd::Shutdown),
            }
        }

        // User commands.
        loop {
            match commands.try_recv() {
                Ok(LinkCommand::GoLocal) => return Ok(OnlineEnd::Mode(LinkMode::Local)),
                Ok(LinkCommand::GoOnline(addr)) => {
                    if addr != relay {
                        return Ok(OnlineEnd::Mode(LinkMode::Online { relay: addr }));
                    }
                }
                Ok(LinkCommand::Join(code)) => {
                    stream.write_all(&Message::Join { code }.encode())?;
                }
                Ok(LinkCommand::Leave) => {
                    stream.write_all(&Message::Leave.encode())?;
                }
                Ok(LinkCommand::Shutdown) => return Ok(OnlineEnd::Shutdown),
                // Local-mode commands; nothing to do here.
                Ok(LinkCommand::ConnectPeer(_)) | Ok(LinkCommand::DisconnectPeer) => {}
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(OnlineEnd::Shutdown),
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
                    if let Some(end) = handle_relay_message(message, &mut stream, incoming, shared)?
                    {
                        return Ok(end);
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
            Err(e) => return Err(e),
        }
    }
}

fn handle_relay_message(
    message: Message,
    stream: &mut TcpStream,
    incoming: &mpsc::SyncSender<TaggedEdge>,
    shared: &Shared,
) -> std::io::Result<Option<OnlineEnd>> {
    match message {
        Message::Welcome { code, .. } => {
            shared.code.store(pack_code(code), Ordering::Relaxed);
            shared.relay_connected.store(true, Ordering::Relaxed);
            eprintln!("IR relay: connected. Your friend code is {code}");
        }
        Message::Paired => {
            shared.set_linked(true);
            eprintln!("IR relay: paired with a peer");
        }
        Message::PeerLeft => {
            shared.set_linked(false);
            eprintln!("IR relay: the peer left");
        }
        Message::IrData { records } => {
            if shared.linked.load(Ordering::Relaxed) {
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
                            return Ok(Some(OnlineEnd::Shutdown))
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
    Ok(None)
}
