use crate::memory::AddressSpace;

use super::Core;

#[derive(Debug)]
pub enum AddressingMode {
    Absolute(u16),                 // OPCODE $WWWW
    AbsoluteXIndexed(u16),         // OPCODE $WWWW,X
    AbsoluteYIndexed(u16),         // OPCODE $WWWW,Y
    AbsoluteXIndexedIndirect(u16), // OPCODE ($WWWW,X)
    Immediate(u8),                 // OPCODE #$BB
    Indirect(u16),                 // OPCODE ($WWWW)
    XIndexedIndirect(u8),          // OPCODE ($LL,X)
    IndirectYIndexed(u8),          // OPCODE ($LL),Y
    Relative(i8),                  // OPCODE $bb
    ZeroPage(u8),                  // OPCODE $LL
    IndirectZeroPage(u8),          // OPCODE ($LL)
    ZeroPageXIndexed(u8),          // OPCODE $LL,X
    ZeroPageYIndexed(u8),          // OPCODE $LL,Y
    ZeroPageRelative(u8, i8),      // OPCODE $BB,$bb
    Implied,                       // OPCODE
    AbsoluteImmediate(u16),        // OPCODE $WWWW; For JMP and JSR since they do not dereference
    JmpIndirect(u16), // OPCODE ($WWWW); For JMP since it just gets the 16 bit value at $WWWW, instead of 8 bits at ($WWWW)
}

impl AddressingMode {
    pub fn encoded_length(&self) -> usize {
        match &self {
            AddressingMode::Absolute(_) => 2,
            AddressingMode::AbsoluteXIndexed(_) => 2,
            AddressingMode::AbsoluteYIndexed(_) => 2,
            AddressingMode::AbsoluteXIndexedIndirect(_) => 2,
            AddressingMode::Immediate(_) => 1,
            AddressingMode::Indirect(_) => 2,
            AddressingMode::XIndexedIndirect(_) => 1,
            AddressingMode::IndirectYIndexed(_) => 1,
            AddressingMode::Relative(_) => 1,
            AddressingMode::ZeroPage(_) => 1,
            AddressingMode::IndirectZeroPage(_) => 1,
            AddressingMode::ZeroPageXIndexed(_) => 1,
            AddressingMode::ZeroPageYIndexed(_) => 1,
            AddressingMode::ZeroPageRelative(_, _) => 2,
            AddressingMode::Implied => 0,
            AddressingMode::AbsoluteImmediate(_) => 2,
            AddressingMode::JmpIndirect(_) => 2,
        }
    }

    // Returns the byte read as well as whether a page boundary was crossed
    pub fn read_operand_u8<A: AddressSpace>(&self, core: &mut Core<A>) -> (u8, bool) {
        match &self {
            AddressingMode::Absolute(addr) => (core.address_space.read_u8(*addr as usize), false),
            AddressingMode::AbsoluteXIndexed(addr) => {
                let read_address = addr.wrapping_add(core.registers.x.into());
                let value = core.address_space.read_u8(read_address as usize);
                (value, crosses_page(*addr, read_address))
            }
            AddressingMode::AbsoluteYIndexed(_) => todo!(),
            AddressingMode::AbsoluteXIndexedIndirect(_) => todo!(),
            AddressingMode::Immediate(imm) => (*imm, false),
            AddressingMode::Indirect(addr) => todo!(),
            AddressingMode::XIndexedIndirect(addr) => {
                // (0,X) should only access ZP, meaning page boundaries can never be crossed
                let read_address = addr.wrapping_add(core.registers.x);
                let value = core.address_space.read_u8(read_address as usize);
                (value, false)
            }
            AddressingMode::IndirectYIndexed(addr) => {
                let ptr = core.address_space.read_u16_le(*addr as usize);
                let ptr_offset = ptr.wrapping_add(core.registers.y.into());
                let value = core.address_space.read_u8(ptr_offset as usize);
                (value, crosses_page(ptr, ptr_offset))
            }
            AddressingMode::Relative(_) => todo!(),
            AddressingMode::ZeroPage(zp_addr) => {
                (core.address_space.read_u8(*zp_addr as usize), false)
            }
            AddressingMode::IndirectZeroPage(zp_addr) => {
                let read_address = core.address_space.read_u16_le(*zp_addr as usize);
                (core.address_space.read_u8(read_address as usize), false)
            }
            AddressingMode::ZeroPageXIndexed(_) => todo!(),
            AddressingMode::ZeroPageYIndexed(_) => todo!(),
            AddressingMode::ZeroPageRelative(_, _) => todo!(),
            AddressingMode::Implied => (core.registers.a, false),
            AddressingMode::AbsoluteImmediate(_) => todo!(),
            AddressingMode::JmpIndirect(_) => todo!(),
        }
    }

    // Returns the byte read as well as whether a page boundary was crossed
    pub fn read_operand_i8<A: AddressSpace>(&self, _core: &mut Core<A>) -> (i8, bool) {
        match &self {
            AddressingMode::Relative(rel) => (*rel, false),
            _ => {
                panic!("It doesn't make sense to read an i8 with addressing mode {self:?}");
            }
        }
    }

    // Basically for JMP and JSR
    pub fn read_operand_u16<A: AddressSpace>(&self, core: &mut Core<A>) -> (u16, bool) {
        match &self {
            AddressingMode::AbsoluteImmediate(addr) => (*addr, false),
            AddressingMode::JmpIndirect(addr) => {
                let value = core.address_space.read_u16_le(*addr as usize);
                (value, false)
            }
            _ => {
                let (value, crossed_boundary) = self.read_operand_u8(core);
                (value.into(), crossed_boundary)
            }
        }
    }

    // Returns whether a page boundary was crossed
    pub fn write_operand_u8<A: AddressSpace>(&self, core: &mut Core<A>, value: u8) -> bool {
        match &self {
            AddressingMode::Absolute(addr) => {
                core.address_space.write_u8(*addr as usize, value);
                false
            }
            AddressingMode::AbsoluteXIndexed(addr) => {
                let write_address = addr.wrapping_add(core.registers.x.into());
                core.address_space.write_u8(write_address as usize, value);
                crosses_page(*addr, write_address)
            }
            AddressingMode::AbsoluteYIndexed(_) => todo!(),
            AddressingMode::AbsoluteXIndexedIndirect(_) => todo!(),
            AddressingMode::Immediate(_) => todo!(),
            AddressingMode::Indirect(_) => todo!(),
            AddressingMode::XIndexedIndirect(_) => todo!(),
            AddressingMode::IndirectYIndexed(_) => todo!(),
            AddressingMode::Relative(_) => todo!(),
            AddressingMode::ZeroPage(zp_addr) => {
                core.address_space.write_u8(*zp_addr as usize, value);
                false
            }
            AddressingMode::IndirectZeroPage(zp_addr) => {
                let write_address = core.address_space.read_u16_le(*zp_addr as usize);
                core.address_space.write_u8(write_address as usize, value);
                false
            }
            AddressingMode::ZeroPageXIndexed(zp_addr) => {
                core.address_space
                    .write_u8(zp_addr.wrapping_add(core.registers.x) as usize, value);
                false
            }
            AddressingMode::ZeroPageYIndexed(_) => todo!(),
            AddressingMode::ZeroPageRelative(_, _) => todo!(),
            AddressingMode::Implied => {
                core.registers.a = value;
                false
            }
            AddressingMode::AbsoluteImmediate(_) => todo!(),
            AddressingMode::JmpIndirect(_) => todo!(),
        }
    }
}

impl ToString for AddressingMode {
    fn to_string(&self) -> String {
        match &self {
            AddressingMode::Absolute(addr) => format!("${addr:04X}"),
            AddressingMode::AbsoluteXIndexed(addr) => format!("${addr:04X},X"),
            AddressingMode::AbsoluteYIndexed(addr) => format!("${addr:04X},Y"),
            AddressingMode::AbsoluteXIndexedIndirect(addr) => format!("(${addr:04X},X)"),
            AddressingMode::Immediate(byte) => format!("#${byte:02X}"),
            AddressingMode::Indirect(addr) => format!("(${addr:04X})"),
            AddressingMode::XIndexedIndirect(addr) => format!("(${addr:02X},X)"),
            AddressingMode::IndirectYIndexed(addr) => format!("(${addr:02X}),Y"),
            AddressingMode::Relative(offset) => {
                if *offset < 0 {
                    format!("${offset:02X}")
                } else {
                    format!("+${offset:02X}")
                }
            }
            AddressingMode::ZeroPage(addr) => format!("${addr:02X}"),
            AddressingMode::IndirectZeroPage(addr) => format!("(${addr:02X})"),
            AddressingMode::ZeroPageXIndexed(addr) => format!("${addr:02X},X"),
            AddressingMode::ZeroPageYIndexed(addr) => format!("${addr:02X},Y"),
            AddressingMode::ZeroPageRelative(zp_addr, offset) => {
                if *offset < 0 {
                    format!("${zp_addr:02X},${offset:02X}")
                } else {
                    format!("${zp_addr:02X},+${offset:02X}")
                }
            }
            AddressingMode::Implied => "".to_owned(),
            AddressingMode::AbsoluteImmediate(addr) => format!("${addr:04X}"),
            AddressingMode::JmpIndirect(addr) => format!("(${addr:04X})"),
        }
    }
}

fn crosses_page(addr1: u16, addr2: u16) -> bool {
    addr1 & 0xFF00 != addr2 & 0xFF00
}
