use super::{st2205u, st7626};
use crate::memory::AddressSpace;
use std::error::Error;

pub type Flash = [u8; 0x200000];

#[derive(Debug)]
enum AddressType {
    Video,
    Otp,
    Flash,
}

impl AddressType {
    pub fn parse_machine_addr(address: usize) -> (Self, usize) {
        let selection_bits = address >> 21;
        let address_bits = address & ((1 << 21) - 1);

        let addr_type = match selection_bits {
            0b00011 => AddressType::Video,
            0b00000 | 0b11111 => AddressType::Otp,
            _ => AddressType::Flash,
        };

        (addr_type, address_bits)
    }
}

pub struct HandheldAddressSpace {
    otp: Box<st2205u::Otp>,
    flash: Box<Flash>,
    lcd: st7626::Lcd,
}

impl HandheldAddressSpace {
    pub fn new(otp: &[u8], flash: &[u8]) -> Result<Self, ConfigurationError> {
        let otp_box = Box::new(
            st2205u::Otp::try_from(otp)
                .map_err(|err| ConfigurationError::InvalidOtp(err.into()))?,
        );

        let flash_box = Box::new(
            Flash::try_from(flash).map_err(|err| ConfigurationError::InvalidFlash(err.into()))?,
        );

        let lcd = st7626::Lcd::new();

        Ok(Self {
            otp: otp_box,
            flash: flash_box,
            lcd,
        })
    }
}

impl AddressSpace for HandheldAddressSpace {
    fn read_u8(&mut self, address: usize) -> u8 {
        match AddressType::parse_machine_addr(address) {
            (AddressType::Video, vid_addr) => self.lcd.read_u8(vid_addr),
            (AddressType::Otp, otp_addr) => self.otp[otp_addr % self.otp.len()],
            (AddressType::Flash, flash_addr) => self.flash[flash_addr % self.flash.len()],
        }
    }

    fn write_u8(&mut self, address: usize, value: u8) {
        match AddressType::parse_machine_addr(address) {
            (AddressType::Video, vid_addr) => self.lcd.write_u8(vid_addr, value),
            (AddressType::Otp, _otp_addr) => {
                todo!("Write OTP???")
            }
            (AddressType::Flash, _flash_addr) => {
                todo!("Write Flash")
            }
        }
    }
}

#[derive(Debug)]
pub enum ConfigurationError {
    InvalidOtp(Box<dyn Error>),
    InvalidFlash(Box<dyn Error>),
}

pub struct Handheld {
    pub mcu: st2205u::Mcu<HandheldAddressSpace>,
}

impl Handheld {
    pub fn new(otp: &[u8], flash: &[u8]) -> Result<Self, ConfigurationError> {
        let machine_address_space = HandheldAddressSpace::new(otp, flash)?;

        let mcu = Self {
            mcu: st2205u::Mcu::new(machine_address_space),
        };

        Ok(mcu)
    }
}
