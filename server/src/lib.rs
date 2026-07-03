//! The emiu2 relay server: pairs emulators by friend code and relays
//! their IR traffic. See the `emiu2-netplay` crate for the protocol.
//!
//! Every accepted connection is served by two threads (a reader and a
//! writer); pairing state lives in one mutex-guarded registry. Friend
//! codes are ephemeral: assigned when a client says hello, gone when it
//! disconnects.
//!
//! Two dialects share the port, selected by the first byte a client
//! sends: native emulators send an 8-byte magic and then the
//! length-prefixed protocol as a plain TCP stream, while browsers send
//! an HTTP `GET` upgrade and then carry each protocol message in one
//! WebSocket binary frame.

mod base64;
mod registry;
mod sha1;
pub mod websocket;

use emiu2_netplay::{error_code, Decoder, FriendCode, Message, CLIENT_MAGIC, PROTOCOL_VERSION};
use registry::{JoinOutcome, Registry};
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use websocket::{FrameAssembler, WsEvent};

/// Refuse connections beyond this many concurrent clients.
const MAX_CLIENTS: usize = 512;

/// The server sends a ping after this much silence...
const PING_INTERVAL: Duration = Duration::from_secs(10);
/// ...and drops a client after this much.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Minimum spacing between join attempts, against code guessing.
const JOIN_INTERVAL: Duration = Duration::from_millis(500);

pub struct RelayServer {
    listener: TcpListener,
    registry: Arc<Mutex<Registry>>,
}

impl RelayServer {
    pub fn bind(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(addr)?,
            registry: Arc::new(Mutex::new(Registry::new())),
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Accepts and serves clients until the listener fails.
    pub fn run(self) {
        loop {
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    if self.registry.lock().expect("registry mutex poisoned").len() >= MAX_CLIENTS {
                        eprintln!("relay: refusing {peer}: server full");
                        drop(stream);
                        continue;
                    }
                    let registry = self.registry.clone();
                    std::thread::spawn(move || {
                        if let Err(why) = serve_client(stream, &registry) {
                            eprintln!("relay: {peer}: {why}");
                        }
                    });
                }
                Err(why) => {
                    eprintln!("relay: accept failed: {why}");
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    /// Runs the server on a background thread; used by tests.
    pub fn spawn(self) -> SocketAddr {
        let addr = self.local_addr().expect("listener has an address");
        std::thread::spawn(move || self.run());
        addr
    }
}

/// What a connection's writer thread can be asked to send.
pub(crate) enum Outgoing {
    /// An encoded protocol message (framed per dialect by the writer).
    Protocol(Vec<u8>),
    /// A WebSocket pong; native connections have no equivalent.
    WsPong(Vec<u8>),
}

/// How a connected client receives messages: queued to its writer.
pub(crate) type ClientSender = mpsc::Sender<Outgoing>;

#[derive(Clone, Copy)]
enum Framing {
    Native,
    WebSocket,
}

/// Per-dialect incremental parsing of the byte stream into messages.
enum Dialect {
    Native(Decoder),
    WebSocket(FrameAssembler),
}

impl Dialect {
    fn ingest(
        &mut self,
        bytes: &[u8],
        sender: &ClientSender,
        messages: &mut VecDeque<Message>,
    ) -> std::io::Result<()> {
        match self {
            Dialect::Native(decoder) => {
                decoder.push(bytes);
                while let Some(message) = decoder
                    .try_next()
                    .map_err(|why| std::io::Error::new(ErrorKind::InvalidData, why))?
                {
                    messages.push_back(message);
                }
            }
            Dialect::WebSocket(assembler) => {
                assembler.push(bytes);
                while let Some(event) = assembler.try_next()? {
                    match event {
                        WsEvent::Message(frame) => {
                            let message = Message::decode_frame(&frame)
                                .map_err(|why| std::io::Error::new(ErrorKind::InvalidData, why))?;
                            messages.push_back(message);
                        }
                        WsEvent::Ping(payload) => {
                            let _ = sender.send(Outgoing::WsPong(payload));
                        }
                        WsEvent::Close => {
                            return Err(std::io::Error::new(
                                ErrorKind::ConnectionAborted,
                                "websocket close",
                            ))
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn serve_client(mut stream: TcpStream, registry: &Arc<Mutex<Registry>>) -> std::io::Result<()> {
    stream.set_nodelay(true).ok();
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    // The first byte selects the dialect: an HTTP method letter means a
    // WebSocket upgrade, anything else must begin the native magic.
    let mut first = [0u8; 1];
    stream.read_exact(&mut first)?;
    if first[0] == b'G' {
        return serve_websocket_client(stream, registry, first[0]);
    }

    let mut magic_rest = [0u8; CLIENT_MAGIC.len() - 1];
    stream.read_exact(&mut magic_rest)?;
    if first[0] != CLIENT_MAGIC[0] || magic_rest != CLIENT_MAGIC[1..] {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "not an emiu2 netplay client",
        ));
    }

    serve_session(
        stream,
        registry,
        Framing::Native,
        Dialect::Native(Decoder::new()),
        Vec::new(),
    )
}

fn serve_websocket_client(
    mut stream: TcpStream,
    registry: &Arc<Mutex<Registry>>,
    first: u8,
) -> std::io::Result<()> {
    // Read the request head; the first byte was consumed by the sniff.
    let mut request = vec![first];
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        if let Some(end) = find_header_end(&request) {
            break end;
        }
        if request.len() > 16 * 1024 {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "oversized HTTP request",
            ));
        }
        match stream.read(&mut chunk)? {
            0 => {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "incomplete HTTP request",
                ))
            }
            n => request.extend_from_slice(&chunk[..n]),
        }
    };

    match websocket::upgrade_response(&request[..header_end]) {
        Ok(response) => stream.write_all(&response)?,
        Err(response) => {
            stream.write_all(&response).ok();
            return Ok(());
        }
    }

    // Bytes past the header already belong to the frame stream.
    let leftover = request[header_end..].to_vec();
    serve_session(
        stream,
        registry,
        Framing::WebSocket,
        Dialect::WebSocket(FrameAssembler::new()),
        leftover,
    )
}

fn find_header_end(request: &[u8]) -> Option<usize> {
    request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

fn serve_session(
    stream: TcpStream,
    registry: &Arc<Mutex<Registry>>,
    framing: Framing,
    mut dialect: Dialect,
    initial: Vec<u8>,
) -> std::io::Result<()> {
    let writer_stream = stream.try_clone()?;
    let (sender, outbox) = mpsc::channel::<Outgoing>();
    let writer = std::thread::spawn(move || writer_loop(writer_stream, outbox, framing));

    let result = reader_session(stream, registry, &sender, &mut dialect, initial);

    // Closing the channel ends the writer; the writer shuts the socket
    // down when it exits, which also unblocks any pending read.
    drop(sender);
    writer.join().ok();
    result
}

fn writer_loop(mut stream: TcpStream, outbox: mpsc::Receiver<Outgoing>, framing: Framing) {
    loop {
        let bytes = match outbox.recv_timeout(PING_INTERVAL) {
            Ok(Outgoing::Protocol(bytes)) => match framing {
                Framing::Native => bytes,
                Framing::WebSocket => websocket::frame_binary(&bytes),
            },
            Ok(Outgoing::WsPong(payload)) => match framing {
                Framing::Native => continue,
                Framing::WebSocket => websocket::frame_pong(&payload),
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let ping = Message::Ping.encode();
                match framing {
                    Framing::Native => ping,
                    Framing::WebSocket => websocket::frame_binary(&ping),
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if stream.write_all(&bytes).is_err() {
            break;
        }
    }
    if let Framing::WebSocket = framing {
        stream.write_all(&websocket::frame_close()).ok();
    }
    stream.shutdown(Shutdown::Both).ok();
}

/// The per-client session state the reader loop tracks.
struct Session {
    code: FriendCode,
    last_join: Option<Instant>,
}

fn reader_session(
    mut stream: TcpStream,
    registry: &Arc<Mutex<Registry>>,
    sender: &ClientSender,
    dialect: &mut Dialect,
    initial: Vec<u8>,
) -> std::io::Result<()> {
    let mut pending = VecDeque::new();
    dialect.ingest(&initial, sender, &mut pending)?;

    // The first message must be a Hello with a compatible version.
    let hello = next_message_blocking(&mut stream, sender, dialect, &mut pending)?;
    let Message::Hello { version } = hello else {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "expected Hello",
        ));
    };
    if version != PROTOCOL_VERSION {
        let _ = sender.send(Outgoing::Protocol(
            Message::Error {
                code: error_code::BAD_VERSION,
                message: format!("server speaks version {PROTOCOL_VERSION}"),
            }
            .encode(),
        ));
        return Ok(());
    }

    let code = registry
        .lock()
        .expect("registry mutex poisoned")
        .register(sender.clone());
    let _ = sender.send(Outgoing::Protocol(
        Message::Welcome {
            version: PROTOCOL_VERSION,
            code,
        }
        .encode(),
    ));

    let mut session = Session {
        code,
        last_join: None,
    };

    let result = session_loop(
        &mut stream,
        registry,
        sender,
        dialect,
        &mut pending,
        &mut session,
    );
    registry
        .lock()
        .expect("registry mutex poisoned")
        .unregister(session.code);
    result
}

fn session_loop(
    stream: &mut TcpStream,
    registry: &Arc<Mutex<Registry>>,
    sender: &ClientSender,
    dialect: &mut Dialect,
    pending: &mut VecDeque<Message>,
    session: &mut Session,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    let mut last_activity = Instant::now();
    let mut chunk = [0u8; 4096];

    loop {
        while let Some(message) = pending.pop_front() {
            handle_message(message, registry, sender, session);
        }

        match stream.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                last_activity = Instant::now();
                dialect.ingest(&chunk[..n], sender, pending)?;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                if last_activity.elapsed() > IDLE_TIMEOUT {
                    return Err(std::io::Error::new(ErrorKind::TimedOut, "client idle"));
                }
            }
            Err(e) => return Err(e),
        }
    }
}

fn handle_message(
    message: Message,
    registry: &Arc<Mutex<Registry>>,
    sender: &ClientSender,
    session: &mut Session,
) {
    match message {
        Message::Join { code } => {
            let now = Instant::now();
            if let Some(last) = session.last_join {
                if now.duration_since(last) < JOIN_INTERVAL {
                    send_error(sender, error_code::THROTTLED, "joining too fast");
                    return;
                }
            }
            session.last_join = Some(now);

            let outcome = registry
                .lock()
                .expect("registry mutex poisoned")
                .join(session.code, code);
            match outcome {
                JoinOutcome::Paired => {}
                JoinOutcome::UnknownCode => {
                    send_error(sender, error_code::UNKNOWN_CODE, "no such friend code")
                }
                JoinOutcome::PeerBusy => send_error(
                    sender,
                    error_code::PEER_BUSY,
                    "that player is already paired",
                ),
                JoinOutcome::SelfJoin => {
                    send_error(sender, error_code::SELF_JOIN, "that is your own code")
                }
                JoinOutcome::AlreadyPaired => send_error(
                    sender,
                    error_code::ALREADY_PAIRED,
                    "leave your current pairing first",
                ),
            }
        }
        Message::Leave => {
            registry
                .lock()
                .expect("registry mutex poisoned")
                .unpair(session.code);
        }
        Message::IrData { records } => {
            registry
                .lock()
                .expect("registry mutex poisoned")
                .relay(session.code, Message::IrData { records });
        }
        Message::Ping => {
            let _ = sender.send(Outgoing::Protocol(Message::Pong.encode()));
        }
        Message::Pong => {}
        // Server-to-client messages or a second Hello from a client are
        // nonsense; ignore rather than kill a working link.
        Message::Hello { .. }
        | Message::Welcome { .. }
        | Message::Paired
        | Message::PeerLeft
        | Message::Error { .. } => {}
    }
}

fn send_error(sender: &ClientSender, code: u8, message: &str) {
    let _ = sender.send(Outgoing::Protocol(
        Message::Error {
            code,
            message: message.into(),
        }
        .encode(),
    ));
}

/// Blocking read of the next complete message (used for the Hello).
fn next_message_blocking(
    stream: &mut TcpStream,
    sender: &ClientSender,
    dialect: &mut Dialect,
    pending: &mut VecDeque<Message>,
) -> std::io::Result<Message> {
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(message) = pending.pop_front() {
            return Ok(message);
        }
        match stream.read(&mut chunk) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "disconnected before Hello",
                ))
            }
            Ok(n) => dialect.ingest(&chunk[..n], sender, pending)?,
            Err(e) => return Err(e),
        }
    }
}
