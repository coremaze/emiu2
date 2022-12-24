use super::{instr, opcode::Opcode, DecodedInstruction};
use crate::memory::AddressSpace;

/// A WDC 65C02 CPU core
pub struct Core<A>
where
    A: AddressSpace,
{
    pub cycles: u64,

    pub address_space: A,

    pub registers: Registers,

    pub flags: Flags,
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

impl<A: AddressSpace> Core<A> {
    pub fn new(address_space: A) -> Self {
        Self {
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
        }
    }

    pub fn decode_next_instruction(&mut self) -> DecodedInstruction {
        DecodedInstruction::decode(&mut self.address_space, self.registers.pc.into())
    }

    pub fn step(&mut self) {
        let dins = self.decode_next_instruction();
        let ins = &dins.instruction;

        // The program counter should be incremented before execution.
        // For example, conditional branches use relative addressing, relative
        // to 2 bytes after the beginning of the instruction.
        // println!("{:04X}: {:<16} {ins:?}", self.registers.pc, ins.to_string());
        self.registers.pc = self.registers.pc.wrapping_add(ins.encoded_length() as u16);

        self.execute_instruction(&dins);
    }

    pub fn push_u8(&mut self, val: u8) {
        self.address_space
            .write_u8(self.registers.full_sp() as usize, val);
        self.registers.sp -= 1;
    }

    pub fn pop_u8(&mut self) -> u8 {
        self.registers.sp += 1;
        self.address_space
            .read_u8(self.registers.full_sp() as usize)
    }

    pub fn push_u16(&mut self, val: u16) {
        let low = (val & 0xFF) as u8;
        let high = ((val & 0xFF00) >> 8) as u8;
        self.push_u8(high);
        self.push_u8(low);
    }

    pub fn pop_u16(&mut self) -> u16 {
        let low = self.pop_u8();
        let high = self.pop_u8();

        low as u16 | ((high as u16) << 8)
    }

    fn execute_instruction(&mut self, dec_inst: &DecodedInstruction) {
        let op_fn = match dec_inst.instruction.opcode {
            Opcode::Adc => instr::adc,
            Opcode::And => instr::and,
            Opcode::Asl => todo!(),
            Opcode::Bbr0 => todo!(),
            Opcode::Bbr1 => todo!(),
            Opcode::Bbr2 => todo!(),
            Opcode::Bbr3 => todo!(),
            Opcode::Bbr4 => todo!(),
            Opcode::Bbr5 => todo!(),
            Opcode::Bbr6 => todo!(),
            Opcode::Bbr7 => todo!(),
            Opcode::Bbs0 => todo!(),
            Opcode::Bbs1 => todo!(),
            Opcode::Bbs2 => todo!(),
            Opcode::Bbs3 => todo!(),
            Opcode::Bbs4 => todo!(),
            Opcode::Bbs5 => todo!(),
            Opcode::Bbs6 => todo!(),
            Opcode::Bbs7 => todo!(),
            Opcode::Bcc => instr::bcc,
            Opcode::Bcs => instr::bcs,
            Opcode::Beq => instr::beq,
            Opcode::Bit => todo!(),
            Opcode::Bmi => todo!(),
            Opcode::Bne => instr::bne,
            Opcode::Bpl => todo!(),
            Opcode::Bra => instr::bra,
            Opcode::Brk => todo!(),
            Opcode::Bvc => todo!(),
            Opcode::Bvs => todo!(),
            Opcode::Clc => instr::clc,
            Opcode::Cld => instr::cld,
            Opcode::Cli => instr::cli,
            Opcode::Clv => todo!(),
            Opcode::Cmp => instr::cmp,
            Opcode::Cpx => instr::cpx,
            Opcode::Cpy => instr::cpy,
            Opcode::Dec => instr::dec,
            Opcode::Dex => instr::dex,
            Opcode::Dey => instr::dey,
            Opcode::Eor => todo!(),
            Opcode::Inc => instr::inc,
            Opcode::Inx => instr::inx,
            Opcode::Iny => instr::iny,
            Opcode::Jmp => instr::jmp,
            Opcode::Jsr => instr::jsr,
            Opcode::Lda => instr::lda,
            Opcode::Ldx => instr::ldx,
            Opcode::Ldy => instr::ldy,
            Opcode::Lsr => todo!(),
            Opcode::Nop => instr::nop,
            Opcode::Ora => instr::ora,
            Opcode::Pha => instr::pha,
            Opcode::Php => instr::php,
            Opcode::Phx => todo!(),
            Opcode::Phy => todo!(),
            Opcode::Pla => instr::pla,
            Opcode::Plp => instr::plp,
            Opcode::Plx => todo!(),
            Opcode::Ply => todo!(),
            Opcode::Rmb0 => instr::rmb0,
            Opcode::Rmb1 => instr::rmb1,
            Opcode::Rmb2 => instr::rmb2,
            Opcode::Rmb3 => instr::rmb3,
            Opcode::Rmb4 => instr::rmb4,
            Opcode::Rmb5 => instr::rmb5,
            Opcode::Rmb6 => instr::rmb6,
            Opcode::Rmb7 => instr::rmb7,
            Opcode::Rol => todo!(),
            Opcode::Ror => todo!(),
            Opcode::Rti => todo!(),
            Opcode::Rts => instr::rts,
            Opcode::Sbc => todo!(),
            Opcode::Sec => todo!(),
            Opcode::Sed => instr::sed,
            Opcode::Sei => instr::sei,
            Opcode::Smb0 => instr::smb0,
            Opcode::Smb1 => instr::smb1,
            Opcode::Smb2 => instr::smb2,
            Opcode::Smb3 => instr::smb3,
            Opcode::Smb4 => instr::smb4,
            Opcode::Smb5 => instr::smb5,
            Opcode::Smb6 => instr::smb6,
            Opcode::Smb7 => instr::smb7,
            Opcode::Sta => instr::sta,
            Opcode::Stp => todo!(),
            Opcode::Stx => instr::stx,
            Opcode::Sty => instr::sty,
            Opcode::Stz => instr::stz,
            Opcode::Tax => todo!(),
            Opcode::Tay => todo!(),
            Opcode::Trb => todo!(),
            Opcode::Tsb => todo!(),
            Opcode::Tsx => instr::tsx,
            Opcode::Txa => todo!(),
            Opcode::Txs => instr::txs,
            Opcode::Tya => instr::tya,
            Opcode::Wai => instr::wai,
        };
        let bounds_extra_cycle = op_fn(self, &dec_inst.instruction);

        self.cycles += dec_inst.cycles;
        if bounds_extra_cycle {
            self.cycles += 1;
        }
    }
}
