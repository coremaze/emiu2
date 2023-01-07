mod display;
mod gpio;
pub mod memory;
mod miuchiz;
use clap::Parser;

#[derive(Parser)]
struct Args {
    /// Miuchiz OTP image
    otp_file: String,

    /// Miuchiz flash image
    flash_file: String,

    /// Pixel scale
    #[arg(short, long, default_value_t = 3)]
    scale: usize,
}

fn main() {
    let args = Args::parse();

    let otp_data = match std::fs::read(args.otp_file) {
        Ok(data) => data,
        Err(why) => {
            eprintln!("Could not read OTP file: {why}");
            return;
        }
    };

    let flash_data = match std::fs::read(args.flash_file) {
        Ok(data) => data,
        Err(why) => {
            eprintln!("Could not read flash file: {why}");
            return;
        }
    };

    let scale = args.scale;

    let screen = display::MiniFbScreen::open("emiu2", 98, 67, scale);

    let mut handheld = miuchiz::Handheld::new(&otp_data, &flash_data, &screen, &screen).unwrap();
    // std::thread::sleep(std::time::Duration::from_secs(3));

    let beginning = std::time::Instant::now();

    while screen.is_open() {
        let now = std::time::Instant::now();
        let elapsed = now - beginning;
        let nanoseconds = elapsed.as_nanos();
        let cycles_required_so_far =
            (nanoseconds * handheld.mcu.core.cycles_per_second() as u128) / 1000000000;

        while (handheld.mcu.core.cycles as u128) < cycles_required_so_far {
            // let pc = handheld.mcu.core.registers.pc;
            // let inst = handheld.mcu.core.decode_next_instruction();
            // println!("{pc:04X}: {}", inst.instruction.to_string());
            handheld.mcu.step();
        }

        std::thread::sleep(std::time::Duration::from_nanos(1));
    }
    println!("{} cycles", handheld.mcu.core.cycles);
}
