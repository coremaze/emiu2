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
use emiu2_netplay::{
    decode_edges, encode_edge, FriendCode, Message, EDGE_RECORD_LEN, PROTOCOL_VERSION,
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{BinaryType, MessageEvent, WebSocket};

const MAX_QUEUED_EDGES: usize = 100_000;

pub struct WebIrState {
    engine: ReplayEngine,
    socket: Option<WebSocket>,
    connected: bool,
    paired: bool,
    code: Option<FriendCode>,
    /// User-facing events (pairing changes, errors) awaiting display,
    /// oldest first.
    notices: VecDeque<String>,
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
            notices: VecDeque::new(),
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

    /// The next user-facing event to show, if any.
    pub fn take_notice(&mut self) -> Option<String> {
        self.notices.pop_front()
    }

    fn notify(&mut self, notice: impl Into<String>) {
        // A UI that stops draining should not make us grow without bound.
        if self.notices.len() >= 8 {
            self.notices.pop_front();
        }
        self.notices.push_back(notice.into());
    }

    fn send(&self, message: &Message) {
        if let Some(socket) = &self.socket {
            if socket.ready_state() == WebSocket::OPEN {
                let _ = socket.send_with_u8_array(&message.encode());
            }
        }
    }

    /// Sends edge records as one IrData message.
    fn send_edges(&self, edges: &[(u64, bool)]) {
        let mut records = Vec::with_capacity(edges.len() * EDGE_RECORD_LEN);
        for &(ns, level) in edges {
            records.extend_from_slice(&encode_edge(ns, level));
        }
        self.send(&Message::IrData { records });
    }

    /// Sends any completed burst's atomic copy, each as one message
    /// (see `crate::ir_replay` on atomic resend).
    fn pump_resends(&mut self, now_cycle: u64) {
        while let Some(burst) = self.engine.take_burst_resend(now_cycle) {
            if self.paired {
                self.send_edges(&burst);
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
        // Failed reconnect attempts also land here; only an established
        // connection dying is worth telling the player about.
        if state.connected {
            state.notify("Lost the connection; reconnecting\u{2026}");
        }
        state.connected = false;
        state.set_paired(false);
        state.code = None;
    }) as Box<dyn FnMut(web_sys::Event)>);
    socket.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();

    state.borrow_mut().socket = Some(socket);
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
        }
        Message::Paired => {
            state.set_paired(true);
            state.notify("Paired with your friend");
        }
        Message::PeerLeft => {
            state.set_paired(false);
            state.notify("Your friend left");
        }
        Message::IrData { records } => {
            if state.paired {
                let Ok(edges) = decode_edges(&records) else {
                    web_sys::console::warn_1(&"IR relay: malformed edge records".into());
                    return;
                };
                for edge in edges {
                    if state.incoming.len() >= MAX_QUEUED_EDGES {
                        state.incoming.pop_front();
                    }
                    state.incoming.push_back(edge);
                }
            }
        }
        Message::Ping => state.send(&Message::Pong),
        Message::Pong => {}
        Message::Error { message, .. } => {
            state.notify(message);
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
        // If this edge started a new burst, the previous burst's atomic
        // copy goes out first, keeping the wire in stream order.
        state.pump_resends(cycle);
        state.send_edges(&[(wire_ns, carrier)]);
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
        let mut state = self.state.borrow_mut();
        let directive = state.engine.poll(now_cycle);
        state.pump_resends(now_cycle);
        directive
    }

    fn snapshot_taken(&mut self, cycle: u64) {
        self.state.borrow_mut().engine.snapshot_taken(cycle);
    }

    fn rolled_back(&mut self, restored_cycle: u64, abandoned_cycle: u64) {
        let mut state = self.state.borrow_mut();
        let close = state.engine.rolled_back(restored_cycle, abandoned_cycle);
        if let Some(close_ns) = close {
            if state.paired {
                state.send_edges(&[(close_ns, false)]);
            }
        }
        state.pump_resends(restored_cycle);
    }
}
