use super::reg::U8Register;

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
}

pub trait GpioInterfaceInternal {
    fn get_inputs(&mut self) -> GpioConnections;
    fn set_outputs(&mut self, state: GpioState);
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

    /// Updates the GPIO inputs and returns true if a port a transition occurred
    pub fn update_gpio_and_detect_pa_transition(&mut self) -> bool {
        let inputs = self.io.get_inputs();

        let old_pa = self.pa.clone();

        for port in [
            GpioPort::PA,
            GpioPort::PB,
            GpioPort::PC,
            GpioPort::PD,
            GpioPort::PE,
            GpioPort::PF,
            GpioPort::PL,
        ] {
            for bit in 0..8 {
                let direction = self.get_direction(port, bit);
                let (internal_port, input_port) = match port {
                    GpioPort::PA => (&mut self.pa, &inputs.pa),
                    GpioPort::PB => (&mut self.pb, &inputs.pb),
                    GpioPort::PC => (&mut self.pc, &inputs.pc),
                    GpioPort::PD => (&mut self.pd, &inputs.pd),
                    GpioPort::PE => (&mut self.pe, &inputs.pe),
                    GpioPort::PF => (&mut self.pf, &inputs.pf),
                    GpioPort::PL => (&mut self.pl, &inputs.pl),
                };
                match direction {
                    PortMode::Input => {
                        let input_bit = (input_port & (1 << bit)) != 0;
                        let connected = (inputs.pa_connections & (1 << bit)) != 0;

                        // Clear the bit
                        internal_port.input = internal_port.input & !(1 << bit);
                        if connected {
                            // Set the bit to the input value
                            internal_port.input |= (input_bit as u8) << bit;
                        } else {
                            // If not connected, use the pull state
                            // Pull up = 1, pull down = 0
                            if internal_port.pull_mask & (1 << bit) == 0 {
                                internal_port.input &= !(1 << bit);
                            } else {
                                internal_port.input |= 1 << bit;
                            }
                        }
                    }
                    PortMode::Output => {}
                }
            }
        }

        let pa_updated = old_pa.input != self.pa.input;

        self.io.set_outputs(GpioState {
            pa: (self.pa.output & self.pca) | (self.pa.input & !self.pca),
            pb: (self.pb.output & self.pcb) | (self.pb.input & !self.pcb),
            pc: (self.pc.output & self.pcc) | (self.pc.input & !self.pcc),
            pd: (self.pd.output & self.pcd) | (self.pd.input & !self.pcd),
            pe: (self.pe.output & self.pce) | (self.pe.input & !self.pce),
            pf: (self.pf.output & self.pcf) | (self.pf.input & !self.pcf),
            pl: (self.pl.output & self.pcl) | (self.pl.input & !self.pcl),
        });

        pa_updated
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
