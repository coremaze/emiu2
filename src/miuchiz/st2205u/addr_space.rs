use super::bank;
use super::base_timer;
use super::dma;
use super::gpio;
use super::interrupt;
use super::timer;
use super::timer::TimerIndex;
use super::wdc_65c02::HandlesInterrupt;
use crate::gpio::Gpio;
use crate::memory::AddressSpace;

pub const OTP_SIZE: usize = 0x4000;
pub type Otp = [u8; OTP_SIZE];
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

const PA: u16 = 0x0000;
const PB: u16 = 0x0001;
const PC: u16 = 0x0002;
const PD: u16 = 0x0003;
const PE: u16 = 0x0004;
const PF: u16 = 0x0005;
const PSC: u16 = 0x0006;
const PSE: u16 = 0x0007;
const PCA: u16 = 0x0008;
const PCB: u16 = 0x0009;
const PCC: u16 = 0x000A;
const PCD: u16 = 0x000B;
const PCE: u16 = 0x000C;
const PCF: u16 = 0x000D;
const PFC: u16 = 0x000E;
const PFD: u16 = 0x000F;

const T0CL: u16 = 0x0020;
const T0CH: u16 = 0x0021;
const T1CL: u16 = 0x0022;
const T1CH: u16 = 0x0023;
const T2CL: u16 = 0x0024;
const T2CH: u16 = 0x0025;
const T3CL: u16 = 0x0026;
const T3CH: u16 = 0x0027;
const TIEN: u16 = 0x0028;

const BTEN: u16 = 0x002A;
const BTREQ: u16 = 0x002B;
const BTC: u16 = 0x002C;

const IRRL: u16 = 0x0030;
const IRRH: u16 = 0x0031;
const PRRL: u16 = 0x0032;
const PRRH: u16 = 0x0033;
const DRRL: u16 = 0x0034;
const DRRH: u16 = 0x0035;
const BRRL: u16 = 0x0036;
const BRRH: u16 = 0x0037;

const PMCR: u16 = 0x003A;

const IREQL: u16 = 0x003C;
const IREQH: u16 = 0x003D;
const IENAL: u16 = 0x003E;
const IENAH: u16 = 0x003F;

const PL: u16 = 0x004E;
const PCL: u16 = 0x004F;

const DPRTL: u16 = 0x0058;
const DPRTH: u16 = 0x0059;
const DBKRL: u16 = 0x005A;
const DBKRH: u16 = 0x005B;
const DCNTL: u16 = 0x005C;
const DCNTH: u16 = 0x005D;
const DSEL: u16 = 0x005E;
const DMOD: u16 = 0x005F;

pub struct St2205uAddressSpace<'a, A: AddressSpace> {
    /// St2205uAddressSpace is 16 bits, but it can itself be used to access a
    /// larger address space through the use of its memory bank registers.
    machine_addr_space: A,

    ram: Ram,

    pub banks: bank::State,
    pub dma: dma::State,
    pub gpio: gpio::State<'a>,
    pub base_timer: base_timer::State,
    pub timer: timer::TimerBlocksState,
    pub interrupt: interrupt::State,
}

impl<'a, A: AddressSpace> St2205uAddressSpace<'a, A> {
    pub fn new(machine_addr_space: A, io: &'a impl Gpio, frequency: u64) -> Self {
        Self {
            machine_addr_space,
            ram: [0u8; 0x8000],

            banks: bank::State::new(),
            dma: dma::State::new(),
            gpio: gpio::State::new(io),
            base_timer: base_timer::State::new(frequency),
            timer: timer::TimerBlocksState::new(),
            interrupt: interrupt::State::new(),
        }
    }

    fn read_register(&mut self, address: u16) -> u8 {
        // println!("Read from register {address:X}");
        match address {
            IRRL => bank::read_irrl(self),
            IRRH => bank::read_irrh(self),
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
            PA => gpio::read_pa(&self.gpio),
            PB => gpio::read_pb(&self.gpio),
            PC => gpio::read_pc(&self.gpio),
            PD => gpio::read_pd(&self.gpio),
            PE => gpio::read_pe(&self.gpio),
            PF => gpio::read_pf(&self.gpio),
            PSC => gpio::read_psc(&self.gpio),
            PSE => gpio::read_pse(&self.gpio),
            PCA => gpio::read_pca(&self.gpio),
            PCB => gpio::read_pcb(&self.gpio),
            PCC => gpio::read_pcc(&self.gpio),
            PCD => gpio::read_pcd(&self.gpio),
            PCE => gpio::read_pce(&self.gpio),
            PCF => gpio::read_pcf(&self.gpio),
            PFC => gpio::read_pfc(&self.gpio),
            PFD => gpio::read_pfd(&self.gpio),
            T0CL => self.timer.read_txcl(TimerIndex::T0),
            T0CH => self.timer.read_txch(TimerIndex::T0),
            T1CL => self.timer.read_txcl(TimerIndex::T1),
            T1CH => self.timer.read_txch(TimerIndex::T1),
            T2CL => self.timer.read_txcl(TimerIndex::T2),
            T2CH => self.timer.read_txch(TimerIndex::T2),
            T3CL => self.timer.read_txcl(TimerIndex::T3),
            T3CH => self.timer.read_txch(TimerIndex::T3),
            TIEN => self.timer.read_tien(),
            PMCR => gpio::read_pmcr(&self.gpio),
            PL => gpio::read_pl(&self.gpio),
            PCL => gpio::read_pcl(&self.gpio),
            BTEN => base_timer::read_bten(&self.base_timer),
            BTREQ => base_timer::read_btreq(&self.base_timer),
            BTC => base_timer::read_btc(&self.base_timer),
            IREQL => interrupt::read_ireql(&self.interrupt),
            IREQH => interrupt::read_ireqh(&self.interrupt),
            IENAL => interrupt::read_ienal(&self.interrupt),
            IENAH => interrupt::read_ienah(&self.interrupt),
            _ => {
                // println!("Unimplemented read of register {address:02X}");
                0
            }
        }
    }

    fn write_register(&mut self, address: u16, value: u8) {
        // println!("Write to register {address:X}");
        match address as u16 {
            IRRL => bank::write_irrl(self, value),
            IRRH => bank::write_irrh(self, value),
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
            PA => gpio::write_pa(&mut self.gpio, value),
            PB => gpio::write_pb(&mut self.gpio, value),
            PC => gpio::write_pc(&mut self.gpio, value),
            PD => gpio::write_pd(&mut self.gpio, value),
            PE => gpio::write_pe(&mut self.gpio, value),
            PF => gpio::write_pf(&mut self.gpio, value),
            PSC => gpio::write_psc(&mut self.gpio, value),
            PSE => gpio::write_pse(&mut self.gpio, value),
            PCA => gpio::write_pca(&mut self.gpio, value),
            PCB => gpio::write_pcb(&mut self.gpio, value),
            PCC => gpio::write_pcc(&mut self.gpio, value),
            PCD => gpio::write_pcd(&mut self.gpio, value),
            PCE => gpio::write_pce(&mut self.gpio, value),
            PCF => gpio::write_pcf(&mut self.gpio, value),
            PFC => gpio::write_pfc(&mut self.gpio, value),
            PFD => gpio::write_pfd(&mut self.gpio, value),
            T0CL => self.timer.write_txcl(TimerIndex::T0, value),
            T0CH => self.timer.write_txch(TimerIndex::T0, value),
            T1CL => self.timer.write_txcl(TimerIndex::T1, value),
            T1CH => self.timer.write_txch(TimerIndex::T1, value),
            T2CL => self.timer.write_txcl(TimerIndex::T2, value),
            T2CH => self.timer.write_txch(TimerIndex::T2, value),
            T3CL => self.timer.write_txcl(TimerIndex::T3, value),
            T3CH => self.timer.write_txch(TimerIndex::T3, value),
            TIEN => self.timer.write_tien(value),
            PMCR => gpio::write_pmcr(&mut self.gpio, value),
            PL => gpio::write_pl(&mut self.gpio, value),
            PCL => gpio::write_pcl(&mut self.gpio, value),
            BTEN => base_timer::write_bten(&mut self.base_timer, value),
            BTREQ => base_timer::write_btreq(&mut self.base_timer, value),
            BTC => base_timer::write_btc(&mut self.base_timer, value),
            IREQL => interrupt::write_ireql(&mut self.interrupt, value),
            IREQH => interrupt::write_ireqh(&mut self.interrupt, value),
            IENAL => interrupt::write_ienal(&mut self.interrupt, value),
            IENAH => interrupt::write_ienah(&mut self.interrupt, value),
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

impl<'a, A: AddressSpace> HandlesInterrupt for St2205uAddressSpace<'a, A> {
    fn set_interrupted(&mut self, interrupted: bool) {
        self.interrupt.set_interrupted(interrupted);
    }

    fn interrupted(&self) -> bool {
        self.interrupt.interrupted()
    }
}

impl<'a, A: AddressSpace> AddressSpace for St2205uAddressSpace<'a, A> {
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
                    PRR_START..=PRR_END => {
                        if self.interrupted() {
                            (bank::irr(self), PRR_BITS)
                        } else {
                            (bank::prr(self), PRR_BITS)
                        }
                    }
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
                    PRR_START..=PRR_END => {
                        if self.interrupted() {
                            (bank::irr(self), PRR_BITS)
                        } else {
                            (bank::prr(self), PRR_BITS)
                        }
                    }
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
