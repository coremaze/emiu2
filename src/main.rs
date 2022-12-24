pub mod memory;
mod miuchiz;

fn main() {
    let otp_data = std::fs::read("OTP.dat").unwrap();
    let flash_data = std::fs::read("Flash.dat").unwrap();
    let mut handheld = miuchiz::Handheld::new(&otp_data, &flash_data).unwrap();

    for _count in 0u64..800_000_000 {
        let inst = handheld.mcu.core.decode_next_instruction();
        let pc = handheld.mcu.core.registers.pc;
        // println!("{pc:04X}: {}", inst.instruction.to_string());
        handheld.mcu.step();
        // println!("{}", handheld.mcu.core.registers.to_string());
        // if pc == 0x5972 {
        //     break;
        // }
    }
    println!("{} cycles", handheld.mcu.core.cycles);
}
