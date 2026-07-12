use crate::snapshot::{SnapshotError, SnapshotReader, SnapshotWriter};

#[derive(Debug, Clone)]
pub struct GpioConnections {
    pub pa: u8,
    pub pa_connections: u8,
    pub pb: u8,
    pub pb_connections: u8,
    pub pc: u8,
    pub pc_connections: u8,
    pub pd: u8,
    pub pd_connections: u8,
    pub pe: u8,
    pub pe_connections: u8,
    pub pf: u8,
    pub pf_connections: u8,
    pub pl: u8,
    pub pl_connections: u8,
}

impl GpioConnections {
    pub fn connect(&mut self, port: GpioPort, bit: u8, value: bool) {
        let (port_reg, port_reg_connections) = match port {
            GpioPort::PA => (&mut self.pa, &mut self.pa_connections),
            GpioPort::PB => (&mut self.pb, &mut self.pb_connections),
            GpioPort::PC => (&mut self.pc, &mut self.pc_connections),
            GpioPort::PD => (&mut self.pd, &mut self.pd_connections),
            GpioPort::PE => (&mut self.pe, &mut self.pe_connections),
            GpioPort::PF => (&mut self.pf, &mut self.pf_connections),
            GpioPort::PL => (&mut self.pl, &mut self.pl_connections),
        };
        let mask = 1 << bit;
        if value {
            *port_reg |= mask;
        } else {
            *port_reg &= !mask;
        }
        *port_reg_connections |= mask;
    }
}

impl Default for GpioConnections {
    fn default() -> Self {
        Self {
            pa: 0,
            pa_connections: 0,
            pb: 0,
            pb_connections: 0,
            pc: 0,
            pc_connections: 0,
            pd: 0,
            pd_connections: 0,
            pe: 0,
            pe_connections: 0,
            pf: 0,
            pf_connections: 0,
            pl: 0,
            pl_connections: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GpioState {
    pub pa: u8,
    pub pb: u8,
    pub pc: u8,
    pub pd: u8,
    pub pe: u8,
    pub pf: u8,
    pub pl: u8,

    /// Whether PE0 is currently driven by Timer0's clock-out (TCO0)
    /// rather than its data register. While active, the pin toggles at
    /// half the Timer0 overflow rate; the toggling itself is not
    /// emulated, and `pe` bit 0 reads as high.
    pub tco0: bool,
}

/// Everything wired to the microcontroller's I/O pins. Timestamps are in
/// oscillator cycles.
pub trait GpioInterfaceInternal {
    fn get_inputs(&mut self, cycle: u64) -> GpioConnections;
    fn set_outputs(&mut self, state: GpioState, cycle: u64);

    /// The oscillator cycle at which this interface next needs an update to
    /// happen on time (`u64::MAX` when periodic polling is enough). Board
    /// circuitry with scheduled activity (an IR edge due for replay)
    /// returns its cycle here so the run loop can update cycle-accurately
    /// instead of at input-poll granularity.
    fn next_event_cycle(&self) -> u64 {
        u64::MAX
    }

    /// Board circuitry between the chip and the outside world may hold
    /// emulation state of its own; host-side endpoints have none and can
    /// leave these defaults.
    fn snapshot(&self, _writer: &mut SnapshotWriter) {}

    fn restore(&mut self, _reader: &mut SnapshotReader) -> Result<(), SnapshotError> {
        Ok(())
    }
}

/// Pin activity detected during a GPIO update that may raise interrupts.
pub struct PortActivity {
    pub port_a_transition: bool,
    pub intx: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum GpioPort {
    PA,
    PB,
    PC,
    PD,
    PE,
    PF,

    // PL is intended for LCD use, but so far no internal LCD controller is implemented
    PL,
}

#[derive(Debug, Clone)]
enum PortMode {
    Input,
    Output,
}

#[derive(Debug, Clone)]
struct PortRegister {
    input: u8,
    output: u8,
    pull_mask: u8,
}

impl Default for PortRegister {
    fn default() -> Self {
        Self {
            input: 0,
            output: 0xFF,
            pull_mask: 0xFF,
        }
    }
}

impl PortRegister {
    fn snapshot(&self, writer: &mut SnapshotWriter) {
        writer.put_u8(self.input);
        writer.put_u8(self.output);
        writer.put_u8(self.pull_mask);
    }

    fn restore(&mut self, reader: &mut SnapshotReader) -> Result<(), SnapshotError> {
        self.input = reader.take_u8()?;
        self.output = reader.take_u8()?;
        self.pull_mask = reader.take_u8()?;
        Ok(())
    }
}

pub struct State {
    // Port data registers
    pa: PortRegister,
    pb: PortRegister,
    pc: PortRegister,
    pd: PortRegister,
    pe: PortRegister,
    pf: PortRegister,
    pl: PortRegister,

    // Port type select registers
    psc: u8,
    pse: u8,

    // Port direction control registers
    pca: u8,
    pcb: u8,
    pcc: u8,
    pcd: u8,
    pce: u8,
    pcf: u8,
    pcl: u8,

    // Port function select registers
    pfc: u8,
    pfd: u8,

    // Port miscellaneous control register
    pmcr: u8,

    /// External interrupt request register, $3B (XREQ). Bits 0..=2 record
    /// edges on PE0..=PE2 when configured as INTX inputs; writing 1 clears.
    xreq: u8,

    /// PE input levels at the previous update, for INTX edge detection.
    last_pe_input: u8,

    /// Whether Timer0's overflow toggle is currently routed to PE0.
    tco0_active: bool,

    io: Box<dyn GpioInterfaceInternal>,
}

impl State {
    pub fn new(io: Box<dyn GpioInterfaceInternal>) -> Self {
        Self {
            pa: PortRegister::default(),
            pb: PortRegister::default(),
            pc: PortRegister::default(),
            pd: PortRegister::default(),
            pe: PortRegister::default(),
            pf: PortRegister::default(),
            pl: PortRegister::default(),

            psc: 0,
            pse: 0,

            pca: 0,
            pcb: 0,
            pcc: 0,
            pcd: 0,
            pce: 0,
            pcf: 0,
            pcl: 0,

            pfc: 0,
            pfd: 0,
            pmcr: 0,

            xreq: 0,
            last_pe_input: 0xFF,
            tco0_active: false,

            io,
        }
    }

    fn get_direction(&self, port: GpioPort, bit: u8) -> PortMode {
        if bit >= 8 {
            println!("Invalid bit {bit} for port {:?}", port);
            return PortMode::Input;
        }

        let control_register = match port {
            GpioPort::PA => &self.pca,
            GpioPort::PB => &self.pcb,
            GpioPort::PC => &self.pcc,
            GpioPort::PD => &self.pcd,
            GpioPort::PE => &self.pce,
            GpioPort::PF => &self.pcf,
            GpioPort::PL => &self.pcl,
        };

        let mask = 1 << bit;

        if *control_register & mask != 0 {
            PortMode::Output
        } else {
            PortMode::Input
        }
    }

    /// The oscillator cycle at which the wired I/O next needs an update
    /// (`u64::MAX` when periodic polling is enough); see
    /// `GpioInterfaceInternal::next_event_cycle`.
    pub fn next_io_event_cycle(&self) -> u64 {
        self.io.next_event_cycle()
    }

    /// Updates the GPIO inputs and outputs and detects the pin activity
    /// that can raise interrupts: Port A transitions and INTX edges.
    ///
    /// `timer0_enabled` feeds the TCO0 clocking output: while PMCR routes
    /// Timer0 to PE0 and the timer runs, PE0 outputs the timer's overflow
    /// toggle (ST2205U section 13).
    pub fn update(&mut self, cycle: u64, timer0_enabled: bool) -> PortActivity {
        let inputs = self.io.get_inputs(cycle);

        // Remember the old PA input
        let old_pa_input = self.pa.input;

        self.update_port_gpio(GpioPort::PA, &inputs);

        self.update_port_gpio(GpioPort::PB, &inputs);
        self.update_port_gpio(GpioPort::PC, &inputs);
        self.update_port_gpio(GpioPort::PD, &inputs);
        self.update_port_gpio(GpioPort::PE, &inputs);
        self.update_port_gpio(GpioPort::PF, &inputs);
        self.update_port_gpio(GpioPort::PL, &inputs);

        let intx = self.detect_intx_edges();

        // Note: the direction requirement (PCE bit 0 as output) is not
        // modeled here; the pin function select alone routes the timer out.
        self.tco0_active = self.pmcr & 0b0000_0001 != 0 && timer0_enabled;

        // While TCO0 drives PE0, report the pin high so transmissions are
        // visible (e.g. in the GPIO LED display) even though the toggling
        // itself is not synthesized.
        let mut pe_out = (self.pe.output & self.pce) | (self.pe.input & !self.pce);
        if self.tco0_active {
            pe_out |= 0b0000_0001;
        }

        self.io.set_outputs(
            GpioState {
                pa: (self.pa.output & self.pca) | (self.pa.input & !self.pca),
                pb: (self.pb.output & self.pcb) | (self.pb.input & !self.pcb),
                pc: (self.pc.output & self.pcc) | (self.pc.input & !self.pcc),
                pd: (self.pd.output & self.pcd) | (self.pd.input & !self.pcd),
                pe: pe_out,
                pf: (self.pf.output & self.pcf) | (self.pf.input & !self.pcf),
                pl: (self.pl.output & self.pcl) | (self.pl.input & !self.pcl),
                tco0: self.tco0_active,
            },
            cycle,
        );

        PortActivity {
            port_a_transition: old_pa_input != self.pa.input,
            intx,
        }
    }

    /// PE0..=PE2 double as the external interrupt inputs INTX0..=INTX2
    /// when their PMCR function-select bits are set and the pin is an
    /// input. An edge of the INTEG-selected polarity latches the pin's
    /// XREQ flag and requests the INTX interrupt.
    fn detect_intx_edges(&mut self) -> bool {
        let old = self.last_pe_input;
        let new = self.pe.input;
        self.last_pe_input = new;

        let rising_selected = self.pmcr & 0b0010_0000 != 0;
        let mut intx = false;
        for pin in 0..3u8 {
            let mask = 1 << pin;
            if self.pmcr & mask == 0 || self.pce & mask != 0 {
                continue;
            }
            let edge = if rising_selected {
                old & mask == 0 && new & mask != 0
            } else {
                old & mask != 0 && new & mask == 0
            };
            if edge {
                self.xreq |= mask;
                intx = true;
            }
        }
        intx
    }

    pub fn snapshot(&self, writer: &mut SnapshotWriter) {
        for port in [
            &self.pa, &self.pb, &self.pc, &self.pd, &self.pe, &self.pf, &self.pl,
        ] {
            port.snapshot(writer);
        }
        writer.put_u8(self.psc);
        writer.put_u8(self.pse);
        for control in [
            self.pca, self.pcb, self.pcc, self.pcd, self.pce, self.pcf, self.pcl,
        ] {
            writer.put_u8(control);
        }
        writer.put_u8(self.pfc);
        writer.put_u8(self.pfd);
        writer.put_u8(self.pmcr);
        writer.put_u8(self.xreq);
        writer.put_u8(self.last_pe_input);
        writer.put_bool(self.tco0_active);
        self.io.snapshot(writer);
    }

    pub fn restore(&mut self, reader: &mut SnapshotReader) -> Result<(), SnapshotError> {
        for port in [
            &mut self.pa,
            &mut self.pb,
            &mut self.pc,
            &mut self.pd,
            &mut self.pe,
            &mut self.pf,
            &mut self.pl,
        ] {
            port.restore(reader)?;
        }
        self.psc = reader.take_u8()?;
        self.pse = reader.take_u8()?;
        for control in [
            &mut self.pca,
            &mut self.pcb,
            &mut self.pcc,
            &mut self.pcd,
            &mut self.pce,
            &mut self.pcf,
            &mut self.pcl,
        ] {
            *control = reader.take_u8()?;
        }
        self.pfc = reader.take_u8()?;
        self.pfd = reader.take_u8()?;
        self.pmcr = reader.take_u8()?;
        self.xreq = reader.take_u8()?;
        self.last_pe_input = reader.take_u8()?;
        self.tco0_active = reader.take_bool()?;
        self.io.restore(reader)
    }

    fn update_port_gpio(&mut self, port: GpioPort, inputs: &GpioConnections) {
        let (port_reg, pc_reg) = match port {
            GpioPort::PA => (&mut self.pa, self.pca),
            GpioPort::PB => (&mut self.pb, self.pcb),
            GpioPort::PC => (&mut self.pc, self.pcc),
            GpioPort::PD => (&mut self.pd, self.pcd),
            GpioPort::PE => (&mut self.pe, self.pce),
            GpioPort::PF => (&mut self.pf, self.pcf),
            GpioPort::PL => (&mut self.pl, self.pcl),
        };

        // Get input data and connection masks
        let (input_data, connections) = match port {
            GpioPort::PA => (inputs.pa, inputs.pa_connections),
            GpioPort::PB => (inputs.pb, inputs.pb_connections),
            GpioPort::PC => (inputs.pc, inputs.pc_connections),
            GpioPort::PD => (inputs.pd, inputs.pd_connections),
            GpioPort::PE => (inputs.pe, inputs.pe_connections),
            GpioPort::PF => (inputs.pf, inputs.pf_connections),
            GpioPort::PL => (inputs.pl, inputs.pl_connections),
        };

        // Process all input bits
        let input_mask = !pc_reg; // Bits that are inputs
        let connected_bits = connections & input_mask;

        // Clear all input bits
        port_reg.input &= pc_reg; // Keep only output bits

        // Apply connected inputs
        port_reg.input |= input_data & connected_bits;

        // Apply pull resistors to non-connected inputs
        let non_connected_inputs = input_mask & !connections;
        port_reg.input |= port_reg.pull_mask & non_connected_inputs;
    }
}

fn read_px(gpio: &State, port: GpioPort) -> u8 {
    let mut result = 0u8;
    for i in 0..8 {
        let direction = gpio.get_direction(port, i);
        let port_register = match port {
            GpioPort::PA => &gpio.pa,
            GpioPort::PB => &gpio.pb,
            GpioPort::PC => &gpio.pc,
            GpioPort::PD => &gpio.pd,
            GpioPort::PE => &gpio.pe,
            GpioPort::PF => &gpio.pf,
            GpioPort::PL => &gpio.pl,
        };
        match direction {
            PortMode::Input => result |= port_register.input & (1 << i),
            PortMode::Output => result |= port_register.output & (1 << i),
        }
    }
    result
}

fn write_px(gpio: &mut State, port: GpioPort, value: u8) {
    for i in 0..8 {
        let direction = gpio.get_direction(port, i);
        let port_register = match port {
            GpioPort::PA => &mut gpio.pa,
            GpioPort::PB => &mut gpio.pb,
            GpioPort::PC => &mut gpio.pc,
            GpioPort::PD => &mut gpio.pd,
            GpioPort::PE => &mut gpio.pe,
            GpioPort::PF => &mut gpio.pf,
            GpioPort::PL => &mut gpio.pl,
        };

        match direction {
            PortMode::Input => {
                // If input, update pull resistor state
                // Clear the pull bit
                port_register.pull_mask &= !(1 << i);

                // Set the pull bit
                port_register.pull_mask |= value & (1 << i);
            }
            PortMode::Output => {
                // Clear the bit
                port_register.output &= !(1 << i);

                // Set the bit
                port_register.output |= value & (1 << i);
            }
        }
    }
}
pub fn read_pa(gpio: &State) -> u8 {
    read_px(gpio, GpioPort::PA)
}

pub fn read_pb(gpio: &State) -> u8 {
    read_px(gpio, GpioPort::PB)
}

pub fn read_pc(gpio: &State) -> u8 {
    read_px(gpio, GpioPort::PC)
}

pub fn read_pd(gpio: &State) -> u8 {
    read_px(gpio, GpioPort::PD)
}

pub fn read_pe(gpio: &State) -> u8 {
    read_px(gpio, GpioPort::PE)
}

pub fn read_pf(gpio: &State) -> u8 {
    read_px(gpio, GpioPort::PF)
}

pub fn read_pl(gpio: &State) -> u8 {
    read_px(gpio, GpioPort::PL)
}

pub fn read_psc(gpio: &State) -> u8 {
    gpio.psc
}

pub fn read_pse(gpio: &State) -> u8 {
    gpio.pse
}

pub fn read_pca(gpio: &State) -> u8 {
    gpio.pca
}

pub fn read_pcb(gpio: &State) -> u8 {
    gpio.pcb
}

pub fn read_pcc(gpio: &State) -> u8 {
    gpio.pcc
}

pub fn read_pcd(gpio: &State) -> u8 {
    gpio.pcd
}

pub fn read_pce(gpio: &State) -> u8 {
    gpio.pce
}

pub fn read_pcf(gpio: &State) -> u8 {
    gpio.pcf
}

pub fn read_pfc(gpio: &State) -> u8 {
    gpio.pfc
}

pub fn read_pfd(gpio: &State) -> u8 {
    gpio.pfd
}

pub fn read_pmcr(gpio: &State) -> u8 {
    gpio.pmcr
}

pub fn read_xreq(gpio: &State) -> u8 {
    gpio.xreq
}

/// Bits written as 1 clear the corresponding request flags.
pub fn write_xreq(gpio: &mut State, value: u8) {
    gpio.xreq &= !value;
}

pub fn read_pcl(gpio: &State) -> u8 {
    gpio.pcl
}

pub fn write_pa(gpio: &mut State, value: u8) {
    write_px(gpio, GpioPort::PA, value);
}

pub fn write_pb(gpio: &mut State, value: u8) {
    write_px(gpio, GpioPort::PB, value);
}

pub fn write_pc(gpio: &mut State, value: u8) {
    write_px(gpio, GpioPort::PC, value);
}

pub fn write_pd(gpio: &mut State, value: u8) {
    write_px(gpio, GpioPort::PD, value);
}

pub fn write_pe(gpio: &mut State, value: u8) {
    write_px(gpio, GpioPort::PE, value);
}

pub fn write_pf(gpio: &mut State, value: u8) {
    write_px(gpio, GpioPort::PF, value);
}

pub fn write_pl(gpio: &mut State, value: u8) {
    write_px(gpio, GpioPort::PL, value);
}

pub fn write_psc(gpio: &mut State, value: u8) {
    gpio.psc = value;
}

pub fn write_pse(gpio: &mut State, value: u8) {
    gpio.pse = value;
}

pub fn write_pca(gpio: &mut State, value: u8) {
    gpio.pca = value;
}

pub fn write_pcb(gpio: &mut State, value: u8) {
    gpio.pcb = value;
}

pub fn write_pcc(gpio: &mut State, value: u8) {
    gpio.pcc = value;
}

pub fn write_pcd(gpio: &mut State, value: u8) {
    gpio.pcd = value;
}

pub fn write_pce(gpio: &mut State, value: u8) {
    gpio.pce = value;
}

pub fn write_pcf(gpio: &mut State, value: u8) {
    gpio.pcf = value;
}

pub fn write_pfc(gpio: &mut State, value: u8) {
    gpio.pfc = value;
}

pub fn write_pfd(gpio: &mut State, value: u8) {
    gpio.pfd = value;
}

pub fn write_pmcr(gpio: &mut State, value: u8) {
    gpio.pmcr = value;
}

pub fn write_pcl(gpio: &mut State, value: u8) {
    gpio.pcl = value;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    struct StubIo {
        /// Level driven onto PE1, or None to leave it unconnected.
        pe1: Rc<Cell<Option<bool>>>,
        states: Rc<RefCell<Vec<GpioState>>>,
    }

    impl GpioInterfaceInternal for StubIo {
        fn get_inputs(&mut self, _cycle: u64) -> GpioConnections {
            let mut connections = GpioConnections::default();
            if let Some(level) = self.pe1.get() {
                connections.connect(GpioPort::PE, 1, level);
            }
            connections
        }

        fn set_outputs(&mut self, state: GpioState, _cycle: u64) {
            self.states.borrow_mut().push(state);
        }
    }

    type Setup = (State, Rc<Cell<Option<bool>>>, Rc<RefCell<Vec<GpioState>>>);

    fn setup() -> Setup {
        let pe1 = Rc::new(Cell::new(None));
        let states = Rc::new(RefCell::new(Vec::new()));
        let gpio = State::new(Box::new(StubIo {
            pe1: pe1.clone(),
            states: states.clone(),
        }));
        (gpio, pe1, states)
    }

    #[test]
    fn tco0_follows_pmcr_and_timer0() {
        let (mut gpio, _pe1, states) = setup();

        gpio.update(0, true);
        assert!(!states.borrow().last().unwrap().tco0);

        write_pmcr(&mut gpio, 0b0000_0001);
        gpio.update(1, false); // Timer0 stopped
        assert!(!states.borrow().last().unwrap().tco0);

        gpio.update(2, true);
        let state = states.borrow().last().unwrap().clone();
        assert!(state.tco0);
        // The pin reads high while the clock-out drives it.
        assert_ne!(state.pe & 0b01, 0);

        write_pmcr(&mut gpio, 0);
        gpio.update(3, true);
        assert!(!states.borrow().last().unwrap().tco0);
    }

    #[test]
    fn intx1_edge_latches_xreq_and_requests_interrupt() {
        let (mut gpio, pe1, _states) = setup();
        write_pmcr(&mut gpio, 0b0000_0010); // PE1 = INTX1, falling edge

        pe1.set(Some(true));
        assert!(!gpio.update(0, false).intx);
        assert_eq!(read_xreq(&gpio), 0);

        // Falling edge: request latched.
        pe1.set(Some(false));
        assert!(gpio.update(1, false).intx);
        assert_eq!(read_xreq(&gpio), 0b0000_0010);
        assert_eq!(read_pe(&gpio) & 0b10, 0);

        // Rising edge is not the selected polarity.
        write_xreq(&mut gpio, 0xFF);
        pe1.set(Some(true));
        assert!(!gpio.update(2, false).intx);
        assert_eq!(read_xreq(&gpio), 0);

        // With INTEG set, rising edges trigger instead.
        write_pmcr(&mut gpio, 0b0010_0010);
        pe1.set(Some(false));
        assert!(!gpio.update(3, false).intx);
        pe1.set(Some(true));
        assert!(gpio.update(4, false).intx);
        assert_eq!(read_xreq(&gpio), 0b0000_0010);
    }

    #[test]
    fn no_intx_without_pin_function_or_input_direction() {
        let (mut gpio, pe1, _states) = setup();

        // PE1 as plain GPIO: edges do not interrupt.
        pe1.set(Some(true));
        gpio.update(0, false);
        pe1.set(Some(false));
        assert!(!gpio.update(1, false).intx);
        assert_eq!(read_xreq(&gpio), 0);

        // PE1 as INTX1 but configured as an output: still nothing.
        write_pmcr(&mut gpio, 0b0000_0010);
        write_pce(&mut gpio, 0b0000_0010);
        pe1.set(Some(true));
        gpio.update(2, false);
        pe1.set(Some(false));
        assert!(!gpio.update(3, false).intx);
        assert_eq!(read_xreq(&gpio), 0);
    }

    #[test]
    fn xreq_write_one_clears() {
        let (mut gpio, _pe1, _states) = setup();
        gpio.xreq = 0b0000_0111;
        write_xreq(&mut gpio, 0b0000_0010);
        assert_eq!(read_xreq(&gpio), 0b0000_0101);
        write_xreq(&mut gpio, 0b0000_0101);
        assert_eq!(read_xreq(&gpio), 0);
    }

    #[test]
    fn unconnected_pe1_idles_high() {
        let (mut gpio, _pe1, _states) = setup();
        write_pmcr(&mut gpio, 0b0000_0010);
        for cycle in 0..8 {
            assert!(!gpio.update(cycle, false).intx);
        }
        assert_eq!(read_pe(&gpio) & 0b10, 0b10);
    }
}
