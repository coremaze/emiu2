use crate::memory::AddressSpace;

use super::{AddressingMode, Core, Flags, HandlesInterrupt, Opcode};

#[derive(Debug)]
pub struct Instruction {
    pub opcode: Opcode,
    pub addressing_mode: AddressingMode,
}

impl Instruction {
    pub fn encoded_length(&self) -> usize {
        self.opcode.encoded_length() + self.addressing_mode.encoded_length()
    }
}

impl ToString for Instruction {
    fn to_string(&self) -> String {
        let opcode_str = self.opcode.to_string();
        let operand_str = self.addressing_mode.to_string();
        if !operand_str.is_empty() {
            format!("{opcode_str} {operand_str}")
        } else {
            opcode_str
        }
    }
}

fn is_negative(val: u8) -> bool {
    (val & (1 << 7)) != 0
}

fn branch<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, relative_offset: i8) {
    let abs_offset = relative_offset.unsigned_abs() as u16;

    core.registers.pc = if relative_offset.is_positive() {
        core.registers.pc.wrapping_add(abs_offset)
    } else {
        core.registers.pc.wrapping_sub(abs_offset)
    };
}

pub fn jmp<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u16(core);
    core.registers.pc = operand;
    bound_crossed
}

pub fn sei<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags.interrupt_disable = true;
    false
}

pub fn ldx<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);
    core.registers.x = operand;
    core.flags.zero = core.registers.x == 0;
    core.flags.negative = is_negative(core.registers.x);
    bound_crossed
}

pub fn ldy<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);
    core.registers.y = operand;
    core.flags.zero = core.registers.y == 0;
    core.flags.negative = is_negative(core.registers.y);
    bound_crossed
}

pub fn txs<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.sp = core.registers.x;
    false
}

pub fn lda<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);
    core.registers.a = operand;
    core.flags.zero = core.registers.a == 0;
    core.flags.negative = is_negative(core.registers.a);
    bound_crossed
}

fn store<A: AddressSpace + HandlesInterrupt>(
    core: &mut Core<A>,
    inst: &Instruction,
    value: u8,
) -> bool {
    inst.addressing_mode.write_operand_u8(core, value)
}

pub fn sta<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    store(core, inst, core.registers.a)
}

pub fn stx<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    store(core, inst, core.registers.x)
}

pub fn sty<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    store(core, inst, core.registers.y)
}

pub fn stz<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    store(core, inst, 0)
}

pub fn smbx<A: AddressSpace + HandlesInterrupt>(
    core: &mut Core<A>,
    inst: &Instruction,
    n: u8,
) -> bool {
    let (mut operand, _) = inst.addressing_mode.read_operand_u8(core);

    operand |= 1 << n;
    core.flags.zero = operand == 0;

    let _ = inst.addressing_mode.write_operand_u8(core, operand);

    false
}

pub fn smb0<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 0)
}

pub fn smb1<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 1)
}

pub fn smb2<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 2)
}

pub fn smb3<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 3)
}

pub fn smb4<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 4)
}

pub fn smb5<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 5)
}

pub fn smb6<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 6)
}

pub fn smb7<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 7)
}

pub fn rmbx<A: AddressSpace + HandlesInterrupt>(
    core: &mut Core<A>,
    inst: &Instruction,
    n: u8,
) -> bool {
    let (mut operand, _) = inst.addressing_mode.read_operand_u8(core);

    operand &= !(1 << n);
    core.flags.zero = operand == 0;

    let _ = inst.addressing_mode.write_operand_u8(core, operand);

    false
}

pub fn rmb0<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 0)
}

pub fn rmb1<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 1)
}

pub fn rmb2<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 2)
}

pub fn rmb3<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 3)
}

pub fn rmb4<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 4)
}

pub fn rmb5<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 5)
}

pub fn rmb6<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 6)
}

pub fn rmb7<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 7)
}

pub fn wai<A: AddressSpace + HandlesInterrupt>(_core: &mut Core<A>, _inst: &Instruction) -> bool {
    // TODO: IMPLEMENT WHEN THERE ARE INTERRUPTS
    // core.registers.pc = core.registers.pc.wrapping_sub(inst.encoded_length() as u16);

    false
}

pub fn inx<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.x = core.registers.x.wrapping_add(1);

    core.flags.zero = core.registers.x == 0;
    core.flags.negative = is_negative(core.registers.x);

    false
}

pub fn iny<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.y = core.registers.y.wrapping_add(1);

    core.flags.zero = core.registers.y == 0;
    core.flags.negative = is_negative(core.registers.y);

    false
}

pub fn bne<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_i8(core);

    if !core.flags.zero {
        branch(core, operand);
        // Extra cycle taken if branch succeeds
        core.cycles += 1;
    }

    bound_crossed
}

pub fn bmi<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_i8(core);

    if core.flags.negative {
        branch(core, operand);
        // Extra cycle taken if branch succeeds
        core.cycles += 1;
    }

    bound_crossed
}

pub fn bpl<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_i8(core);

    if !core.flags.negative {
        branch(core, operand);
        // Extra cycle taken if branch succeeds
        core.cycles += 1;
    }

    bound_crossed
}

pub fn pha<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.push_u8(core.registers.a);
    false
}

pub fn phx<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.push_u8(core.registers.x);
    false
}

pub fn phy<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.push_u8(core.registers.y);
    false
}

pub fn pla<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.a = core.pop_u8();

    core.flags.zero = core.registers.a == 0;
    core.flags.negative = is_negative(core.registers.a);

    false
}

pub fn plx<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.x = core.pop_u8();

    core.flags.zero = core.registers.x == 0;
    core.flags.negative = is_negative(core.registers.x);

    false
}

pub fn ply<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.y = core.pop_u8();

    core.flags.zero = core.registers.y == 0;
    core.flags.negative = is_negative(core.registers.y);

    false
}

pub fn inc<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (mut operand, _) = inst.addressing_mode.read_operand_u8(core);

    operand = operand.wrapping_add(1);

    core.flags.zero = operand == 0;
    core.flags.negative = is_negative(operand);

    let _ = inst.addressing_mode.write_operand_u8(core, operand);

    false
}

pub fn cmpx<A: AddressSpace + HandlesInterrupt>(
    core: &mut Core<A>,
    inst: &Instruction,
    compare_val: u8,
) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);

    core.flags.carry = compare_val >= operand;
    core.flags.zero = compare_val == operand;
    let sub_result = compare_val.wrapping_sub(operand);
    core.flags.negative = is_negative(sub_result);

    bound_crossed
}

pub fn cmp<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    cmpx(core, inst, core.registers.a)
}

pub fn cpx<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    cmpx(core, inst, core.registers.x)
}

pub fn cpy<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    cmpx(core, inst, core.registers.y)
}

pub fn and<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);

    core.registers.a &= operand;
    core.flags.zero = core.registers.a == 0;
    core.flags.negative = is_negative(core.registers.a);

    bound_crossed
}

pub fn asl<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (mut operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);

    core.flags.carry = operand & (1 << 7) != 0;

    operand <<= 1;

    core.flags.negative = is_negative(operand);
    core.flags.zero = operand == 0;

    inst.addressing_mode.write_operand_u8(core, operand);

    bound_crossed
}

pub fn ora<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);

    core.registers.a |= operand;
    core.flags.zero = core.registers.a == 0;
    core.flags.negative = is_negative(core.registers.a);

    bound_crossed
}

pub fn jsr<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u16(core);

    // PC increment should be done prior to execution, so pushing this pushes
    // the correct return address
    core.push_u16(core.registers.pc);

    core.registers.pc = operand;

    bound_crossed
}

pub fn nop<A: AddressSpace + HandlesInterrupt>(_core: &mut Core<A>, _inst: &Instruction) -> bool {
    false
}

pub fn dec<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (mut operand, _) = inst.addressing_mode.read_operand_u8(core);

    operand = operand.wrapping_sub(1);

    core.flags.zero = operand == 0;
    core.flags.negative = is_negative(operand);

    let _ = inst.addressing_mode.write_operand_u8(core, operand);

    false
}

pub fn rti<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags = Flags::from_u8(core.pop_u8());
    core.registers.pc = core.pop_u16();
    core.address_space.set_interrupted(false);

    false
}

pub fn rts<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.pc = core.pop_u16();

    false
}

pub fn dex<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.x = core.registers.x.wrapping_sub(1);

    core.flags.zero = core.registers.x == 0;
    core.flags.negative = is_negative(core.registers.x);

    false
}

pub fn dey<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.y = core.registers.y.wrapping_sub(1);

    core.flags.zero = core.registers.y == 0;
    core.flags.negative = is_negative(core.registers.y);

    false
}

pub fn bra<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_i8(core);

    branch(core, operand);

    bound_crossed
}

pub fn bcc<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_i8(core);

    if !core.flags.carry {
        branch(core, operand);
        // Extra cycle taken if branch succeeds
        core.cycles += 1;
    }

    bound_crossed
}

pub fn bcs<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_i8(core);

    if core.flags.carry {
        branch(core, operand);
        // Extra cycle taken if branch succeeds
        core.cycles += 1;
    }

    bound_crossed
}

pub fn bbr<A: AddressSpace + HandlesInterrupt>(
    core: &mut Core<A>,
    inst: &Instruction,
    bit: u8,
) -> bool {
    let ((operand, offset), bound_crossed) = inst.addressing_mode.read_operand_u8_i8(core);

    if operand & (1 << bit) == 0 {
        branch(core, offset);
        // Extra cycle taken if branch succeeds
        core.cycles += 1;
    }

    bound_crossed
}

pub fn bbr0<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbr(core, inst, 0)
}

pub fn bbr1<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbr(core, inst, 1)
}

pub fn bbr2<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbr(core, inst, 2)
}

pub fn bbr3<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbr(core, inst, 3)
}

pub fn bbr4<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbr(core, inst, 4)
}

pub fn bbr5<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbr(core, inst, 5)
}

pub fn bbr6<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbr(core, inst, 6)
}

pub fn bbr7<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbr(core, inst, 7)
}

pub fn bbs<A: AddressSpace + HandlesInterrupt>(
    core: &mut Core<A>,
    inst: &Instruction,
    bit: u8,
) -> bool {
    let ((operand, offset), bound_crossed) = inst.addressing_mode.read_operand_u8_i8(core);

    if operand & (1 << bit) != 0 {
        branch(core, offset);
        // Extra cycle taken if branch succeeds
        core.cycles += 1;
    }

    bound_crossed
}

pub fn bbs0<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbs(core, inst, 0)
}

pub fn bbs1<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbs(core, inst, 1)
}

pub fn bbs2<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbs(core, inst, 2)
}

pub fn bbs3<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbs(core, inst, 3)
}

pub fn bbs4<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbs(core, inst, 4)
}

pub fn bbs5<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbs(core, inst, 5)
}

pub fn bbs6<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbs(core, inst, 6)
}

pub fn bbs7<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    bbs(core, inst, 7)
}

pub fn cli<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags.interrupt_disable = false;
    false
}

pub fn clv<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags.overflow = false;
    false
}

pub fn cld<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags.decimal = false;
    false
}

pub fn clc<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags.carry = false;
    false
}

pub fn sed<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags.decimal = true;
    false
}

pub fn sec<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags.carry = true;
    false
}

pub fn tya<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.a = core.registers.y;

    core.flags.zero = core.registers.a == 0;
    core.flags.negative = is_negative(core.registers.a);

    false
}

pub fn tsx<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.x = core.registers.sp;

    core.flags.zero = core.registers.x == 0;
    core.flags.negative = is_negative(core.registers.x);

    false
}

pub fn tay<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.y = core.registers.a;

    core.flags.zero = core.registers.y == 0;
    core.flags.negative = is_negative(core.registers.y);

    false
}

pub fn tax<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.x = core.registers.a;

    core.flags.zero = core.registers.x == 0;
    core.flags.negative = is_negative(core.registers.x);

    false
}

pub fn txa<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.a = core.registers.x;

    core.flags.zero = core.registers.a == 0;
    core.flags.negative = is_negative(core.registers.a);

    false
}

pub fn beq<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_i8(core);

    if core.flags.zero {
        branch(core, operand);
        // Extra cycle taken if branch succeeds
        core.cycles += 1;
    }

    bound_crossed
}

pub fn php<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.push_u8(core.flags.to_u8());
    false
}

pub fn plp<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags = Flags::from_u8(core.pop_u8());
    false
}

pub fn adc<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);

    let mut sum = operand as u16 + core.registers.a as u16 + core.flags.carry as u16;

    if core.flags.decimal {
        let mut low_result =
            (core.registers.a as u16 & 0x0F) + (operand as u16 & 0x0F) + (core.flags.carry as u16);
        if low_result > 9 {
            low_result = ((low_result + 6) & 0x0F) + 16;
        }
        sum = (core.registers.a as u16 & 0xF0) + (operand as u16 & 0xF0) + low_result;
        if sum > 0x90 {
            sum = sum + 0x60;
        }
        core.cycles += 1;
    }

    core.registers.a = (sum & 0xFF) as u8;
    core.flags.carry = sum >= 0x100;
    core.flags.zero = core.registers.a == 0;
    core.flags.negative = is_negative(core.registers.a);

    let c_6 = (((core.registers.a & 0x7F) + (operand & 0x7F) + (core.flags.carry as u8))
        & 0b10000000)
        != 0;
    core.flags.overflow = c_6 ^ core.flags.carry;

    bound_crossed
}

pub fn sbc<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);

    let old_carry = core.flags.carry;

    let mut result = u16::from(core.registers.a) + u16::from(!operand) + old_carry as u16;

    core.flags.carry = result > u8::MAX as u16;
    core.flags.zero = result & 0xFF == 0;
    core.flags.overflow =
        ((core.registers.a ^ operand) & (core.registers.a ^ result as u8) & 0b10000000 as u8) > 0;

    core.flags.negative = is_negative((result & 0xFF) as u8);

    if core.flags.decimal {
        let value = operand as i16;

        let mut sum = (core.registers.a & 0xf) as i16 - (value & 0xf) + old_carry as i16 - 1;
        if sum < 0 {
            sum = ((sum - 0x6) & 0xf) - 0x10;
        }
        let mut sum = (core.registers.a & 0xf0) as i16 - (value & 0xf0) + sum;
        if sum < 0 {
            sum -= 0x60;
        }
        result = (sum & 0xff) as u16;
    }
    core.registers.a = (result & 0xFF) as u8;

    bound_crossed
}

pub fn lsr<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (mut operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);

    core.flags.carry = operand & 1 != 0;
    operand >>= 1;
    core.flags.zero = operand == 0;
    core.flags.negative = is_negative(operand);

    let _ = inst.addressing_mode.write_operand_u8(core, operand);

    bound_crossed
}

pub fn rol<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (mut operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);

    let old_carry = core.flags.carry;
    // Carry if the top bit is set
    core.flags.carry = operand & (1 << 7) != 0;
    operand <<= 1;
    // Set the low bit if carry was previously set
    operand |= if old_carry { 1 } else { 0 };
    core.flags.zero = operand == 0;
    core.flags.negative = is_negative(operand);

    let _ = inst.addressing_mode.write_operand_u8(core, operand);

    bound_crossed
}

pub fn ror<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (mut operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);

    let old_carry = core.flags.carry;
    // Carry if the low bit is set
    core.flags.carry = operand & (1 << 0) != 0;
    operand >>= 1;
    // Set the high bit if carry was previously set
    operand |= if old_carry { (1 << 7) } else { 0 };
    core.flags.zero = operand == 0;
    core.flags.negative = is_negative(operand);

    let _ = inst.addressing_mode.write_operand_u8(core, operand);

    bound_crossed
}

pub fn eor<A: AddressSpace + HandlesInterrupt>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);

    core.registers.a ^= operand;
    core.flags.zero = core.registers.a == 0;
    core.flags.negative = is_negative(core.registers.a);

    bound_crossed
}
