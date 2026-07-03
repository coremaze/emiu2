//! Minimal WebSocket server support (RFC 6455): the HTTP upgrade
//! handshake and a frame codec. Only what a browser needs to speak the
//! relay protocol: binary messages (with fragmentation), ping/pong, and
//! close. Each relay protocol message travels as exactly one binary
//! message.

use crate::{base64, sha1};
use std::io::ErrorKind;

const ACCEPT_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Upper bound on one WebSocket message, comfortably above the relay
/// protocol's own payload cap.
const MAX_MESSAGE: usize = 128 * 1024;

/// The frame opcodes the protocol uses (RFC 6455 §5.2). Unknown bytes
/// are rejected once, in `TryFrom`; everything downstream matches
/// exhaustively.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WsOpcode {
    Continuation = 0x0,
    Text = 0x1,
    Binary = 0x2,
    Close = 0x8,
    Ping = 0x9,
    Pong = 0xA,
}

impl TryFrom<u8> for WsOpcode {
    type Error = std::io::Error;

    fn try_from(byte: u8) -> std::io::Result<Self> {
        Ok(match byte {
            0x0 => Self::Continuation,
            0x1 => Self::Text,
            0x2 => Self::Binary,
            0x8 => Self::Close,
            0x9 => Self::Ping,
            0xA => Self::Pong,
            other => return Err(protocol_error(&format!("unknown opcode {other}"))),
        })
    }
}

/// The `Sec-WebSocket-Accept` value for a client key.
pub fn accept_key(client_key: &str) -> String {
    let mut input = client_key.trim().as_bytes().to_vec();
    input.extend_from_slice(ACCEPT_GUID.as_bytes());
    base64::encode(&sha1::sha1(&input))
}

/// Examines a complete HTTP request head and produces the response to
/// send: `Ok` for a valid WebSocket upgrade (the connection continues as
/// WebSocket), `Err` for anything else (the connection closes after the
/// response).
pub fn upgrade_response(request: &[u8]) -> Result<Vec<u8>, Vec<u8>> {
    let text = String::from_utf8_lossy(request);
    let mut lines = text.split("\r\n");

    let request_line = lines.next().unwrap_or_default();
    if !request_line.starts_with("GET ") {
        return Err(simple_response("405 Method Not Allowed", "websocket only"));
    }

    let mut upgrade_requested = false;
    let mut key = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        match name.as_str() {
            "upgrade" if value.to_ascii_lowercase().contains("websocket") => {
                upgrade_requested = true;
            }
            "sec-websocket-key" => key = Some(value.to_string()),
            _ => {}
        }
    }

    match (upgrade_requested, key) {
        (true, Some(key)) => {
            let response = format!(
                "HTTP/1.1 101 Switching Protocols\r\n\
                 Upgrade: websocket\r\n\
                 Connection: Upgrade\r\n\
                 Sec-WebSocket-Accept: {}\r\n\r\n",
                accept_key(&key)
            );
            Ok(response.into_bytes())
        }
        (true, None) => Err(simple_response(
            "400 Bad Request",
            "missing Sec-WebSocket-Key",
        )),
        // A plain GET: answer something friendly for health checks.
        (false, _) => Err(simple_response(
            "200 OK",
            "emiu2 relay: connect with the emiu2 emulator or web client",
        )),
    }
}

fn simple_response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Wraps a payload in one unmasked binary frame (server to client).
pub fn frame_binary(payload: &[u8]) -> Vec<u8> {
    frame(WsOpcode::Binary, payload)
}

/// Wraps a payload in one unmasked pong frame.
pub fn frame_pong(payload: &[u8]) -> Vec<u8> {
    frame(WsOpcode::Pong, payload)
}

/// Wraps a payload in one unmasked close frame.
pub fn frame_close() -> Vec<u8> {
    frame(WsOpcode::Close, &[])
}

fn frame(opcode: WsOpcode, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 10);
    out.push(0x80 | opcode as u8); // FIN set, no fragmentation on our side
    if payload.len() < 126 {
        out.push(payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        out.push(126);
        out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        out.push(127);
        out.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    out.extend_from_slice(payload);
    out
}

/// Something a client frame produced.
pub enum WsEvent {
    /// A complete binary message (fragments already reassembled).
    Message(Vec<u8>),
    /// The client pinged; reply with `frame_pong` on this payload.
    Ping(Vec<u8>),
    /// The client is closing the connection.
    Close,
}

/// Incremental parser for the client-to-server frame stream: feed bytes,
/// take events.
#[derive(Default)]
pub struct FrameAssembler {
    buffer: Vec<u8>,
    /// Reassembly buffer for a fragmented binary message.
    fragments: Option<Vec<u8>>,
}

impl FrameAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// Takes the next event off the stream, or `None` if more bytes are
    /// needed. Errors are fatal to the connection.
    pub fn try_next(&mut self) -> std::io::Result<Option<WsEvent>> {
        loop {
            let Some((fin, opcode, payload, consumed)) = self.parse_frame()? else {
                return Ok(None);
            };
            self.buffer.drain(..consumed);

            match opcode {
                WsOpcode::Binary | WsOpcode::Continuation => {
                    if opcode == WsOpcode::Binary {
                        if self.fragments.is_some() {
                            return Err(protocol_error("interleaved fragmented messages"));
                        }
                        self.fragments = Some(payload);
                    } else {
                        let Some(fragments) = self.fragments.as_mut() else {
                            return Err(protocol_error("continuation without a start"));
                        };
                        if fragments.len() + payload.len() > MAX_MESSAGE {
                            return Err(protocol_error("oversized fragmented message"));
                        }
                        fragments.extend_from_slice(&payload);
                    }
                    if fin {
                        let message = self.fragments.take().expect("just set");
                        return Ok(Some(WsEvent::Message(message)));
                    }
                }
                WsOpcode::Ping => return Ok(Some(WsEvent::Ping(payload))),
                WsOpcode::Pong => continue,
                WsOpcode::Close => return Ok(Some(WsEvent::Close)),
                WsOpcode::Text => {
                    return Err(protocol_error("text frames are not part of the protocol"))
                }
            }
        }
    }

    /// Parses one frame if completely buffered:
    /// (fin, opcode, unmasked payload, bytes consumed). Every read pulls
    /// from the front of a cursor, so an incomplete frame surfaces as a
    /// failed take (-> wait for more bytes), never as an index panic.
    #[allow(clippy::type_complexity)]
    fn parse_frame(&self) -> std::io::Result<Option<(bool, WsOpcode, Vec<u8>, usize)>> {
        let mut cursor = Cursor::new(&self.buffer);

        let Some(first) = cursor.take_u8() else {
            return Ok(None);
        };
        let fin = first & 0x80 != 0;
        if first & 0x70 != 0 {
            return Err(protocol_error("reserved bits set"));
        }
        let opcode = WsOpcode::try_from(first & 0x0F)?;

        let Some(second) = cursor.take_u8() else {
            return Ok(None);
        };
        if second & 0x80 == 0 {
            // Clients MUST mask (RFC 6455 §5.1).
            return Err(protocol_error("unmasked client frame"));
        }
        let length = match second & 0x7F {
            126 => match cursor.take_array::<2>() {
                Some(bytes) => u64::from(u16::from_be_bytes(bytes)),
                None => return Ok(None),
            },
            127 => match cursor.take_array::<8>() {
                Some(bytes) => u64::from_be_bytes(bytes),
                None => return Ok(None),
            },
            short => u64::from(short),
        };
        if length > MAX_MESSAGE as u64 {
            return Err(protocol_error("oversized frame"));
        }

        let Some(mask) = cursor.take_array::<4>() else {
            return Ok(None);
        };
        let Some(masked_payload) = cursor.take(length as usize) else {
            return Ok(None);
        };
        let payload = masked_payload
            .iter()
            .zip(mask.iter().cycle())
            .map(|(byte, mask_byte)| byte ^ mask_byte)
            .collect();
        Ok(Some((fin, opcode, payload, cursor.position())))
    }
}

/// Structural reads from the front of a buffer; a failed take means the
/// data has not fully arrived yet.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(len)?;
        let bytes = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(bytes)
    }

    fn take_u8(&mut self) -> Option<u8> {
        Some(self.take_array::<1>()?[0])
    }

    fn take_array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)
            .map(|bytes| bytes.try_into().expect("take yielded N bytes"))
    }

    fn position(&self) -> usize {
        self.pos
    }
}

fn protocol_error(what: &str) -> std::io::Error {
    std::io::Error::new(ErrorKind::InvalidData, format!("websocket: {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a masked client frame, the way a browser would.
    fn client_frame(fin: bool, opcode: WsOpcode, payload: &[u8]) -> Vec<u8> {
        let mask = [0x12u8, 0x34, 0x56, 0x78];
        let mut out = vec![if fin {
            0x80 | opcode as u8
        } else {
            opcode as u8
        }];
        if payload.len() < 126 {
            out.push(0x80 | payload.len() as u8);
        } else {
            out.push(0x80 | 126);
            out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        out.extend_from_slice(&mask);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        out
    }

    #[test]
    fn rfc_6455_accept_vector() {
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn upgrade_needs_the_key() {
        let request = b"GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\
                        Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";
        let response = upgrade_response(request).expect("valid upgrade");
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 101"));
        assert!(response.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));

        let plain = b"GET / HTTP/1.1\r\nHost: x\r\n\r\n";
        let response = upgrade_response(plain).expect_err("not an upgrade");
        assert!(String::from_utf8(response)
            .unwrap()
            .starts_with("HTTP/1.1 200"));
    }

    #[test]
    fn masked_binary_frames_round_trip() {
        let mut assembler = FrameAssembler::new();
        assembler.push(&client_frame(true, WsOpcode::Binary, b"hello"));
        match assembler.try_next().unwrap() {
            Some(WsEvent::Message(payload)) => assert_eq!(payload, b"hello"),
            _ => panic!("expected a message"),
        }
        assert!(assembler.try_next().unwrap().is_none());
    }

    #[test]
    fn split_delivery_and_fragmentation_reassemble() {
        let mut assembler = FrameAssembler::new();
        let mut stream = Vec::new();
        stream.extend_from_slice(&client_frame(false, WsOpcode::Binary, b"hel"));
        stream.extend_from_slice(&client_frame(true, WsOpcode::Continuation, b"lo"));

        // Feed byte by byte.
        let mut messages = Vec::new();
        for &byte in &stream {
            assembler.push(&[byte]);
            while let Some(event) = assembler.try_next().unwrap() {
                match event {
                    WsEvent::Message(payload) => messages.push(payload),
                    _ => panic!("unexpected event"),
                }
            }
        }
        assert_eq!(messages, vec![b"hello".to_vec()]);
    }

    #[test]
    fn ping_and_close_surface_as_events() {
        let mut assembler = FrameAssembler::new();
        assembler.push(&client_frame(true, WsOpcode::Ping, b"hi"));
        assert!(matches!(
            assembler.try_next().unwrap(),
            Some(WsEvent::Ping(payload)) if payload == b"hi"
        ));

        assembler.push(&client_frame(true, WsOpcode::Close, &[]));
        assert!(matches!(
            assembler.try_next().unwrap(),
            Some(WsEvent::Close)
        ));
    }

    #[test]
    fn unmasked_client_frames_are_rejected() {
        let mut assembler = FrameAssembler::new();
        assembler.push(&frame_binary(b"nope")); // server framing = unmasked
        assert!(assembler.try_next().is_err());
    }
}
