use super::bank;
use super::base_timer;
use super::decode_cache::{DecodeCache, RamDecodeCache};
use super::dma;
use super::gpio;
use super::gpio::GpioInterfaceInternal;
use super::interrupt;
use super::psg;
use super::psg::PsgChannel;
use super::rtc;
use super::timer;
use super::timer::TimerIndex;
use super::wdc_65c02::{DecodedInstruction, FetchedInstruction, FetchesDecoded, HandlesInterrupt};
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
const PSG0A: u16 = 0x0010;
const PSG0B: u16 = 0x0011;
const PSG1A: u16 = 0x0012;
const PSG1B: u16 = 0x0013;
const PSG2A: u16 = 0x0014;
const PSG2B: u16 = 0x0015;
const PSG3A: u16 = 0x0016;
const PSG3B: u16 = 0x0017;
const VOL0: u16 = 0x0018;
const VOL1: u16 = 0x0019;
const VOL2: u16 = 0x001A;
const VOL3: u16 = 0x001B;

const PSGC: u16 = 0x001E;
const PSGM: u16 = 0x001F;
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
const RCTR: u16 = 0x002E;
const RTC: u16 = 0x002F;

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

const MULL: u16 = 0x006E;
const MULH: u16 = 0x006F;

enum TimerRegister {
    Tcl,
    Tch,
}

/// Cached resolution of one 8K region of the virtual address space for
/// instruction fetch — a fetch TLB slot. A bank window covers one or more
/// slots (BRR one, PRR two, DRR four); each slot records the containing
/// window's bounds and kind. The table has separate halves for normal and
/// interrupt mode (whose PRR window maps through IRR instead), so interrupt
/// entry/exit invalidates nothing. Slots stay valid until their bank
/// register is written or a machine content change occurs.
#[derive(Clone, Copy)]
struct FetchSlot {
    /// Start of the containing bank window
    window_start: u32,
    /// Exclusive end of the containing bank window; 0 marks an invalid
    /// slot, and instructions must fit strictly inside (`pc + 2 < end`)
    window_end: u32,
    kind: FetchKind,
}

#[derive(Clone, Copy)]
enum FetchKind {
    /// Window maps internal RAM: byte-verified RAM cache applies
    Ram,
    /// Window maps plain machine memory; cache key = base + (pc - start)
    Machine { key_base: usize },
    /// Reads have side effects or transient values: always decode fresh
    Uncached,
}

impl FetchSlot {
    const INVALID: FetchSlot = FetchSlot {
        window_start: 1,
        window_end: 0,
        kind: FetchKind::Uncached,
    };
}

/// Virtual address space divided into eight 8K fetch slots
const FETCH_SLOT_SHIFT: u32 = 13;

/// Slots 8..16 describe interrupt mode, where PRR maps through IRR
const FETCH_SLOT_INTERRUPTED: usize = 8;

/// The address space visible to the 65C02 core, generic over the machine
/// address space `M` so that machine accesses dispatch statically and can be
/// inlined into the memory hot path.
pub struct St2205uAddressSpace<M: AddressSpace> {
    /// St2205uAddressSpace is 16 bits, but it can itself be used to access a
    /// larger address space through the use of its memory bank registers.
    pub machine_addr_space: M,

    ram: Ram,

    pub banks: bank::State,
    pub dma: dma::State,
    pub gpio: gpio::State,
    pub base_timer: base_timer::State,
    pub timer: timer::TimerBlocksState,
    pub psg: psg::State,
    pub interrupt: interrupt::State,
    pub rtc: rtc::State,

    /// The instruction cycle count at the start of the instruction currently
    /// executing. Register accesses take effect at this cycle: peripherals
    /// are only ever advanced through completed instructions.
    pub boundary_sysck: u64,

    /// Set when a register write may have changed a peripheral's next event
    /// time, so the MCU rechecks its event schedule.
    pub events_dirty: bool,

    /// Cache of decoded instructions, keyed by machine address
    decode_cache: DecodeCache,

    /// Cache of decoded instructions in internal RAM
    pub ram_decode_cache: RamDecodeCache,

    /// Fetch TLB: one slot per 8K of virtual address space, one half per
    /// interrupt mode
    fetch_slots: [FetchSlot; 16],

    /// Fetch statistics: [cache hits, cache misses, uncacheable fetches]
    pub fetch_stats: [u64; 3],
}

impl<M: AddressSpace> St2205uAddressSpace<M> {
    pub fn new(machine_addr_space: M, io: Box<dyn GpioInterfaceInternal>, frequency: u64) -> Self {
        Self {
            machine_addr_space,
            ram: [0u8; 0x8000],

            banks: bank::State::new(),
            dma: dma::State::new(),
            gpio: gpio::State::new(io),
            base_timer: base_timer::State::new(frequency),
            timer: timer::TimerBlocksState::new(),
            psg: psg::State::new(),
            interrupt: interrupt::State::new(),
            rtc: rtc::State::new(frequency),

            boundary_sysck: 0,
            events_dirty: true,

            decode_cache: DecodeCache::new(),
            ram_decode_cache: RamDecodeCache::new(0x8000),
            fetch_slots: [FetchSlot::INVALID; 16],

            fetch_stats: [0; 3],
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
            PSG0B => self.psg.read_psgxb(PsgChannel::Channel0),
            PSG1B => self.psg.read_psgxb(PsgChannel::Channel1),
            PSG2B => self.psg.read_psgxb(PsgChannel::Channel2),
            PSG3B => self.psg.read_psgxb(PsgChannel::Channel3),
            VOL0 => self.psg.read_volx(PsgChannel::Channel0),
            VOL1 => self.psg.read_volx(PsgChannel::Channel1),
            VOL2 => self.psg.read_volx(PsgChannel::Channel2),
            VOL3 => self.psg.read_volx(PsgChannel::Channel3),
            PSGC => self.psg.read_psgc(),
            PSGM => self.psg.read_psgm(),
            T0CL => self.timer.read_txcl(TimerIndex::T0, self.boundary_sysck),
            T0CH => self.timer.read_txch(TimerIndex::T0, self.boundary_sysck),
            T1CL => self.timer.read_txcl(TimerIndex::T1, self.boundary_sysck),
            T1CH => self.timer.read_txch(TimerIndex::T1, self.boundary_sysck),
            T2CL => self.timer.read_txcl(TimerIndex::T2, self.boundary_sysck),
            T2CH => self.timer.read_txch(TimerIndex::T2, self.boundary_sysck),
            T3CL => self.timer.read_txcl(TimerIndex::T3, self.boundary_sysck),
            T3CH => self.timer.read_txch(TimerIndex::T3, self.boundary_sysck),
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
            MULL => self.psg.read_mull(),
            MULH => self.psg.read_mulh(),
            RTC => self.rtc.read_rtc(),
            RCTR => self.rtc.read_rctr(),
            _ => {
                // println!("Unimplemented read of register {address:02X}");
                0
            }
        }
    }

    fn write_register(&mut self, address: u16, value: u8) {
        // println!("Write to register {address:X}");
        match address as u16 {
            IRRL => {
                bank::write_irrl(self, value);
                let (first, last) = (2, 3);
                self.invalidate_fetch_slots(first, last);
            }
            IRRH => {
                bank::write_irrh(self, value);
                let (first, last) = (2, 3);
                self.invalidate_fetch_slots(first, last);
            }
            PRRL => {
                bank::write_prrl(self, value);
                let (first, last) = (2, 3);
                self.invalidate_fetch_slots(first, last);
            }
            PRRH => {
                bank::write_prrh(self, value);
                let (first, last) = (2, 3);
                self.invalidate_fetch_slots(first, last);
            }
            DRRL => {
                bank::write_drrl(self, value);
                let (first, last) = (4, 7);
                self.invalidate_fetch_slots(first, last);
            }
            DRRH => {
                bank::write_drrh(self, value);
                let (first, last) = (4, 7);
                self.invalidate_fetch_slots(first, last);
            }
            BRRL => {
                bank::write_brrl(self, value);
                let (first, last) = (1, 1);
                self.invalidate_fetch_slots(first, last);
            }
            BRRH => {
                bank::write_brrh(self, value);
                let (first, last) = (1, 1);
                self.invalidate_fetch_slots(first, last);
            }
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
            PSG0A => self.psg.write_psgxa(PsgChannel::Channel0, value),
            PSG0B => self.psg.write_psgxb(PsgChannel::Channel0, value),
            PSG1A => self.psg.write_psgxa(PsgChannel::Channel1, value),
            PSG1B => self.psg.write_psgxb(PsgChannel::Channel1, value),
            PSG2A => self.psg.write_psgxa(PsgChannel::Channel2, value),
            PSG2B => self.psg.write_psgxb(PsgChannel::Channel2, value),
            PSG3A => self.psg.write_psgxa(PsgChannel::Channel3, value),
            PSG3B => self.psg.write_psgxb(PsgChannel::Channel3, value),
            VOL0 => self.psg.write_volx(PsgChannel::Channel0, value),
            VOL1 => self.psg.write_volx(PsgChannel::Channel1, value),
            VOL2 => self.psg.write_volx(PsgChannel::Channel2, value),
            VOL3 => self.psg.write_volx(PsgChannel::Channel3, value),
            PSGC => self.psg.write_psgc(value),
            PSGM => self.psg.write_psgm(value),
            T0CL => self.write_timer_register(TimerIndex::T0, value, TimerRegister::Tcl),
            T0CH => self.write_timer_register(TimerIndex::T0, value, TimerRegister::Tch),
            T1CL => self.write_timer_register(TimerIndex::T1, value, TimerRegister::Tcl),
            T1CH => self.write_timer_register(TimerIndex::T1, value, TimerRegister::Tch),
            T2CL => self.write_timer_register(TimerIndex::T2, value, TimerRegister::Tcl),
            T2CH => self.write_timer_register(TimerIndex::T2, value, TimerRegister::Tch),
            T3CL => self.write_timer_register(TimerIndex::T3, value, TimerRegister::Tcl),
            T3CH => self.write_timer_register(TimerIndex::T3, value, TimerRegister::Tch),
            TIEN => {
                self.timer.write_tien(value, self.boundary_sysck);
                self.events_dirty = true;
            }
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
            MULL => self.psg.write_mull(value),
            MULH => self.psg.write_mulh(value),
            RCTR => self.rtc.write_rctr(value),
            RTC => self.rtc.write_rtc(value),
            _ => {
                println!("Unimplemented write of register {address:02X}");
            }
        }
    }

    /// The full contents of internal RAM, e.g. for state fingerprinting.
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    fn write_timer_register(&mut self, timer: TimerIndex, value: u8, register: TimerRegister) {
        match register {
            TimerRegister::Tcl => self.timer.write_txcl(timer, value, self.boundary_sysck),
            TimerRegister::Tch => self.timer.write_txch(timer, value, self.boundary_sysck),
        }
        // The write may have moved the timer's next overflow
        self.events_dirty = true;
    }

    fn read_ram(&self, address: usize) -> u8 {
        self.ram[address % self.ram.len()]
    }

    fn write_ram(&mut self, address: usize, value: u8) {
        // println!("Write to RAM {address:X}");
        self.ram[address % self.ram.len()] = value;
    }
}

impl<M: AddressSpace> FetchesDecoded for St2205uAddressSpace<M> {
    #[inline(always)]
    fn fetch_decoded(&mut self, pc: u16) -> FetchedInstruction {
        match self.fetch_in_window(pc) {
            Some(fetched) => fetched,
            None => self.fetch_window_miss(pc),
        }
    }
}

impl<M: AddressSpace> St2205uAddressSpace<M> {
    /// Fetch through the current fetch window; None if `pc` misses it.
    /// This is the per-instruction hot path: cache hits return without
    /// leaving it, everything else is outlined.
    #[inline(always)]
    fn fetch_in_window(&mut self, pc: u16) -> Option<FetchedInstruction> {
        let vpc = pc as u32;
        let slot_index = (vpc >> FETCH_SLOT_SHIFT) as usize
            | (self.interrupted() as usize) * FETCH_SLOT_INTERRUPTED;
        let window = self.fetch_slots[slot_index];
        if vpc < window.window_start || vpc + 2 >= window.window_end {
            return None;
        }

        Some(match window.kind {
            FetchKind::Ram => {
                let ram_index = pc as usize % self.ram.len();
                // Entries must not span a 256-byte RAM page (the byte
                // check reads up to ram_index + 2, and any 8K-aligned
                // bank window boundary is also a page boundary)
                if (ram_index ^ (ram_index + 2)) & !0xFF != 0 {
                    return Some(self.fetch_uncached(pc));
                }
                match self.ram_decode_cache.get(ram_index, &self.ram) {
                    Some(fetched) => {
                        self.fetch_stats[0] += 1;
                        fetched
                    }
                    None => self.fetch_ram_miss(pc, ram_index),
                }
            }
            FetchKind::Machine { key_base } => {
                let key = key_base + (vpc - window.window_start) as usize;
                // Entries must not span a cache page (invalidation is
                // page-granular)
                if (key ^ (key + 2)) & !0xFFF != 0 {
                    return Some(self.fetch_uncached(pc));
                }
                match self.decode_cache.get(key) {
                    Some(fetched) => {
                        self.fetch_stats[0] += 1;
                        fetched
                    }
                    None => self.fetch_machine_miss(pc, key),
                }
            }
            FetchKind::Uncached => self.fetch_uncached(pc),
        })
    }

    /// The program counter's fetch slot is invalid: re-resolve and retry
    #[cold]
    fn fetch_window_miss(&mut self, pc: u16) -> FetchedInstruction {
        self.refill_fetch_slots(pc);
        match self.fetch_in_window(pc) {
            Some(fetched) => fetched,
            // The instruction's bytes may extend past the window into a
            // different mapping: never cached
            None => self.fetch_uncached(pc),
        }
    }

    #[cold]
    fn fetch_uncached(&mut self, pc: u16) -> FetchedInstruction {
        self.fetch_stats[2] += 1;
        let opcode_byte = self.read_u8(pc.into());
        let dins = DecodedInstruction::decode_from_byte(opcode_byte, self, pc.into());
        FetchedInstruction::new(&dins, opcode_byte)
    }

    #[cold]
    fn fetch_ram_miss(&mut self, pc: u16, ram_index: usize) -> FetchedInstruction {
        self.fetch_stats[1] += 1;
        let opcode_byte = self.ram[ram_index];
        let dins = DecodedInstruction::decode_from_byte(opcode_byte, self, pc.into());
        let fetched = FetchedInstruction::new(&dins, opcode_byte);
        let bytes = [
            self.ram[ram_index],
            self.ram[ram_index + 1],
            self.ram[ram_index + 2],
        ];
        self.ram_decode_cache.insert(ram_index, fetched, bytes);
        fetched
    }

    #[cold]
    fn fetch_machine_miss(&mut self, pc: u16, key: usize) -> FetchedInstruction {
        self.fetch_stats[1] += 1;
        let opcode_byte = self.read_u8(pc.into());
        let dins = DecodedInstruction::decode_from_byte(opcode_byte, self, pc.into());
        let fetched = FetchedInstruction::new(&dins, opcode_byte);
        self.decode_cache.insert(key, fetched);
        fetched
    }

    /// Resolve the bank window containing `pc` and fill its fetch slots.
    /// The PRR window depends on the interrupt mode, so it only fills the
    /// current mode's half of the table; every other window is
    /// mode-independent and fills both halves.
    fn refill_fetch_slots(&mut self, pc: u16) {
        let mut both_halves = true;
        let (start, len, kind) = match pc {
            // Hardware registers: reads have side effects
            0x0000..=0x007F => (0u32, 0x80u32, FetchKind::Uncached),
            LOW_RAM_START..=LOW_RAM_END => (
                LOW_RAM_START as u32,
                (LOW_RAM_END - LOW_RAM_START + 1) as u32,
                FetchKind::Ram,
            ),
            _ => {
                let (reg, bits, start) = match pc {
                    BRR_START..=BRR_END => (bank::brr(self), BRR_BITS, BRR_START),
                    PRR_START..=PRR_END => {
                        both_halves = false;
                        if self.interrupted() {
                            (bank::irr(self), PRR_BITS, PRR_START)
                        } else {
                            (bank::prr(self), PRR_BITS, PRR_START)
                        }
                    }
                    DRR_START..=DRR_END => (bank::drr(self), DRR_BITS, DRR_START),
                    0..=0x1FFF => unreachable!("This range is excluded by parent match."),
                };

                let len = 1u32 << bits;
                if reg & (1 << 15) != 0 {
                    // The uppermost bank register bit selects internal RAM
                    (start as u32, len, FetchKind::Ram)
                } else {
                    let machine_base = (reg as usize) << bits;
                    match self
                        .machine_addr_space
                        .code_cache_key_range(machine_base, len as usize)
                    {
                        Some(key_base) => (start as u32, len, FetchKind::Machine { key_base }),
                        None => (start as u32, len, FetchKind::Uncached),
                    }
                }
            }
        };

        let slot = FetchSlot {
            window_start: start,
            window_end: start + len,
            kind,
        };
        let first = (start >> FETCH_SLOT_SHIFT) as usize;
        let last = ((start + len - 1) >> FETCH_SLOT_SHIFT) as usize;
        let halves = if both_halves {
            [true, true]
        } else {
            let interrupted = self.interrupted();
            [!interrupted, interrupted]
        };
        for (half, fill) in halves.into_iter().enumerate() {
            if fill {
                let base = half * FETCH_SLOT_INTERRUPTED;
                for entry in &mut self.fetch_slots[base + first..=base + last] {
                    *entry = slot;
                }
            }
        }
    }

    /// Invalidate the fetch slots covering one bank window, in both halves
    fn invalidate_fetch_slots(&mut self, first: usize, last: usize) {
        for half in [0, FETCH_SLOT_INTERRUPTED] {
            for entry in &mut self.fetch_slots[half + first..=half + last] {
                *entry = FetchSlot::INVALID;
            }
        }
    }

    #[inline]
    fn invalidate_fetch_window(&mut self) {
        self.fetch_slots = [FetchSlot::INVALID; 16];
    }
}

impl<M: AddressSpace> HandlesInterrupt for St2205uAddressSpace<M> {
    fn set_interrupted(&mut self, interrupted: bool) {
        self.interrupt.set_interrupted(interrupted);
        // No fetch slot invalidation: the interrupt mode selects the other
        // half of the slot table, which carries its own PRR/IRR resolution
    }

    fn interrupted(&self) -> bool {
        self.interrupt.interrupted()
    }
}

impl<M: AddressSpace> AddressSpace for St2205uAddressSpace<M> {
    #[inline(always)]
    fn read_u8(&mut self, address: usize) -> u8 {
        // The ST2205U address space is only 16 bits wide
        match address as u16 {
            REGISTERS_START..=REGISTERS_END => self.read_register(address as u16),
            0x80..=0x1FFF => self.read_ram(address),
            _ => self.read_banked(address),
        }
    }

    #[inline(always)]
    fn write_u8(&mut self, address: usize, value: u8) {
        match address as u16 {
            REGISTERS_START..=REGISTERS_END => self.write_register(address as u16, value),
            LOW_RAM_START..=LOW_RAM_END => self.write_ram(address, value),
            _ => self.write_banked(address, value),
        }
    }
}

impl<M: AddressSpace> St2205uAddressSpace<M> {
    fn read_banked(&mut self, address: usize) -> u8 {
        match address as u16 {
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
                    self.read_ram(address)
                } else {
                    // Otherwise, access a larger address which is governed by the machine
                    // (i.e. hardware configuration, not ST2205U's responsibility)
                    // Only the relevant bits of the address should be kept
                    let addr_mask = (1 << left_shift) - 1;
                    let machine_addr = ((reg as usize) << left_shift) | (address & addr_mask);
                    self.machine_addr_space.read_u8(machine_addr)
                }
            }
            _ => unreachable!("Only banked ranges reach read_banked."),
        }
    }

    fn write_banked(&mut self, address: usize, value: u8) {
        match address as u16 {
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
                    self.write_ram(address, value);
                } else {
                    // Otherwise, access a larger address which is governed by the machine
                    // (i.e. hardware configuration, not ST2205U's responsibility)
                    // Only the relevant bits of the address should be kept
                    let addr_mask = (1 << left_shift) - 1;
                    let machine_addr = ((reg as usize) << left_shift) | (address & addr_mask);
                    self.machine_addr_space.write_u8(machine_addr, value);
                    // Rewritable code storage (flash) may have changed:
                    // drop decodes cached from the affected range, and
                    // re-validate the fetch window (the machine's cacheable
                    // ranges may have changed with it)
                    let change = self.machine_addr_space.take_content_change();
                    if !matches!(change, crate::memory::ContentChange::None) {
                        self.decode_cache.apply_change(change);
                        self.invalidate_fetch_window();
                    }
                }
            }
            _ => unreachable!("Only banked ranges reach write_banked."),
        }
    }
}
