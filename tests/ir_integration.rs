//! End-to-end tests of the IR chip layer against the real firmware's IR
//! driver: the 8192Hz base-timer soft-modem in flash bank 1.
//!
//! Rather than booting and navigating the UI, each test synthesizes the
//! machine state the firmware's own IR start routines establish (interrupt
//! bank IRR=$0201, base timer on, driver state in RAM), parks the CPU on a
//! spin loop, and lets the firmware's real ISRs do the transmitting and
//! receiving through the emulated chip layer.
//!
//! The tests are skipped when the firmware images are not present in
//! `firmware/` (they are not distributable).

use emiu2::audio::AudioInterface;
use emiu2::ir::IrInterface;
use emiu2::memory::AddressSpace;
use emiu2::miuchiz::{GpioConnections, GpioInterfaceInternal, GpioState, Handheld, SYSTEM_FREQ};
use emiu2::platform::socket_ir::SocketIr;
use emiu2::rollback::RollbackDriver;
use emiu2::screen::{Pixel, Screen};
use std::cell::{Cell, RefCell};
use std::io::{Read as IoRead, Write as IoWrite};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

// ST2205U registers
const PB: usize = 0x01;
const PCB: usize = 0x09;
const PMCR: usize = 0x3A;
const XREQ: usize = 0x3B;
const IENAL: usize = 0x3E;
const IRRL: usize = 0x30;
const IRRH: usize = 0x31;
const BTEN: usize = 0x2A;
const BTC: usize = 0x2C;

// The firmware's IR driver state in RAM
const IR_STATE: usize = 0x0A03; // bit0 TX active, bit1 RX active, bit7 RX success
const IR_LEN: usize = 0x0A04;
const IR_TX_BUF: usize = 0x0A0C;
const IR_RX_BUF: usize = 0x08A0;

const IDLE_LOOP: usize = 0x0400;

/// Cycles at which base-timer tick `tick` occurs (8192Hz).
fn tick_cycles(tick: u64) -> u64 {
    tick * SYSTEM_FREQ / 8192
}

struct NullScreen;

impl Screen for NullScreen {
    fn set_pixels(&self, _pixels: &[Pixel]) {}
}

struct NullGpio;

impl GpioInterfaceInternal for NullGpio {
    fn get_inputs(&mut self, _cycle: u64) -> GpioConnections {
        GpioConnections::default()
    }

    fn set_outputs(&mut self, _state: GpioState, _cycle: u64) {}
}

struct NullAudio;

impl AudioInterface for NullAudio {
    fn set_clock_rate(&mut self, _emulated_clock_rate: u64) {}

    fn needs_sample(&self, _current_cycle: u64) -> bool {
        false
    }

    fn add_sample(&mut self, _value: f32) {}
}

/// Records the transmit envelope edges a device produces.
struct RecordingIr {
    edges: Rc<RefCell<Vec<(u64, bool)>>>,
}

impl IrInterface for RecordingIr {
    fn set_clock_rate(&mut self, _emulated_clock_rate: u64) {}

    fn set_carrier(&mut self, cycle: u64, carrier: bool) {
        self.edges.borrow_mut().push((cycle, carrier));
    }

    fn carrier_detected(&mut self, _cycle: u64) -> bool {
        false
    }
}

/// Plays a prerecorded envelope into a device's receiver.
struct PlaybackIr {
    edges: Vec<(u64, bool)>,
    pos: usize,
    level: bool,
}

impl PlaybackIr {
    fn new(edges: Vec<(u64, bool)>) -> Self {
        Self {
            edges,
            pos: 0,
            level: false,
        }
    }
}

impl IrInterface for PlaybackIr {
    fn set_clock_rate(&mut self, _emulated_clock_rate: u64) {}

    fn set_carrier(&mut self, _cycle: u64, _carrier: bool) {}

    fn carrier_detected(&mut self, cycle: u64) -> bool {
        while self.pos < self.edges.len() && self.edges[self.pos].0 <= cycle {
            self.level = self.edges[self.pos].1;
            self.pos += 1;
        }
        self.level
    }
}

/// One endpoint of a zero-latency shared IR bus between two devices.
struct BusIr {
    own_tx: Rc<Cell<bool>>,
    peer_tx: Rc<Cell<bool>>,
}

impl IrInterface for BusIr {
    fn set_clock_rate(&mut self, _emulated_clock_rate: u64) {}

    fn set_carrier(&mut self, _cycle: u64, carrier: bool) {
        self.own_tx.set(carrier);
    }

    fn carrier_detected(&mut self, _cycle: u64) -> bool {
        self.peer_tx.get()
    }
}

/// The on-air frame for a payload: preamble, length, payload, and a
/// checksum chosen so that length + payload + checksum sums to zero.
fn frame_bytes(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u8;
    let sum = payload.iter().fold(len, |acc, &b| acc.wrapping_add(b));
    let mut frame = vec![0xAA, len];
    frame.extend_from_slice(payload);
    frame.push(sum.wrapping_neg());
    frame
}

/// Per-tick carrier levels for a frame. Bytes go on the air complemented
/// and LSB first; each bit lasts 8 ticks, Manchester encoded: a wire 1 is
/// carrier for the first 4 ticks, a wire 0 carrier for the last 4.
fn encode_frame(payload: &[u8]) -> Vec<bool> {
    let mut ticks = Vec::new();
    for byte in frame_bytes(payload) {
        let wire = !byte;
        for bit in 0..8 {
            let carrier_first = wire & (1 << bit) != 0;
            ticks.extend_from_slice(&[carrier_first; 4]);
            ticks.extend_from_slice(&[!carrier_first; 4]);
        }
    }
    ticks
}

/// Converts per-tick levels to envelope edges. `phase` offsets the
/// waveform from the receiver's base-timer tick boundaries, as an
/// unsynchronized real transmitter would be.
fn ticks_to_edges(ticks: &[bool], start_tick: u64, phase: u64) -> Vec<(u64, bool)> {
    let mut edges = Vec::new();
    let mut level = false;
    for (i, &l) in ticks.iter().enumerate() {
        if l != level {
            edges.push((tick_cycles(start_tick + i as u64) + phase, l));
            level = l;
        }
    }
    if level {
        edges.push((tick_cycles(start_tick + ticks.len() as u64) + phase, false));
    }
    edges
}

fn level_at(edges: &[(u64, bool)], cycle: u64) -> bool {
    let mut level = false;
    for &(c, l) in edges {
        if c > cycle {
            break;
        }
        level = l;
    }
    level
}

/// Decodes `count` bytes from a recorded envelope, sampling the middle of
/// each Manchester half-bit. The first carrier-on edge starts bit 0.
fn decode_frame(edges: &[(u64, bool)], count: usize) -> Vec<u8> {
    let start = edges
        .iter()
        .find(|&&(_, l)| l)
        .expect("no carrier was ever transmitted")
        .0;
    let bit_len = tick_cycles(8);
    let half = tick_cycles(2);
    let mut bytes = Vec::new();
    for byte_index in 0..count {
        let mut byte = 0u8;
        for bit in 0..8 {
            let bit_start = start + (byte_index as u64 * 8 + bit) * bit_len;
            let wire = level_at(edges, bit_start + half);
            if !wire {
                byte |= 1 << bit;
            }
        }
        bytes.push(byte);
    }
    bytes
}

fn load_firmware() -> Option<(Vec<u8>, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("firmware");
    let otp = std::fs::read(dir.join("OTP.dat")).ok()?;
    let flash = std::fs::read(dir.join("Spike 1.02.dat")).ok()?;
    Some((otp, flash))
}

fn make_handheld(otp: &[u8], flash: &[u8], ir: Box<dyn IrInterface>) -> Handheld {
    Handheld::new(
        otp,
        flash,
        Box::new(NullScreen),
        Box::new(NullGpio),
        Box::new(NullAudio),
        ir,
    )
    .expect("could not construct handheld")
}

fn write(h: &mut Handheld, address: usize, value: u8) {
    h.mcu.core.address_space.write_u8(address, value);
}

fn read(h: &mut Handheld, address: usize) -> u8 {
    h.mcu.core.address_space.read_u8(address)
}

/// Establishes the state the firmware sets up before using IR: flash bank 1
/// mapped as the interrupt bank (its vectors and ISRs hold the IR driver)
/// and the 8192Hz base-timer tick running. The CPU is parked on a spin loop
/// in RAM with interrupts enabled, standing in for the firmware's idle loop.
fn setup_interrupt_bank(h: &mut Handheld) {
    write(h, IRRL, 0x01);
    write(h, IRRH, 0x02);
    write(h, BTC, 0x01);
    write(h, BTEN, 0x80);

    // BRA -2
    write(h, IDLE_LOOP, 0x80);
    write(h, IDLE_LOOP + 1, 0xFE);
    h.mcu.core.registers.pc = IDLE_LOOP as u16;
    h.mcu.core.registers.sp = 0xFF;
    h.mcu.core.flags.interrupt_disable = false;
}

/// Stages an outgoing packet the way the firmware's TX start routine does.
fn setup_transmit(h: &mut Handheld, payload: &[u8]) {
    write(h, IR_LEN, payload.len() as u8);
    for (i, &b) in payload.iter().enumerate() {
        write(h, IR_TX_BUF + i, b);
    }
    // Clear the modem's shift registers, counters and substate ($0A05-$0A0A),
    // as the firmware's TX start routine does. A completed reception leaves
    // the substate nonzero, which would otherwise abort the transmission.
    for address in 0x0A05..=0x0A0A {
        write(h, address, 0);
    }
    write(h, IR_STATE, 0x01); // TX active; the BT ISR does the rest
    write(h, IENAL, 0x40); // base timer only
}

/// Arms reception the way the firmware's RX start routine does. The first
/// INTX edge starts the sampler.
fn setup_receive(h: &mut Handheld) {
    write(h, 0x0A0A, 0); // modem substate: idle, so INTX arms the sampler
    write(h, PMCR, 0x02); // PE1 = INTX1, falling edge
    write(h, IENAL, 0x41); // base timer + INTX
}

/// Steps until `done` or the cycle cap is reached; returns whether `done`.
fn run_until(h: &mut Handheld, cap: u64, mut done: impl FnMut(&mut Handheld) -> bool) -> bool {
    while h.mcu.core.cycles < cap {
        for _ in 0..500 {
            h.mcu.step();
        }
        if done(h) {
            return true;
        }
    }
    false
}

/// Steps two handhelds in small interleaved quanta, paced against the wall
/// clock like two separately running emulators, until `tick` returns true.
/// Panics if that takes longer than `limit`.
fn run_paced_pair(
    a: &mut Handheld,
    b: &mut Handheld,
    limit: Duration,
    mut tick: impl FnMut(&mut Handheld, &mut Handheld) -> bool,
) {
    let begin = Instant::now();
    // Note: the core counts CPU cycles, which run at half the oscillator.
    let cps = a.mcu.core.cycles_per_second() as u128;
    let quantum = (cps / 2000) as u64; // 0.5ms of emulated time
    let mut target = 0u64;
    loop {
        assert!(begin.elapsed() < limit, "paced run timed out");
        target += quantum;
        while begin.elapsed().as_nanos() * cps / 1_000_000_000 < target as u128 {
            std::thread::sleep(Duration::from_micros(100));
        }
        while a.mcu.core.cycles < target {
            a.mcu.step();
        }
        while b.mcu.core.cycles < target {
            b.mcu.step();
        }
        if tick(a, b) {
            return;
        }
    }
}

#[test]
fn firmware_transmits_expected_ir_frame() {
    let Some((otp, flash)) = load_firmware() else {
        eprintln!("skipping: firmware images not present");
        return;
    };

    let edges = Rc::new(RefCell::new(Vec::new()));
    let mut h = make_handheld(
        &otp,
        &flash,
        Box::new(RecordingIr {
            edges: edges.clone(),
        }),
    );
    setup_interrupt_bank(&mut h);
    let payload = [0x12, 0x34, 0x56, 0x78];
    setup_transmit(&mut h, &payload);

    // The modulator clears the TX-active bit when the whole frame is out.
    assert!(
        run_until(&mut h, 10_000_000, |h| read(h, IR_STATE) & 0x01 == 0),
        "firmware never finished transmitting"
    );

    let expected = frame_bytes(&payload);
    let decoded = decode_frame(&edges.borrow(), expected.len());
    assert_eq!(decoded, expected);

    // The envelope must end with the carrier off.
    assert!(!edges.borrow().last().unwrap().1);
}

#[test]
fn firmware_receives_scripted_ir_frame() {
    let Some((otp, flash)) = load_firmware() else {
        eprintln!("skipping: firmware images not present");
        return;
    };

    let payload = [0xC3, 0x5A, 0x01, 0xFE];
    // Start the waveform on no particular tick boundary, half a tick out of
    // phase with the receiver's base timer.
    let edges = ticks_to_edges(&encode_frame(&payload), 600, tick_cycles(1) / 2);

    let mut h = make_handheld(&otp, &flash, Box::new(PlaybackIr::new(edges)));
    setup_interrupt_bank(&mut h);
    setup_receive(&mut h);

    // Bit 7 of the driver state is set once a frame with a valid checksum
    // has been received.
    assert!(
        run_until(&mut h, 10_000_000, |h| read(h, IR_STATE) & 0x80 != 0),
        "firmware never received the frame"
    );

    assert_eq!(read(&mut h, IR_LEN) as usize, payload.len());
    for (i, &b) in payload.iter().enumerate() {
        assert_eq!(read(&mut h, IR_RX_BUF + i), b, "payload byte {i}");
    }

    // The firmware acks each INTX in its handler, so no request remains.
    assert_eq!(read(&mut h, XREQ) & 0x02, 0);
}

#[test]
fn two_handhelds_exchange_ir_packet_over_lockstep_bus() {
    let Some((otp, flash)) = load_firmware() else {
        eprintln!("skipping: firmware images not present");
        return;
    };

    let a_tx = Rc::new(Cell::new(false));
    let b_tx = Rc::new(Cell::new(false));

    let mut sender = make_handheld(
        &otp,
        &flash,
        Box::new(BusIr {
            own_tx: a_tx.clone(),
            peer_tx: b_tx.clone(),
        }),
    );
    let mut receiver = make_handheld(
        &otp,
        &flash,
        Box::new(BusIr {
            own_tx: b_tx.clone(),
            peer_tx: a_tx.clone(),
        }),
    );

    setup_interrupt_bank(&mut sender);
    setup_interrupt_bank(&mut receiver);
    let payload = [0xDE, 0xAD, 0xBE, 0xEF, 0x42];
    setup_transmit(&mut sender, &payload);
    setup_receive(&mut receiver);

    let mut received = false;
    while sender.mcu.core.cycles < 10_000_000 && !received {
        for _ in 0..100 {
            sender.mcu.step();
            while receiver.mcu.core.cycles < sender.mcu.core.cycles {
                receiver.mcu.step();
            }
        }
        received = read(&mut receiver, IR_STATE) & 0x80 != 0;
    }
    assert!(received, "receiver never got the frame");

    assert_eq!(read(&mut receiver, IR_LEN) as usize, payload.len());
    for (i, &b) in payload.iter().enumerate() {
        assert_eq!(read(&mut receiver, IR_RX_BUF + i), b, "payload byte {i}");
    }
}

/// Models the firmware's IR handshake timing over the socket transport.
/// After transmitting, the sender listens for a reply for only ~98ms (200
/// ticks of its 2048Hz script delay) before retrying, and it cannot hear
/// while retrying. The transport's replay latency is paid once in each
/// direction, so the round trip only closes if that budget is small.
#[test]
fn firmware_reply_arrives_within_the_senders_listen_window() {
    let Some((otp, flash)) = load_firmware() else {
        eprintln!("skipping: firmware images not present");
        return;
    };

    let listen_ir = SocketIr::listen(("127.0.0.1", 0)).expect("could not bind IR socket");
    let port = listen_ir.local_addr().unwrap().port();
    let connect_ir = SocketIr::connect(format!("127.0.0.1:{port}"));

    let start = Instant::now();
    while !(listen_ir.connected() && connect_ir.connected()) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "IR sockets never connected"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let mut requester = make_handheld(&otp, &flash, Box::new(connect_ir));
    let mut responder = make_handheld(&otp, &flash, Box::new(listen_ir));
    setup_interrupt_bank(&mut requester);
    setup_interrupt_bank(&mut responder);

    // The factory test sends a 16-byte packet and the peer answers with 2
    // bytes; use the same shape.
    let request: Vec<u8> = (0..16).map(|i| 0xC0 + i).collect();
    let reply = [0xF1, 0x02];
    setup_transmit(&mut requester, &request);
    setup_receive(&mut responder);

    let mut request_done_cycle: Option<u64> = None;
    let mut responder_replied = false;
    let mut reply_elapsed = None;
    run_paced_pair(
        &mut requester,
        &mut responder,
        Duration::from_secs(15),
        |requester, responder| {
            // Requester finished transmitting: it turns around and listens.
            if request_done_cycle.is_none() && read(requester, IR_STATE) & 0x01 == 0 {
                request_done_cycle = Some(requester.mcu.core.cycles);
                setup_receive(requester);
            }

            // Responder got the request: it answers immediately, like the
            // firmware's response script does.
            if !responder_replied && read(responder, IR_STATE) & 0x80 != 0 {
                responder_replied = true;
                setup_transmit(responder, &reply);
            }

            if let Some(done) = request_done_cycle {
                if read(requester, IR_STATE) & 0x80 != 0 {
                    reply_elapsed = Some(requester.mcu.core.cycles - done);
                    return true;
                }
            }
            false
        },
    );

    // 200 ticks at 2048Hz: the reply must arrive within this much of the
    // requester finishing its transmission. In CPU cycles, which run at
    // half the oscillator frequency.
    let listen_window = 200 * (SYSTEM_FREQ / 2) / 2048;
    let elapsed = reply_elapsed.unwrap();
    assert!(
        elapsed < listen_window,
        "reply arrived after {elapsed} cycles, past the \
         {listen_window}-cycle listen window"
    );

    assert_eq!(read(&mut responder, IR_RX_BUF), request[0]);
    assert_eq!(read(&mut requester, IR_LEN) as usize, reply.len());
    for (i, &b) in reply.iter().enumerate() {
        assert_eq!(read(&mut requester, IR_RX_BUF + i), b, "reply byte {i}");
    }
}

/// Drives PB6, the receiver module's power supply, the way the firmware
/// does: off while transmitting, on while listening. `PCB` bit 6 must be
/// configured as an output first.
fn set_rx_power(h: &mut Handheld, powered: bool) {
    write(h, PB, if powered { 0x40 } else { 0x00 });
}

/// A TCP proxy that delays every byte by a fixed amount in each
/// direction, modeling a high-latency internet link.
fn spawn_delay_proxy(target_port: u16, delay: Duration) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("could not bind proxy");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let Ok((client, _)) = listener.accept() else {
            return;
        };
        let Ok(upstream) = TcpStream::connect(("127.0.0.1", target_port)) else {
            return;
        };
        client.set_nodelay(true).ok();
        upstream.set_nodelay(true).ok();
        let (Ok(client_copy), Ok(upstream_copy)) = (client.try_clone(), upstream.try_clone())
        else {
            return;
        };
        std::thread::spawn(move || delay_pipe(client, upstream_copy, delay));
        delay_pipe(upstream, client_copy, delay);
    });
    port
}

fn delay_pipe(mut from: TcpStream, mut to: TcpStream, delay: Duration) {
    let (tx, rx) = mpsc::channel::<(Instant, Vec<u8>)>();
    std::thread::spawn(move || {
        while let Ok((arrived, data)) = rx.recv() {
            let release = arrived + delay;
            let now = Instant::now();
            if release > now {
                std::thread::sleep(release - now);
            }
            if to.write_all(&data).is_err() {
                return;
            }
        }
    });
    let mut buf = [0u8; 4096];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if tx.send((Instant::now(), buf[..n].to_vec())).is_err() {
                    return;
                }
            }
        }
    }
}

/// On a link whose round trip far exceeds the firmware's ~98ms listen
/// window, the handshake can only complete through rollback: replies
/// always arrive after the requester has stopped listening, so the
/// requester's machine must be rewound to its listen-window snapshot and
/// the reply replayed inside it. Models the full firmware behavior
/// including retransmission with the receiver powered off.
#[test]
fn rollback_recovers_handshake_over_high_latency_link() {
    let Some((otp, flash)) = load_firmware() else {
        eprintln!("skipping: firmware images not present");
        return;
    };

    // Both machines are paced at 1/8 of real time: a debug build cannot
    // sustain two full-speed machines on one thread, and uniform slow
    // motion is invisible to the transport (all scheduling is in
    // emulated time). The proxy delay is in wall time, so 400ms here is
    // 50ms of emulated one-way delay: a 100ms emulated round trip,
    // comfortably past the ~98ms listen window.
    const PACE_DIV: u32 = 8;
    const ONE_WAY_DELAY: Duration = Duration::from_millis(400);

    let listen_ir = SocketIr::listen(("127.0.0.1", 0)).expect("could not bind IR socket");
    let port = listen_ir.local_addr().unwrap().port();
    let proxy_port = spawn_delay_proxy(port, ONE_WAY_DELAY);
    let connect_ir = SocketIr::connect(format!("127.0.0.1:{proxy_port}"));

    let requester_control = connect_ir.rollback_handle();
    let responder_control = listen_ir.rollback_handle();

    let start = Instant::now();
    while !(listen_ir.connected() && connect_ir.connected()) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "IR sockets never connected through the proxy"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let mut requester = make_handheld(&otp, &flash, Box::new(connect_ir));
    let mut responder = make_handheld(&otp, &flash, Box::new(listen_ir));
    setup_interrupt_bank(&mut requester);
    setup_interrupt_bank(&mut responder);

    // PB6 (receiver power) as an output on both boards.
    write(&mut requester, PCB, 0x40);
    write(&mut responder, PCB, 0x40);

    let mut requester_driver = RollbackDriver::new(Box::new(requester_control));
    let mut responder_driver = RollbackDriver::new(Box::new(responder_control));

    let request: Vec<u8> = (0..16).map(|i| 0xC0 + i).collect();
    let reply = [0xF1, 0x02];

    // The requester transmits blind, receiver off, like the firmware.
    set_rx_power(&mut requester, false);
    setup_transmit(&mut requester, &request);
    set_rx_power(&mut responder, true);
    setup_receive(&mut responder);

    let listen_window = 200 * (SYSTEM_FREQ / 2) / 2048; // core cycles

    let mut listening_since: Option<u64> = None;
    let mut retransmits = 0u32;
    let mut responder_transmitting = false;
    let mut replies_sent = 0u32;
    let mut rollbacks = 0u32;

    // Each machine is paced against the wall clock independently, with
    // its own anchor, exactly like the real run loop: after a rollback
    // the machine must resume at 1x from the restored point, because the
    // rest of the frame it is waiting for is still arriving over the
    // network in real time. A shared fast-forwarding target would race
    // ahead of the arriving edges.
    struct Paced {
        anchor_wall: Instant,
        anchor_cycles: u64,
    }

    impl Paced {
        fn new(h: &Handheld) -> Self {
            Self {
                anchor_wall: Instant::now(),
                anchor_cycles: h.mcu.core.cycles,
            }
        }

        fn run_to_target(&self, h: &mut Handheld) {
            let cps = h.mcu.core.cycles_per_second() as u128;
            let target = self.anchor_cycles as u128
                + self.anchor_wall.elapsed().as_nanos() * cps
                    / 1_000_000_000
                    / u128::from(PACE_DIV);
            while (h.mcu.core.cycles as u128) < target {
                h.mcu.step();
            }
        }
    }

    let begin = Instant::now();
    let mut requester_pace = Paced::new(&requester);
    let mut responder_pace = Paced::new(&responder);
    let heard_reply = loop {
        if begin.elapsed() > Duration::from_secs(90) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(2));
        requester_pace.run_to_target(&mut requester);
        responder_pace.run_to_target(&mut responder);

        if requester_driver.run(&mut requester) {
            rollbacks += 1;
            requester_pace = Paced::new(&requester);
        }
        if responder_driver.run(&mut responder) {
            responder_pace = Paced::new(&responder);
        }

        // Requester: TX done -> listen; window expired -> retransmit.
        if listening_since.is_none() && read(&mut requester, IR_STATE) & 0x01 == 0 {
            set_rx_power(&mut requester, true);
            setup_receive(&mut requester);
            listening_since = Some(requester.mcu.core.cycles);
        }
        if let Some(since) = listening_since {
            if read(&mut requester, IR_STATE) & 0x80 != 0 {
                break true; // reply heard
            }
            if requester.mcu.core.cycles.saturating_sub(since) > listen_window {
                retransmits += 1;
                listening_since = None;
                set_rx_power(&mut requester, false);
                setup_transmit(&mut requester, &request);
            }
        }

        // Responder: answer every received request, like the firmware's
        // response script (duplicates get re-answered).
        if !responder_transmitting && read(&mut responder, IR_STATE) & 0x80 != 0 {
            responder_transmitting = true;
            replies_sent += 1;
            set_rx_power(&mut responder, false);
            setup_transmit(&mut responder, &reply);
        }
        if responder_transmitting && read(&mut responder, IR_STATE) & 0x01 == 0 {
            responder_transmitting = false;
            set_rx_power(&mut responder, true);
            setup_receive(&mut responder);
        }
    };
    assert!(heard_reply, "handshake never completed over the slow link");

    assert!(
        retransmits >= 1,
        "the reply should not fit the listen window on this link \
         (retransmits: {retransmits})"
    );
    assert!(
        rollbacks >= 1,
        "the handshake completed without any rollback, so the link \
         latency did not exercise the mechanism"
    );
    assert_eq!(read(&mut requester, IR_LEN) as usize, reply.len());
    for (i, &b) in reply.iter().enumerate() {
        assert_eq!(read(&mut requester, IR_RX_BUF + i), b, "reply byte {i}");
    }
    assert!(replies_sent >= 1);
}

#[test]
fn socket_transport_delivers_frame_between_paced_emulators() {
    let Some((otp, flash)) = load_firmware() else {
        eprintln!("skipping: firmware images not present");
        return;
    };

    let listen_ir = SocketIr::listen(("127.0.0.1", 0)).expect("could not bind IR socket");
    let port = listen_ir.local_addr().unwrap().port();
    let connect_ir = SocketIr::connect(format!("127.0.0.1:{port}"));

    // The handshake happens on background threads.
    let start = Instant::now();
    while !(listen_ir.connected() && connect_ir.connected()) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "IR sockets never connected"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let mut sender = make_handheld(&otp, &flash, Box::new(connect_ir));
    let mut receiver = make_handheld(&otp, &flash, Box::new(listen_ir));
    setup_interrupt_bank(&mut sender);
    setup_interrupt_bank(&mut receiver);
    let payload = [0x21, 0x43, 0x65];
    setup_transmit(&mut sender, &payload);
    setup_receive(&mut receiver);

    // Pace both emulators against the wall clock, as the real emulator
    // loop does. The transport schedules burst replay in emulated time, so
    // a receiver racing ahead of real time would play past edges that are
    // still in flight.
    run_paced_pair(
        &mut sender,
        &mut receiver,
        Duration::from_secs(10),
        |_, receiver| read(receiver, IR_STATE) & 0x80 != 0,
    );

    assert_eq!(read(&mut receiver, IR_LEN) as usize, payload.len());
    for (i, &b) in payload.iter().enumerate() {
        assert_eq!(read(&mut receiver, IR_RX_BUF + i), b, "payload byte {i}");
    }
}
