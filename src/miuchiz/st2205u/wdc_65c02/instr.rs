use crate::memory::AddressSpace;

use super::{AddressingMode, Core, Flags, Opcode};

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

fn branch<A: AddressSpace>(core: &mut Core<A>, relative_offset: i8) {
    let abs_offset = relative_offset.unsigned_abs() as u16;

    core.registers.pc = if relative_offset.is_positive() {
        core.registers.pc.wrapping_add(abs_offset)
    } else {
        core.registers.pc.wrapping_sub(abs_offset)
    };
}

pub fn jmp<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u16(core);
    core.registers.pc = operand;
    bound_crossed
}

pub fn sei<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags.interrupt_disable = true;
    false
}

pub fn ldx<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);
    core.registers.x = operand;
    core.flags.zero = core.registers.x == 0;
    core.flags.negative = is_negative(core.registers.x);
    bound_crossed
}

pub fn ldy<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);
    core.registers.y = operand;
    core.flags.zero = core.registers.y == 0;
    core.flags.negative = is_negative(core.registers.y);
    bound_crossed
}

pub fn txs<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.sp = core.registers.x;
    false
}

pub fn lda<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);
    core.registers.a = operand;
    core.flags.zero = core.registers.a == 0;
    core.flags.negative = is_negative(core.registers.a);
    bound_crossed
}

fn store<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction, value: u8) -> bool {
    inst.addressing_mode.write_operand_u8(core, value)
}

pub fn sta<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    store(core, inst, core.registers.a)
}

pub fn stx<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    store(core, inst, core.registers.x)
}

pub fn sty<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    store(core, inst, core.registers.y)
}

pub fn stz<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    store(core, inst, 0)
}

pub fn smbx<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction, n: u8) -> bool {
    let (mut operand, _) = inst.addressing_mode.read_operand_u8(core);

    operand |= 1 << n;
    core.flags.zero = operand == 0;

    let _ = inst.addressing_mode.write_operand_u8(core, operand);

    false
}

pub fn smb0<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 0)
}

pub fn smb1<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 1)
}

pub fn smb2<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 2)
}

pub fn smb3<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 3)
}

pub fn smb4<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 4)
}

pub fn smb5<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 5)
}

pub fn smb6<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 6)
}

pub fn smb7<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    smbx(core, inst, 7)
}

pub fn rmbx<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction, n: u8) -> bool {
    let (mut operand, _) = inst.addressing_mode.read_operand_u8(core);

    operand &= !(1 << n);
    core.flags.zero = operand == 0;

    let _ = inst.addressing_mode.write_operand_u8(core, operand);

    false
}

pub fn rmb0<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 0)
}

pub fn rmb1<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 1)
}

pub fn rmb2<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 2)
}

pub fn rmb3<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 3)
}

pub fn rmb4<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 4)
}

pub fn rmb5<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 5)
}

pub fn rmb6<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 6)
}

pub fn rmb7<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    rmbx(core, inst, 7)
}

pub fn wai<A: AddressSpace>(_core: &mut Core<A>, _inst: &Instruction) -> bool {
    // TODO: IMPLEMENT WHEN THERE ARE INTERRUPTS
    // core.registers.pc = core.registers.pc.wrapping_sub(inst.encoded_length() as u16);

    false
}

pub fn inx<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.x = core.registers.x.wrapping_add(1);

    core.flags.zero = core.registers.x == 0;
    core.flags.negative = is_negative(core.registers.x);

    false
}

pub fn iny<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.y = core.registers.y.wrapping_add(1);

    core.flags.zero = core.registers.y == 0;
    core.flags.negative = is_negative(core.registers.y);

    false
}

pub fn bne<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_i8(core);

    if !core.flags.zero {
        branch(core, operand);
        // Extra cycle taken if branch succeeds
        core.cycles += 1;
    }

    bound_crossed
}

pub fn pha<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.push_u8(core.registers.a);
    false
}

pub fn pla<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.a = core.pop_u8();

    core.flags.zero = core.registers.a == 0;
    core.flags.negative = is_negative(core.registers.a);

    false
}

pub fn inc<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (mut operand, _) = inst.addressing_mode.read_operand_u8(core);

    operand = operand.wrapping_add(1);

    core.flags.zero = operand == 0;
    core.flags.negative = is_negative(operand);

    let _ = inst.addressing_mode.write_operand_u8(core, operand);

    false
}

pub fn cmpx<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction, compare_val: u8) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);

    core.flags.carry = compare_val >= operand;
    core.flags.zero = compare_val == operand;
    let sub_result = compare_val.wrapping_sub(operand);
    core.flags.negative = is_negative(sub_result);

    bound_crossed
}

pub fn cmp<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    cmpx(core, inst, core.registers.a)
}

pub fn cpx<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    cmpx(core, inst, core.registers.x)
}

pub fn cpy<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    cmpx(core, inst, core.registers.y)
}

pub fn and<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);

    core.registers.a &= operand;
    core.flags.zero = core.registers.a == 0;
    core.flags.negative = is_negative(core.registers.a);

    bound_crossed
}

pub fn ora<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u8(core);

    core.registers.a |= operand;
    core.flags.zero = core.registers.a == 0;
    core.flags.negative = is_negative(core.registers.a);

    bound_crossed
}

pub fn jsr<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_u16(core);

    // PC increment should be done prior to execution, so pushing this pushes
    // the correct return address
    core.push_u16(core.registers.pc);

    core.registers.pc = operand;

    bound_crossed
}

pub fn nop<A: AddressSpace>(_core: &mut Core<A>, _inst: &Instruction) -> bool {
    false
}

pub fn dec<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (mut operand, _) = inst.addressing_mode.read_operand_u8(core);

    operand = operand.wrapping_sub(1);

    core.flags.zero = operand == 0;
    core.flags.negative = is_negative(operand);

    let _ = inst.addressing_mode.write_operand_u8(core, operand);

    false
}

pub fn rts<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.pc = core.pop_u16();

    false
}

pub fn dex<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.x = core.registers.x.wrapping_sub(1);

    core.flags.zero = core.registers.x == 0;
    core.flags.negative = is_negative(core.registers.x);

    false
}

pub fn dey<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.y = core.registers.y.wrapping_sub(1);

    core.flags.zero = core.registers.y == 0;
    core.flags.negative = is_negative(core.registers.y);

    false
}

pub fn bra<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_i8(core);

    branch(core, operand);

    bound_crossed
}

pub fn bcc<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_i8(core);

    if !core.flags.carry {
        branch(core, operand);
        // Extra cycle taken if branch succeeds
        core.cycles += 1;
    }

    bound_crossed
}

pub fn bcs<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_i8(core);

    if core.flags.carry {
        branch(core, operand);
        // Extra cycle taken if branch succeeds
        core.cycles += 1;
    }

    bound_crossed
}

pub fn cli<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags.interrupt_disable = false;
    false
}

pub fn cld<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags.decimal = false;
    false
}

pub fn sed<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags.decimal = true;
    false
}

pub fn tya<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.a = core.registers.y;

    core.flags.zero = core.registers.a == 0;
    core.flags.negative = is_negative(core.registers.a);

    false
}

pub fn tsx<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.registers.x = core.registers.sp;

    core.flags.zero = core.registers.x == 0;
    core.flags.negative = is_negative(core.registers.x);

    false
}

pub fn beq<A: AddressSpace>(core: &mut Core<A>, inst: &Instruction) -> bool {
    let (operand, bound_crossed) = inst.addressing_mode.read_operand_i8(core);

    if core.flags.zero {
        branch(core, operand);
        // Extra cycle taken if branch succeeds
        core.cycles += 1;
    }

    bound_crossed
}

pub fn php<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.push_u8(core.flags.to_u8());
    false
}

pub fn plp<A: AddressSpace>(core: &mut Core<A>, _inst: &Instruction) -> bool {
    core.flags = Flags::from_u8(core.pop_u8());
    false
}
