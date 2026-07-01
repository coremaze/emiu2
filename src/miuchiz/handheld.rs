use super::{
    sst39vf1681,
    st2205u::{self, GpioInterfaceInternal},
    st7626,
};
use crate::{audio::AudioInterface, memory::AddressSpace, screen::Screen};
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

pub struct HandheldAddressSpace {
    otp: Box<st2205u::Otp>,
    flash: sst39vf1681::Flash,
    lcd: st7626::Lcd,
}

impl HandheldAddressSpace {
    pub fn new(
        otp: &[u8],
        flash: &[u8],
        screen: Box<dyn Screen>,
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

impl AddressSpace for HandheldAddressSpace {
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

    fn code_cache_key_range(&self, address: usize, len: usize) -> Option<usize> {
        // The device selection (address bits 21+) must not change within the
        // range, so a single region lookup covers all of it
        let device_bits = (1 << 21) - 1;
        if (address & device_bits) + len > device_bits + 1 {
            return None;
        }

        match AddressType::parse_machine_addr(address) {
            (AddressType::Video, _) => None,
            // OTP keys sit directly above the flash's key range; the range
            // must not wrap around the OTP mirror
            (AddressType::Otp, otp_addr) => {
                let offset = otp_addr % self.otp.len();
                (offset + len <= self.otp.len()).then(|| sst39vf1681::Flash::len() + offset)
            }
            (AddressType::Flash, flash_addr) => self.flash.code_cache_key_range(flash_addr, len),
        }
    }

    fn take_content_change(&mut self) -> crate::memory::ContentChange {
        // Flash is the only writable code storage
        self.flash.take_content_change()
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

pub struct Handheld {
    pub mcu: st2205u::Mcu<HandheldAddressSpace>,
}

impl Handheld {
    pub fn new(
        otp: &[u8],
        flash: &[u8],
        screen: Box<dyn Screen>,
        io: Box<dyn GpioInterfaceInternal>,
        audio_sender: Box<dyn AudioInterface>,
    ) -> Result<Self, ConfigurationError> {
        let machine_address_space = HandheldAddressSpace::new(otp, flash, screen)?;

        let mcu = Self {
            mcu: st2205u::Mcu::new(SYSTEM_FREQ, machine_address_space, io, audio_sender),
        };

        Ok(mcu)
    }

    pub fn make_flash_dump(&mut self) -> Vec<u8> {
        let start = 1 << 25;
        let size = sst39vf1681::Flash::len();
        self.mcu.read_machine_area(start, size)
    }
}
