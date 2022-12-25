use super::dma;
use super::bank;
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

const DPRTL: u16 = 0x0058;
const DPRTH: u16 = 0x0059;
const DBKRL: u16 = 0x005A;
const DBKRH: u16 = 0x005B;
const DCNTL: u16 = 0x005C;
const DCNTH: u16 = 0x005D;
const DSEL: u16 = 0x005E;
const DMOD: u16 = 0x005F;

pub struct St2205uAddressSpace<A: AddressSpace> {
    /// St2205uAddressSpace is 16 bits, but it can itself be used to access a
    /// larger address space through the use of its memory bank registers.
    machine_addr_space: A,

    ram: Ram,

    pub banks: bank::State,
    pub dma: dma::State,
}

impl<A: AddressSpace> St2205uAddressSpace<A> {
    pub fn new(machine_addr_space: A) -> Self {
        Self {
            machine_addr_space,
            ram: [0u8; 0x8000],

            banks: bank::State::new(),
            dma: dma::State::new(),
        }
    }

    fn read_register(&mut self, address: u16) -> u8 {
        // println!("Read from register {address:X}");
        match address {
            PRRL => bank::read_prrl(self),
            PRRH => bank::read_prrh(self),
            DRRL => bank::read_drrl(self),
            DRRH => bank::read_drrh(self),
            BRRL => bank::read_brrl(self),
            BRRH => bank::read_brrh(self),
            DPRTL => dma::read_dptrl(self),
            DPRTH => dma::read_dptrh(self),
            DBKRL => dma::read_dbkrl(self),
            DBKRH => dma::read_dbkrh(self),
            DCNTL => dma::read_dcntl(self),
            DCNTH => dma::read_dcnth(self),
            DSEL => dma::read_dsel(self),
            DMOD => dma::read_dmod(self),
            0 => 0xFF, // TODO: controls
            1 => 0xFF, // TODO: controls
            _ => {
                println!("Unimplemented read of register {address:02X}");
                0
            }
        }
    }

    fn write_register(&mut self, address: u16, value: u8) {
        // println!("Write to register {address:X}");
        match address as u16 {
            PRRL => bank::write_prrl(self, value),
            PRRH => bank::write_prrh(self, value),
            DRRL => bank::write_drrl(self, value),
            DRRH => bank::write_drrh(self, value),
            BRRL => bank::write_brrl(self, value),
            BRRH => bank::write_brrh(self, value),
            DPRTL => dma::write_dptrl(self, value),
            DPRTH => dma::write_dptrh(self, value),
            DBKRL => dma::write_dbkrl(self, value),
            DBKRH => dma::write_dbkrh(self, value),
            DCNTL => dma::write_dcntl(self, value),
            DCNTH => dma::write_dcnth(self, value),
            DSEL => dma::write_dsel(self, value),
            DMOD => dma::write_dmod(self, value),
            _ => {
                println!("Unimplemented write of register {address:02X}");
            }
        }
    }

    fn read_ram(&self, address: usize) -> u8 {
        self.ram[address % self.ram.len()]
    }

    fn write_ram(&mut self, address: usize, value: u8) {
        // println!("Write to RAM {address:X}");
        self.ram[address % self.ram.len()] = value;
    }
}

impl<A: AddressSpace> AddressSpace for St2205uAddressSpace<A> {
    fn read_u8(&mut self, address: usize) -> u8 {
        // The ST2205U address space is only 16 bits wide
        match address as u16 {
            REGISTERS_START..=REGISTERS_END => self.read_register(address as u16),
            0x80..=0x1FFF => self.read_ram(address),
            BRR_START..=BRR_END | PRR_START..=PRR_END | DRR_START..=DRR_END => {
                // left_shift represents how much the bank register needs to be shifted
                // to represent its component of the larger machine address.
                // i.e. BRR will use the address as its lower 13 bits
                let (reg, left_shift) = match address as u16 {
                    BRR_START..=BRR_END => (bank::brr(self), BRR_BITS),
                    PRR_START..=PRR_END => (bank::prr(self), PRR_BITS),
                    DRR_START..=DRR_END => (bank::drr(self), DRR_BITS),
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
                    // Only the relevant bits of the address should be kept
                    let addr_mask = (1 << left_shift) - 1;
                    let machine_addr = ((reg as usize) << left_shift) | (address & addr_mask);
                    self.machine_addr_space.read_u8(machine_addr)
                }
            }
        }
    }

    fn write_u8(&mut self, address: usize, value: u8) {
        match address as u16 {
            REGISTERS_START..=REGISTERS_END => self.write_register(address as u16, value),
            LOW_RAM_START..=LOW_RAM_END => self.write_ram(address, value),
            BRR_START..=BRR_END | PRR_START..=PRR_END | DRR_START..=DRR_END => {
                // left_shift represents how much the bank register needs to be shifted
                // to represent its component of the larger machine address.
                // i.e. BRR will use the address as its lower 13 bits
                let (reg, left_shift) = match address as u16 {
                    BRR_START..=BRR_END => (bank::brr(self), BRR_BITS),
                    PRR_START..=PRR_END => (bank::prr(self), PRR_BITS),
                    DRR_START..=DRR_END => (bank::drr(self), DRR_BITS),
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
                    // Only the relevant bits of the address should be kept
                    let addr_mask = (1 << left_shift) - 1;
                    let machine_addr = ((reg as usize) << left_shift) | (address & addr_mask);
                    self.machine_addr_space.write_u8(machine_addr, value);
                }
            }
        }
    }
}
