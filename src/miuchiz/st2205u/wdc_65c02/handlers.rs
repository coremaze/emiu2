//! Fused instruction handlers.
//!
//! Execution dispatches through a 256-entry table indexed by the raw opcode
//! byte. Each entry is a monomorphic function for one (operation,
//! addressing mode) pair: it rebuilds the `AddressingMode` from the raw
//! operand payload and calls the generic instruction implementation, which
//! inlines with the mode statically known — the addressing-mode matches in
//! `instr`/`addr_mode` fold away entirely.
//!
//! The table is built at `Core` construction by decoding every opcode byte,
//! so the decoder remains the single source of truth for which byte means
//! which (operation, mode) pair.

use super::{instr, AddressingMode, Core, DecodedInstruction, HandlesInterrupt, Opcode};
use crate::memory::AddressSpace;

pub type Handler<A> = fn(&mut Core<A>, u16) -> bool;

// Reconstruct an `AddressingMode` from the raw payload produced by
// `AddressingMode::payload`; the two must stay in sync.
macro_rules! mode_ctors {
    ($( $name:ident => $build:expr ),* $(,)?) => {
        $(
            #[inline(always)]
            fn $name(operand: u16) -> AddressingMode {
                let _ = operand;
                $build(operand)
            }
        )*
    };
}

mode_ctors! {
    m_abs => |op| AddressingMode::Absolute(op),
    m_abs_x => |op| AddressingMode::AbsoluteXIndexed(op),
    m_abs_y => |op| AddressingMode::AbsoluteYIndexed(op),
    m_imm => |op: u16| AddressingMode::Immediate(op as u8),
    m_x_ind => |op: u16| AddressingMode::XIndexedIndirect(op as u8),
    m_ind_y => |op: u16| AddressingMode::IndirectYIndexed(op as u8),
    m_rel => |op: u16| AddressingMode::Relative(op as u8 as i8),
    m_zp => |op: u16| AddressingMode::ZeroPage(op as u8),
    m_ind_zp => |op: u16| AddressingMode::IndirectZeroPage(op as u8),
    m_zp_x => |op: u16| AddressingMode::ZeroPageXIndexed(op as u8),
    m_zp_y => |op: u16| AddressingMode::ZeroPageYIndexed(op as u8),
    m_zp_rel => |op: u16| AddressingMode::ZeroPageRelative(op as u8, (op >> 8) as u8 as i8),
    m_impl => |_op| AddressingMode::Implied,
    m_abs_addr => |op| AddressingMode::AbsoluteAddress(op),
    m_ind_addr => |op| AddressingMode::IndirectAddress(op),
    m_abs_x_ind_addr => |op| AddressingMode::AbsoluteXIndexedIndirectAddress(op),
}

/// A handler fusing one instruction implementation with one statically
/// known addressing mode
macro_rules! fused {
    ($instr:path, $ctor:ident) => {{
        fn fused_handler<A: AddressSpace + HandlesInterrupt>(
            core: &mut Core<A>,
            operand: u16,
        ) -> bool {
            $instr(core, &$ctor(operand))
        }
        fused_handler::<A> as Handler<A>
    }};
}

/// Modes shared by the accumulator-operand operations (ADC, AND, CMP, EOR,
/// LDA, ORA, SBC) and the loads/compares with narrower mode sets
macro_rules! mem_read_op {
    ($instr:path, $mode:expr, $op:expr) => {
        match $mode {
            AddressingMode::Immediate(_) => fused!($instr, m_imm),
            AddressingMode::ZeroPage(_) => fused!($instr, m_zp),
            AddressingMode::ZeroPageXIndexed(_) => fused!($instr, m_zp_x),
            AddressingMode::ZeroPageYIndexed(_) => fused!($instr, m_zp_y),
            AddressingMode::Absolute(_) => fused!($instr, m_abs),
            AddressingMode::AbsoluteXIndexed(_) => fused!($instr, m_abs_x),
            AddressingMode::AbsoluteYIndexed(_) => fused!($instr, m_abs_y),
            AddressingMode::XIndexedIndirect(_) => fused!($instr, m_x_ind),
            AddressingMode::IndirectYIndexed(_) => fused!($instr, m_ind_y),
            AddressingMode::IndirectZeroPage(_) => fused!($instr, m_ind_zp),
            mode => unsupported($op, mode),
        }
    };
}

/// Modes shared by the store operations (STA, STX, STY, STZ)
macro_rules! store_op {
    ($instr:path, $mode:expr, $op:expr) => {
        match $mode {
            AddressingMode::ZeroPage(_) => fused!($instr, m_zp),
            AddressingMode::ZeroPageXIndexed(_) => fused!($instr, m_zp_x),
            AddressingMode::ZeroPageYIndexed(_) => fused!($instr, m_zp_y),
            AddressingMode::Absolute(_) => fused!($instr, m_abs),
            AddressingMode::AbsoluteXIndexed(_) => fused!($instr, m_abs_x),
            AddressingMode::AbsoluteYIndexed(_) => fused!($instr, m_abs_y),
            AddressingMode::XIndexedIndirect(_) => fused!($instr, m_x_ind),
            AddressingMode::IndirectYIndexed(_) => fused!($instr, m_ind_y),
            AddressingMode::IndirectZeroPage(_) => fused!($instr, m_ind_zp),
            mode => unsupported($op, mode),
        }
    };
}

/// Modes shared by the read-modify-write operations (ASL, LSR, ROL, ROR,
/// INC, DEC); Implied operates on the accumulator
macro_rules! rmw_op {
    ($instr:path, $mode:expr, $op:expr) => {
        match $mode {
            AddressingMode::Implied => fused!($instr, m_impl),
            AddressingMode::ZeroPage(_) => fused!($instr, m_zp),
            AddressingMode::ZeroPageXIndexed(_) => fused!($instr, m_zp_x),
            AddressingMode::Absolute(_) => fused!($instr, m_abs),
            AddressingMode::AbsoluteXIndexed(_) => fused!($instr, m_abs_x),
            mode => unsupported($op, mode),
        }
    };
}

#[cold]
fn unsupported(opcode: Opcode, mode: &AddressingMode) -> ! {
    panic!("no fused handler for {opcode:?} with addressing mode {mode:?}");
}

fn todo_handler<A: AddressSpace + HandlesInterrupt>() -> Handler<A> {
    fn h<A: AddressSpace + HandlesInterrupt>(_core: &mut Core<A>, _operand: u16) -> bool {
        todo!()
    }
    h::<A> as Handler<A>
}

/// Select the fused handler for one decoded (operation, addressing mode)
/// pair. Combinations the decoder never produces panic at table-build time.
fn select_handler<A: AddressSpace + HandlesInterrupt>(
    opcode: Opcode,
    mode: &AddressingMode,
) -> Handler<A> {
    match opcode {
        Opcode::Adc => mem_read_op!(instr::adc, mode, opcode),
        Opcode::And => mem_read_op!(instr::and, mode, opcode),
        Opcode::Cmp => mem_read_op!(instr::cmp, mode, opcode),
        Opcode::Cpx => mem_read_op!(instr::cpx, mode, opcode),
        Opcode::Cpy => mem_read_op!(instr::cpy, mode, opcode),
        Opcode::Eor => mem_read_op!(instr::eor, mode, opcode),
        Opcode::Lda => mem_read_op!(instr::lda, mode, opcode),
        Opcode::Ldx => mem_read_op!(instr::ldx, mode, opcode),
        Opcode::Ldy => mem_read_op!(instr::ldy, mode, opcode),
        Opcode::Ora => mem_read_op!(instr::ora, mode, opcode),
        Opcode::Sbc => mem_read_op!(instr::sbc, mode, opcode),

        Opcode::Sta => store_op!(instr::sta, mode, opcode),
        Opcode::Stx => store_op!(instr::stx, mode, opcode),
        Opcode::Sty => store_op!(instr::sty, mode, opcode),
        Opcode::Stz => store_op!(instr::stz, mode, opcode),

        Opcode::Asl => rmw_op!(instr::asl, mode, opcode),
        Opcode::Dec => rmw_op!(instr::dec, mode, opcode),
        Opcode::Inc => rmw_op!(instr::inc, mode, opcode),
        Opcode::Lsr => rmw_op!(instr::lsr, mode, opcode),
        Opcode::Rol => rmw_op!(instr::rol, mode, opcode),
        Opcode::Ror => rmw_op!(instr::ror, mode, opcode),

        Opcode::Bcc => fused!(instr::bcc, m_rel),
        Opcode::Bcs => fused!(instr::bcs, m_rel),
        Opcode::Beq => fused!(instr::beq, m_rel),
        Opcode::Bmi => fused!(instr::bmi, m_rel),
        Opcode::Bne => fused!(instr::bne, m_rel),
        Opcode::Bpl => fused!(instr::bpl, m_rel),
        Opcode::Bra => fused!(instr::bra, m_rel),

        Opcode::Bbr0 => fused!(instr::bbr0, m_zp_rel),
        Opcode::Bbr1 => fused!(instr::bbr1, m_zp_rel),
        Opcode::Bbr2 => fused!(instr::bbr2, m_zp_rel),
        Opcode::Bbr3 => fused!(instr::bbr3, m_zp_rel),
        Opcode::Bbr4 => fused!(instr::bbr4, m_zp_rel),
        Opcode::Bbr5 => fused!(instr::bbr5, m_zp_rel),
        Opcode::Bbr6 => fused!(instr::bbr6, m_zp_rel),
        Opcode::Bbr7 => fused!(instr::bbr7, m_zp_rel),
        Opcode::Bbs0 => fused!(instr::bbs0, m_zp_rel),
        Opcode::Bbs1 => fused!(instr::bbs1, m_zp_rel),
        Opcode::Bbs2 => fused!(instr::bbs2, m_zp_rel),
        Opcode::Bbs3 => fused!(instr::bbs3, m_zp_rel),
        Opcode::Bbs4 => fused!(instr::bbs4, m_zp_rel),
        Opcode::Bbs5 => fused!(instr::bbs5, m_zp_rel),
        Opcode::Bbs6 => fused!(instr::bbs6, m_zp_rel),
        Opcode::Bbs7 => fused!(instr::bbs7, m_zp_rel),

        Opcode::Rmb0 => fused!(instr::rmb0, m_zp),
        Opcode::Rmb1 => fused!(instr::rmb1, m_zp),
        Opcode::Rmb2 => fused!(instr::rmb2, m_zp),
        Opcode::Rmb3 => fused!(instr::rmb3, m_zp),
        Opcode::Rmb4 => fused!(instr::rmb4, m_zp),
        Opcode::Rmb5 => fused!(instr::rmb5, m_zp),
        Opcode::Rmb6 => fused!(instr::rmb6, m_zp),
        Opcode::Rmb7 => fused!(instr::rmb7, m_zp),
        Opcode::Smb0 => fused!(instr::smb0, m_zp),
        Opcode::Smb1 => fused!(instr::smb1, m_zp),
        Opcode::Smb2 => fused!(instr::smb2, m_zp),
        Opcode::Smb3 => fused!(instr::smb3, m_zp),
        Opcode::Smb4 => fused!(instr::smb4, m_zp),
        Opcode::Smb5 => fused!(instr::smb5, m_zp),
        Opcode::Smb6 => fused!(instr::smb6, m_zp),
        Opcode::Smb7 => fused!(instr::smb7, m_zp),

        Opcode::Jmp => match mode {
            AddressingMode::AbsoluteAddress(_) => fused!(instr::jmp, m_abs_addr),
            AddressingMode::IndirectAddress(_) => fused!(instr::jmp, m_ind_addr),
            AddressingMode::AbsoluteXIndexedIndirectAddress(_) => {
                fused!(instr::jmp, m_abs_x_ind_addr)
            }
            mode => unsupported(opcode, mode),
        },
        Opcode::Jsr => fused!(instr::jsr, m_abs_addr),

        // NOP ignores its operand in every encoding
        Opcode::Nop => fused!(instr::nop, m_impl),

        Opcode::Clc => fused!(instr::clc, m_impl),
        Opcode::Cld => fused!(instr::cld, m_impl),
        Opcode::Cli => fused!(instr::cli, m_impl),
        Opcode::Clv => fused!(instr::clv, m_impl),
        Opcode::Dex => fused!(instr::dex, m_impl),
        Opcode::Dey => fused!(instr::dey, m_impl),
        Opcode::Inx => fused!(instr::inx, m_impl),
        Opcode::Iny => fused!(instr::iny, m_impl),
        Opcode::Pha => fused!(instr::pha, m_impl),
        Opcode::Php => fused!(instr::php, m_impl),
        Opcode::Phx => fused!(instr::phx, m_impl),
        Opcode::Phy => fused!(instr::phy, m_impl),
        Opcode::Pla => fused!(instr::pla, m_impl),
        Opcode::Plp => fused!(instr::plp, m_impl),
        Opcode::Plx => fused!(instr::plx, m_impl),
        Opcode::Ply => fused!(instr::ply, m_impl),
        Opcode::Rti => fused!(instr::rti, m_impl),
        Opcode::Rts => fused!(instr::rts, m_impl),
        Opcode::Sec => fused!(instr::sec, m_impl),
        Opcode::Sed => fused!(instr::sed, m_impl),
        Opcode::Sei => fused!(instr::sei, m_impl),
        Opcode::Tax => fused!(instr::tax, m_impl),
        Opcode::Tay => fused!(instr::tay, m_impl),
        Opcode::Tsx => fused!(instr::tsx, m_impl),
        Opcode::Txa => fused!(instr::txa, m_impl),
        Opcode::Txs => fused!(instr::txs, m_impl),
        Opcode::Tya => fused!(instr::tya, m_impl),
        Opcode::Wai => fused!(instr::wai, m_impl),

        // Not implemented (as in the previous match-based dispatch): these
        // panic when executed, not at table-build time
        Opcode::Bit
        | Opcode::Brk
        | Opcode::Bvc
        | Opcode::Bvs
        | Opcode::Trb
        | Opcode::Tsb
        | Opcode::Stp => todo_handler(),
    }
}

/// Zero-filled memory for probing the decoder's (operation, mode) mapping
struct ZeroMemory;

impl AddressSpace for ZeroMemory {
    fn read_u8(&mut self, _address: usize) -> u8 {
        0
    }

    fn write_u8(&mut self, _address: usize, _value: u8) {}
}

/// Build the fused handler table: one handler per opcode byte, selected
/// from what the decoder says that byte means
pub fn build_handler_table<A: AddressSpace + HandlesInterrupt>() -> [Handler<A>; 256] {
    let mut zero = ZeroMemory;
    let mut table = [todo_handler::<A>(); 256];
    for (byte, entry) in table.iter_mut().enumerate() {
        let dins = DecodedInstruction::decode_from_byte(byte as u8, &mut zero, 0);
        *entry = select_handler::<A>(dins.instruction.opcode, &dins.instruction.addressing_mode);
    }
    table
}
