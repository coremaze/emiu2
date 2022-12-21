use crate::memory::AddressSpace;

pub type Otp = [u8; 0x4000];
pub type Flash = [u8; 0x200000];
type Ram = [u8; 0x8000];

const REGISTERS_START: u16 = 0x0000;
const REGISTERS_END: u16 = 0x007F;

const BRR_START: u16 = 0x2000;
const BRR_END: u16 = 0x3FFF;
const BRR_SIZE: u16 = 0x2000;

const PRR_START: u16 = 0x4000;
const PRR_END: u16 = 0x7FFF;
const PRR_SIZE: u16 = 0x4000;

const DRR_START: u16 = 0x8000;
const DRR_END: u16 = 0xFFFF;
const DRR_SIZE: u16 = 0x8000;

const LOW_RAM_START: u16 = 0x0080;
const LOW_RAM_END: u16 = 0x1FFF;

const PRRL: u16 = 0x0032;
const PRRH: u16 = 0x0033;
const DRRL: u16 = 0x0034;
const DRRH: u16 = 0x0035;
const BRRL: u16 = 0x0036;
const BRRH: u16 = 0x0037;

pub struct St2205uAddressSpace {
    otp: Box<Otp>,
    flash: Box<Flash>,
    ram: Ram,

    brr: U16Register,
    prr: U16Register,
    drr: U16Register,
}

#[derive(Default)]
pub struct U16Register {
    l: u8,
    h: u8,
}

impl U16Register {
    pub fn new(value: u16) -> Self {
        let mut reg = Self::default();
        reg.set_u16(value);
        reg
    }

    pub fn u16(&self) -> u16 {
        (self.l as u16) | ((self.h as u16) << 8)
    }

    pub fn set_u16(&mut self, value: u16) {
        self.l = (value & 0x00FF) as u8;
        self.h = ((value & 0xFF00) >> 8) as u8;
    }
}

struct MemoryMap {
    pub bank_type: MemoryBankType,

    /// The address that should be shown at the beginning of the window
    pub contents_offset: usize,
}

enum MemoryBankType {
    Otp,       // One-Time-Programmable ROM
    Lcd,       // Control registers for the LCD
    Flash,     // Flash chip
    Ram,       // 32K RAM
    Registers, // MCU control registers
}

fn memory_bank_type_from_selection_bits(bits: u16) -> MemoryBankType {
    // 0000x or 1111x is OTP
    if (bits & 0b11110 == 0b00000) || (bits & 0b11110 == 0b11110) {
        MemoryBankType::Otp
    }
    // 00011 is Lcd
    else if bits == 0b00011 {
        MemoryBankType::Lcd
    }
    // 001xx is flash
    else if bits & 0b11100 == 0b00100 {
        MemoryBankType::Flash
    } else {
        // Technically this should be open bus
        MemoryBankType::Ram
    }
}

impl St2205uAddressSpace {
    pub fn new(otp: Box<Otp>, flash: Box<Flash>) -> Self {
        Self {
            otp,
            flash,
            ram: [0u8; 0x8000],
            brr: U16Register::new(0),
            prr: U16Register::new(0),
            drr: U16Register::new(0),
        }
    }

    fn brr_map(&self) -> MemoryMap {
        // If the high bit is set, use RAM
        if self.brr.u16() & (1 << 15) != 0 {
            MemoryMap {
                bank_type: MemoryBankType::Ram,
                contents_offset: 0x2000,
            }
        } else {
            // bits 8:12 are selection bits
            let selection_bits = (self.brr.u16() >> 8) & 0b11111;
            let bank_type = memory_bank_type_from_selection_bits(selection_bits);

            // bits 0:7 are page bits
            let page_bits = self.brr.u16() & 0b11111111;
            let contents_offset = page_bits as usize * BRR_SIZE as usize;

            MemoryMap {
                bank_type,
                contents_offset,
            }
        }
    }

    fn prr_map(&self) -> MemoryMap {
        // If the high bit is set, use RAM
        if self.prr.u16() & (1 << 15) != 0 {
            MemoryMap {
                bank_type: MemoryBankType::Ram,
                contents_offset: 0x4000,
            }
        } else {
            // bits 7:11 are selection bits
            let selection_bits = (self.prr.u16() >> 7) & 0b11111;
            let bank_type = memory_bank_type_from_selection_bits(selection_bits);

            // bits 0:6 are page bits
            let page_bits = self.brr.u16() & 0b1111111;
            let contents_offset = page_bits as usize * PRR_SIZE as usize;

            MemoryMap {
                bank_type,
                contents_offset,
            }
        }
    }

    fn drr_map(&self) -> MemoryMap {
        // If the high bit is set, use RAM
        if self.drr.u16() & (1 << 15) != 0 {
            MemoryMap {
                bank_type: MemoryBankType::Ram,
                contents_offset: 0x4000,
            }
        } else {
            // bits 6:10 are selection bits
            let selection_bits = (self.drr.u16() >> 6) & 0b11111;
            let bank_type = memory_bank_type_from_selection_bits(selection_bits);

            // bits 0:5 are page bits
            let page_bits = self.drr.u16() & 0b111111;
            let contents_offset = page_bits as usize * DRR_SIZE as usize;

            MemoryMap {
                bank_type,
                contents_offset,
            }
        }
    }

    fn get_memory_bank_map(&self, address: u16) -> MemoryMap {
        match address {
            REGISTERS_START..=REGISTERS_END => MemoryMap {
                bank_type: MemoryBankType::Registers,
                contents_offset: 0,
            },
            LOW_RAM_START..=LOW_RAM_END => MemoryMap {
                bank_type: MemoryBankType::Ram,
                contents_offset: LOW_RAM_START as usize,
            },
            BRR_START..=BRR_END => self.brr_map(),
            PRR_START..=PRR_END => {
                // println!("PRR Reading from OTP");
                self.prr_map()
            }
            DRR_START..=DRR_END => {
                // println!("PRR Reading from OTP");
                self.drr_map()
            }
        }
    }
}

impl AddressSpace for St2205uAddressSpace {
    fn read_u8(&mut self, address: usize) -> u8 {
        // This could probably be optimized by storing the result for each memory range
        // and only updating that cache whenever a bank register is written to
        let memory_map = self.get_memory_bank_map(address as u16);
        let offset = memory_map.contents_offset;
        // println!("Reading {address:04X}");
        match memory_map.bank_type {
            MemoryBankType::Otp => self.otp[(offset + address) % self.otp.len()],
            MemoryBankType::Lcd => panic!("TODO: implement lcd"),
            MemoryBankType::Flash => self.flash[(offset + address) % self.flash.len()],
            MemoryBankType::Ram => self.ram[(offset + address) % self.ram.len()],
            MemoryBankType::Registers => match address as u16 {
                PRRL => self.prr.l,
                PRRH => self.prr.h,
                DRRL => self.drr.l,
                DRRH => self.drr.h,
                BRRL => self.brr.l,
                BRRH => self.brr.h,
                _ => {
                    println!("Unimplemented read of register {address:02X}");
                    0
                }
            },
        }
    }

    fn write_u8(&mut self, address: usize, value: u8) {
        let memory_map = self.get_memory_bank_map(address as u16);
        let offset = memory_map.contents_offset;
        match memory_map.bank_type {
            MemoryBankType::Otp => panic!("Can't write OTP"),
            MemoryBankType::Lcd => {
                println!("Unimplemented write of LCD register {address:02X} {value:02X}");
            }
            MemoryBankType::Flash => panic!("TODO: implement flash commands"),
            MemoryBankType::Ram => self.ram[(offset + address) % self.ram.len()] = value,
            MemoryBankType::Registers => {
                match address as u16 {
                    PRRL => {
                        // println!("PRRL set to {value:02X}");
                        self.prr.l = value
                    }
                    PRRH => {
                        // println!("PRRH set to {value:02X}");
                        self.prr.h = value
                    }
                    DRRL => self.drr.l = value,
                    DRRH => self.drr.h = value,
                    BRRL => self.brr.l = value,
                    BRRH => self.brr.h = value,
                    _ => {
                        println!("Unimplemented write of register {address:02X}");
                    }
                }
            }
        }
    }
}
