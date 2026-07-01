use super::handlers::{self, Handler};
use super::{DecodedInstruction, FetchedInstruction, FetchesDecoded, HandlesInterrupt};
use crate::memory::AddressSpace;

// This core should tick every 2 oscillations
const CYCLE_FREQUENCY_DIVISOR: u64 = 2;

/// A WDC 65C02 CPU core
pub struct Core<A>
where
    A: AddressSpace + HandlesInterrupt,
{
    frequency: u64,

    pub cycles: u64,

    pub address_space: A,

    pub registers: Registers,

    pub flags: Flags,

    pub waiting_for_interrupt: bool,

    /// Fused execution handlers, indexed by raw opcode byte
    handlers: [Handler<A>; 256],
}

#[derive(Default)]
pub struct Flags {
    // There are some more flags: https://www.nesdev.org/wiki/Status_flags#The_B_flag
    pub carry: bool,
    pub zero: bool,
    pub interrupt_disable: bool,
    pub decimal: bool,
    pub overflow: bool,
    pub negative: bool,
}

impl Flags {
    pub fn to_u8(&self) -> u8 {
        let mut p = 0u8;
        p |= self.negative as u8;
        p <<= 1;

        p |= self.overflow as u8;
        p <<= 1;

        p |= 0;
        p <<= 1;

        p |= 0;
        p <<= 1;

        p |= self.decimal as u8;
        p <<= 1;

        p |= self.interrupt_disable as u8;
        p <<= 1;

        p |= self.zero as u8;
        p <<= 1;

        p |= self.carry as u8;

        p
    }

    pub fn from_u8(val: u8) -> Self {
        Self {
            negative: val & 0b10000000 != 0,
            overflow: val & 0b01000000 != 0,
            decimal: val & 0b00001000 != 0,
            interrupt_disable: val & 0b00000100 != 0,
            zero: val & 0b00000010 != 0,
            carry: val & 0b00000001 != 0,
        }
    }
}

pub struct Registers {
    /// Represents the lowest 8 bits of the stack pointer. The next bit is
    /// always 1, so the full stack pointer should range 0x100~0x1FF.
    pub sp: u8,
    /// Pointer to the instruction currently being executed
    pub pc: u16,

    pub a: u8,
    pub x: u8,
    pub y: u8,
}

impl Registers {
    #[inline(always)]
    pub fn full_sp(&self) -> u16 {
        self.sp as u16 | 0x100
    }
}

impl ToString for Registers {
    fn to_string(&self) -> String {
        format!(
            "SP: 0x(1){:02X}; PC: {:04X}; A: {:02X}, X: {:02X}; Y: {:02X}",
            self.sp, self.pc, self.a, self.x, self.y
        )
    }
}

impl<A: AddressSpace + HandlesInterrupt> HandlesInterrupt for Core<A> {
    fn set_interrupted(&mut self, interrupted: bool) {
        self.address_space.set_interrupted(interrupted);
    }

    fn interrupted(&self) -> bool {
        self.address_space.interrupted()
    }
}

impl<A: AddressSpace + HandlesInterrupt> Core<A> {
    pub fn new(frequency: u64, address_space: A) -> Self {
        Self {
            frequency,
            cycles: 0,
            flags: Flags::default(),
            address_space,
            registers: Registers {
                sp: 0,
                pc: 0,
                a: 0,
                x: 0,
                y: 0,
            },
            waiting_for_interrupt: false,
            handlers: handlers::build_handler_table::<A>(),
        }
    }

    pub fn cycles_per_second(&self) -> u64 {
        self.frequency / CYCLE_FREQUENCY_DIVISOR
    }

    pub fn instruction_cycles(&self) -> u64 {
        self.cycles
    }

    pub fn oscillator_cycles(&self) -> u64 {
        self.cycles * CYCLE_FREQUENCY_DIVISOR
    }

    pub fn decode_next_instruction(&mut self) -> DecodedInstruction {
        DecodedInstruction::decode(&mut self.address_space, self.registers.pc.into())
    }

    #[inline]
    pub fn step(&mut self)
    where
        A: FetchesDecoded,
    {
        // handle WAI mode
        if self.waiting_for_interrupt {
            self.cycles += 1;
            return;
        }

        let fins = self.address_space.fetch_decoded(self.registers.pc);
        self.execute_fetched(&fins);
    }

    /// Execute one already-fetched instruction (no WAI handling)
    #[inline(always)]
    pub fn execute_fetched(&mut self, fins: &FetchedInstruction) {
        // The program counter should be incremented before execution.
        // For example, conditional branches use relative addressing, relative
        // to 2 bytes after the beginning of the instruction.
        self.registers.pc = self.registers.pc.wrapping_add(fins.length as u16);

        self.execute_instruction(fins);
    }

    #[inline(always)]
    pub fn push_u8(&mut self, val: u8) {
        self.address_space
            .write_u8(self.registers.full_sp() as usize, val);
        self.registers.sp = self.registers.sp.wrapping_sub(1);
    }

    #[inline(always)]
    pub fn pop_u8(&mut self) -> u8 {
        self.registers.sp = self.registers.sp.wrapping_add(1);
        self.address_space
            .read_u8(self.registers.full_sp() as usize)
    }

    #[inline(always)]
    pub fn push_u16(&mut self, val: u16) {
        let low = (val & 0xFF) as u8;
        let high = ((val & 0xFF00) >> 8) as u8;
        self.push_u8(high);
        self.push_u8(low);
    }

    #[inline(always)]
    pub fn pop_u16(&mut self) -> u16 {
        let low = self.pop_u8();
        let high = self.pop_u8();

        low as u16 | ((high as u16) << 8)
    }

    fn execute_instruction(&mut self, dec_inst: &FetchedInstruction) {
        let op_fn = self.handlers[dec_inst.opcode_byte as usize];
        let bounds_extra_cycle = op_fn(self, dec_inst.operand);

        self.cycles += dec_inst.cycles as u64;
        if bounds_extra_cycle && dec_inst.extra_page_boundary_cycle {
            self.cycles += 1;
        }
    }
}
