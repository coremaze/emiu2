//! End-to-end tests of the relay over real sockets, using a minimal
//! native-protocol test client.

use emiu2_netplay::{error_code, Decoder, FriendCode, Message, CLIENT_MAGIC, PROTOCOL_VERSION};
use emiu2_relay::RelayServer;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

struct TestClient {
    stream: TcpStream,
    decoder: Decoder,
    pub code: FriendCode,
}

impl TestClient {
    fn connect(addr: SocketAddr) -> Self {
        let mut stream = TcpStream::connect(addr).expect("connect to relay");
        stream.set_nodelay(true).ok();
        stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        stream.write_all(&CLIENT_MAGIC).unwrap();
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
            decoder: Decoder::new(),
            code: FriendCode::parse("222222").unwrap(), // replaced below
        };
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
        self.stream.write_all(&message.encode()).unwrap();
    }

    /// Waits for the next non-Ping message, up to a couple of seconds.
    fn expect_message(&mut self) -> Message {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut chunk = [0u8; 1024];
        loop {
            if let Some(message) = self.decoder.try_next().unwrap() {
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

    fn expect_error(&mut self, expected: u8) {
        match self.expect_message() {
            Message::Error { code, .. } => assert_eq!(code, expected),
            other => panic!("expected Error({expected}), got {other:?}"),
        }
    }
}

fn start_relay() -> SocketAddr {
    RelayServer::bind(("127.0.0.1", 0))
        .expect("bind relay")
        .spawn()
}

#[test]
fn welcome_assigns_distinct_codes() {
    let addr = start_relay();
    let a = TestClient::connect(addr);
    let b = TestClient::connect(addr);
    assert_ne!(a.code, b.code);
}

#[test]
fn pairing_relays_ir_data_both_ways() {
    let addr = start_relay();
    let mut a = TestClient::connect(addr);
    let mut b = TestClient::connect(addr);

    a.send(Message::Join { code: b.code });
    assert_eq!(a.expect_message(), Message::Paired);
    assert_eq!(b.expect_message(), Message::Paired);

    a.send(Message::IrData {
        records: vec![1, 2, 3],
    });
    assert_eq!(
        b.expect_message(),
        Message::IrData {
            records: vec![1, 2, 3]
        }
    );

    b.send(Message::IrData {
        records: vec![9, 8, 7],
    });
    assert_eq!(
        a.expect_message(),
        Message::IrData {
            records: vec![9, 8, 7]
        }
    );
}

#[test]
fn leave_notifies_both_and_frees_them_for_new_pairings() {
    let addr = start_relay();
    let mut a = TestClient::connect(addr);
    let mut b = TestClient::connect(addr);
    let mut c = TestClient::connect(addr);

    a.send(Message::Join { code: b.code });
    assert_eq!(a.expect_message(), Message::Paired);
    assert_eq!(b.expect_message(), Message::Paired);

    a.send(Message::Leave);
    assert_eq!(a.expect_message(), Message::PeerLeft);
    assert_eq!(b.expect_message(), Message::PeerLeft);

    // Both are free again.
    std::thread::sleep(Duration::from_millis(600)); // join throttle
    b.send(Message::Join { code: c.code });
    assert_eq!(b.expect_message(), Message::Paired);
    assert_eq!(c.expect_message(), Message::Paired);
}

#[test]
fn disconnect_notifies_the_peer() {
    let addr = start_relay();
    let mut a = TestClient::connect(addr);
    let mut b = TestClient::connect(addr);

    a.send(Message::Join { code: b.code });
    assert_eq!(a.expect_message(), Message::Paired);
    assert_eq!(b.expect_message(), Message::Paired);

    drop(a);
    assert_eq!(b.expect_message(), Message::PeerLeft);
}

#[test]
fn join_error_cases() {
    let addr = start_relay();
    let mut a = TestClient::connect(addr);
    let mut b = TestClient::connect(addr);
    let mut c = TestClient::connect(addr);

    // Unknown code.
    let bogus = FriendCode::parse("999999").unwrap();
    let code = if a.code == bogus {
        FriendCode::parse("888888").unwrap()
    } else {
        bogus
    };
    a.send(Message::Join { code });
    a.expect_error(error_code::UNKNOWN_CODE);

    // Self join.
    std::thread::sleep(Duration::from_millis(600));
    a.send(Message::Join { code: a.code });
    a.expect_error(error_code::SELF_JOIN);

    // Peer busy / already paired.
    std::thread::sleep(Duration::from_millis(600));
    a.send(Message::Join { code: b.code });
    assert_eq!(a.expect_message(), Message::Paired);
    assert_eq!(b.expect_message(), Message::Paired);

    c.send(Message::Join { code: b.code });
    c.expect_error(error_code::PEER_BUSY);

    std::thread::sleep(Duration::from_millis(600));
    a.send(Message::Join { code: c.code });
    a.expect_error(error_code::ALREADY_PAIRED);
}

#[test]
fn rapid_joins_are_throttled() {
    let addr = start_relay();
    let mut a = TestClient::connect(addr);
    let bogus = FriendCode::parse("999999").unwrap();

    a.send(Message::Join { code: bogus });
    a.expect_error(error_code::UNKNOWN_CODE);
    a.send(Message::Join { code: bogus });
    a.expect_error(error_code::THROTTLED);
}

#[test]
fn version_mismatch_is_rejected() {
    let addr = start_relay();
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.write_all(&CLIENT_MAGIC).unwrap();
    stream
        .write_all(&Message::Hello { version: 999 }.encode())
        .unwrap();

    let mut decoder = Decoder::new();
    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer).unwrap(); // server closes after erroring
    decoder.push(&buffer);
    match decoder.try_next().unwrap() {
        Some(Message::Error { code, .. }) => assert_eq!(code, error_code::BAD_VERSION),
        other => panic!("expected version error, got {other:?}"),
    }
}

#[test]
fn garbage_magic_is_dropped() {
    let addr = start_relay();
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.write_all(b"NOTMAGIC").unwrap();
    let mut buffer = Vec::new();
    // The server hangs up without sending anything.
    stream.read_to_end(&mut buffer).unwrap();
    assert!(buffer.is_empty());
}
