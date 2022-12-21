pub mod memory;
mod st2205u;

use st2205u::St2205u;

fn main() {
    let otp_data = std::fs::read("OTP.dat").unwrap();
    let flash_data = std::fs::read("Flash.dat").unwrap();
    let mut mcu = St2205u::new(&otp_data, &flash_data).unwrap();

    for _count in 0u64..800_000_000 {
        mcu.step();
        // println!("{}", mcu.core.registers.to_string());
    }
    println!("{} cycles", mcu.core.cycles);
}
