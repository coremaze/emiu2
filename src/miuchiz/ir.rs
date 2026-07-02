//! The Miuchiz PCB's infrared circuitry.
//!
//! The microcontroller itself has no IR hardware; the board wires generic
//! chip features to optics:
//! - an IR LED on PE0, whose ~38kHz carrier is Timer0's TCO0 clocking
//!   output (the firmware gates it on and off to modulate),
//! - a demodulating receiver module on PE1, whose active-low output idles
//!   high and is powered from PB6 (the firmware powers it off while
//!   transmitting so it does not hear its own LED).
//!
//! This adapter sits between the chip's generic port interface and both
//! the rest of the board I/O (buttons, LEDs) and the IR medium.

use super::st2205u::{GpioConnections, GpioInterfaceInternal, GpioPort, GpioState};
use crate::ir::IrInterface;
use crate::state::{StateError, StateReader, StateWriter};

/// How often to poll the receive side of the transceiver, in oscillator
/// cycles. The firmware samples the line from its 8192Hz base timer ISR
/// (one bit time is 8 such ticks), so this leaves ample resolution while
/// keeping the virtual call off the per-instruction hot path.
const RX_POLL_INTERVAL: u64 = 256;

const PB_RX_POWER: u8 = 0b0100_0000;

pub struct IrCircuit {
    io: Box<dyn GpioInterfaceInternal>,
    transceiver: Box<dyn IrInterface>,

    carrier_out: bool,
    rx_powered: bool,
    carrier_seen: bool,
    next_rx_poll: u64,
}

impl IrCircuit {
    pub fn new(
        io: Box<dyn GpioInterfaceInternal>,
        mut transceiver: Box<dyn IrInterface>,
        frequency: u64,
    ) -> Self {
        transceiver.set_clock_rate(frequency);
        Self {
            io,
            transceiver,
            carrier_out: false,
            // PB6 idles high (pulled up) until the firmware drives it.
            rx_powered: true,
            carrier_seen: false,
            next_rx_poll: 0,
        }
    }
}

impl GpioInterfaceInternal for IrCircuit {
    fn get_inputs(&mut self, cycle: u64) -> GpioConnections {
        let mut inputs = self.io.get_inputs(cycle);

        // The transceiver is always polled, even while the receiver cannot
        // be heard: transports track the passage of time through this call.
        if cycle >= self.next_rx_poll {
            self.next_rx_poll = cycle + RX_POLL_INTERVAL;
            self.carrier_seen = self.transceiver.carrier_detected(cycle);
        }

        // The receiver's demodulated output: low while a carrier is
        // detected, high at idle. Unpowered, it rests at the idle level.
        let line = !(self.rx_powered && self.carrier_seen);
        inputs.connect(GpioPort::PE, 1, line);

        inputs
    }

    fn set_outputs(&mut self, state: GpioState, cycle: u64) {
        if state.tco0 != self.carrier_out {
            self.carrier_out = state.tco0;
            self.transceiver.set_carrier(cycle, state.tco0);
        }
        self.rx_powered = state.pb & PB_RX_POWER != 0;
        self.io.set_outputs(state, cycle);
    }

    // The circuit's own latches are board-level emulation state; the
    // transceiver and inner io are host connections and stay live.
    fn save_state(&self, writer: &mut StateWriter) {
        writer.put_bool(self.carrier_out);
        writer.put_bool(self.rx_powered);
        writer.put_bool(self.carrier_seen);
        writer.put_u64(self.next_rx_poll);
        self.io.save_state(writer);
    }

    fn load_state(&mut self, reader: &mut StateReader) -> Result<(), StateError> {
        self.carrier_out = reader.take_bool()?;
        self.rx_powered = reader.take_bool()?;
        self.carrier_seen = reader.take_bool()?;
        self.next_rx_poll = reader.take_u64()?;
        self.io.load_state(reader)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    struct NullGpioIo;

    impl GpioInterfaceInternal for NullGpioIo {
        fn get_inputs(&mut self, _cycle: u64) -> GpioConnections {
            GpioConnections::default()
        }

        fn set_outputs(&mut self, _state: GpioState, _cycle: u64) {}
    }

    struct ScriptedIr {
        detected: Rc<Cell<bool>>,
        polls: Rc<Cell<u32>>,
        carrier_log: Rc<RefCell<Vec<(u64, bool)>>>,
    }

    impl IrInterface for ScriptedIr {
        fn set_clock_rate(&mut self, _emulated_clock_rate: u64) {}

        fn set_carrier(&mut self, cycle: u64, carrier: bool) {
            self.carrier_log.borrow_mut().push((cycle, carrier));
        }

        fn carrier_detected(&mut self, _cycle: u64) -> bool {
            self.polls.set(self.polls.get() + 1);
            self.detected.get()
        }
    }

    struct Harness {
        circuit: IrCircuit,
        detected: Rc<Cell<bool>>,
        polls: Rc<Cell<u32>>,
        carrier_log: Rc<RefCell<Vec<(u64, bool)>>>,
    }

    fn harness() -> Harness {
        let detected = Rc::new(Cell::new(false));
        let polls = Rc::new(Cell::new(0));
        let carrier_log = Rc::new(RefCell::new(Vec::new()));
        let circuit = IrCircuit::new(
            Box::new(NullGpioIo),
            Box::new(ScriptedIr {
                detected: detected.clone(),
                polls: polls.clone(),
                carrier_log: carrier_log.clone(),
            }),
            16_000_000,
        );
        Harness {
            circuit,
            detected,
            polls,
            carrier_log,
        }
    }

    fn outputs(pb: u8, tco0: bool) -> GpioState {
        GpioState {
            pa: 0,
            pb,
            pc: 0,
            pd: 0,
            pe: 0,
            pf: 0,
            pl: 0,
            tco0,
        }
    }

    fn pe1(inputs: &GpioConnections) -> bool {
        assert_ne!(inputs.pe_connections & 0b10, 0, "PE1 must be driven");
        inputs.pe & 0b10 != 0
    }

    #[test]
    fn tco0_envelope_reaches_the_transceiver() {
        let mut h = harness();
        h.circuit.set_outputs(outputs(0xFF, false), 0);
        assert!(h.carrier_log.borrow().is_empty());

        h.circuit.set_outputs(outputs(0xFF, true), 100);
        h.circuit.set_outputs(outputs(0xFF, true), 200); // no change, no edge
        h.circuit.set_outputs(outputs(0xFF, false), 300);
        assert_eq!(*h.carrier_log.borrow(), vec![(100, true), (300, false)]);
    }

    #[test]
    fn receiver_drives_pe1_active_low() {
        let mut h = harness();
        assert!(pe1(&h.circuit.get_inputs(0)));

        h.detected.set(true);
        assert!(!pe1(&h.circuit.get_inputs(RX_POLL_INTERVAL)));

        h.detected.set(false);
        assert!(pe1(&h.circuit.get_inputs(RX_POLL_INTERVAL * 2)));
    }

    #[test]
    fn unpowered_receiver_rests_high_but_transceiver_stays_polled() {
        let mut h = harness();
        h.circuit.set_outputs(outputs(0x00, false), 0); // PB6 low
        h.detected.set(true);

        let polls_before = h.polls.get();
        assert!(pe1(&h.circuit.get_inputs(RX_POLL_INTERVAL)));
        assert!(h.polls.get() > polls_before);
    }

    #[test]
    fn rx_polling_is_rate_limited() {
        let mut h = harness();
        h.circuit.get_inputs(0);
        let polls = h.polls.get();
        h.circuit.get_inputs(1);
        h.circuit.get_inputs(RX_POLL_INTERVAL - 1);
        assert_eq!(h.polls.get(), polls);
        h.circuit.get_inputs(RX_POLL_INTERVAL);
        assert_eq!(h.polls.get(), polls + 1);
    }
}
