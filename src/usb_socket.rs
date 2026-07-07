//! Socket bridges for the USB seam: external software speaks raw
//! `UsbTransaction`/`UsbResponse` over a stream, and the emulator services them
//! through the normal peripheral interface.
//!
//! This deliberately does **not** implement USB/IP framing - it's the minimal
//! transport. Anything higher (enumeration, SCSI, the flash protocol) is the
//! connecting software's job; the emulator only moves transactions.
//!
//! Two kinds of endpoint share one protocol:
//! - A **discovery endpoint**, created unconditionally at emulator start, that
//!   host tools find by scanning a well-known runtime directory - the software
//!   analogue of enumerating the USB bus. One Unix socket per emulator instance
//!   on Unix; a loopback TCP port published through a `<pid>.port` file on
//!   Windows (where std has no local-socket type). The endpoint file is the
//!   registry entry: it is removed on clean exit, and a stale one left by a
//!   crash answers `ECONNREFUSED` and is pruned by whoever finds it.
//! - An optional **TCP endpoint** (`--usb-socket ADDR`) for explicit/remote use.
//!
//! Both feed the same [`UsbCable`], which admits one client at a time - a
//! device has one USB port.
//!
//! Whether the cable is **plugged** is the emulator's own state (the player
//! plugs it in - the U key, or `--usb-plugged`/`--connect-mode`), not a side
//! effect of a tool being connected: firmware reacts to bare attachment
//! through the USBCON connect-status bit and needs it stable, exactly like a
//! real cable that stays in whether or not host software is talking. Tools
//! can only exchange transactions while the cable is plugged; an unplugged
//! emulator still answers the hello (so tools can say "plug in the cable")
//! but then closes - nothing is on the bus.
//!
//! Wire format (one outstanding transaction at a time):
//! - on connect, server sends a hello: `"EMIU2USB"  version:u16le  flags:u8
//!   (bit 0 = cable plugged)  identity_len:u32le  identity[..]` (identity is
//!   UTF-8, e.g. the flash image name)
//! - request : `endpoint:u8  token:u8(0=Setup,1=In,2=Out)  len:u32le  data[len]`
//! - response: `kind:u8(0=Ack,1=Nak,2=Stall,3=Data)` then, for Data, `len:u32le data[len]`

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::miuchiz::{UsbResponse, UsbToken, UsbTransaction};
use crate::usb_interface::UsbHostPort;

/// First bytes the server sends on every accepted connection.
pub const HELLO_MAGIC: &[u8; 8] = b"EMIU2USB";
/// Bumped on any incompatible change to the framing below.
pub const PROTOCOL_VERSION: u16 = 2;

/// Hello flags bit: the emulator's USB cable is currently plugged in.
const HELLO_FLAG_PLUGGED: u8 = 1;

fn token_to_u8(token: UsbToken) -> u8 {
    match token {
        UsbToken::Setup => 0,
        UsbToken::In => 1,
        UsbToken::Out => 2,
    }
}

fn token_from_u8(byte: u8) -> io::Result<UsbToken> {
    match byte {
        0 => Ok(UsbToken::Setup),
        1 => Ok(UsbToken::In),
        2 => Ok(UsbToken::Out),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid USB token byte {other}"),
        )),
    }
}

fn read_blob(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    let mut data = vec![0u8; len];
    stream.read_exact(&mut data)?;
    Ok(data)
}

fn write_blob(stream: &mut impl Write, data: &[u8]) -> io::Result<()> {
    stream.write_all(&(data.len() as u32).to_le_bytes())?;
    stream.write_all(data)
}

fn write_transaction(stream: &mut impl Write, txn: &UsbTransaction) -> io::Result<()> {
    stream.write_all(&[txn.endpoint, token_to_u8(txn.token)])?;
    write_blob(stream, &txn.data)
}

fn read_transaction(stream: &mut impl Read) -> io::Result<UsbTransaction> {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header)?;
    Ok(UsbTransaction {
        endpoint: header[0],
        token: token_from_u8(header[1])?,
        data: read_blob(stream)?,
    })
}

fn write_response(stream: &mut impl Write, response: &UsbResponse) -> io::Result<()> {
    match response {
        UsbResponse::Ack => stream.write_all(&[0]),
        UsbResponse::Nak => stream.write_all(&[1]),
        UsbResponse::Stall => stream.write_all(&[2]),
        UsbResponse::Data(data) => {
            stream.write_all(&[3])?;
            write_blob(stream, data)
        }
    }
}

fn read_response(stream: &mut impl Read) -> io::Result<UsbResponse> {
    let mut kind = [0u8; 1];
    stream.read_exact(&mut kind)?;
    match kind[0] {
        0 => Ok(UsbResponse::Ack),
        1 => Ok(UsbResponse::Nak),
        2 => Ok(UsbResponse::Stall),
        3 => Ok(UsbResponse::Data(read_blob(stream)?)),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid USB response kind {other}"),
        )),
    }
}

fn write_hello(stream: &mut impl Write, identity: &str, plugged: bool) -> io::Result<()> {
    stream.write_all(HELLO_MAGIC)?;
    stream.write_all(&PROTOCOL_VERSION.to_le_bytes())?;
    stream.write_all(&[if plugged { HELLO_FLAG_PLUGGED } else { 0 }])?;
    write_blob(stream, identity.as_bytes())
}

/// The identity and cable state an endpoint reports on connect.
struct Hello {
    identity: String,
    plugged: bool,
}

fn read_hello(stream: &mut impl Read) -> io::Result<Hello> {
    let mut magic = [0u8; 8];
    stream.read_exact(&mut magic)?;
    if &magic != HELLO_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not an emiu2 USB endpoint (bad hello magic)",
        ));
    }
    let mut version = [0u8; 2];
    stream.read_exact(&mut version)?;
    let version = u16::from_le_bytes(version);
    if version != PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported USB endpoint protocol version {version}"),
        ));
    }
    let mut flags = [0u8; 1];
    stream.read_exact(&mut flags)?;
    let identity = String::from_utf8(read_blob(stream)?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "hello identity is not UTF-8"))?;
    Ok(Hello {
        identity,
        plugged: flags[0] & HELLO_FLAG_PLUGGED != 0,
    })
}

/// The device's single USB port, shared by every listener. Admits one client
/// at a time; a second connection attempt while a session is running is
/// refused (closed without a hello), the same way a physical port can't take
/// two plugs.
///
/// The cable's plugged state belongs to the emulator (toggled by the player),
/// and the device sees it directly through the USBCON connect-status bit,
/// client or no client. Clients can only exchange transactions while it is
/// plugged.
pub struct UsbCable {
    port: Mutex<UsbHostPort>,
    identity: String,
    /// The cable-presence flag shared with the device (see
    /// `UsbHostPort::connected_flag`) - stored here so it can be driven while
    /// the port itself is locked inside a client session.
    plugged: Arc<AtomicBool>,
}

impl UsbCable {
    /// Wraps the external half of a `channel_pair`. `identity` is reported to
    /// every client in the hello (e.g. the flash image name). The cable
    /// starts unplugged.
    pub fn new(port: UsbHostPort, identity: String) -> Arc<Self> {
        let plugged = port.connected_flag();
        plugged.store(false, Ordering::Relaxed);
        Arc::new(Self {
            port: Mutex::new(port),
            identity,
            plugged,
        })
    }

    /// Plugs or unplugs the cable. The device notices through the USBCON
    /// connect-status bit; unplugging also ends any running client session
    /// (at its next transaction).
    pub fn set_plugged(&self, plugged: bool) {
        self.plugged.store(plugged, Ordering::Relaxed);
    }

    pub fn plugged(&self) -> bool {
        self.plugged.load(Ordering::Relaxed)
    }

    /// Serve one client stream until it disconnects. Returns immediately (with
    /// no hello sent) if another client currently holds the cable, and right
    /// after the hello when the cable is unplugged (nothing is on the bus).
    pub fn attach(&self, mut stream: impl Read + Write) -> io::Result<()> {
        let Ok(port) = self.port.try_lock() else {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "another client holds the USB cable",
            ));
        };
        let plugged = self.plugged();
        write_hello(&mut stream, &self.identity, plugged)?;
        if !plugged {
            return Ok(());
        }
        self.serve_client(stream, &port)
    }

    fn serve_client(&self, mut stream: impl Read + Write, port: &UsbHostPort) -> io::Result<()> {
        loop {
            // Blocks until the client sends a transaction (or disconnects).
            let txn = read_transaction(&mut stream)?;
            // An unplugged cable carries nothing: end the session.
            if !self.plugged() {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "the USB cable was unplugged",
                ));
            }
            port.submit(txn);
            // Blocks until the emulator's main loop services it and responds.
            let response = port.response_blocking().ok_or_else(|| {
                io::Error::new(io::ErrorKind::BrokenPipe, "device interface gone")
            })?;
            write_response(&mut stream, &response)?;
        }
    }
}

/// Run the TCP bridge (the `--usb-socket ADDR` endpoint): accept one client at
/// a time and serve it through the shared cable. Intended for its own thread.
///
/// Blocks forever (until a fatal listener error). Per-client I/O errors just
/// end that connection and wait for the next.
pub fn serve<A: ToSocketAddrs>(addr: A, cable: Arc<UsbCable>) -> io::Result<()> {
    serve_tcp(TcpListener::bind(addr)?, cable)
}

/// Like [`serve`], but on an already-bound listener (useful when the caller
/// needs the resolved address first, e.g. an ephemeral port).
pub fn serve_tcp(listener: TcpListener, cable: Arc<UsbCable>) -> io::Result<()> {
    // One thread per connection so that a client arriving while the cable is
    // held gets refused promptly instead of waiting in the accept backlog.
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                stream.set_nodelay(true).ok();
                let cable = cable.clone();
                std::thread::spawn(move || {
                    if let Err(why) = cable.attach(stream) {
                        eprintln!("USB socket client disconnected: {why}");
                    }
                });
            }
            Err(why) => eprintln!("USB socket accept error: {why}"),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn serve_unix(listener: std::os::unix::net::UnixListener, cable: Arc<UsbCable>) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let cable = cable.clone();
                std::thread::spawn(move || {
                    if let Err(why) = cable.attach(stream) {
                        eprintln!("USB endpoint client disconnected: {why}");
                    }
                });
            }
            Err(why) => eprintln!("USB endpoint accept error: {why}"),
        }
    }
}

/// The directory host tools scan to find running emulators: emiu2's runtime
/// directory under the shared Miuchiz Reborn storage-location policy
/// (miuchiz-reborn-paths; the C library mirrors the same spec), so ecosystem
/// tools agree without configuration and `MIUCHIZ_REBORN_HOME` reroots
/// everything at once. `EMIU2_USB_DIR` remains a narrower, higher-priority
/// override of just this directory.
pub fn endpoint_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("EMIU2_USB_DIR") {
        return PathBuf::from(dir);
    }
    miuchiz_reborn_paths::AppDirs::new("emiu2")
        .runtime_dir()
        .to_path_buf()
}

/// Removes the discovery endpoint file when the emulator exits. Held by main
/// for the process lifetime; a crash leaves the file behind, which is fine -
/// stale endpoints refuse connections and get pruned by the next scan.
pub struct EndpointGuard {
    path: PathBuf,
}

impl EndpointGuard {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for EndpointGuard {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}

/// Creates this emulator's discovery endpoint in `dir` and starts serving it
/// on a background thread. See [`create_discovery_endpoint`].
pub fn create_discovery_endpoint_in(dir: &Path, cable: Arc<UsbCable>) -> io::Result<EndpointGuard> {
    std::fs::create_dir_all(dir)?;
    prune_stale_endpoints(dir);
    let pid = std::process::id();

    #[cfg(unix)]
    {
        let path = dir.join(format!("{pid}.sock"));
        // A previous process with our pid may have crashed and left its socket.
        std::fs::remove_file(&path).ok();
        let listener = std::os::unix::net::UnixListener::bind(&path)?;
        let guard = EndpointGuard { path };
        std::thread::spawn(move || serve_unix(listener, cable));
        Ok(guard)
    }
    #[cfg(windows)]
    {
        // Windows has no std local-socket type, so the endpoint is a loopback
        // TCP port published through a port file.
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let path = dir.join(format!("{pid}.port"));
        std::fs::write(&path, port.to_string())?;
        let guard = EndpointGuard { path };
        std::thread::spawn(move || {
            if let Err(why) = serve_tcp(listener, cable) {
                eprintln!("USB endpoint server failed: {why}");
            }
        });
        Ok(guard)
    }
}

/// Creates the discovery endpoint in the default [`endpoint_dir`].
pub fn create_discovery_endpoint(cable: Arc<UsbCable>) -> io::Result<EndpointGuard> {
    create_discovery_endpoint_in(&endpoint_dir(), cable)
}

/// Removes endpoint files whose emulator is gone (connecting fails outright).
/// Best-effort housekeeping; scanning tools do the same.
fn prune_stale_endpoints(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if endpoint_connect(&path).is_err() {
            std::fs::remove_file(&path).ok();
        }
    }
}

/// A stream to an endpoint, abstracting over the per-platform socket type.
type EndpointStream = Box<dyn ReadWrite>;

pub trait ReadWrite: Read + Write + Send {}
impl<T: Read + Write + Send> ReadWrite for T {}

/// Connects to the endpoint behind a discovery file (`.sock` or `.port`).
/// Fails fast when the emulator behind it is gone.
fn endpoint_connect(path: &Path) -> io::Result<EndpointStream> {
    match path.extension().and_then(|e| e.to_str()) {
        #[cfg(unix)]
        Some("sock") => {
            let stream = std::os::unix::net::UnixStream::connect(path)?;
            Ok(Box::new(stream))
        }
        Some("port") => {
            let port: u16 = std::fs::read_to_string(path)?
                .trim()
                .parse()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad port file"))?;
            let stream = TcpStream::connect(("127.0.0.1", port))?;
            stream.set_nodelay(true).ok();
            Ok(Box::new(stream))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not an endpoint file",
        )),
    }
}

/// A discovered emulator endpoint.
pub struct DiscoveredEndpoint {
    /// The endpoint file (pass to [`RemoteUsbDevice::connect_endpoint`]).
    pub path: PathBuf,
    /// The identity string from the hello (e.g. the flash image name).
    pub identity: String,
    /// Whether the emulator's USB cable is plugged in. Transactions are only
    /// possible while it is.
    pub plugged: bool,
}

/// Scans `dir` for live emulator endpoints, pruning stale ones. Endpoints
/// whose cable is currently held by another client are skipped.
pub fn discover_in(dir: &Path) -> Vec<DiscoveredEndpoint> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match endpoint_connect(&path).and_then(|mut stream| read_hello(&mut stream)) {
            Ok(hello) => found.push(DiscoveredEndpoint {
                path,
                identity: hello.identity,
                plugged: hello.plugged,
            }),
            Err(why) if why.kind() == io::ErrorKind::ConnectionRefused => {
                std::fs::remove_file(&path).ok();
            }
            Err(_) => {}
        }
    }
    found
}

/// Scans the default [`endpoint_dir`] for live emulator endpoints.
pub fn discover() -> Vec<DiscoveredEndpoint> {
    discover_in(&endpoint_dir())
}

/// Client library: connect to an emulator USB endpoint and issue transactions.
/// The interfacing software builds enumeration / SCSI / flash protocol on top.
pub struct RemoteUsbDevice {
    stream: EndpointStream,
    identity: String,
    plugged: bool,
}

impl RemoteUsbDevice {
    /// Connect to a `--usb-socket` TCP endpoint.
    pub fn connect<A: ToSocketAddrs>(addr: A) -> io::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true).ok();
        Self::from_stream(Box::new(stream))
    }

    /// Connect to a discovery endpoint file (from [`discover`]).
    pub fn connect_endpoint(path: &Path) -> io::Result<Self> {
        Self::from_stream(endpoint_connect(path)?)
    }

    fn from_stream(mut stream: EndpointStream) -> io::Result<Self> {
        let hello = read_hello(&mut stream)?;
        Ok(Self {
            stream,
            identity: hello.identity,
            plugged: hello.plugged,
        })
    }

    /// The identity string the emulator reported (e.g. the flash image name).
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Whether the emulator's cable was plugged at connect time. When false,
    /// the endpoint has already closed the connection - nothing is on the bus
    /// until someone plugs the cable in.
    pub fn plugged(&self) -> bool {
        self.plugged
    }

    /// Issue one transaction and block for the device's response.
    pub fn transaction(&mut self, txn: &UsbTransaction) -> io::Result<UsbResponse> {
        write_transaction(&mut self.stream, txn)?;
        read_response(&mut self.stream)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::usb_interface::channel_pair;

    /// Services the device half of a channel pair with a fixed response.
    fn respond_with(
        internal: crate::usb_interface::ChannelUsbInterface,
        response: UsbResponse,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            use crate::usb_interface::UsbInterfaceInternal;
            let mut internal = internal;
            loop {
                if let Some(_txn) = internal.poll_transaction() {
                    internal.respond(response.clone());
                } else {
                    std::thread::sleep(std::time::Duration::from_micros(50));
                }
            }
        })
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("emiu2-usb-test-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn endpoint_hello_and_transaction_roundtrip() {
        let dir = temp_dir("roundtrip");
        let (port, internal) = channel_pair();
        respond_with(internal, UsbResponse::Data(vec![1, 2, 3]));

        let cable = UsbCable::new(port, "Spike 1.02.dat".into());
        cable.set_plugged(true);
        let _guard = create_discovery_endpoint_in(&dir, cable).unwrap();

        let found = discover_in(&dir);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].identity, "Spike 1.02.dat");
        assert!(found[0].plugged);

        let mut device = RemoteUsbDevice::connect_endpoint(&found[0].path).unwrap();
        assert_eq!(device.identity(), "Spike 1.02.dat");
        let response = device
            .transaction(&UsbTransaction {
                endpoint: 1,
                token: UsbToken::In,
                data: vec![],
            })
            .unwrap();
        assert_eq!(response, UsbResponse::Data(vec![1, 2, 3]));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cable_admits_one_client_at_a_time() {
        let dir = temp_dir("busy");
        let (port, internal) = channel_pair();
        respond_with(internal, UsbResponse::Ack);

        let cable = UsbCable::new(port, "flash.dat".into());
        cable.set_plugged(true);
        let guard = create_discovery_endpoint_in(&dir, cable).unwrap();

        let _first = RemoteUsbDevice::connect_endpoint(guard.path()).unwrap();
        // The second client is refused before the hello.
        assert!(RemoteUsbDevice::connect_endpoint(guard.path()).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unplugged_cable_reports_in_hello_and_carries_nothing() {
        let dir = temp_dir("unplugged");
        let (port, internal) = channel_pair();
        respond_with(internal, UsbResponse::Ack);

        let cable = UsbCable::new(port, "flash.dat".into());
        let guard = create_discovery_endpoint_in(&dir, cable.clone()).unwrap();

        // Discovery still sees the emulator, marked unplugged.
        let found = discover_in(&dir);
        assert_eq!(found.len(), 1);
        assert!(!found[0].plugged);

        // A client learns the cable state from the hello, but the connection
        // carries no transactions.
        let mut device = RemoteUsbDevice::connect_endpoint(guard.path()).unwrap();
        assert!(!device.plugged());
        assert!(device
            .transaction(&UsbTransaction {
                endpoint: 1,
                token: UsbToken::In,
                data: vec![],
            })
            .is_err());

        // Plugging the cable makes the next session work.
        cable.set_plugged(true);
        let mut device = RemoteUsbDevice::connect_endpoint(guard.path()).unwrap();
        assert!(device.plugged());
        assert_eq!(
            device
                .transaction(&UsbTransaction {
                    endpoint: 1,
                    token: UsbToken::In,
                    data: vec![],
                })
                .unwrap(),
            UsbResponse::Ack
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stale_endpoints_are_pruned_by_discovery() {
        let dir = temp_dir("stale");
        // An orphaned socket file: the listener is gone but the file remains
        // (a crashed emulator), so connecting to it is refused.
        let stale = dir.join("99999.sock");
        drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());

        assert!(stale.exists());
        assert!(discover_in(&dir).is_empty());
        assert!(!stale.exists(), "stale endpoint should have been pruned");

        std::fs::remove_dir_all(&dir).ok();
    }
}
