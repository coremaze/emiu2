use crate::memory::AddressSpace;

const COMMAND_REG: usize = 0;
const DATA_REG: usize = 1;
const REG_COUNT: usize = 2;

pub struct Lcd {
    ext: bool,
    active_command: Option<Command>,
}

#[derive(Debug)]
enum Command {
    ExtOn,
    ExtOff,

    DisplayOn,
    DisplayOff,
    NormalDisplay,
    InverseDisplay,
    ComScanDirection,
    DisplayControl,
    SleepInOutPreparation,
    SleepIn,
    SleepOut,
    PageAddressSet,
    ColumnAddressSet,
    DataScanDirection,
    WritingToMemory,
    ReadingFromMemory,
    PartialDisplayIn,
    PartialDisplayOut,
    ReadModifyWriteIn,
    ReadModifyWriteOut,
    AreaScrollSet,
    ScrollStartSet,
    InternalOscOn,
    InternalOscOff,
    PowerControl,
    EcControl,
    EcIncrease1,
    EcDecrease1,
    ReadRegister1,
    ReadRegister2,
    NoOperation,
    EepromFunctionStart,

    Frame1PwmSet,
    Frame2PwmSet,
    Frame3PwmSet,
    Frame4PwmSet,
    AnalogSet,
    ControlEeprom,
    CancelEeprom,
    WriteToEeprom,
    ReadFromEeprom,
    DisplayPerformanceAdjustment,
    InternalInitializePreparation,
}

impl Command {
    pub fn from_val(ext: bool, val: u8) -> Option<Self> {
        let command = match (ext, val) {
            (false, 0xAF) => Self::DisplayOn,
            (false, 0xAE) => Self::DisplayOff,
            (false, 0xA6) => Self::NormalDisplay,
            (false, 0xA7) => Self::InverseDisplay,
            (false, 0xBB) => Self::ComScanDirection,
            (false, 0xCA) => Self::DisplayControl,
            (false, 0x04) => Self::SleepInOutPreparation,
            (false, 0x95) => Self::SleepIn,
            (false, 0x94) => Self::SleepOut,
            (false, 0x75) => Self::PageAddressSet,
            (false, 0x15) => Self::ColumnAddressSet,
            (false, 0xBC) => Self::DataScanDirection,
            (false, 0x5C) => Self::WritingToMemory,
            (false, 0x5D) => Self::ReadingFromMemory,
            (false, 0xA8) => Self::PartialDisplayIn,
            (false, 0xA9) => Self::PartialDisplayOut,
            (false, 0xE0) => Self::ReadModifyWriteIn,
            (false, 0xEE) => Self::ReadModifyWriteOut,
            (false, 0xAA) => Self::AreaScrollSet,
            (false, 0xAB) => Self::ScrollStartSet,
            (false, 0xD1) => Self::InternalOscOn,
            (false, 0xD2) => Self::InternalOscOff,
            (false, 0x20) => Self::PowerControl,
            (false, 0x81) => Self::EcControl,
            (false, 0xD6) => Self::EcIncrease1,
            (false, 0xD7) => Self::EcDecrease1,
            (false, 0x7C) => Self::ReadRegister1,
            (false, 0x7D) => Self::ReadRegister2,
            (false, 0x25) => Self::NoOperation,
            (false, 0x07) => Self::EepromFunctionStart,

            (true, 0x20) => Self::Frame1PwmSet,
            (true, 0x21) => Self::Frame2PwmSet,
            (true, 0x22) => Self::Frame3PwmSet,
            (true, 0x23) => Self::Frame4PwmSet,
            (true, 0x32) => Self::AnalogSet,
            (true, 0xCD) => Self::ControlEeprom,
            (true, 0xCC) => Self::CancelEeprom,
            (true, 0xFC) => Self::WriteToEeprom,
            (true, 0xFD) => Self::ReadFromEeprom,
            (true, 0xFA) => Self::DisplayPerformanceAdjustment,
            (true, 0xF4) => Self::InternalInitializePreparation,

            (_, 0x30) => Self::ExtOff,
            (_, 0x31) => Self::ExtOn,

            _ => return None,
        };

        Some(command)
    }
}

enum Register {
    Command,
    Data,
}

impl Register {
    pub fn from_address(address: usize) -> Self {
        let reg_addr = address % REG_COUNT;
        match reg_addr {
            COMMAND_REG => Register::Command,
            DATA_REG => Register::Data,
            _ => unreachable!("This device has only 2 registers"),
        }
    }
}

impl Lcd {
    pub fn new() -> Self {
        Self {
            ext: false,
            active_command: None,
        }
    }
}

impl Lcd {
    fn handle_command(&mut self, command: Command) {
        println!("Video write command {command:?}");
        match command {
            Command::ExtOn => self.ext = true,
            Command::ExtOff => self.ext = false,
            _ => println!("Unimplemented LCD command {command:?}"),
        }
        self.active_command = Some(command);
    }

    fn handle_data(&mut self, value: u8) {
        let Some(command) = &self.active_command else {
            println!("LCD received data with no active command.");
            return;
        };
        println!("LCD data {value:02X} to command {:?}", command);
    }
}

impl AddressSpace for Lcd {
    fn read_u8(&mut self, address: usize) -> u8 {
        todo!("Read u8 LCD");
    }

    fn write_u8(&mut self, address: usize, value: u8) {
        match Register::from_address(address) {
            Register::Command => {
                let Some(command) = Command::from_val(self.ext, value) else {
                    println!("Write invalid video command {value:02X} ext: {}", self.ext);
                    return;
                };
                self.handle_command(command);
            }
            Register::Data => self.handle_data(value),
        }
    }
}
