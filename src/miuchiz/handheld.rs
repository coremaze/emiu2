use super::{sst39vf1681, st2205u, st7626};
use crate::{display::Screen, gpio::Gpio, memory::AddressSpace};
use std::fmt::Display;

pub const SYSTEM_FREQ: u64 = 16_000_000;

#[derive(Debug)]
enum AddressType {
    Video,
    Otp,
    Flash,
}

impl AddressType {
    pub fn parse_machine_addr(address: usize) -> (Self, usize) {
        let selection_bits = (address >> 21) & 0b00011111;
        let address_bits = address & ((1 << 21) - 1);

        let addr_type = match selection_bits {
            0b00011 => AddressType::Video,
            0b00000 | 0b11111 => AddressType::Otp,
            _ => AddressType::Flash,
        };

        (addr_type, address_bits)
    }
}

pub struct HandheldAddressSpace<'a> {
    otp: Box<st2205u::Otp>,
    flash: sst39vf1681::Flash,
    lcd: st7626::Lcd<'a>,
}

impl<'a, 'b> HandheldAddressSpace<'a> {
    pub fn new(
        otp: &[u8],
        flash: &[u8],
        screen: &'a impl Screen,
    ) -> Result<Self, ConfigurationError> {
        let otp_box = Box::new(
            st2205u::Otp::try_from(otp)
                .map_err(|_| ConfigurationError::InvalidOtpSize(otp.len()))?,
        );

        let flash = sst39vf1681::Flash::new(flash)
            .map_err(|err| ConfigurationError::InvalidFlashSize(flash.len()))?;

        let lcd = st7626::Lcd::new(screen);

        Ok(Self {
            otp: otp_box,
            flash,
            lcd,
        })
    }
}

impl<'a, 'b> AddressSpace for HandheldAddressSpace<'a> {
    fn read_u8(&mut self, address: usize) -> u8 {
        // println!("Read {address:X}");
        match AddressType::parse_machine_addr(address) {
            (AddressType::Video, vid_addr) => self.lcd.read_u8(vid_addr),
            (AddressType::Otp, otp_addr) => self.otp[otp_addr % self.otp.len()],
            (AddressType::Flash, flash_addr) => self.flash.read_u8(flash_addr),
        }
    }

    fn write_u8(&mut self, address: usize, value: u8) {
        match AddressType::parse_machine_addr(address) {
            (AddressType::Video, vid_addr) => self.lcd.write_u8(vid_addr, value),
            (AddressType::Otp, otp_addr) => println!("Attempt to write to OTP addr {otp_addr:X}"),
            (AddressType::Flash, flash_addr) => self.flash.write_u8(flash_addr, value),
        }
    }
}

#[derive(Debug)]
pub enum ConfigurationError {
    InvalidOtpSize(usize),
    InvalidFlashSize(usize),
}

impl Display for ConfigurationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&match &self {
            ConfigurationError::InvalidOtpSize(size) => format!(
                "The OTP is invalid because it is {size} bytes, but must be {} bytes",
                st2205u::OTP_SIZE
            ),
            ConfigurationError::InvalidFlashSize(size) => format!(
                "The flash is invalid because it is {size} bytes, but must be {} bytes",
                sst39vf1681::Flash::len()
            ),
        })
    }
}

pub struct Handheld<'a, 'b> {
    pub mcu: st2205u::Mcu<'b, HandheldAddressSpace<'a>>,
}

impl<'a, 'b> Handheld<'a, 'b> {
    pub fn new(
        otp: &[u8],
        flash: &[u8],
        screen: &'a impl Screen,
        io: &'b impl Gpio,
    ) -> Result<Self, ConfigurationError> {
        let machine_address_space = HandheldAddressSpace::new(otp, flash, screen)?;

        let mcu = Self {
            mcu: st2205u::Mcu::new(SYSTEM_FREQ, machine_address_space, io),
        };

        Ok(mcu)
    }
}
