use crate::memory::AddressSpace;

use super::Core;
use super::HandlesInterrupt;

#[derive(Debug)]
pub enum AddressingMode {
    Absolute(u16),                        // OPCODE $WWWW
    AbsoluteXIndexed(u16),                // OPCODE $WWWW,X
    AbsoluteYIndexed(u16),                // OPCODE $WWWW,Y
    AbsoluteXIndexedIndirect(u16),        // OPCODE ($WWWW,X)
    Immediate(u8),                        // OPCODE #$BB
    Indirect(u16),                        // OPCODE ($WWWW)
    XIndexedIndirect(u8),                 // OPCODE ($LL,X)
    IndirectYIndexed(u8),                 // OPCODE ($LL),Y
    Relative(i8),                         // OPCODE $bb
    ZeroPage(u8),                         // OPCODE $LL
    IndirectZeroPage(u8),                 // OPCODE ($LL)
    ZeroPageXIndexed(u8),                 // OPCODE $LL,X
    ZeroPageYIndexed(u8),                 // OPCODE $LL,Y
    ZeroPageRelative(u8, i8),             // OPCODE $BB,$bb
    Implied,                              // OPCODE
    AbsoluteAddress(u16), // OPCODE $WWWW; For JMP and JSR since they do not dereference
    IndirectAddress(u16), // OPCODE ($WWWW); For JMP
    AbsoluteXIndexedIndirectAddress(u16), // OPCODE ($WWWW,X); for JMP
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
            AddressingMode::AbsoluteAddress(_) => 2,
            AddressingMode::IndirectAddress(_) => 2,
            AddressingMode::AbsoluteXIndexedIndirectAddress(_) => 2,
        }
    }

    // Returns the byte read as well as whether a page boundary was crossed
    pub fn read_operand_u8<A: AddressSpace + HandlesInterrupt>(
        &self,
        core: &mut Core<A>,
    ) -> (u8, bool) {
        match &self {
            AddressingMode::Absolute(addr) => (core.address_space.read_u8(*addr as usize), false),
            AddressingMode::AbsoluteXIndexed(addr) => {
                let read_address = addr.wrapping_add(core.registers.x.into());
                let value = core.address_space.read_u8(read_address as usize);
                (value, crosses_page(*addr, read_address))
            }
            AddressingMode::AbsoluteYIndexed(addr) => {
                let read_address = addr.wrapping_add(core.registers.y.into());
                let value = core.address_space.read_u8(read_address as usize);
                (value, crosses_page(*addr, read_address))
            }
            AddressingMode::AbsoluteXIndexedIndirect(_) => todo!(),
            AddressingMode::Immediate(imm) => (*imm, false),
            AddressingMode::Indirect(_) => todo!(),
            AddressingMode::XIndexedIndirect(addr) => {
                // (0,X) should only access ZP, meaning page boundaries can never be crossed
                let offset_addr = addr.wrapping_add(core.registers.x);
                let read_addr = core.address_space.read_u16_le(offset_addr as usize);
                let value = core.address_space.read_u8(read_addr as usize);
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
            AddressingMode::ZeroPageXIndexed(zp_addr) => {
                let value = core
                    .address_space
                    .read_u8(zp_addr.wrapping_add(core.registers.x) as usize);
                (value, false)
            }
            AddressingMode::ZeroPageYIndexed(_) => todo!(),
            AddressingMode::ZeroPageRelative(_, _) => todo!(),
            AddressingMode::Implied => (core.registers.a, false),
            AddressingMode::AbsoluteAddress(_)
            | AddressingMode::IndirectAddress(_)
            | AddressingMode::AbsoluteXIndexedIndirectAddress(_) => {
                panic!("Addressing mode doesn't return u8")
            }
        }
    }

    // Returns the byte read as well as whether a page boundary was crossed
    pub fn read_operand_i8<A: AddressSpace + HandlesInterrupt>(
        &self,
        _core: &mut Core<A>,
    ) -> (i8, bool) {
        match &self {
            AddressingMode::Relative(rel) => (*rel, false),
            _ => {
                panic!("It doesn't make sense to read an i8 with addressing mode {self:?}");
            }
        }
    }

    // Basically for JMP and JSR
    pub fn read_operand_u16<A: AddressSpace + HandlesInterrupt>(
        &self,
        core: &mut Core<A>,
    ) -> (u16, bool) {
        match &self {
            AddressingMode::AbsoluteAddress(addr) => (*addr, false),
            AddressingMode::IndirectAddress(addr) => {
                let value = core.address_space.read_u16_le(*addr as usize);
                (value, false)
            }
            AddressingMode::AbsoluteXIndexedIndirectAddress(addr) => {
                let address_address = addr.wrapping_add(core.registers.x.into());
                println!("{address_address:X}");
                let jmp_addr = core.address_space.read_u16_le(address_address as usize);
                (jmp_addr, crosses_page(*addr, address_address))
            }
            _ => todo!(),
        }
    }

    // BBR and BBS
    pub fn read_operand_u8_i8<A: AddressSpace + HandlesInterrupt>(
        &self,
        core: &mut Core<A>,
    ) -> ((u8, i8), bool) {
        match &self {
            AddressingMode::ZeroPageRelative(zp_addr, offset) => {
                let value = core.address_space.read_u8(*zp_addr as usize);
                ((value, *offset), false)
            }
            _ => todo!(),
        }
    }

    // Returns whether a page boundary was crossed
    pub fn write_operand_u8<A: AddressSpace + HandlesInterrupt>(
        &self,
        core: &mut Core<A>,
        value: u8,
    ) -> bool {
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
            AddressingMode::AbsoluteYIndexed(addr) => {
                let write_address = addr.wrapping_add(core.registers.y.into());
                core.address_space.write_u8(write_address as usize, value);
                crosses_page(*addr, write_address)
            }
            AddressingMode::AbsoluteXIndexedIndirect(_) => todo!(),
            AddressingMode::Immediate(_) => todo!(),
            AddressingMode::Indirect(_) => todo!(),
            AddressingMode::XIndexedIndirect(_) => todo!(),
            AddressingMode::IndirectYIndexed(addr) => {
                let address1 = core.address_space.read_u16_le(*addr as usize);
                let address2 = address1.wrapping_add(core.registers.y.into());
                core.address_space.write_u8(address2 as usize, value);
                crosses_page(address1, address2)
            }
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
            AddressingMode::AbsoluteAddress(_)
            | AddressingMode::IndirectAddress(_)
            | AddressingMode::AbsoluteXIndexedIndirectAddress(_) => {
                panic!("Addressing mode doesn't return u8")
            }
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
            AddressingMode::AbsoluteAddress(addr) => format!("${addr:04X}"),
            AddressingMode::IndirectAddress(addr) => format!("(${addr:04X})"),
            AddressingMode::AbsoluteXIndexedIndirectAddress(addr) => format!("(${addr:04X},X)"),
        }
    }
}

fn crosses_page(addr1: u16, addr2: u16) -> bool {
    addr1 & 0xFF00 != addr2 & 0xFF00
}
