//! Tests of the relay's WebSocket dialect, driven by a minimal
//! WebSocket client that speaks the way a browser does: masked client
//! frames, one protocol message per binary frame.

use emiu2_netplay::{FriendCode, Message, PROTOCOL_VERSION};
use emiu2_relay::{websocket, RelayServer};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

const CLIENT_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

struct WsClient {
    stream: TcpStream,
    buffer: Vec<u8>,
    pub code: FriendCode,
}

impl WsClient {
    fn connect(addr: SocketAddr) -> Self {
        let mut stream = TcpStream::connect(addr).expect("connect to relay");
        stream.set_nodelay(true).ok();
        stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();

        let request = format!(
            "GET / HTTP/1.1\r\nHost: relay\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Key: {CLIENT_KEY}\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).unwrap();

        // Read the 101 response head.
        let mut response = Vec::new();
        let mut chunk = [0u8; 1024];
        let deadline = Instant::now() + Duration::from_secs(5);
        let header_end = loop {
            if let Some(position) = response.windows(4).position(|w| w == b"\r\n\r\n") {
                break position + 4;
            }
            assert!(Instant::now() < deadline, "no handshake response");
            match stream.read(&mut chunk) {
                Ok(0) => panic!("relay closed during handshake"),
                Ok(n) => response.extend_from_slice(&chunk[..n]),
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => panic!("handshake read failed: {e}"),
            }
        };

        let head = String::from_utf8_lossy(&response[..header_end]).to_string();
        assert!(
            head.starts_with("HTTP/1.1 101"),
            "unexpected response: {head}"
        );
        assert!(
            head.contains(&websocket::accept_key(CLIENT_KEY)),
            "wrong Sec-WebSocket-Accept in: {head}"
        );

        let mut client = Self {
            stream,
            buffer: response[header_end..].to_vec(),
            code: FriendCode::parse("222222").unwrap(), // replaced below
        };
        client.send(Message::Hello {
            version: PROTOCOL_VERSION,
        });
        match client.expect_message() {
            Message::Welcome { version, code } => {
                assert_eq!(version, PROTOCOL_VERSION);
                client.code = code;
            }
            other => panic!("expected Welcome, got {other:?}"),
        }
        client
    }

    fn send(&mut self, message: Message) {
        self.send_frame(0x2, &message.encode());
    }

    /// Sends one masked client frame.
    fn send_frame(&mut self, opcode: u8, payload: &[u8]) {
        let mask = [0x21u8, 0x43, 0x65, 0x87];
        let mut frame = vec![0x80 | opcode];
        if payload.len() < 126 {
            frame.push(0x80 | payload.len() as u8);
        } else {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.stream.write_all(&frame).unwrap();
    }

    /// Parses one unmasked server frame off the buffer, if complete:
    /// (opcode, payload, consumed).
    fn parse_server_frame(&self) -> Option<(u8, Vec<u8>, usize)> {
        let buffer = &self.buffer;
        if buffer.len() < 2 {
            return None;
        }
        assert_eq!(buffer[1] & 0x80, 0, "server frames must be unmasked");
        let opcode = buffer[0] & 0x0F;
        let (length, offset) = match buffer[1] & 0x7F {
            126 => {
                if buffer.len() < 4 {
                    return None;
                }
                (u16::from_be_bytes([buffer[2], buffer[3]]) as usize, 4)
            }
            127 => panic!("unexpectedly large server frame"),
            short => (short as usize, 2),
        };
        if buffer.len() < offset + length {
            return None;
        }
        Some((
            opcode,
            buffer[offset..offset + length].to_vec(),
            offset + length,
        ))
    }

    /// Waits for the next protocol message, answering pings, up to a
    /// few seconds.
    fn expect_message(&mut self) -> Message {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut chunk = [0u8; 1024];
        loop {
            if let Some((opcode, payload, consumed)) = self.parse_server_frame() {
                self.buffer.drain(..consumed);
                match opcode {
                    0x2 => {
                        let message = Message::decode_frame(&payload).expect("valid message");
                        match message {
                            Message::Ping => {
                                self.send(Message::Pong);
                                continue;
                            }
                            other => return other,
                        }
                    }
                    0x9 => {
                        self.send_frame(0xA, &payload); // pong
                        continue;
                    }
                    0xA => continue,
                    0x8 => panic!("relay closed the websocket"),
                    other => panic!("unexpected server opcode {other}"),
                }
            }
            assert!(Instant::now() < deadline, "timed out waiting for message");
            match self.stream.read(&mut chunk) {
                Ok(0) => panic!("relay closed the connection"),
                Ok(n) => self.buffer.extend_from_slice(&chunk[..n]),
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => panic!("read failed: {e}"),
            }
        }
    }
}

/// A minimal native-dialect client, for bridging tests.
struct NativeClient {
    stream: TcpStream,
    decoder: emiu2_netplay::Decoder,
    pub code: FriendCode,
}

impl NativeClient {
    fn connect(addr: SocketAddr) -> Self {
        let mut stream = TcpStream::connect(addr).expect("connect to relay");
        stream.set_nodelay(true).ok();
        stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        stream.write_all(&emiu2_netplay::CLIENT_MAGIC).unwrap();
        stream
            .write_all(
                &Message::Hello {
                    version: PROTOCOL_VERSION,
                }
                .encode(),
            )
            .unwrap();

        let mut client = Self {
            stream,
            decoder: emiu2_netplay::Decoder::new(),
            code: FriendCode::parse("222222").unwrap(),
        };
        match client.expect_message() {
            Message::Welcome { code, .. } => client.code = code,
            other => panic!("expected Welcome, got {other:?}"),
        }
        client
    }

    fn send(&mut self, message: Message) {
        self.stream.write_all(&message.encode()).unwrap();
    }

    fn expect_message(&mut self) -> Message {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut chunk = [0u8; 1024];
        loop {
            if let Some(message) = self.decoder.next().unwrap() {
                match message {
                    Message::Ping => {
                        self.send(Message::Pong);
                        continue;
                    }
                    other => return other,
                }
            }
            assert!(Instant::now() < deadline, "timed out waiting for message");
            match self.stream.read(&mut chunk) {
                Ok(0) => panic!("relay closed the connection"),
                Ok(n) => self.decoder.push(&chunk[..n]),
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => panic!("read failed: {e}"),
            }
        }
    }
}

fn start_relay() -> SocketAddr {
    RelayServer::bind(("127.0.0.1", 0))
        .expect("bind relay")
        .spawn()
}

#[test]
fn websocket_client_pairs_with_native_client_and_relays_ir() {
    let addr = start_relay();
    let mut browser = WsClient::connect(addr);
    let mut native = NativeClient::connect(addr);
    assert_ne!(browser.code, native.code);

    browser.send(Message::Join { code: native.code });
    assert_eq!(browser.expect_message(), Message::Paired);
    assert_eq!(native.expect_message(), Message::Paired);

    browser.send(Message::IrData {
        records: vec![1, 2, 3, 4, 5, 6, 7, 8, 9],
    });
    assert_eq!(
        native.expect_message(),
        Message::IrData {
            records: vec![1, 2, 3, 4, 5, 6, 7, 8, 9]
        }
    );

    native.send(Message::IrData {
        records: vec![9, 8, 7],
    });
    assert_eq!(
        browser.expect_message(),
        Message::IrData {
            records: vec![9, 8, 7]
        }
    );

    // Breaking the pairing works from the WebSocket side too.
    browser.send(Message::Leave);
    assert_eq!(browser.expect_message(), Message::PeerLeft);
    assert_eq!(native.expect_message(), Message::PeerLeft);
}

#[test]
fn two_websocket_clients_pair_with_each_other() {
    let addr = start_relay();
    let mut a = WsClient::connect(addr);
    let mut b = WsClient::connect(addr);

    a.send(Message::Join { code: b.code });
    assert_eq!(a.expect_message(), Message::Paired);
    assert_eq!(b.expect_message(), Message::Paired);

    a.send(Message::IrData { records: vec![42] });
    assert_eq!(b.expect_message(), Message::IrData { records: vec![42] });
}

#[test]
fn plain_http_get_receives_an_info_page() {
    let addr = start_relay();
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: relay\r\n\r\n")
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let response = String::from_utf8_lossy(&response);
    assert!(response.starts_with("HTTP/1.1 200"), "got: {response}");
    assert!(response.contains("emiu2 relay"));
}
