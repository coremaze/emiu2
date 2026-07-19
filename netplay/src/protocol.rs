//! Message encoding: `[u8 type][u32 LE payload length][payload]`.

use crate::code::{FriendCode, CODE_LEN};

/// Sent once by native TCP clients before any message; lets the server
/// tell a native connection from a WebSocket upgrade by the first byte.
pub const CLIENT_MAGIC: [u8; 8] = *b"\xE2MIU2NP\x01";

pub const PROTOCOL_VERSION: u16 = 2;

/// Upper bound on any payload; an `IrData` this large would be absurd.
pub const MAX_PAYLOAD: usize = 64 * 1024;

/// The wire tag of each [`Message`] variant. Unknown bytes are rejected
/// once, in `TryFrom`; everything downstream matches exhaustively, so a
/// new message cannot be added without teaching the decoder about it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MessageType {
    Hello = 0x01,
    Join = 0x02,
    Leave = 0x03,
    IrData = 0x04,
    Ping = 0x05,
    Pong = 0x06,
    Welcome = 0x81,
    Paired = 0x82,
    PeerLeft = 0x83,
    Error = 0x84,
}

impl TryFrom<u8> for MessageType {
    type Error = ProtocolError;

    fn try_from(byte: u8) -> Result<Self, ProtocolError> {
        Ok(match byte {
            0x01 => Self::Hello,
            0x02 => Self::Join,
            0x03 => Self::Leave,
            0x04 => Self::IrData,
            0x05 => Self::Ping,
            0x06 => Self::Pong,
            0x81 => Self::Welcome,
            0x82 => Self::Paired,
            0x83 => Self::PeerLeft,
            0x84 => Self::Error,
            other => return Err(ProtocolError::UnknownType(other)),
        })
    }
}

/// Error codes carried by `Message::Error`.
///
/// Deliberately plain constants rather than an enum: these are
/// display-only advisories, and an older client must tolerate (not
/// reject) codes a newer server invents.
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
    /// Either direction once paired: opaque IR edge records (see
    /// [`encode_edge`]).
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
    /// The data ended before the message did.
    Truncated,
    /// The message ended before the data did.
    TrailingBytes,
    /// A field held a value the protocol does not allow.
    MalformedPayload(&'static str),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolError::UnknownType(t) => write!(f, "unknown message type {t:#04x}"),
            ProtocolError::OversizedPayload(len) => write!(f, "oversized payload ({len} bytes)"),
            ProtocolError::Truncated => f.write_str("truncated message"),
            ProtocolError::TrailingBytes => f.write_str("trailing bytes after message"),
            ProtocolError::MalformedPayload(what) => write!(f, "malformed payload: {what}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

/// Structural reads from a byte buffer: every access is bounds-checked
/// by pulling values off the front, so nothing is ever indexed and no
/// length is ever validated separately from the read it guards.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take_bytes(&mut self, len: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self.pos.checked_add(len).ok_or(ProtocolError::Truncated)?;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or(ProtocolError::Truncated)?;
        self.pos = end;
        Ok(bytes)
    }

    fn take_array<const N: usize>(&mut self) -> Result<[u8; N], ProtocolError> {
        self.take_bytes(N)
            .map(|bytes| bytes.try_into().expect("take_bytes yielded N bytes"))
    }

    fn take_u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.take_array::<1>()?[0])
    }

    fn take_u16(&mut self) -> Result<u16, ProtocolError> {
        Ok(u16::from_le_bytes(self.take_array()?))
    }

    fn take_u32(&mut self) -> Result<u32, ProtocolError> {
        Ok(u32::from_le_bytes(self.take_array()?))
    }

    fn take_u64(&mut self) -> Result<u64, ProtocolError> {
        Ok(u64::from_le_bytes(self.take_array()?))
    }

    fn take_code(&mut self) -> Result<FriendCode, ProtocolError> {
        let bytes = self.take_array::<CODE_LEN>()?;
        FriendCode::from_wire(bytes)
            .ok_or(ProtocolError::MalformedPayload("friend code characters"))
    }

    /// Everything not yet consumed, consuming it.
    fn take_rest(&mut self) -> &'a [u8] {
        let rest = &self.data[self.pos..];
        self.pos = self.data.len();
        rest
    }

    fn has_remaining(&self) -> bool {
        self.pos < self.data.len()
    }

    fn position(&self) -> usize {
        self.pos
    }

    /// Verifies every byte was consumed.
    fn finish(&self) -> Result<(), ProtocolError> {
        if self.has_remaining() {
            Err(ProtocolError::TrailingBytes)
        } else {
            Ok(())
        }
    }
}

impl Message {
    pub fn encode(&self) -> Vec<u8> {
        let (message_type, payload): (MessageType, Vec<u8>) = match self {
            Message::Hello { version } => (MessageType::Hello, version.to_le_bytes().to_vec()),
            Message::Join { code } => (MessageType::Join, code.as_bytes().to_vec()),
            Message::Leave => (MessageType::Leave, Vec::new()),
            Message::IrData { records } => (MessageType::IrData, records.clone()),
            Message::Ping => (MessageType::Ping, Vec::new()),
            Message::Pong => (MessageType::Pong, Vec::new()),
            Message::Welcome { version, code } => {
                let mut payload = version.to_le_bytes().to_vec();
                payload.extend_from_slice(code.as_bytes());
                (MessageType::Welcome, payload)
            }
            Message::Paired => (MessageType::Paired, Vec::new()),
            Message::PeerLeft => (MessageType::PeerLeft, Vec::new()),
            Message::Error { code, message } => {
                let mut payload = vec![*code];
                payload.extend_from_slice(message.as_bytes());
                (MessageType::Error, payload)
            }
        };

        let mut bytes = Vec::with_capacity(5 + payload.len());
        bytes.push(message_type as u8);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);
        bytes
    }

    /// Decodes one message body. Exhaustive over [`MessageType`], so a
    /// new variant cannot be added without a decode arm.
    fn decode(message_type: MessageType, payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader::new(payload);
        let message = match message_type {
            MessageType::Hello => Message::Hello {
                version: reader.take_u16()?,
            },
            MessageType::Join => Message::Join {
                code: reader.take_code()?,
            },
            MessageType::Leave => Message::Leave,
            MessageType::IrData => Message::IrData {
                records: reader.take_rest().to_vec(),
            },
            MessageType::Ping => Message::Ping,
            MessageType::Pong => Message::Pong,
            MessageType::Welcome => Message::Welcome {
                version: reader.take_u16()?,
                code: reader.take_code()?,
            },
            MessageType::Paired => Message::Paired,
            MessageType::PeerLeft => Message::PeerLeft,
            MessageType::Error => Message::Error {
                code: reader.take_u8()?,
                message: String::from_utf8_lossy(reader.take_rest()).into_owned(),
            },
        };
        reader.finish()?;
        Ok(message)
    }

    /// Decodes a message from one complete encoded frame, e.g. the
    /// contents of a WebSocket binary message.
    pub fn decode_frame(frame: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader::new(frame);
        let message_type = MessageType::try_from(reader.take_u8()?)?;
        let declared = reader.take_u32()? as usize;
        if declared > MAX_PAYLOAD {
            return Err(ProtocolError::OversizedPayload(declared));
        }
        let payload = reader.take_bytes(declared)?;
        reader.finish()?;
        Self::decode(message_type, payload)
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
    pub fn try_next(&mut self) -> Result<Option<Message>, ProtocolError> {
        let mut reader = Reader::new(&self.buffer);
        // A short read here just means the header has not fully arrived.
        let (Ok(type_byte), Ok(declared)) = (reader.take_u8(), reader.take_u32()) else {
            return Ok(None);
        };
        let declared = declared as usize;
        if declared > MAX_PAYLOAD {
            return Err(ProtocolError::OversizedPayload(declared));
        }
        let Ok(payload) = reader.take_bytes(declared) else {
            return Ok(None); // payload still arriving
        };
        let message = Message::decode(MessageType::try_from(type_byte)?, payload)?;
        let consumed = reader.position();
        self.buffer.drain(..consumed);
        Ok(Some(message))
    }
}

/// The length of one IR edge record inside `IrData` (and on the direct
/// `--ir listen/connect` TCP wire, which is a bare stream of them).
pub const EDGE_RECORD_LEN: usize = 9;

/// Encodes one IR envelope edge: sender time in emulated nanoseconds,
/// and the new carrier level.
pub fn encode_edge(sender_ns: u64, level: bool) -> [u8; EDGE_RECORD_LEN] {
    let mut record = [0u8; EDGE_RECORD_LEN];
    record[..8].copy_from_slice(&sender_ns.to_le_bytes());
    record[8] = level as u8;
    record
}

/// Decodes a run of edge records; partial or malformed records are
/// errors.
pub fn decode_edges(payload: &[u8]) -> Result<Vec<(u64, bool)>, ProtocolError> {
    let mut reader = Reader::new(payload);
    let mut edges = Vec::with_capacity(payload.len() / EDGE_RECORD_LEN);
    while reader.has_remaining() {
        let sender_ns = reader.take_u64()?;
        let level = match reader.take_u8()? {
            0 => false,
            1 => true,
            _ => return Err(ProtocolError::MalformedPayload("edge level")),
        };
        edges.push((sender_ns, level));
    }
    Ok(edges)
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
            while let Some(message) = decoder.try_next().unwrap() {
                decoded.push(message);
            }
        }
        assert_eq!(decoded, all_messages());
    }

    #[test]
    fn oversized_length_is_rejected() {
        let mut decoder = Decoder::new();
        let mut frame = vec![MessageType::IrData as u8];
        frame.extend_from_slice(&(MAX_PAYLOAD as u32 + 1).to_le_bytes());
        decoder.push(&frame);
        assert!(matches!(
            decoder.try_next(),
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
    fn short_and_long_payloads_are_rejected() {
        // A Hello whose payload is one byte short of its u16.
        let frame = [MessageType::Hello as u8, 1, 0, 0, 0, 0xAA];
        assert_eq!(Message::decode_frame(&frame), Err(ProtocolError::Truncated));

        // A Leave carrying an unexpected byte.
        let frame = [MessageType::Leave as u8, 1, 0, 0, 0, 0xAA];
        assert_eq!(
            Message::decode_frame(&frame),
            Err(ProtocolError::TrailingBytes)
        );
    }

    #[test]
    fn edge_records_round_trip() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&encode_edge(123_456_789, true));
        payload.extend_from_slice(&encode_edge(123_756_789, false));
        assert_eq!(
            decode_edges(&payload),
            Ok(vec![(123_456_789, true), (123_756_789, false)])
        );
    }

    #[test]
    fn bad_edge_records_are_rejected() {
        // A level byte that is neither 0 nor 1.
        let mut record = encode_edge(1, true);
        record[8] = 7;
        assert_eq!(
            decode_edges(&record),
            Err(ProtocolError::MalformedPayload("edge level"))
        );

        // A trailing partial record.
        let mut payload = encode_edge(1, true).to_vec();
        payload.push(0);
        assert_eq!(decode_edges(&payload), Err(ProtocolError::Truncated));
    }

    #[test]
    fn magic_does_not_look_like_http() {
        // The relay tells native connections from WebSocket upgrades by
        // the first byte; HTTP methods are ASCII letters.
        assert!(!CLIENT_MAGIC[0].is_ascii_alphabetic());
    }
}
