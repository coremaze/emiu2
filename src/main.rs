mod display;
mod gpio;
pub mod memory;
mod miuchiz;

fn main() {
    let screen = display::MiniFbScreen::open("emiu2", 98, 67, 3);

    let otp_data = std::fs::read("OTP.dat").unwrap();
    let flash_data = std::fs::read("Flash.dat").unwrap();
    let mut handheld = miuchiz::Handheld::new(&otp_data, &flash_data, &screen, &screen).unwrap();
    // std::thread::sleep(std::time::Duration::from_secs(3));

    let beginning = std::time::Instant::now();

    while screen.is_open() {
        let now = std::time::Instant::now();
        let elapsed = now - beginning;
        let microseconds = elapsed.as_nanos();
        let cycles_required_so_far =
            (microseconds * handheld.mcu.core.cycles_per_second() as u128) / 1000000000;

        // println!("need to do {cycles_to_do:?} cycles");
        while (handheld.mcu.core.cycles as u128) < cycles_required_so_far {
            // let pc = handheld.mcu.core.registers.pc;
            // let inst = handheld.mcu.core.decode_next_instruction();
            // println!("{pc:04X}: {}", inst.instruction.to_string());
            handheld.mcu.step();
        }

        std::thread::sleep(std::time::Duration::from_nanos(1));
    }

    // for _count in 0u64..800_000_000 {
    //     // let inst = handheld.mcu.core.decode_next_instruction();
    //     let pc = handheld.mcu.core.registers.pc;
    //     // println!("{pc:04X}: {}", inst.instruction.to_string());
    //     handheld.mcu.step();
    //     // println!("{}", handheld.mcu.core.registers.to_string());
    //     // if pc == 0x5972 {
    //     //     break;
    //     // }
    //     if !screen.is_open() {
    //         break;
    //     }
    // }
    println!("{} cycles", handheld.mcu.core.cycles);
}
