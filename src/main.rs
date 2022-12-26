mod display;
pub mod memory;
mod miuchiz;
use display::Screen;

use display::Pixel;

fn main() {
    let screen = display::MiniFbScreen::open("emiu2", 98, 67, 3);

    let otp_data = std::fs::read("OTP.dat").unwrap();
    let flash_data = std::fs::read("Flash.dat").unwrap();
    let mut handheld = miuchiz::Handheld::new(&otp_data, &flash_data, &screen).unwrap();

    for _count in 0u64..800_000_000 {
        let inst = handheld.mcu.core.decode_next_instruction();
        let pc = handheld.mcu.core.registers.pc;
        // println!("{pc:04X}: {}", inst.instruction.to_string());
        handheld.mcu.step();
        // println!("{}", handheld.mcu.core.registers.to_string());
        // if pc == 0x5972 {
        //     break;
        // }
        if !screen.is_open() {
            break;
        }
    }
    println!("{} cycles", handheld.mcu.core.cycles);
}
