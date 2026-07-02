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

const OPCODE_CONTINUATION: u8 = 0x0;
const OPCODE_TEXT: u8 = 0x1;
const OPCODE_BINARY: u8 = 0x2;
const OPCODE_CLOSE: u8 = 0x8;
const OPCODE_PING: u8 = 0x9;
const OPCODE_PONG: u8 = 0xA;

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
    frame(OPCODE_BINARY, payload)
}

/// Wraps a payload in one unmasked pong frame.
pub fn frame_pong(payload: &[u8]) -> Vec<u8> {
    frame(OPCODE_PONG, payload)
}

/// Wraps a payload in one unmasked close frame.
pub fn frame_close() -> Vec<u8> {
    frame(OPCODE_CLOSE, &[])
}

fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 10);
    out.push(0x80 | opcode); // FIN set, no fragmentation on our side
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
    pub fn next(&mut self) -> std::io::Result<Option<WsEvent>> {
        loop {
            let Some((fin, opcode, payload, consumed)) = self.parse_frame()? else {
                return Ok(None);
            };
            self.buffer.drain(..consumed);

            match opcode {
                OPCODE_BINARY | OPCODE_CONTINUATION => {
                    if opcode == OPCODE_BINARY {
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
                OPCODE_PING => return Ok(Some(WsEvent::Ping(payload))),
                OPCODE_PONG => continue,
                OPCODE_CLOSE => return Ok(Some(WsEvent::Close)),
                OPCODE_TEXT => {
                    return Err(protocol_error("text frames are not part of the protocol"))
                }
                other => return Err(protocol_error_owned(format!("unknown opcode {other}"))),
            }
        }
    }

    /// Parses one frame if completely buffered:
    /// (fin, opcode, unmasked payload, bytes consumed).
    #[allow(clippy::type_complexity)]
    fn parse_frame(&self) -> std::io::Result<Option<(bool, u8, Vec<u8>, usize)>> {
        let buffer = &self.buffer;
        if buffer.len() < 2 {
            return Ok(None);
        }
        let fin = buffer[0] & 0x80 != 0;
        if buffer[0] & 0x70 != 0 {
            return Err(protocol_error("reserved bits set"));
        }
        let opcode = buffer[0] & 0x0F;
        let masked = buffer[1] & 0x80 != 0;
        if !masked {
            // Clients MUST mask (RFC 6455 §5.1).
            return Err(protocol_error("unmasked client frame"));
        }

        let (length, mut offset) = match buffer[1] & 0x7F {
            126 => {
                if buffer.len() < 4 {
                    return Ok(None);
                }
                (u64::from(u16::from_be_bytes([buffer[2], buffer[3]])), 4)
            }
            127 => {
                if buffer.len() < 10 {
                    return Ok(None);
                }
                (u64::from_be_bytes(buffer[2..10].try_into().unwrap()), 10)
            }
            short => (u64::from(short), 2),
        };
        if length > MAX_MESSAGE as u64 {
            return Err(protocol_error("oversized frame"));
        }
        let length = length as usize;

        if buffer.len() < offset + 4 + length {
            return Ok(None);
        }
        let mask: [u8; 4] = buffer[offset..offset + 4].try_into().unwrap();
        offset += 4;

        let mut payload = buffer[offset..offset + length].to_vec();
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[i % 4];
        }
        Ok(Some((fin, opcode, payload, offset + length)))
    }
}

fn protocol_error(what: &str) -> std::io::Error {
    std::io::Error::new(ErrorKind::InvalidData, format!("websocket: {what}"))
}

fn protocol_error_owned(what: String) -> std::io::Error {
    std::io::Error::new(ErrorKind::InvalidData, format!("websocket: {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a masked client frame, the way a browser would.
    fn client_frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mask = [0x12u8, 0x34, 0x56, 0x78];
        let mut out = vec![if fin { 0x80 | opcode } else { opcode }];
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
        assembler.push(&client_frame(true, OPCODE_BINARY, b"hello"));
        match assembler.next().unwrap() {
            Some(WsEvent::Message(payload)) => assert_eq!(payload, b"hello"),
            _ => panic!("expected a message"),
        }
        assert!(assembler.next().unwrap().is_none());
    }

    #[test]
    fn split_delivery_and_fragmentation_reassemble() {
        let mut assembler = FrameAssembler::new();
        let mut stream = Vec::new();
        stream.extend_from_slice(&client_frame(false, OPCODE_BINARY, b"hel"));
        stream.extend_from_slice(&client_frame(true, OPCODE_CONTINUATION, b"lo"));

        // Feed byte by byte.
        let mut messages = Vec::new();
        for &byte in &stream {
            assembler.push(&[byte]);
            while let Some(event) = assembler.next().unwrap() {
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
        assembler.push(&client_frame(true, OPCODE_PING, b"hi"));
        assert!(matches!(
            assembler.next().unwrap(),
            Some(WsEvent::Ping(payload)) if payload == b"hi"
        ));

        assembler.push(&client_frame(true, OPCODE_CLOSE, &[]));
        assert!(matches!(assembler.next().unwrap(), Some(WsEvent::Close)));
    }

    #[test]
    fn unmasked_client_frames_are_rejected() {
        let mut assembler = FrameAssembler::new();
        assembler.push(&frame_binary(b"nope")); // server framing = unmasked
        assert!(assembler.next().is_err());
    }
}
