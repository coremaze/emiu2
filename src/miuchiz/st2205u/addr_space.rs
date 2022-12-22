use crate::memory::AddressSpace;

pub type Otp = [u8; 0x4000];
type Ram = [u8; 0x8000];

const REGISTERS_START: u16 = 0x0000;
const REGISTERS_END: u16 = 0x007F;

// This represents the bit range that is operated on by a given bank register.
//
// For example, `PRR_BITS` is `14`. This means that the processor addresses 14
// bits directly. Those bits start at (1<<14) or address 0x4000, and the range
// is another 0x4000. In other words, addresses 0x4000 ~ 0x7FFF are mapped by
// PRR using the lowest 14 bits of the address.
// It also means that the PRR register contains a value which must be shifted
// left by 14 bits to represent their proper significance when addressing the
// machine's address space. In other words, the resulting address when the
// processor accesses the PRR region is (PRR << 14) | address.

const BRR_BITS: usize = 13;
const PRR_BITS: usize = 14;
const DRR_BITS: usize = 15;

const BRR_START: u16 = bank_start(BRR_BITS) as u16;
const BRR_END: u16 = bank_end(BRR_BITS) as u16;

const PRR_START: u16 = bank_start(PRR_BITS) as u16;
const PRR_END: u16 = bank_end(PRR_BITS) as u16;

const DRR_START: u16 = bank_start(DRR_BITS) as u16;
const DRR_END: u16 = bank_end(DRR_BITS) as u16;

const fn bank_start(bits: usize) -> usize {
    1usize << bits
}

const fn bank_end(bits: usize) -> usize {
    (1usize << (bits + 1)) - 1
}

const LOW_RAM_START: u16 = 0x0080;
const LOW_RAM_END: u16 = 0x1FFF;

const PRRL: u16 = 0x0032;
const PRRH: u16 = 0x0033;
const DRRL: u16 = 0x0034;
const DRRH: u16 = 0x0035;
const BRRL: u16 = 0x0036;
const BRRH: u16 = 0x0037;

pub struct St2205uAddressSpace<A: AddressSpace> {
    /// St2205uAddressSpace is 16 bits, but it can itself be used to access a
    /// larger address space through the use of its memory bank registers.
    machine_addr_space: A,

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

impl<A: AddressSpace> St2205uAddressSpace<A> {
    pub fn new(machine_addr_space: A) -> Self {
        Self {
            machine_addr_space,
            ram: [0u8; 0x8000],
            brr: U16Register::new(0),
            prr: U16Register::new(0),
            drr: U16Register::new(0),
        }
    }
}

impl<A: AddressSpace> AddressSpace for St2205uAddressSpace<A> {
    fn read_u8(&mut self, address: usize) -> u8 {
        // The ST2205U address space is only 16 bits wide
        match address as u16 {
            REGISTERS_START..=REGISTERS_END => {
                println!("Read from register {address:X}");
                match address as u16 {
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
                }
            }
            0x80..=0x1FFF => self.ram[address % self.ram.len()],
            BRR_START..=BRR_END | PRR_START..=PRR_END | DRR_START..=DRR_END => {
                // left_shift represents how much the bank register needs to be shifted
                // to represent its component of the larger machine address.
                // i.e. BRR will use the address as its lower 13 bits
                let (reg, left_shift) = match address as u16 {
                    BRR_START..=BRR_END => (self.brr.u16(), BRR_BITS),
                    PRR_START..=PRR_END => (self.prr.u16(), PRR_BITS),
                    DRR_START..=DRR_END => (self.drr.u16(), DRR_BITS),
                    0..=0x1FFF => {
                        unreachable!("This range is excluded by parent match.");
                    }
                };

                if reg & (1 << 15) != 0 {
                    // RAM access if uppermost bit is set
                    self.ram[address % self.ram.len()]
                } else {
                    // Otherwise, access a larger address which is governed by the machine
                    // (i.e. hardware configuration, not ST2205U's responsibility)
                    let machine_addr = ((reg as usize) << left_shift) | address;
                    self.machine_addr_space.read_u8(machine_addr)
                }
            }
        }
    }

    fn write_u8(&mut self, address: usize, value: u8) {
        match address as u16 {
            REGISTERS_START..=REGISTERS_END => {
                // println!("Write to register {address:X}");
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
            LOW_RAM_START..=LOW_RAM_END => {
                // println!("Write to RAM {address:X}");
                self.ram[address % self.ram.len()] = value;
            }
            BRR_START..=BRR_END | PRR_START..=PRR_END | DRR_START..=DRR_END => {
                // left_shift represents how much the bank register needs to be shifted
                // to represent its component of the larger machine address.
                // i.e. BRR will use the address as its lower 13 bits
                let (reg, left_shift) = match address as u16 {
                    BRR_START..=BRR_END => (self.brr.u16(), BRR_BITS),
                    PRR_START..=PRR_END => (self.prr.u16(), PRR_BITS),
                    DRR_START..=DRR_END => (self.drr.u16(), DRR_BITS),
                    0..=0x1FFF => {
                        unreachable!("This range is excluded by parent match.");
                    }
                };

                if reg & (1 << 15) != 0 {
                    // RAM access if uppermost bit is set
                    self.ram[address % self.ram.len()] = value;
                } else {
                    // Otherwise, access a larger address which is governed by the machine
                    // (i.e. hardware configuration, not ST2205U's responsibility)
                    let machine_addr = ((reg as usize) << left_shift) | address;
                    self.machine_addr_space.write_u8(machine_addr, value);
                }
            }
        }
    }
}
