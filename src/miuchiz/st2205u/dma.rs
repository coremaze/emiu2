use super::{
    reg::{U16Register, U8Register},
    St2205uAddressSpace,
};
use crate::memory::AddressSpace;

// DMA channels and function modes are not implemented yet.

pub struct State {
    /// DMA Pointer Register (DSEL = 0)
    src_dptr: U16Register,

    /// DMA Pointer Register (DSEL = 1)
    dest_dptr: U16Register,

    /// DMA Bank Register (DSEL = 0)
    src_dbkr: U16Register,

    /// DMA Bank Register (DSEL = 1)
    dest_dbkr: U16Register,

    /// DMA Length Register
    dcnt: U16Register,

    /// DMA Register Select bits
    dsel: U8Register,

    /// DMA Mode Selection Register
    dmod: U8Register,
}

enum PointerSelection {
    Source,
    Destination,
}

enum Mode {
    /// Pointer continues when next DMA starts
    Continue,
    /// Pointer restores its original value when next DMA starts
    Reload,
    /// Pointer is fixed
    Fixed,
}

impl State {
    fn get_ptr_selection(&self) -> PointerSelection {
        if self.dsel.get() & 0b01 == 0 {
            PointerSelection::Source
        } else {
            PointerSelection::Destination
        }
    }

    fn get_src_mode(&self) -> Mode {
        let src_mode_bits = self.dmod.get() & 0b11;
        Self::get_mode(src_mode_bits)
    }

    fn get_dest_mode(&self) -> Mode {
        let dest_mode_bits = (self.dmod.get() & 0b1100) >> 2;
        Self::get_mode(dest_mode_bits)
    }

    fn get_mode(mode_bits: u8) -> Mode {
        match mode_bits {
            0b00 => Mode::Continue,
            0b01 => Mode::Reload,
            0b10 | 0b11 => Mode::Fixed,
            _ => unreachable!("All 2 bit possibilities have been handled"),
        }
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            src_dptr: U16Register::new(0b0000_0000_0000_0000, 0b0111_1111_1111_1111),
            dest_dptr: U16Register::new(0b0000_0000_0000_0000, 0b0111_1111_1111_1111),
            src_dbkr: U16Register::new(0b0000_0000_0000_0000, 0b1000_0111_1111_1111),
            dest_dbkr: U16Register::new(0b0000_0000_0000_0000, 0b1000_0111_1111_1111),
            dcnt: U16Register::new(0b0000_0000_0000_0000, 0b0111_1111_1111_1111),
            dsel: U8Register::new(0b0000_0000, 0b0000_0011),
            dmod: U8Register::new(0b0000_0000, 0b0011_1111),
        }
    }
}

pub fn write_dptrl<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dptrl {val:02X}");
    let dma = &mut st2205u.dma;
    match dma.get_ptr_selection() {
        PointerSelection::Source => dma.src_dptr.set_l(val),
        PointerSelection::Destination => dma.dest_dptr.set_l(val),
    }
}

pub fn write_dptrh<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dptrh {val:02X}");
    let dma = &mut st2205u.dma;
    match dma.get_ptr_selection() {
        PointerSelection::Source => dma.src_dptr.set_h(val),
        PointerSelection::Destination => dma.dest_dptr.set_h(val),
    }
}

pub fn write_dbkrl<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dbkrl {val:02X}");
    let dma = &mut st2205u.dma;
    match dma.get_ptr_selection() {
        PointerSelection::Source => dma.src_dbkr.set_l(val),
        PointerSelection::Destination => dma.dest_dbkr.set_l(val),
    }
}

pub fn write_dbkrh<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dbkrh {val:02X}");
    let dma = &mut st2205u.dma;
    match dma.get_ptr_selection() {
        PointerSelection::Source => dma.src_dbkr.set_h(val),
        PointerSelection::Destination => dma.dest_dbkr.set_h(val),
    }
}

pub fn write_dcntl<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dcntl {val:02X}");
    st2205u.dma.dcnt.set_l(val);
}

pub fn write_dcnth<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dcnth {val:02X}");
    st2205u.dma.dcnt.set_h(val);
    execute_dma(st2205u);
}

pub fn write_dsel<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dsel {val:02X}");
    st2205u.dma.dsel.set(val);
}

pub fn write_dmod<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dmod {val:02X}");
    st2205u.dma.dmod.set(val);
}

pub fn read_dptrl<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dptrl");
    let dma = &mut st2205u.dma;
    match dma.get_ptr_selection() {
        PointerSelection::Source => dma.src_dptr.l(),
        PointerSelection::Destination => dma.dest_dptr.l(),
    }
}

pub fn read_dptrh<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dptrh");
    let dma = &mut st2205u.dma;
    match dma.get_ptr_selection() {
        PointerSelection::Source => dma.src_dptr.h(),
        PointerSelection::Destination => dma.dest_dptr.h(),
    }
}

pub fn read_dbkrl<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dbkrl");
    let dma = &mut st2205u.dma;
    match dma.get_ptr_selection() {
        PointerSelection::Source => dma.src_dbkr.l(),
        PointerSelection::Destination => dma.dest_dbkr.l(),
    }
}

pub fn read_dbkrh<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dbkrh");
    let dma = &mut st2205u.dma;
    match dma.get_ptr_selection() {
        PointerSelection::Source => dma.src_dbkr.h(),
        PointerSelection::Destination => dma.dest_dbkr.h(),
    }
}

pub fn read_dcntl<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dcntl");
    st2205u.dma.dcnt.l()
}

pub fn read_dcnth<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dcnth");
    st2205u.dma.dcnt.h()
}

pub fn read_dsel<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dsel");
    st2205u.dma.dsel.get()
}

pub fn read_dmod<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dmod");
    st2205u.dma.dmod.get()
}

fn execute_dma<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) {
    // Must be restored at end
    let original_drr = st2205u.drr.clone();

    // Can be restored at end if reload mode
    let original_src_dptr = st2205u.dma.src_dptr.clone();

    // Can be restored at end if reload mode
    let original_dest_dptr = st2205u.dma.dest_dptr.clone();

    for _ in 0..st2205u.dma.dcnt.u16() {
        st2205u.drr = st2205u.dma.src_dbkr.clone(); // Switch to src bank
        let src_ptr = st2205u.dma.src_dptr.u16() | (1 << 15); // Get src ptr
        let src_byte = st2205u.read_u8(src_ptr as usize); // Read src byte

        st2205u.drr = st2205u.dma.dest_dbkr.clone(); // Switch to dest bank
        let dest_ptr = st2205u.dma.dest_dptr.u16() | (1 << 15); // Get dest ptr
        st2205u.write_u8(dest_ptr as usize, src_byte); // Write dest byte

        // Increment src ptr if applicable
        match st2205u.dma.get_src_mode() {
            Mode::Continue | Mode::Reload => st2205u.dma.src_dptr.set_u16(src_ptr.wrapping_add(1)),
            Mode::Fixed => { /* Do nothing, pointer is fixed */ }
        }

        // Increment dest ptr if applicable
        match st2205u.dma.get_dest_mode() {
            Mode::Continue | Mode::Reload => {
                st2205u.dma.dest_dptr.set_u16(dest_ptr.wrapping_add(1))
            }
            Mode::Fixed => { /* Do nothing, pointer is fixed */ }
        }
    }

    println!(
        "Move {} bytes from DRR {:04X} addr {:04X} to DRR {:04X} addr {:04X}",
        st2205u.dma.dcnt.u16(),
        st2205u.dma.src_dbkr.u16(),
        st2205u.dma.src_dptr.u16() | 0x8000,
        st2205u.dma.dest_dbkr.u16(),
        st2205u.dma.dest_dptr.u16() | 0x8000
    );

    // Restore src ptr if in reload mode
    if let Mode::Reload = st2205u.dma.get_src_mode() {
        st2205u.dma.src_dptr = original_src_dptr;
    }

    // Restore dest ptr if in reload mode
    if let Mode::Reload = st2205u.dma.get_dest_mode() {
        st2205u.dma.dest_dptr = original_dest_dptr;
    }

    // Restore original DRR bank register
    st2205u.drr = original_drr;
}
