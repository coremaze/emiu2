use crate::{
    display::{Pixel, Screen},
    memory::AddressSpace,
};

const COMMAND_REG: usize = 0;
const DATA_REG: usize = 1;
const REG_COUNT: usize = 2;

const LCD_WIDTH: usize = 98;
const LCD_HEIGHT: usize = 67;

const DDRAM_PAGE: usize = 68;
const DDRAM_COLUMN: usize = 98;
const DDRAM_WIDTH: usize = 2;
const DDRAM_COUNT: usize = DDRAM_COLUMN * DDRAM_PAGE;

pub struct Lcd<'a> {
    ext: bool,
    active_command: Option<Command>,
    byte_since_command: usize,
    ddram: [u8; DDRAM_COUNT * DDRAM_WIDTH],
    ddram_ptr: usize,

    // Controlled by PASET, both are inclusive
    start_page: u8,
    end_page: u8,

    // controlled by CASET, both are inclusive
    start_column: u8,
    end_column: u8,

    screen: &'a dyn Screen,
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

impl<'a> Lcd<'a> {
    pub fn new(screen: &'a impl Screen) -> Self {
        Self {
            ext: false,
            active_command: None,
            byte_since_command: 0,
            ddram: [0u8; DDRAM_COUNT * DDRAM_WIDTH],
            ddram_ptr: 0,
            start_page: 0,
            end_page: 0,
            start_column: 0,
            end_column: 0,
            screen,
        }
    }
}

impl<'a> Lcd<'a> {
    fn handle_command(&mut self, command: Command) {
        // println!("Video write command {command:?}");
        match command {
            Command::ExtOn => self.ext = true,
            Command::ExtOff => self.ext = false,
            _ => {
                // println!("Unimplemented LCD command {command:?}")
            }
        }
        self.active_command = Some(command);
        self.byte_since_command = 0;
    }

    fn handle_data(&mut self, value: u8) {
        let Some(command) = &self.active_command else {
            println!("LCD received data with no active command.");
            return;
        };
        // println!("LCD data {value:02X} to command {:?} (byte {})", command, self.byte_since_command);

        match command {
            Command::PageAddressSet => {
                // println!("PASET {value}");
                if self.byte_since_command == 0 {
                    self.start_page = value;
                } else if self.byte_since_command == 1 {
                    self.end_page = value;
                    self.ddram_set_column_and_page(self.ddram_column(), self.start_page);
                }
            }
            Command::ColumnAddressSet => {
                // println!("CASET {value}");
                if self.byte_since_command == 0 {
                    self.start_column = value;
                } else if self.byte_since_command == 1 {
                    self.end_column = value;
                    self.ddram_set_column_and_page(self.start_column, self.ddram_page());
                }
            }
            Command::WritingToMemory => {
                self.ddram[self.ddram_ptr] = value;

                self.ddram_ptr += 1;

                // println!("ddram ptr: {} column: {} end column: {} page: {} end page {}", self.ddram_ptr, self.ddram_column(), self.end_column, self.ddram_page(), self.end_page);
                if self.ddram_column() > self.end_column {
                    // println!("Resetting column");
                    self.ddram_set_column_and_page(self.start_column, self.ddram_page() + 1);
                }

                if self.ddram_page() > self.end_page {
                    // println!("Resetting page");
                    self.ddram_set_column_and_page(self.ddram_column(), self.start_page);
                    self.update_display();
                }
            }
            _ => {
                println!("Received unhandled data for command {command:?}");
            }
        }

        self.byte_since_command += 1;
    }

    fn ddram_page(&self) -> u8 {
        ((self.ddram_ptr / DDRAM_WIDTH) / DDRAM_COLUMN) as u8
    }

    fn ddram_column(&self) -> u8 {
        ((self.ddram_ptr / DDRAM_WIDTH) % DDRAM_COLUMN) as u8
    }

    fn ddram_set_column_and_page(&mut self, column: u8, page: u8) {
        self.ddram_ptr = Self::column_and_page_ptr(column, page);
    }

    fn column_and_page_ptr(column: u8, page: u8) -> usize {
        (page as usize * DDRAM_COLUMN + column as usize) * DDRAM_WIDTH
    }

    fn update_display(&self) {
        let mut pixels = Vec::<Pixel>::new();

        for page in (self.start_page..=self.end_page) {
            for column in (self.start_column..=self.end_column) {
                let addr = Self::column_and_page_ptr(column, page);
                let pix_1 = self.ddram[addr];
                let pix_2 = self.ddram[addr + 1];

                let red = 255 - ((pix_1 & 0x0F) as u8 * 17);
                let green = 255 - (((pix_2 & 0xF0) >> 4) as u8 * 17);
                let blue = 255 - ((pix_2 & 0x0F) as u8 * 17);
                let pixel = Pixel { red, green, blue };
                pixels.push(pixel);
            }
        }

        // println!("Pixel len {}, pages {} col {}", pixels.len(), self.end_page, self.end_column);

        self.screen.set_pixels(&pixels);
    }
}

impl<'a> AddressSpace for Lcd<'a> {
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
