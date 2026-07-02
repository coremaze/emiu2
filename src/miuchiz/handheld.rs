use super::{
    ir::IrCircuit,
    sst39vf1681,
    st2205u::{self, GpioInterfaceInternal},
    st7626,
};
use crate::{
    audio::AudioInterface,
    ir::IrInterface,
    memory::AddressSpace,
    screen::Screen,
    state::{StateError, StateReader, StateWriter, MAGIC, VERSION},
};
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

    // The OTP is read-only and reconstructed from its image file, so only
    // the flash and LCD carry state.
    fn save_state(&self, writer: &mut StateWriter) {
        self.flash.save_state(writer);
        self.lcd.save_state(writer);
    }

    fn load_state(&mut self, reader: &mut StateReader) -> Result<(), StateError> {
        self.flash.load_state(reader)?;
        self.lcd.load_state(reader)
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
    pub mcu: st2205u::Mcu,
}

impl Handheld {
    pub fn new(
        otp: &[u8],
        flash: &[u8],
        screen: Box<dyn Screen>,
        io: Box<dyn GpioInterfaceInternal>,
        audio_sender: Box<dyn AudioInterface>,
        ir_transceiver: Box<dyn IrInterface>,
    ) -> Result<Self, ConfigurationError> {
        let machine_address_space = Box::new(HandheldAddressSpace::new(otp, flash, screen)?);

        // The PCB's IR circuitry sits between the chip's ports and the
        // rest of the board I/O.
        let io = Box::new(IrCircuit::new(io, ir_transceiver, SYSTEM_FREQ));

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

    /// Serializes the complete machine state. The result is self-contained
    /// (it includes the flash) but assumes the same OTP image on restore.
    pub fn save_state(&self) -> Vec<u8> {
        self.save_state_reusing(Vec::new())
    }

    /// Like `save_state`, but reuses an existing buffer's allocation.
    /// Useful when snapshotting frequently (e.g. IR rollback).
    pub fn save_state_reusing(&self, buf: Vec<u8>) -> Vec<u8> {
        let mut writer = StateWriter::from_vec(buf);
        writer.put_bytes(MAGIC);
        writer.put_u16(VERSION);
        self.mcu.save_state(&mut writer);
        writer.into_bytes()
    }

    /// Restores state captured by `save_state`, in place. Host connections
    /// (screen, audio, buttons, IR transport) are unaffected.
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), StateError> {
        let mut reader = StateReader::new(data);
        if reader.take_bytes(MAGIC.len())? != MAGIC {
            return Err(StateError::BadMagic);
        }
        let version = reader.take_u16()?;
        if version != VERSION {
            return Err(StateError::UnsupportedVersion(version));
        }
        self.mcu.load_state(&mut reader)?;
        reader.finish()
    }
}
