use super::vector;
use super::wdc_65c02;
use super::St2205uAddressSpace;
use crate::memory::AddressSpace;

/// Representation of a ST2205U microcontroller.
///
/// This microcontroller is capable of, through the use of bank registers,
/// accessing a larger address space than the 65C02 core itself can, represented
/// by `A`.
///
/// This device also implements its own address space, which is addressible using
/// 16 bits, which is directly exposed to the underlying 65C02.
pub struct Mcu<A: AddressSpace> {
    pub core: wdc_65c02::Core<St2205uAddressSpace<A>>,
}

impl<A: AddressSpace> Mcu<A> {
    pub fn new(address_space: A) -> Self {
        let mut mcu = Self {
            core: wdc_65c02::Core::new(St2205uAddressSpace::new(address_space)),
        };

        mcu.reset();

        mcu
    }

    pub fn step(&mut self) {
        self.core.step();
    }

    pub fn reset(&mut self) {
        let reset_vector = self.core.address_space.read_u16_le(vector::RESET.into());
        self.core.registers.pc = reset_vector;
    }
}
