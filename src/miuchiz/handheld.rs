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
    snapshot::{SnapshotError, SnapshotReader, SnapshotWriter, MAGIC, VERSION},
    usb_interface::UsbInterfaceInternal,
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

    // The OTP is read-only, but it is included anyway: it is tiny next
    // to the flash, and carrying it makes a savestate immune to the OTP
    // file going missing or changing between save and load. Savestates
    // are how players will usually stop playing (the firmware only
    // persists to flash when the device sleeps), so they must not
    // depend on anything external.
    fn snapshot(&self, writer: &mut SnapshotWriter) {
        writer.put_bytes(&self.otp[..]);
        self.flash.snapshot(writer);
        self.lcd.snapshot(writer);
    }

    fn restore(&mut self, reader: &mut SnapshotReader) -> Result<(), SnapshotError> {
        reader.take_into(&mut self.otp[..])?;
        self.flash.restore(reader)?;
        self.lcd.restore(reader)
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
        usb_interface: Box<dyn UsbInterfaceInternal>,
    ) -> Result<Self, ConfigurationError> {
        let machine_address_space = Box::new(HandheldAddressSpace::new(otp, flash, screen)?);

        // The PCB's IR circuitry sits between the chip's ports and the
        // rest of the board I/O.
        let io = Box::new(IrCircuit::new(io, ir_transceiver, SYSTEM_FREQ));

        let mcu = Self {
            mcu: st2205u::Mcu::new(
                SYSTEM_FREQ,
                machine_address_space,
                io,
                audio_sender,
                usb_interface,
            ),
        };

        Ok(mcu)
    }

    pub fn make_flash_dump(&mut self) -> Vec<u8> {
        let start = 1 << 25;
        let size = sst39vf1681::Flash::len();
        self.mcu.read_machine_area(start, size)
    }

    /// Serializes the complete machine state. The result is fully
    /// self-contained, including the OTP and flash contents.
    pub fn snapshot(&self) -> Vec<u8> {
        self.snapshot_reusing(Vec::new())
    }

    /// Like `snapshot`, but reuses an existing buffer's allocation.
    /// Useful when snapshotting frequently (e.g. IR rollback).
    pub fn snapshot_reusing(&self, buf: Vec<u8>) -> Vec<u8> {
        let mut writer = SnapshotWriter::from_vec(buf);
        writer.put_bytes(MAGIC);
        writer.put_u16(VERSION);
        self.mcu.snapshot(&mut writer);
        writer.into_bytes()
    }

    /// Restores state captured by `snapshot`, in place. Host connections
    /// (screen, audio, buttons, IR transport) are unaffected.
    pub fn restore(&mut self, data: &[u8]) -> Result<(), SnapshotError> {
        let mut reader = SnapshotReader::new(data);
        if reader.take_bytes(MAGIC.len())? != MAGIC {
            return Err(SnapshotError::BadMagic);
        }
        let version = reader.take_u16()?;
        if version != VERSION {
            return Err(SnapshotError::UnsupportedVersion(version));
        }
        self.mcu.restore(&mut reader)?;
        reader.finish()
    }
}
