mod audio;
mod ir;
pub mod memory;
mod miuchiz;
mod platform;
mod screen;
pub mod ssc;
pub mod state;

use std::path::PathBuf;

use clap::Parser;
use cpal::traits::StreamTrait;

#[derive(Parser)]
struct Args {
    /// Miuchiz OTP image
    otp_file: String,

    /// Miuchiz flash image to load
    flash_file: String,

    /// Flash image to save
    #[arg(long)]
    save_file: Option<PathBuf>,

    /// Savestate file used by the F5 (save) / F9 (load) hotkeys.
    /// Defaults to the flash image path with ".state" appended.
    #[arg(long)]
    state_file: Option<PathBuf>,

    /// Pixel scale
    #[arg(long, default_value_t = 3)]
    scale: usize,

    /// Show GPIO LED display
    #[arg(long, default_value_t = false)]
    show_gpio: bool,

    /// IR transceiver: none, listen:<port>, or connect:<host:port>
    #[arg(long, default_value = "none")]
    ir: String,
}

fn build_ir(spec: &str) -> Result<Box<dyn ir::IrInterface + Send>, String> {
    if spec == "none" {
        return Ok(Box::new(ir::DisconnectedIr));
    }
    if let Some(port) = spec.strip_prefix("listen:") {
        let port: u16 = port
            .parse()
            .map_err(|_| format!("Invalid IR listen port: {port}"))?;
        let socket = platform::socket_ir::SocketIr::listen(("0.0.0.0", port))
            .map_err(|why| format!("Could not listen for IR peers on port {port}: {why}"))?;
        eprintln!("IR: listening on port {port}");
        Ok(Box::new(socket))
    } else if let Some(addr) = spec.strip_prefix("connect:") {
        Ok(Box::new(platform::socket_ir::SocketIr::connect(addr)))
    } else {
        Err(format!(
            "Invalid --ir value {spec:?} (expected none, listen:<port>, or connect:<host:port>)"
        ))
    }
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

    let flash_data = match std::fs::read(&args.flash_file) {
        Ok(data) => data,
        Err(why) => {
            eprintln!("Could not read flash file: {why}");
            return;
        }
    };

    let scale = args.scale;
    let show_gpio = args.show_gpio;
    let save_file = args.save_file;
    let state_file = args
        .state_file
        .unwrap_or_else(|| PathBuf::from(format!("{}.state", args.flash_file)));

    let ir_transceiver = match build_ir(&args.ir) {
        Ok(ir_transceiver) => ir_transceiver,
        Err(why) => {
            eprintln!("{why}");
            return;
        }
    };

    let (screen, minifb_gpio, screen_tx, worker) =
        platform::minifb_screen_gpio::MiniFbScreen::open("emiu2", scale, show_gpio);

    let minifb_screen = platform::minifb_screen_gpio::MiniFbScreenInterface::new(screen_tx);

    // The emulator runs on a background thread so that the minifb window can be
    // created and pumped on the main thread, which macOS's AppKit requires.
    let emulator = std::thread::spawn(move || {
        run_emulator(
            otp_data,
            flash_data,
            minifb_screen,
            minifb_gpio,
            screen,
            save_file,
            state_file,
            ir_transceiver,
        );
    });

    // Blocks on the main thread until the window is closed.
    worker.run();

    if let Err(why) = emulator.join() {
        eprintln!("Emulator thread panicked: {why:?}");
    }
}

#[allow(clippy::too_many_arguments)]
fn run_emulator(
    otp_data: Vec<u8>,
    flash_data: Vec<u8>,
    minifb_screen: platform::minifb_screen_gpio::MiniFbScreenInterface,
    minifb_gpio: platform::minifb_screen_gpio::MiniFbGpioInternalInterface,
    mut screen: platform::minifb_screen_gpio::MiniFbScreen,
    save_file: Option<PathBuf>,
    state_file: PathBuf,
    ir_transceiver: Box<dyn ir::IrInterface + Send>,
) {
    // Keep the audio stream alive for the lifetime of this thread. cpal's
    // `Stream` is `!Send`, so it must be created and dropped on the same thread.
    let (stream, sender) = match platform::cpal_audio::stream_setup_for() {
        Ok((stream, sender)) => (stream, sender),
        Err(why) => {
            eprintln!("Could not setup audio stream: {why}");
            return;
        }
    };

    if let Err(why) = stream.play() {
        eprintln!("Could not play audio stream: {why}");
        return;
    }

    let mut handheld = match miuchiz::Handheld::new(
        &otp_data,
        &flash_data,
        Box::new(minifb_screen),
        Box::new(minifb_gpio),
        Box::new(sender),
        ir_transceiver,
    ) {
        Ok(handheld) => handheld,
        Err(why) => {
            eprintln!("Could not initialize the Miuchiz handheld device: {why}");
            return;
        }
    };
    // std::thread::sleep(std::time::Duration::from_secs(3));

    // Wall-clock pacing anchor. Re-anchored whenever the emulated cycle
    // counter jumps (savestate load), so the machine always runs at 1x
    // from its current position instead of fast-forwarding to catch up.
    let mut anchor_time = std::time::Instant::now();
    let mut anchor_cycles = handheld.mcu.core.cycles;

    while screen.is_open() {
        let nanoseconds = anchor_time.elapsed().as_nanos();
        let cycles_required_so_far = anchor_cycles as u128
            + (nanoseconds * handheld.mcu.core.cycles_per_second() as u128) / 1000000000;

        while (handheld.mcu.core.cycles as u128) < cycles_required_so_far {
            // let pc = handheld.mcu.core.registers.pc;
            // let inst = handheld.mcu.core.decode_next_instruction();
            // println!("{pc:04X}: {}", inst.instruction.to_string());
            handheld.mcu.step();
        }

        screen.update_state();

        if screen.take_save_state_request() {
            match std::fs::write(&state_file, handheld.save_state()) {
                Ok(()) => println!("Saved state to {state_file:?}"),
                Err(why) => eprintln!("Failed to save state: {why}"),
            }
        }

        if screen.take_load_state_request() {
            match std::fs::read(&state_file) {
                Ok(data) => match handheld.load_state(&data) {
                    Ok(()) => {
                        anchor_time = std::time::Instant::now();
                        anchor_cycles = handheld.mcu.core.cycles;
                        println!("Loaded state from {state_file:?}");
                    }
                    Err(why) => eprintln!("Failed to load state: {why}"),
                },
                Err(why) => eprintln!("Failed to read state file: {why}"),
            }
        }

        std::thread::sleep(std::time::Duration::from_nanos(1));
    }

    if let Some(save_file) = save_file {
        match std::fs::write(&save_file, handheld.make_flash_dump()) {
            Ok(_) => {
                println!("Saved flash to {save_file:?}");
            }
            Err(why) => {
                eprintln!("Failed to save flash: {why}");
            }
        }
    }

    // println!("{} cycles", handheld.mcu.core.cycles);
}
