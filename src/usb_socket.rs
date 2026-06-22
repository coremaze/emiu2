//! A plain TCP bridge for the USB seam: external software speaks raw
//! `UsbTransaction`/`UsbResponse` over a socket, and the emulator services them
//! through the normal peripheral interface.
//!
//! This deliberately does **not** implement USB/IP framing - it's the minimal
//! transport. Anything higher (enumeration, SCSI, the flash protocol) is the
//! connecting software's job; the emulator only moves transactions.
//!
//! Layout, mirroring the other peripherals' two-halves pattern:
//! - [`serve`] runs on its own thread holding the external [`UsbHostPort`]; the
//!   emulator's main loop holds the internal half and stays the sole CPU driver.
//! - [`RemoteUsbDevice`] is the client library the interfacing software uses.
//!
//! Wire format (one outstanding transaction at a time):
//! - request : `endpoint:u8  token:u8(0=Setup,1=In,2=Out)  len:u32le  data[len]`
//! - response: `kind:u8(0=Ack,1=Nak,2=Stall,3=Data)` then, for Data, `len:u32le data[len]`

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};

use crate::miuchiz::{UsbResponse, UsbToken, UsbTransaction};
use crate::usb_interface::UsbHostPort;

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

/// Run the bridge server: accept one client at a time and, for each transaction
/// it sends, submit it to the device and return the device's response. Intended
/// to run on its own thread with the external half of a `channel_pair`.
///
/// Blocks forever (until a fatal listener error). Per-client I/O errors just end
/// that connection and wait for the next.
pub fn serve<A: ToSocketAddrs>(addr: A, port: UsbHostPort) -> io::Result<()> {
    serve_on(TcpListener::bind(addr)?, port)
}

/// Like [`serve`], but on an already-bound listener (useful when the caller
/// needs the resolved address first, e.g. an ephemeral test port).
pub fn serve_on(listener: TcpListener, port: UsbHostPort) -> io::Result<()> {
    // Listening with no client attached: the device should see an unplugged
    // cable (USBCON connect bit clear) until a client actually connects.
    port.set_connected(false);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                // A client is attached for the duration of this connection - the
                // device's connect-status bit goes high, which is how firmware
                // detects USB. It drops again when the client disconnects.
                port.set_connected(true);
                if let Err(why) = serve_client(stream, &port) {
                    eprintln!("USB socket client disconnected: {why}");
                }
                port.set_connected(false);
            }
            Err(why) => eprintln!("USB socket accept error: {why}"),
        }
    }
    Ok(())
}

fn serve_client(mut stream: TcpStream, port: &UsbHostPort) -> io::Result<()> {
    stream.set_nodelay(true).ok();
    loop {
        // Blocks until the client sends a transaction (or disconnects).
        let txn = read_transaction(&mut stream)?;
        port.submit(txn);
        // Blocks until the emulator's main loop services it and responds.
        let response = port
            .response_blocking()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "device interface gone"))?;
        write_response(&mut stream, &response)?;
    }
}

/// Client library: connect to a [`serve`] endpoint and issue USB transactions.
/// The interfacing software builds enumeration / SCSI / flash protocol on top.
pub struct RemoteUsbDevice {
    stream: TcpStream,
}

impl RemoteUsbDevice {
    /// Connect to a running USB transaction socket.
    pub fn connect<A: ToSocketAddrs>(addr: A) -> io::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true).ok();
        Ok(Self { stream })
    }

    /// Issue one transaction and block for the device's response.
    pub fn transaction(&mut self, txn: &UsbTransaction) -> io::Result<UsbResponse> {
        write_transaction(&mut self.stream, txn)?;
        read_response(&mut self.stream)
    }
}
