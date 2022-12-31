use super::clock::Clock;
use super::interrupt::Interrupt;
use super::vector;
use super::wdc_65c02;
use super::wdc_65c02::HandlesInterrupt;
use super::St2205uAddressSpace;
use crate::gpio::Gpio;
use crate::memory::AddressSpace;

/// Representation of a ST2205U microcontroller.
///
/// This microcontroller is capable of, through the use of bank registers,
/// accessing a larger address space than the 65C02 core itself can, represented
/// by `A`.
///
/// This device also implements its own address space, which is addressible using
/// 16 bits, which is directly exposed to the underlying 65C02.
pub struct Mcu<'a, A: AddressSpace> {
    pub core: wdc_65c02::Core<St2205uAddressSpace<'a, A>>,
}

impl<'a, A: AddressSpace> Mcu<'a, A> {
    pub fn new(frequency: u64, address_space: A, io: &'a impl Gpio) -> Self {
        let mut mcu = Self {
            core: wdc_65c02::Core::new(
                frequency,
                St2205uAddressSpace::new(address_space, io, frequency),
            ),
        };

        mcu.reset();

        mcu
    }

    pub fn step(&mut self) {
        self.core.step();
        self.core
            .address_space
            .set_clocks(self.core.oscillator_cycles());
        let bt_int = self.core.address_space.base_timer.update();

        if !self.core.flags.interrupt_disable {
            if bt_int {
                // println!("Starting interrupt");
                self.core
                    .address_space
                    .interrupt
                    .assert_interrupt(Interrupt::BaseTimer);
                self.core.address_space.set_interrupted(true);
                self.core.push_u16(self.core.registers.pc);
                self.core.push_u8(self.core.flags.to_u8());
                self.core.registers.pc = self.core.address_space.read_u16_le(vector::BT.into());
            }
        }
    }

    pub fn reset(&mut self) {
        self.core.set_interrupted(true);
        let reset_vector = self.core.address_space.read_u16_le(vector::RESET.into());
        self.core.registers.pc = reset_vector;
        self.core.set_interrupted(false);
    }
}
