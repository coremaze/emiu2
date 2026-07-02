//! Message encoding: `[u8 type][u32 LE payload length][payload]`.

use crate::code::{FriendCode, CODE_LEN};

/// Sent once by native TCP clients before any message; lets the server
/// tell a native connection from a WebSocket upgrade by the first byte.
pub const CLIENT_MAGIC: [u8; 8] = *b"\xE2MIU2NP\x01";

pub const PROTOCOL_VERSION: u16 = 1;

/// Upper bound on any payload; an `IrData` this large would be absurd.
pub const MAX_PAYLOAD: usize = 64 * 1024;

const TYPE_HELLO: u8 = 0x01;
const TYPE_JOIN: u8 = 0x02;
const TYPE_LEAVE: u8 = 0x03;
const TYPE_IR_DATA: u8 = 0x04;
const TYPE_PING: u8 = 0x05;
const TYPE_PONG: u8 = 0x06;
const TYPE_WELCOME: u8 = 0x81;
const TYPE_PAIRED: u8 = 0x82;
const TYPE_PEER_LEFT: u8 = 0x83;
const TYPE_ERROR: u8 = 0x84;

/// Error codes carried by `Message::Error`.
pub mod error_code {
    pub const UNKNOWN_CODE: u8 = 1;
    pub const PEER_BUSY: u8 = 2;
    pub const SELF_JOIN: u8 = 3;
    pub const ALREADY_PAIRED: u8 = 4;
    pub const BAD_VERSION: u8 = 5;
    pub const THROTTLED: u8 = 6;
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Message {
    /// Client -> server, first message on the connection.
    Hello {
        version: u16,
    },
    /// Client -> server: pair me with the owner of this code.
    Join {
        code: FriendCode,
    },
    /// Client -> server: unpair (both stay connected to the relay).
    Leave,
    /// Either direction once paired: opaque IR edge records.
    IrData {
        records: Vec<u8>,
    },
    Ping,
    Pong,
    /// Server -> client, in response to `Hello`.
    Welcome {
        version: u16,
        code: FriendCode,
    },
    /// Server -> client, to both sides of a new pairing.
    Paired,
    /// Server -> client: the peer left or disconnected.
    PeerLeft,
    /// Server -> client: a request failed. `code` is one of
    /// [`error_code`]; `message` is human-readable.
    Error {
        code: u8,
        message: String,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProtocolError {
    /// The message type byte is unknown.
    UnknownType(u8),
    /// A declared payload length exceeds `MAX_PAYLOAD`.
    OversizedPayload(usize),
    /// The payload did not decode as the declared type.
    MalformedPayload(&'static str),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolError::UnknownType(t) => write!(f, "unknown message type {t:#04x}"),
            ProtocolError::OversizedPayload(len) => write!(f, "oversized payload ({len} bytes)"),
            ProtocolError::MalformedPayload(what) => write!(f, "malformed payload: {what}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl Message {
    pub fn encode(&self) -> Vec<u8> {
        let (message_type, payload): (u8, Vec<u8>) = match self {
            Message::Hello { version } => (TYPE_HELLO, version.to_le_bytes().to_vec()),
            Message::Join { code } => (TYPE_JOIN, code.as_bytes().to_vec()),
            Message::Leave => (TYPE_LEAVE, Vec::new()),
            Message::IrData { records } => (TYPE_IR_DATA, records.clone()),
            Message::Ping => (TYPE_PING, Vec::new()),
            Message::Pong => (TYPE_PONG, Vec::new()),
            Message::Welcome { version, code } => {
                let mut payload = version.to_le_bytes().to_vec();
                payload.extend_from_slice(code.as_bytes());
                (TYPE_WELCOME, payload)
            }
            Message::Paired => (TYPE_PAIRED, Vec::new()),
            Message::PeerLeft => (TYPE_PEER_LEFT, Vec::new()),
            Message::Error { code, message } => {
                let mut payload = vec![*code];
                payload.extend_from_slice(message.as_bytes());
                (TYPE_ERROR, payload)
            }
        };

        let mut bytes = Vec::with_capacity(5 + payload.len());
        bytes.push(message_type);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);
        bytes
    }

    /// Decodes one message from a complete `[type][payload]` pair (the
    /// length prefix already consumed and validated by the caller).
    fn decode(message_type: u8, payload: &[u8]) -> Result<Self, ProtocolError> {
        fn code_from(payload: &[u8]) -> Result<FriendCode, ProtocolError> {
            let bytes: [u8; CODE_LEN] = payload
                .try_into()
                .map_err(|_| ProtocolError::MalformedPayload("friend code length"))?;
            FriendCode::from_wire(bytes)
                .ok_or(ProtocolError::MalformedPayload("friend code characters"))
        }

        match message_type {
            TYPE_HELLO => {
                let bytes: [u8; 2] = payload
                    .try_into()
                    .map_err(|_| ProtocolError::MalformedPayload("hello length"))?;
                Ok(Message::Hello {
                    version: u16::from_le_bytes(bytes),
                })
            }
            TYPE_JOIN => Ok(Message::Join {
                code: code_from(payload)?,
            }),
            TYPE_LEAVE => Ok(Message::Leave),
            TYPE_IR_DATA => Ok(Message::IrData {
                records: payload.to_vec(),
            }),
            TYPE_PING => Ok(Message::Ping),
            TYPE_PONG => Ok(Message::Pong),
            TYPE_WELCOME => {
                if payload.len() != 2 + CODE_LEN {
                    return Err(ProtocolError::MalformedPayload("welcome length"));
                }
                Ok(Message::Welcome {
                    version: u16::from_le_bytes([payload[0], payload[1]]),
                    code: code_from(&payload[2..])?,
                })
            }
            TYPE_PAIRED => Ok(Message::Paired),
            TYPE_PEER_LEFT => Ok(Message::PeerLeft),
            TYPE_ERROR => {
                let (&code, message) = payload
                    .split_first()
                    .ok_or(ProtocolError::MalformedPayload("error length"))?;
                Ok(Message::Error {
                    code,
                    message: String::from_utf8_lossy(message).into_owned(),
                })
            }
            other => Err(ProtocolError::UnknownType(other)),
        }
    }

    /// Decodes a message from one complete encoded frame, e.g. the
    /// contents of a WebSocket binary message.
    pub fn decode_frame(frame: &[u8]) -> Result<Self, ProtocolError> {
        if frame.len() < 5 {
            return Err(ProtocolError::MalformedPayload("frame header"));
        }
        let declared = u32::from_le_bytes([frame[1], frame[2], frame[3], frame[4]]) as usize;
        if declared > MAX_PAYLOAD {
            return Err(ProtocolError::OversizedPayload(declared));
        }
        if frame.len() != 5 + declared {
            return Err(ProtocolError::MalformedPayload("frame length"));
        }
        Self::decode(frame[0], &frame[5..])
    }
}

/// Incremental decoder for a byte stream (the TCP framing): feed bytes,
/// take messages.
#[derive(Default)]
pub struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// Takes the next complete message off the stream, or `None` if more
    /// bytes are needed. Errors are fatal to the stream.
    pub fn next(&mut self) -> Result<Option<Message>, ProtocolError> {
        if self.buffer.len() < 5 {
            return Ok(None);
        }
        let declared = u32::from_le_bytes([
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
            self.buffer[4],
        ]) as usize;
        if declared > MAX_PAYLOAD {
            return Err(ProtocolError::OversizedPayload(declared));
        }
        let total = 5 + declared;
        if self.buffer.len() < total {
            return Ok(None);
        }
        let message = Message::decode(self.buffer[0], &self.buffer[5..total])?;
        self.buffer.drain(..total);
        Ok(Some(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_messages() -> Vec<Message> {
        let code = FriendCode::parse("ABCDEF").unwrap();
        vec![
            Message::Hello {
                version: PROTOCOL_VERSION,
            },
            Message::Join { code },
            Message::Leave,
            Message::IrData {
                records: vec![1, 2, 3, 4, 5, 6, 7, 8, 9],
            },
            Message::Ping,
            Message::Pong,
            Message::Welcome {
                version: PROTOCOL_VERSION,
                code,
            },
            Message::Paired,
            Message::PeerLeft,
            Message::Error {
                code: error_code::UNKNOWN_CODE,
                message: "no such code".into(),
            },
        ]
    }

    #[test]
    fn every_message_round_trips_as_a_frame() {
        for message in all_messages() {
            let encoded = message.encode();
            assert_eq!(Message::decode_frame(&encoded), Ok(message));
        }
    }

    #[test]
    fn decoder_reassembles_split_stream() {
        let mut stream = Vec::new();
        for message in all_messages() {
            stream.extend_from_slice(&message.encode());
        }

        // Feed the stream one byte at a time.
        let mut decoder = Decoder::new();
        let mut decoded = Vec::new();
        for &byte in &stream {
            decoder.push(&[byte]);
            while let Some(message) = decoder.next().unwrap() {
                decoded.push(message);
            }
        }
        assert_eq!(decoded, all_messages());
    }

    #[test]
    fn oversized_length_is_rejected() {
        let mut decoder = Decoder::new();
        let mut frame = vec![TYPE_IR_DATA];
        frame.extend_from_slice(&(MAX_PAYLOAD as u32 + 1).to_le_bytes());
        decoder.push(&frame);
        assert!(matches!(
            decoder.next(),
            Err(ProtocolError::OversizedPayload(_))
        ));
    }

    #[test]
    fn unknown_type_is_rejected() {
        let frame = [0x7Fu8, 0, 0, 0, 0];
        assert_eq!(
            Message::decode_frame(&frame),
            Err(ProtocolError::UnknownType(0x7F))
        );
    }

    #[test]
    fn magic_does_not_look_like_http() {
        // The relay tells native connections from WebSocket upgrades by
        // the first byte; HTTP methods are ASCII letters.
        assert!(!CLIENT_MAGIC[0].is_ascii_alphabetic());
    }
}
