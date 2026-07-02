//! The emiu2 relay server: pairs emulators by friend code and relays
//! their IR traffic. See the `emiu2-netplay` crate for the protocol.
//!
//! Every accepted connection is served by two threads (a reader and a
//! writer); pairing state lives in one mutex-guarded registry. Friend
//! codes are ephemeral: assigned when a client says hello, gone when it
//! disconnects.
//!
//! Native emulators speak the length-prefixed protocol directly over
//! TCP after an 8-byte magic. The first byte a client sends selects the
//! dialect, which is how WebSocket support (an HTTP `GET` upgrade)
//! shares the port.

mod registry;

use emiu2_netplay::{error_code, Decoder, FriendCode, Message, CLIENT_MAGIC, PROTOCOL_VERSION};
use registry::{JoinOutcome, Registry};
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
                    if self.registry.lock().unwrap().len() >= MAX_CLIENTS {
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

/// How a connected client receives bytes: pre-encoded messages queued to
/// its writer thread.
pub(crate) type ClientSender = mpsc::Sender<Vec<u8>>;

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

    serve_native_client(stream, registry)
}

/// Placeholder until WebSocket support lands: refuse politely.
fn serve_websocket_client(
    mut stream: TcpStream,
    _registry: &Arc<Mutex<Registry>>,
    _first: u8,
) -> std::io::Result<()> {
    stream
        .write_all(b"HTTP/1.1 501 Not Implemented\r\nConnection: close\r\n\r\n")
        .ok();
    Ok(())
}

fn serve_native_client(stream: TcpStream, registry: &Arc<Mutex<Registry>>) -> std::io::Result<()> {
    let writer_stream = stream.try_clone()?;
    let (sender, outbox) = mpsc::channel::<Vec<u8>>();
    let writer = std::thread::spawn(move || writer_loop(writer_stream, outbox));

    let result = native_reader_loop(stream, registry, &sender);

    // Closing the channel ends the writer; the writer shuts the socket
    // down when it exits, which also unblocks any pending read.
    drop(sender);
    writer.join().ok();
    result
}

fn writer_loop(mut stream: TcpStream, outbox: mpsc::Receiver<Vec<u8>>) {
    loop {
        match outbox.recv_timeout(PING_INTERVAL) {
            Ok(bytes) => {
                if stream.write_all(&bytes).is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if stream.write_all(&Message::Ping.encode()).is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    stream.shutdown(Shutdown::Both).ok();
}

/// The per-client session state the reader loop tracks.
struct Session {
    code: FriendCode,
    last_join: Option<Instant>,
}

fn native_reader_loop(
    mut stream: TcpStream,
    registry: &Arc<Mutex<Registry>>,
    sender: &ClientSender,
) -> std::io::Result<()> {
    let mut decoder = Decoder::new();

    // The first message must be a Hello with a compatible version.
    let hello = read_message(&mut stream, &mut decoder)?;
    let Message::Hello { version } = hello else {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "expected Hello",
        ));
    };
    if version != PROTOCOL_VERSION {
        let _ = sender.send(
            Message::Error {
                code: error_code::BAD_VERSION,
                message: format!("server speaks version {PROTOCOL_VERSION}"),
            }
            .encode(),
        );
        return Ok(());
    }

    let code = registry.lock().unwrap().register(sender.clone());
    let _ = sender.send(
        Message::Welcome {
            version: PROTOCOL_VERSION,
            code,
        }
        .encode(),
    );

    let mut session = Session {
        code,
        last_join: None,
    };

    let result = session_loop(&mut stream, registry, sender, &mut decoder, &mut session);
    registry.lock().unwrap().unregister(session.code);
    result
}

fn session_loop(
    stream: &mut TcpStream,
    registry: &Arc<Mutex<Registry>>,
    sender: &ClientSender,
    decoder: &mut Decoder,
    session: &mut Session,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    let mut last_activity = Instant::now();
    let mut chunk = [0u8; 4096];

    loop {
        match stream.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                last_activity = Instant::now();
                decoder.push(&chunk[..n]);
                while let Some(message) = decoder
                    .next()
                    .map_err(|why| std::io::Error::new(ErrorKind::InvalidData, why))?
                {
                    handle_message(message, registry, sender, session);
                }
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

            let outcome = registry.lock().unwrap().join(session.code, code);
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
            registry.lock().unwrap().unpair(session.code);
        }
        Message::IrData { records } => {
            registry
                .lock()
                .unwrap()
                .relay(session.code, Message::IrData { records });
        }
        Message::Ping => {
            let _ = sender.send(Message::Pong.encode());
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
    let _ = sender.send(
        Message::Error {
            code,
            message: message.into(),
        }
        .encode(),
    );
}

/// Blocking read of the next complete message (used for the Hello).
fn read_message(stream: &mut TcpStream, decoder: &mut Decoder) -> std::io::Result<Message> {
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(message) = decoder
            .next()
            .map_err(|why| std::io::Error::new(ErrorKind::InvalidData, why))?
        {
            return Ok(message);
        }
        match stream.read(&mut chunk) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "disconnected before Hello",
                ))
            }
            Ok(n) => decoder.push(&chunk[..n]),
            Err(e) => return Err(e),
        }
    }
}
