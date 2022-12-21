use crate::memory::AddressSpace;

use super::addr_mode::AddressingMode;
use super::instr::Instruction;
use super::opcode::Opcode;

pub struct DecodedInstruction {
    pub instruction: Instruction,

    /// The number of cycles that the instruction takes to execute under normal conditions
    pub cycles: u64,

    /// Whether this instruction should take an extra cycle if its operand crosses a page boundary
    pub extra_page_boundary_cycle: bool,
}

impl DecodedInstruction {
    /// Determines operation, addressing, and cycle information from an encoded 65C02 instruction
    pub fn decode(memory: &mut impl AddressSpace, offset: usize) -> Self {
        let opcode = memory.read_u8(offset);
        // Cycles for conditional branches should be increased by 1 if taken
        // ADC and SBC should have one more cycle if the decimal flag is set
        match opcode {
            0x00 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Brk,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 7,
                extra_page_boundary_cycle: false,
            },
            0x01 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ora,
                    addressing_mode: AddressingMode::XIndexedIndirect(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x02 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x03 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x04 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Tsb,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x05 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ora,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x06 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Asl,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x07 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rmb0,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x08 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Php,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x09 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ora,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x0A => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Asl,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x0B => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x0C => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Tsb,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x0D => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ora,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x0E => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Asl,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x0F => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbr0,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x10 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bpl,
                    addressing_mode: AddressingMode::Relative(memory.read_u8(offset + 1) as i8),
                },
                cycles: 2,
                extra_page_boundary_cycle: true,
            },
            0x11 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ora,
                    addressing_mode: AddressingMode::IndirectYIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: true,
            },
            0x12 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ora,
                    addressing_mode: AddressingMode::IndirectZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x13 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x14 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Trb,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x15 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ora,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x16 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Asl,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x17 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rmb1,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x18 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Clc,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x19 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ora,
                    addressing_mode: AddressingMode::AbsoluteYIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0x1A => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Inc,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x1B => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x1C => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Trb,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x1D => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ora,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0x1E => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Asl,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: true,
            },
            0x1F => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbr1,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x20 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Jsr,
                    // This should be Absolute, but JMP and JSR do not dereference
                    // the pointer. It is instead used as a literal to set PC to.
                    addressing_mode: AddressingMode::AbsoluteImmediate(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x21 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::And,
                    addressing_mode: AddressingMode::XIndexedIndirect(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x22 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x23 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x24 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bit,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x25 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::And,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x26 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rol,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x27 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rmb2,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x28 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Plp,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x29 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::And,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x2A => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rol,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x2B => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x2C => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bit,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x2D => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::And,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x2E => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rol,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x2F => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbr2,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x30 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bmi,
                    addressing_mode: AddressingMode::Relative(memory.read_u8(offset + 1) as i8),
                },
                cycles: 2,
                extra_page_boundary_cycle: true,
            },
            0x31 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::And,
                    addressing_mode: AddressingMode::IndirectYIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: true,
            },
            0x32 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::And,
                    addressing_mode: AddressingMode::IndirectZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x33 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x34 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bit,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x35 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::And,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x36 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rol,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x37 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rmb3,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x38 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sec,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x39 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::And,
                    addressing_mode: AddressingMode::AbsoluteYIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0x3A => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Dec,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x3B => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x3C => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bit,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0x3D => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::And,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0x3E => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rol,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: true,
            },
            0x3F => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbr3,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x40 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rti,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x41 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Eor,
                    addressing_mode: AddressingMode::XIndexedIndirect(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x42 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x43 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x44 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x45 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Eor,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x46 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lsr,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x47 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rmb4,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x48 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Pha,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x49 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Eor,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x4A => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lsr,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x4B => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x4C => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Jmp,
                    // This should be Absolute, but JMP and JSR do not dereference
                    // the pointer. It is instead used as a literal to set PC to.
                    addressing_mode: AddressingMode::AbsoluteImmediate(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x4D => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Eor,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x4E => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lsr,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x4F => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbr4,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x50 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bvc,
                    addressing_mode: AddressingMode::Relative(memory.read_u8(offset + 1) as i8),
                },
                cycles: 2,
                extra_page_boundary_cycle: true,
            },
            0x51 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Eor,
                    addressing_mode: AddressingMode::IndirectYIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: true,
            },
            0x52 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Eor,
                    addressing_mode: AddressingMode::IndirectZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x53 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x54 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x55 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Eor,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x56 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lsr,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x57 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rmb5,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x58 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cli,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x59 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Eor,
                    addressing_mode: AddressingMode::AbsoluteYIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0x5A => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Phy,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x5B => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x5C => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 8,
                extra_page_boundary_cycle: false,
            },
            0x5D => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Eor,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0x5E => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lsr,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: true,
            },
            0x5F => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbr5,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x60 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rts,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x61 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Adc,
                    addressing_mode: AddressingMode::XIndexedIndirect(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x62 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x63 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x64 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Stz,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x65 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Adc,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x66 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ror,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x67 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rmb6,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x68 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Pla,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x69 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Adc,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x6A => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ror,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x6B => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x6C => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Jmp,
                    addressing_mode: AddressingMode::Indirect(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x6D => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Adc,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x6E => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ror,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x6F => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbr6,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x70 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bvs,
                    addressing_mode: AddressingMode::Relative(memory.read_u8(offset + 1) as i8),
                },
                cycles: 2,
                extra_page_boundary_cycle: true,
            },
            0x71 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Adc,
                    addressing_mode: AddressingMode::IndirectYIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: true,
            },
            0x72 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Adc,
                    addressing_mode: AddressingMode::IndirectZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x73 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x74 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Stz,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x75 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Adc,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x76 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ror,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x77 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Rmb7,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x78 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sei,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x79 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Adc,
                    addressing_mode: AddressingMode::AbsoluteYIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0x7A => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ply,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x7B => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x7C => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Jmp,
                    addressing_mode: AddressingMode::AbsoluteXIndexedIndirect(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x7D => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Adc,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0x7E => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ror,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: true,
            },
            0x7F => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbr7,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x80 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bra,
                    addressing_mode: AddressingMode::Relative(memory.read_u8(offset + 1) as i8),
                },
                cycles: 3,
                extra_page_boundary_cycle: true,
            },
            0x81 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sta,
                    addressing_mode: AddressingMode::XIndexedIndirect(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x82 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x83 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x84 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sty,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x85 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sta,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x86 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Stx,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0x87 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Smb0,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x88 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Dey,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x89 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bit,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x8A => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Txa,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x8B => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x8C => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sty,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x8D => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sta,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x8E => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Stx,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x8F => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbs0,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x90 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bcc,
                    addressing_mode: AddressingMode::Relative(memory.read_u8(offset + 1) as i8),
                },
                cycles: 2,
                extra_page_boundary_cycle: true,
            },
            0x91 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sta,
                    addressing_mode: AddressingMode::IndirectYIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0x92 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sta,
                    addressing_mode: AddressingMode::IndirectZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x93 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x94 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sty,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x95 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sta,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x96 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Stx,
                    addressing_mode: AddressingMode::ZeroPageYIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x97 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Smb1,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x98 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Tya,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x99 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sta,
                    addressing_mode: AddressingMode::AbsoluteYIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x9A => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Txs,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0x9B => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0x9C => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Stz,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0x9D => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sta,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x9E => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Stz,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0x9F => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbs1,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xA0 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ldy,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xA1 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lda,
                    addressing_mode: AddressingMode::XIndexedIndirect(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0xA2 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ldx,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xA3 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0xA4 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ldy,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0xA5 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lda,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0xA6 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ldx,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0xA7 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Smb2,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xA8 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Tay,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xA9 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lda,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xAA => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Tax,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xAB => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0xAC => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ldy,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xAD => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lda,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xAE => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ldx,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xAF => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbs2,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xB0 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bcs,
                    addressing_mode: AddressingMode::Relative(memory.read_u8(offset + 1) as i8),
                },
                cycles: 2,
                extra_page_boundary_cycle: true,
            },
            0xB1 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lda,
                    addressing_mode: AddressingMode::IndirectYIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: true,
            },
            0xB2 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lda,
                    addressing_mode: AddressingMode::IndirectZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xB3 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0xB4 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ldy,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xB5 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lda,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xB6 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ldx,
                    addressing_mode: AddressingMode::ZeroPageYIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xB7 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Smb3,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xB8 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Clv,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xB9 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lda,
                    addressing_mode: AddressingMode::AbsoluteYIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0xBA => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Tsx,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xBB => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0xBC => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ldy,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0xBD => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Lda,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0xBE => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Ldx,
                    addressing_mode: AddressingMode::AbsoluteYIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0xBF => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbs3,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xC0 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cpy,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xC1 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cmp,
                    addressing_mode: AddressingMode::XIndexedIndirect(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0xC2 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xC3 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0xC4 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cpy,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0xC5 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cmp,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0xC6 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Dec,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xC7 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Smb4,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xC8 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Iny,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xC9 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cmp,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xCA => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Dex,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xCB => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Wai,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0xCC => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cpy,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xCD => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cmp,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xCE => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Dec,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0xCF => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbs4,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xD0 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bne,
                    addressing_mode: AddressingMode::Relative(memory.read_u8(offset + 1) as i8),
                },
                cycles: 2,
                extra_page_boundary_cycle: true,
            },
            0xD1 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cmp,
                    addressing_mode: AddressingMode::IndirectYIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: true,
            },
            0xD2 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cmp,
                    addressing_mode: AddressingMode::IndirectZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xD3 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0xD4 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xD5 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cmp,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xD6 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Dec,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0xD7 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Smb5,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xD8 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cld,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xD9 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cmp,
                    addressing_mode: AddressingMode::AbsoluteYIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0xDA => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Phx,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0xDB => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Stp,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0xDC => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xDD => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cmp,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0xDE => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Dec,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 7,
                extra_page_boundary_cycle: false,
            },
            0xDF => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbs5,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xE0 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cpx,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xE1 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sbc,
                    addressing_mode: AddressingMode::XIndexedIndirect(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0xE2 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xE3 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0xE4 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cpx,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0xE5 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sbc,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 3,
                extra_page_boundary_cycle: false,
            },
            0xE6 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Inc,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xE7 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Smb6,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xE8 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Inx,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xE9 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sbc,
                    addressing_mode: AddressingMode::Immediate(memory.read_u8(offset + 1)),
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xEA => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xEB => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0xEC => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Cpx,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xED => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sbc,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xEE => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Inc,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0xEF => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbs6,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xF0 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Beq,
                    addressing_mode: AddressingMode::Relative(memory.read_u8(offset + 1) as i8),
                },
                cycles: 2,
                extra_page_boundary_cycle: true,
            },
            0xF1 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sbc,
                    addressing_mode: AddressingMode::IndirectYIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: true,
            },
            0xF2 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sbc,
                    addressing_mode: AddressingMode::IndirectZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xF3 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0xF4 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xF5 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sbc,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xF6 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Inc,
                    addressing_mode: AddressingMode::ZeroPageXIndexed(memory.read_u8(offset + 1)),
                },
                cycles: 6,
                extra_page_boundary_cycle: false,
            },
            0xF7 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Smb7,
                    addressing_mode: AddressingMode::ZeroPage(memory.read_u8(offset + 1)),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
            0xF8 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sed,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 2,
                extra_page_boundary_cycle: false,
            },
            0xF9 => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sbc,
                    addressing_mode: AddressingMode::AbsoluteYIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0xFA => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Plx,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xFB => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Implied,
                },
                cycles: 1,
                extra_page_boundary_cycle: false,
            },
            0xFC => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Nop,
                    addressing_mode: AddressingMode::Absolute(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: false,
            },
            0xFD => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Sbc,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 4,
                extra_page_boundary_cycle: true,
            },
            0xFE => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Inc,
                    addressing_mode: AddressingMode::AbsoluteXIndexed(to_word(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2),
                    )),
                },
                cycles: 7,
                extra_page_boundary_cycle: false,
            },
            0xFF => DecodedInstruction {
                instruction: Instruction {
                    opcode: Opcode::Bbs7,
                    addressing_mode: AddressingMode::ZeroPageRelative(
                        memory.read_u8(offset + 1),
                        memory.read_u8(offset + 2) as i8,
                    ),
                },
                cycles: 5,
                extra_page_boundary_cycle: false,
            },
        }
    }
}

fn to_word(low: u8, high: u8) -> u16 {
    ((high as u16) << 8) | (low as u16)
}
