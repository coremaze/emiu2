use std::error::Error;

use super::super::memory::AddressSpace;
use super::addr_space::{Flash, Otp};
use super::vector;
use super::wdc_65c02;
use super::St2205uAddressSpace;

pub struct St2205u {
    pub core: wdc_65c02::Core<St2205uAddressSpace>,
}

#[derive(Debug)]
pub enum McuError {
    InvalidOtp(Box<dyn Error>),
    InvalidFlash(Box<dyn Error>),
}

impl St2205u {
    pub fn new(otp: &[u8], flash: &[u8]) -> Result<Self, McuError> {
        let otp_box = Box::new(Otp::try_from(otp).map_err(|err| McuError::InvalidOtp(err.into()))?);

        let flash_box =
            Box::new(Flash::try_from(flash).map_err(|err| McuError::InvalidFlash(err.into()))?);

        let mut mcu = Self {
            core: wdc_65c02::Core::new(St2205uAddressSpace::new(otp_box, flash_box)),
        };

        mcu.reset();

        Ok(mcu)
    }

    pub fn step(&mut self) {
        self.core.step();
    }

    pub fn reset(&mut self) {
        let reset_vector = self.core.address_space.read_u16_le(vector::RESET.into());
        self.core.registers.pc = reset_vector;
    }
}
