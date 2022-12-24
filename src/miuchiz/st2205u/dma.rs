use super::{
    reg::{U16Register, U8Register},
    St2205uAddressSpace,
};
use crate::memory::AddressSpace;

pub struct State {
    dptr: U16Register,
    dbkr: U16Register,
    dcnt: U16Register,
    dsel: U8Register,
    dmod: U8Register,
}

impl State {
    pub fn new() -> Self {
        Self {
            dptr: U16Register::new(0b0000_0000_0000_0000, 0b0111_1111_1111_1111),
            dbkr: U16Register::new(0b0000_0000_0000_0000, 0b1000_0111_1111_1111),
            dcnt: U16Register::new(0b0000_0000_0000_0000, 0b0111_1111_1111_1111),
            dsel: U8Register::new(0b0000_0000, 0b0000_0011),
            dmod: U8Register::new(0b0000_0000, 0b0011_1111),
        }
    }
}

pub fn write_dptrl<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dptrl {val:02X}");
}

pub fn write_dptrh<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dptrh {val:02X}");
}

pub fn write_dbkrl<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dbkrl {val:02X}");
}

pub fn write_dbkrh<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dbkrh {val:02X}");
}

pub fn write_dcntl<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dcntl {val:02X}");
}

pub fn write_dcnth<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dcnth {val:02X}");
}

pub fn write_dsel<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dsel {val:02X}");
}

pub fn write_dmod<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>, val: u8) {
    println!("Write dmod {val:02X}");
}

pub fn read_dptrl<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dptrl");
    0
}

pub fn read_dptrh<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dptrh");
    0
}

pub fn read_dbkrl<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dbkrl");
    0
}

pub fn read_dbkrh<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dbkrh");
    0
}

pub fn read_dcntl<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dcntl");
    0
}

pub fn read_dcnth<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dcnth");
    0
}

pub fn read_dsel<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dsel");
    0
}

pub fn read_dmod<A: AddressSpace>(st2205u: &mut St2205uAddressSpace<A>) -> u8 {
    println!("Read dmod");
    0
}
