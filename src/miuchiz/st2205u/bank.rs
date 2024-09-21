use crate::{gpio::GpioInterface, memory::AddressSpace};

use super::{reg::U16Register, St2205uAddressSpace};

pub struct State {
    brr: U16Register,
    prr: U16Register,
    irr: U16Register,
    drr: U16Register,
}

impl State {
    pub fn new() -> Self {
        Self {
            // BRR is the only bank register with a nonzero default value
            brr: U16Register::new(0b1000_0000_0000_0000, 0b1001_1111_1111_1111),
            prr: U16Register::new(0b0000_0000_0000_0000, 0b1000_1111_1111_1111),
            irr: U16Register::new(0b0000_0000_0000_0000, 0b1000_1111_1111_1111),
            drr: U16Register::new(0b0000_0000_0000_0000, 0b1000_0111_1111_1111),
        }
    }
}

// The bits which are not used are always read as 1

pub fn read_brrl(st2205u: &St2205uAddressSpace) -> u8 {
    st2205u.banks.brr.l() | !st2205u.banks.brr.l_mask()
}

pub fn read_brrh(st2205u: &St2205uAddressSpace) -> u8 {
    st2205u.banks.brr.h() | !st2205u.banks.brr.h_mask()
}

pub fn read_prrl(st2205u: &St2205uAddressSpace) -> u8 {
    st2205u.banks.prr.l() | !st2205u.banks.prr.l_mask()
}

pub fn read_prrh(st2205u: &St2205uAddressSpace) -> u8 {
    st2205u.banks.prr.h() | !st2205u.banks.prr.h_mask()
}

pub fn read_irrl(st2205u: &St2205uAddressSpace) -> u8 {
    st2205u.banks.irr.l() | !st2205u.banks.irr.l_mask()
}

pub fn read_irrh(st2205u: &St2205uAddressSpace) -> u8 {
    st2205u.banks.irr.h() | !st2205u.banks.irr.h_mask()
}

pub fn read_drrl(st2205u: &St2205uAddressSpace) -> u8 {
    st2205u.banks.drr.l() | !st2205u.banks.drr.l_mask()
}

pub fn read_drrh(st2205u: &St2205uAddressSpace) -> u8 {
    st2205u.banks.drr.h() | !st2205u.banks.drr.h_mask()
}

pub fn write_brrl(st2205u: &mut St2205uAddressSpace, value: u8) {
    st2205u.banks.brr.set_l(value)
}

pub fn write_brrh(st2205u: &mut St2205uAddressSpace, value: u8) {
    st2205u.banks.brr.set_h(value)
}

pub fn write_prrl(st2205u: &mut St2205uAddressSpace, value: u8) {
    st2205u.banks.prr.set_l(value)
}

pub fn write_prrh(st2205u: &mut St2205uAddressSpace, value: u8) {
    st2205u.banks.prr.set_h(value)
}

pub fn write_irrl(st2205u: &mut St2205uAddressSpace, value: u8) {
    st2205u.banks.irr.set_l(value)
}

pub fn write_irrh(st2205u: &mut St2205uAddressSpace, value: u8) {
    st2205u.banks.irr.set_h(value)
}

pub fn write_drrl(st2205u: &mut St2205uAddressSpace, value: u8) {
    st2205u.banks.drr.set_l(value)
}

pub fn write_drrh(st2205u: &mut St2205uAddressSpace, value: u8) {
    st2205u.banks.drr.set_h(value)
}

pub fn brr(st2205u: &St2205uAddressSpace) -> u16 {
    (read_brrl(st2205u) as u16) | ((read_brrh(st2205u) as u16) << 8)
}

pub fn set_brr(st2205u: &mut St2205uAddressSpace, value: u16) {
    st2205u.banks.brr.set_u16(value)
}

pub fn prr(st2205u: &St2205uAddressSpace) -> u16 {
    (read_prrl(st2205u) as u16) | ((read_prrh(st2205u) as u16) << 8)
}

pub fn set_prr(st2205u: &mut St2205uAddressSpace, value: u16) {
    st2205u.banks.prr.set_u16(value)
}

pub fn drr(st2205u: &St2205uAddressSpace) -> u16 {
    (read_drrl(st2205u) as u16) | ((read_drrh(st2205u) as u16) << 8)
}

pub fn set_drr(st2205u: &mut St2205uAddressSpace, value: u16) {
    st2205u.banks.drr.set_u16(value)
}

pub fn irr(st2205u: &St2205uAddressSpace) -> u16 {
    (read_irrl(st2205u) as u16) | ((read_irrh(st2205u) as u16) << 8)
}

pub fn set_irr(st2205u: &mut St2205uAddressSpace, value: u16) {
    st2205u.banks.irr.set_u16(value)
}
