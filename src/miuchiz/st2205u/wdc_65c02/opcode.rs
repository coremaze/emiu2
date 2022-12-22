#[derive(Debug)]
pub enum Opcode {
    Adc,  // Add with carry
    And,  // Bitwise AND
    Asl,  // Arithmetic shift left
    Bbr0, // Branch if bit 0 reset
    Bbr1, // Branch if bit 1 reset
    Bbr2, // Branch if bit 2 reset
    Bbr3, // Branch if bit 3 reset
    Bbr4, // Branch if bit 4 reset
    Bbr5, // Branch if bit 5 reset
    Bbr6, // Branch if bit 6 reset
    Bbr7, // Branch if bit 7 reset
    Bbs0, // Branch if bit 0 set
    Bbs1, // Branch if bit 1 set
    Bbs2, // Branch if bit 2 set
    Bbs3, // Branch if bit 3 set
    Bbs4, // Branch if bit 4 set
    Bbs5, // Branch if bit 5 set
    Bbs6, // Branch if bit 6 set
    Bbs7, // Branch if bit 7 set
    Bcc,  // Branch if carry clear
    Bcs,  // Branch if carry set
    Beq,  // Branch if equal (Branch if zero set)
    Bit,  // Bit test
    Bmi,  // Branch if minus (Branch if negative set)
    Bne,  // Branch if not equal (Branch if zero clear)
    Bpl,  // Branch if plus (Branch if negative clear)
    Bra,  // Branch always
    Brk,  // Break
    Bvc,  // Branch if overflow clear
    Bvs,  // Branch if overflow set
    Clc,  // Clear carry
    Cld,  // Clear decimal
    Cli,  // Clear interrupt disable
    Clv,  // Clear overflow
    Cmp,  // Compare
    Cpx,  // Compare X
    Cpy,  // Compare Y
    Dec,  // Decrement
    Dex,  // Decrement X
    Dey,  // Decrement Y
    Eor,  // Bitwise exclusive OR
    Inc,  // Increment
    Inx,  // Increment X
    Iny,  // Increment Y
    Jmp,  // Jump
    Jsr,  // Jump to subroutine
    Lda,  // Load A
    Ldx,  // Load X
    Ldy,  // Load Y
    Lsr,  // Logical shift right
    Nop,  // No operation
    Ora,  // Bitwise OR with accumulator
    Pha,  // Push accumulator
    Php,  // Push processor status
    Phx,  // Push X
    Phy,  // Push Y
    Pla,  // Pull A
    Plp,  // Pull processor status
    Plx,  // Pull X
    Ply,  // Pull Y
    Rmb0, // Reset memory bit 0
    Rmb1, // Reset memory bit 1
    Rmb2, // Reset memory bit 2
    Rmb3, // Reset memory bit 3
    Rmb4, // Reset memory bit 4
    Rmb5, // Reset memory bit 5
    Rmb6, // Reset memory bit 6
    Rmb7, // Reset memory bit 7
    Rol,  // Rotate left
    Ror,  // Rotate right
    Rti,  // Return from interrupt
    Rts,  // Return from subroutine
    Sbc,  // Subtract with carry
    Sec,  // Set carry
    Sed,  // Set decimal
    Sei,  // Set interrupt disable
    Smb0, // Set memory bit 0
    Smb1, // Set memory bit 1
    Smb2, // Set memory bit 2
    Smb3, // Set memory bit 3
    Smb4, // Set memory bit 4
    Smb5, // Set memory bit 5
    Smb6, // Set memory bit 6
    Smb7, // Set memory bit 7
    Sta,  // Store A
    Stp,  // Stop the processor
    Stx,  // Store X
    Sty,  // Store Y
    Stz,  // Store zero
    Tax,  // Transfer A to X
    Tay,  // Transfer A to Y
    Trb,  // Test and reset bits
    Tsb,  // Test and set bits
    Tsx,  // Transfer stack pointer to X
    Txa,  // Transfer X to A
    Txs,  // Transfer X to stack pointer
    Tya,  // Transfer Y to A
    Wai,  // Wait for interrupt
}

impl Opcode {
    pub fn encoded_length(&self) -> usize {
        1
    }
}

impl ToString for Opcode {
    fn to_string(&self) -> String {
        match &self {
            Opcode::Adc => "ADC",
            Opcode::And => "AND",
            Opcode::Asl => "ASL",
            Opcode::Bbr0 => "BBR0",
            Opcode::Bbr1 => "BBR1",
            Opcode::Bbr2 => "BBR2",
            Opcode::Bbr3 => "BBR3",
            Opcode::Bbr4 => "BBR4",
            Opcode::Bbr5 => "BBR5",
            Opcode::Bbr6 => "BBR6",
            Opcode::Bbr7 => "BBR7",
            Opcode::Bbs0 => "BBS0",
            Opcode::Bbs1 => "BBS1",
            Opcode::Bbs2 => "BBS2",
            Opcode::Bbs3 => "BBS3",
            Opcode::Bbs4 => "BBS4",
            Opcode::Bbs5 => "BBS5",
            Opcode::Bbs6 => "BBS6",
            Opcode::Bbs7 => "BBS7",
            Opcode::Bcc => "BCC",
            Opcode::Bcs => "BCS",
            Opcode::Beq => "BEQ",
            Opcode::Bit => "BIT",
            Opcode::Bmi => "BMI",
            Opcode::Bne => "BNE",
            Opcode::Bpl => "BPL",
            Opcode::Bra => "BRA",
            Opcode::Brk => "BRK",
            Opcode::Bvc => "BVC",
            Opcode::Bvs => "BVS",
            Opcode::Clc => "CLC",
            Opcode::Cld => "CLD",
            Opcode::Cli => "CLI",
            Opcode::Clv => "CLV",
            Opcode::Cmp => "CMP",
            Opcode::Cpx => "CPX",
            Opcode::Cpy => "CPY",
            Opcode::Dec => "DEC",
            Opcode::Dex => "DEX",
            Opcode::Dey => "DEY",
            Opcode::Eor => "EOR",
            Opcode::Inc => "INC",
            Opcode::Inx => "INX",
            Opcode::Iny => "INY",
            Opcode::Jmp => "JMP",
            Opcode::Jsr => "JSR",
            Opcode::Lda => "LDA",
            Opcode::Ldx => "LDX",
            Opcode::Ldy => "LDY",
            Opcode::Lsr => "LSR",
            Opcode::Nop => "NOP",
            Opcode::Ora => "ORA",
            Opcode::Pha => "PHA",
            Opcode::Php => "PHP",
            Opcode::Phx => "PHX",
            Opcode::Phy => "PHY",
            Opcode::Pla => "PLA",
            Opcode::Plp => "PLP",
            Opcode::Plx => "PLX",
            Opcode::Ply => "PLY",
            Opcode::Rmb0 => "RMB0",
            Opcode::Rmb1 => "RMB1",
            Opcode::Rmb2 => "RMB2",
            Opcode::Rmb3 => "RMB3",
            Opcode::Rmb4 => "RMB4",
            Opcode::Rmb5 => "RMB5",
            Opcode::Rmb6 => "RMB6",
            Opcode::Rmb7 => "RMB7",
            Opcode::Rol => "ROL",
            Opcode::Ror => "ROR",
            Opcode::Rti => "RTI",
            Opcode::Rts => "RTS",
            Opcode::Sbc => "SBC",
            Opcode::Sec => "SEC",
            Opcode::Sed => "SED",
            Opcode::Sei => "SEI",
            Opcode::Smb0 => "SMB0",
            Opcode::Smb1 => "SMB1",
            Opcode::Smb2 => "SMB2",
            Opcode::Smb3 => "SMB3",
            Opcode::Smb4 => "SMB4",
            Opcode::Smb5 => "SMB5",
            Opcode::Smb6 => "SMB6",
            Opcode::Smb7 => "SMB7",
            Opcode::Sta => "STA",
            Opcode::Stp => "STP",
            Opcode::Stx => "STX",
            Opcode::Sty => "STY",
            Opcode::Stz => "STZ",
            Opcode::Tax => "TAX",
            Opcode::Tay => "TAY",
            Opcode::Trb => "TRB",
            Opcode::Tsb => "TSB",
            Opcode::Tsx => "TSX",
            Opcode::Txa => "TXA",
            Opcode::Txs => "TXS",
            Opcode::Tya => "TYA",
            Opcode::Wai => "WAI",
        }
        .to_owned()
    }
}
