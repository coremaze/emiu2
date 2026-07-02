//! Relay transport for the browser build: the same relay protocol as
//! the native client, carried over a `web_sys::WebSocket` (one protocol
//! message per binary frame; the relay server terminates the WebSocket
//! framing).
//!
//! Everything is single-threaded on the JS event loop, so all state
//! lives in one `Rc<RefCell<..>>` shared between the emulator's
//! transceiver, the rollback control, the UI exports, and the socket
//! callbacks. The state exists from emulator creation onward — before
//! `connect` it simply behaves as a disconnected transceiver — which is
//! what lets the player connect, join, leave and reconnect while the
//! emulator runs.

use crate::ir::{IrInterface, IrRollbackControl, RollbackDirective};
use crate::ir_replay::ReplayEngine;
use emiu2_netplay::{FriendCode, Message, PROTOCOL_VERSION};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{BinaryType, MessageEvent, WebSocket};

const RECORD_LEN: usize = 9;
const MAX_QUEUED_EDGES: usize = 100_000;

pub struct WebIrState {
    engine: ReplayEngine,
    socket: Option<WebSocket>,
    connected: bool,
    paired: bool,
    code: Option<FriendCode>,
    status: String,
    /// Received edges: (sender time in ns, carrier level).
    incoming: VecDeque<(u64, bool)>,
}

pub type SharedWebIr = Rc<RefCell<WebIrState>>;

impl WebIrState {
    pub fn shared() -> SharedWebIr {
        Rc::new(RefCell::new(Self {
            engine: ReplayEngine::new(),
            socket: None,
            connected: false,
            paired: false,
            code: None,
            status: "not connected".into(),
            incoming: VecDeque::new(),
        }))
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    pub fn paired(&self) -> bool {
        self.paired
    }

    pub fn code(&self) -> Option<FriendCode> {
        self.code
    }

    pub fn status(&self) -> String {
        self.status.clone()
    }

    fn send(&self, message: &Message) {
        if let Some(socket) = &self.socket {
            if socket.ready_state() == WebSocket::OPEN {
                let _ = socket.send_with_u8_array(&message.encode());
            }
        }
    }

    fn set_paired(&mut self, paired: bool) {
        if self.paired != paired {
            self.paired = paired;
            self.engine.reset();
            self.incoming.clear();
        }
    }
}

/// Opens (or replaces) the WebSocket connection to a relay. The URL is
/// a full `ws://` or `wss://` endpoint.
pub fn connect(state: &SharedWebIr, url: &str) -> Result<(), JsValue> {
    disconnect(state);

    let socket = WebSocket::new(url)?;
    socket.set_binary_type(BinaryType::Arraybuffer);

    let onopen_state = state.clone();
    let onopen = Closure::wrap(Box::new(move |_event: web_sys::Event| {
        let state = onopen_state.borrow();
        state.send(&Message::Hello {
            version: PROTOCOL_VERSION,
        });
    }) as Box<dyn FnMut(web_sys::Event)>);
    socket.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();

    let onmessage_state = state.clone();
    let onmessage = Closure::wrap(Box::new(move |event: MessageEvent| {
        let Ok(buffer) = event.data().dyn_into::<web_sys::js_sys::ArrayBuffer>() else {
            return;
        };
        let bytes = web_sys::js_sys::Uint8Array::new(&buffer).to_vec();
        let Ok(message) = Message::decode_frame(&bytes) else {
            web_sys::console::warn_1(&"IR relay: undecodable message".into());
            return;
        };
        handle_message(&mut onmessage_state.borrow_mut(), message);
    }) as Box<dyn FnMut(MessageEvent)>);
    socket.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    let onclose_state = state.clone();
    let onclose = Closure::wrap(Box::new(move |_event: web_sys::Event| {
        let mut state = onclose_state.borrow_mut();
        state.connected = false;
        state.set_paired(false);
        state.code = None;
        state.status = "connection lost".into();
    }) as Box<dyn FnMut(web_sys::Event)>);
    socket.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();

    let mut state = state.borrow_mut();
    state.status = "connecting...".into();
    state.socket = Some(socket);
    Ok(())
}

pub fn disconnect(state: &SharedWebIr) {
    let mut state = state.borrow_mut();
    if let Some(socket) = state.socket.take() {
        socket.set_onopen(None);
        socket.set_onmessage(None);
        socket.set_onclose(None);
        socket.close().ok();
    }
    state.connected = false;
    state.set_paired(false);
    state.code = None;
    state.status = "not connected".into();
}

pub fn join(state: &SharedWebIr, code: FriendCode) {
    state.borrow().send(&Message::Join { code });
}

pub fn leave(state: &SharedWebIr) {
    state.borrow().send(&Message::Leave);
}

fn handle_message(state: &mut WebIrState, message: Message) {
    match message {
        Message::Welcome { code, .. } => {
            state.connected = true;
            state.code = Some(code);
            state.status = format!("connected; your code is {code}");
        }
        Message::Paired => {
            state.set_paired(true);
            state.status = "paired with a peer".into();
        }
        Message::PeerLeft => {
            state.set_paired(false);
            state.status = "the peer left".into();
        }
        Message::IrData { records } => {
            if state.paired {
                for record in records.chunks_exact(RECORD_LEN) {
                    let ns = u64::from_le_bytes(record[..8].try_into().unwrap());
                    let level = record[8] != 0;
                    if state.incoming.len() >= MAX_QUEUED_EDGES {
                        state.incoming.pop_front();
                    }
                    state.incoming.push_back((ns, level));
                }
            }
        }
        Message::Ping => state.send(&Message::Pong),
        Message::Pong => {}
        Message::Error { message, .. } => {
            state.status = message;
        }
        Message::Hello { .. } | Message::Join { .. } | Message::Leave => {}
    }
}

/// The emulator-facing transceiver.
pub struct WebIr {
    state: SharedWebIr,
}

impl WebIr {
    pub fn new(state: SharedWebIr) -> Self {
        Self { state }
    }
}

impl IrInterface for WebIr {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64) {
        self.state
            .borrow_mut()
            .engine
            .set_clock_rate(emulated_clock_rate);
    }

    fn set_carrier(&mut self, cycle: u64, carrier: bool) {
        let mut state = self.state.borrow_mut();
        if !state.paired {
            return;
        }
        let wire_ns = state.engine.outgoing_wire_ns(cycle, carrier);
        let mut records = Vec::with_capacity(RECORD_LEN);
        records.extend_from_slice(&wire_ns.to_le_bytes());
        records.push(carrier as u8);
        state.send(&Message::IrData { records });
    }

    fn carrier_detected(&mut self, cycle: u64) -> bool {
        let mut state = self.state.borrow_mut();
        while let Some((ns, level)) = state.incoming.pop_front() {
            state.engine.push_incoming(cycle, ns, level);
        }
        state.engine.current_level(cycle)
    }

    fn set_receiver_power(&mut self, cycle: u64, powered: bool) {
        let mut state = self.state.borrow_mut();
        let paired = state.paired;
        state.engine.receiver_power_changed(cycle, powered, paired);
    }
}

/// The run-loop-facing rollback control.
pub struct WebIrControl {
    state: SharedWebIr,
}

impl WebIrControl {
    pub fn new(state: SharedWebIr) -> Self {
        Self { state }
    }
}

impl IrRollbackControl for WebIrControl {
    fn poll(&mut self, now_cycle: u64) -> RollbackDirective {
        self.state.borrow_mut().engine.poll(now_cycle)
    }

    fn snapshot_taken(&mut self, cycle: u64) {
        self.state.borrow_mut().engine.snapshot_taken(cycle);
    }

    fn rolled_back(&mut self, restored_cycle: u64, abandoned_cycle: u64) {
        let mut state = self.state.borrow_mut();
        let close = state.engine.rolled_back(restored_cycle, abandoned_cycle);
        if let Some(close_ns) = close {
            if state.paired {
                let mut records = Vec::with_capacity(RECORD_LEN);
                records.extend_from_slice(&close_ns.to_le_bytes());
                records.push(0);
                state.send(&Message::IrData { records });
            }
        }
    }
}
