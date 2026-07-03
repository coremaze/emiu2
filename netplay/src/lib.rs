//! The emiu2 netplay relay protocol, shared by the relay server, the
//! native client and the browser (wasm) client.
//!
//! A client connects to the relay, sends the 8-byte [`CLIENT_MAGIC`]
//! followed by a [`Message::Hello`], and receives a [`Message::Welcome`]
//! carrying its ephemeral friend code. Sending [`Message::Join`] with a
//! peer's code pairs the two clients; from then on every
//! [`Message::IrData`] is relayed verbatim to the peer until either side
//! leaves or disconnects.
//!
//! Over TCP, messages follow the magic as a plain stream. Over
//! WebSocket, each binary frame carries exactly one encoded message (the
//! same bytes, including the header), and no magic is sent — the HTTP
//! upgrade already identifies the protocol.
//!
//! `IrData` payloads are opaque to the relay: concatenated 9-byte edge
//! records of `(u64 LE sender time in emulated nanoseconds, u8 carrier
//! level)`, identical to the direct `--ir listen/connect` wire format.

pub mod code;
pub mod protocol;

pub use code::FriendCode;
pub use protocol::error_code;
pub use protocol::{decode_edges, encode_edge, EDGE_RECORD_LEN};
pub use protocol::{Decoder, Message, ProtocolError, CLIENT_MAGIC, PROTOCOL_VERSION};
