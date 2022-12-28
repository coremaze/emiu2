use super::reg::U8Register;
use crate::gpio::Gpio;

pub enum Port {
    A,
    B,
    C,
    D,
    E,
    F,
    L,
}

enum PortMode {
    Input,
    Output,
}

pub struct State<'a> {
    // Port data registers
    pa: U8Register,
    pb: U8Register,
    pc: U8Register,
    pd: U8Register,
    pe: U8Register,
    pf: U8Register,
    pl: U8Register,

    // Port type select registers
    psc: U8Register,
    pse: U8Register,

    // Port direction control registers
    pca: U8Register,
    pcb: U8Register,
    pcc: U8Register,
    pcd: U8Register,
    pce: U8Register,
    pcf: U8Register,
    pcl: U8Register,

    // Port function select registers
    pfc: U8Register,
    pfd: U8Register,

    // Port miscellaneous control register
    pmcr: U8Register,

    io: &'a dyn Gpio,
}

impl<'a> State<'a> {
    pub fn new(io: &'a impl Gpio) -> Self {
        Self {
            pa: U8Register::new(0b1111_1111, 0b1111_1111),
            pb: U8Register::new(0b1111_1111, 0b1111_1111),
            pc: U8Register::new(0b1111_1111, 0b1111_1111),
            pd: U8Register::new(0b1111_1111, 0b1111_1111),
            pe: U8Register::new(0b1111_1111, 0b1111_1111),
            pf: U8Register::new(0b1111_1111, 0b1111_1111),
            pl: U8Register::new(0b1111_1111, 0b1111_1111),

            psc: U8Register::new(0b1111_1111, 0b1111_1111),
            pse: U8Register::new(0b1111_1111, 0b1111_1111),

            pca: U8Register::new(0b0000_0000, 0b1111_1111),
            pcb: U8Register::new(0b0000_0000, 0b1111_1111),
            pcc: U8Register::new(0b0000_0000, 0b1111_1111),
            pcd: U8Register::new(0b0000_0000, 0b1111_1111),
            pce: U8Register::new(0b0000_0000, 0b1111_1111),
            pcf: U8Register::new(0b0000_0000, 0b1111_1111),
            pcl: U8Register::new(0b0000_0000, 0b1111_1111),

            pfc: U8Register::new(0b0000_0000, 0b1111_1111),
            pfd: U8Register::new(0b0000_0000, 0b1111_1110),
            pmcr: U8Register::new(0b1000_0000, 0b1111_1111),

            io,
        }
    }
}

pub fn read_pa(gpio: &State) -> u8 {
    let mut result = 0u8;
    for i in 0..u8::BITS {
        result |= (gpio.io.get_input(0 * u8::BITS + i) as u8) << i;
    }
    !result
}

pub fn read_pb(gpio: &State) -> u8 {
    let mut result = 0u8;
    for i in 0..u8::BITS {
        result |= (gpio.io.get_input(1 * u8::BITS + i) as u8) << i;
    }
    !result
}

pub fn read_pc(gpio: &State) -> u8 {
    gpio.pc.get()
}

pub fn read_pd(gpio: &State) -> u8 {
    gpio.pd.get()
}

pub fn read_pe(gpio: &State) -> u8 {
    gpio.pe.get()
}

pub fn read_pf(gpio: &State) -> u8 {
    gpio.pf.get()
}

pub fn read_psc(gpio: &State) -> u8 {
    gpio.psc.get()
}

pub fn read_pse(gpio: &State) -> u8 {
    gpio.pse.get()
}

pub fn read_pca(gpio: &State) -> u8 {
    gpio.pca.get()
}

pub fn read_pcb(gpio: &State) -> u8 {
    gpio.pcb.get()
}

pub fn read_pcc(gpio: &State) -> u8 {
    gpio.pcc.get()
}

pub fn read_pcd(gpio: &State) -> u8 {
    gpio.pcd.get()
}

pub fn read_pce(gpio: &State) -> u8 {
    gpio.pce.get()
}

pub fn read_pcf(gpio: &State) -> u8 {
    gpio.pcf.get()
}

pub fn read_pfc(gpio: &State) -> u8 {
    gpio.pfc.get()
}

pub fn read_pfd(gpio: &State) -> u8 {
    gpio.pfd.get()
}

pub fn read_pmcr(gpio: &State) -> u8 {
    gpio.pmcr.get()
}

pub fn read_pl(gpio: &State) -> u8 {
    gpio.pl.get()
}

pub fn read_pcl(gpio: &State) -> u8 {
    gpio.pcl.get()
}

pub fn write_pa(gpio: &mut State, value: u8) {
    println!("Unimplemented write {value:02X} to PA");
}

pub fn write_pb(gpio: &mut State, value: u8) {
    println!("Unimplemented write {value:02X} to PB");
}

pub fn write_pc(gpio: &mut State, value: u8) {
    gpio.pc.set(value);
}

pub fn write_pd(gpio: &mut State, value: u8) {
    gpio.pd.set(value);
}

pub fn write_pe(gpio: &mut State, value: u8) {
    gpio.pe.set(value);
}

pub fn write_pf(gpio: &mut State, value: u8) {
    gpio.pf.set(value);
}

pub fn write_psc(gpio: &mut State, value: u8) {
    gpio.psc.set(value);
}

pub fn write_pse(gpio: &mut State, value: u8) {
    gpio.pse.set(value);
}

pub fn write_pca(gpio: &mut State, value: u8) {
    gpio.pca.set(value);
}

pub fn write_pcb(gpio: &mut State, value: u8) {
    gpio.pcb.set(value);
}

pub fn write_pcc(gpio: &mut State, value: u8) {
    gpio.pcc.set(value);
}

pub fn write_pcd(gpio: &mut State, value: u8) {
    gpio.pcd.set(value);
}

pub fn write_pce(gpio: &mut State, value: u8) {
    gpio.pce.set(value);
}

pub fn write_pcf(gpio: &mut State, value: u8) {
    gpio.pcf.set(value);
}

pub fn write_pfc(gpio: &mut State, value: u8) {
    gpio.pfc.set(value);
}

pub fn write_pfd(gpio: &mut State, value: u8) {
    gpio.pfd.set(value);
}

pub fn write_pmcr(gpio: &mut State, value: u8) {
    gpio.pmcr.set(value);
}

pub fn write_pl(gpio: &mut State, value: u8) {
    println!("Unimplemented write {value:02X} to PL");
}

pub fn write_pcl(gpio: &mut State, value: u8) {
    gpio.pcl.set(value);
}
