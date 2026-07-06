mod audio;
mod ir;
mod ir_replay;
pub mod memory;
mod miuchiz;
mod platform;
mod rollback;
mod screen;
pub mod snapshot;
pub mod ssc;
mod usb_interface;
mod usb_socket;
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
    savestate_file: Option<PathBuf>,

    /// Pixel scale
    #[arg(long, default_value_t = 3)]
    scale: usize,

    /// Show GPIO LED display
    #[arg(long, default_value_t = false)]
    show_gpio: bool,

    /// Expose the emulated USB device on a TCP transaction socket (e.g.
    /// 127.0.0.1:3240), in addition to the local discovery endpoint that
    /// host tools find automatically.
    #[arg(long, value_name = "ADDR")]
    usb_socket: Option<String>,

    /// Hold the D-pad through early boot so the device starts in its
    /// "Please Connect to PC" (USB) mode.
    #[arg(long, default_value_t = false)]
    connect_mode: bool,

    /// IR transceiver: none, listen:<port>, connect:<host:port>, or
    /// relay:<host:port> (an emiu2 relay server; pair with friend codes
    /// by typing "join <code>" on standard input)
    #[arg(long, default_value = "none")]
    ir: String,
}

/// A validated `--ir` argument. The transports themselves are not
/// `Send` (their engine is single-threaded by construction), so they
/// are created by `start` on the emulator thread; the plan carries only
/// what can cross threads. Binding the listener here, on the main
/// thread, lets address errors surface before the window opens.
enum IrPlan {
    Disconnected,
    Listen(std::net::TcpListener),
    Connect(String),
    Relay(String),
}

type IrSetup = (
    Box<dyn ir::IrInterface>,
    Option<Box<dyn ir::IrRollbackControl>>,
    Option<platform::relay_ir::RelayCommander>,
);

fn prepare_ir(spec: &str) -> Result<IrPlan, String> {
    if spec == "none" {
        return Ok(IrPlan::Disconnected);
    }
    if let Some(port) = spec.strip_prefix("listen:") {
        let port: u16 = port
            .parse()
            .map_err(|_| format!("Invalid IR listen port: {port}"))?;
        let listener = std::net::TcpListener::bind(("0.0.0.0", port))
            .map_err(|why| format!("Could not listen for IR peers on port {port}: {why}"))?;
        eprintln!("IR: listening on port {port}");
        Ok(IrPlan::Listen(listener))
    } else if let Some(addr) = spec.strip_prefix("connect:") {
        Ok(IrPlan::Connect(addr.to_string()))
    } else if let Some(addr) = spec.strip_prefix("relay:") {
        Ok(IrPlan::Relay(addr.to_string()))
    } else {
        Err(format!(
            "Invalid --ir value {spec:?} (expected none, listen:<port>, \
             connect:<host:port>, or relay:<host:port>)"
        ))
    }
}

impl IrPlan {
    /// Creates the transport on the calling (emulator) thread.
    fn start(self) -> IrSetup {
        match self {
            IrPlan::Disconnected => (Box::new(ir::DisconnectedIr), None, None),
            IrPlan::Listen(listener) => {
                let socket = platform::socket_ir::SocketIr::from_listener(listener);
                let rollback = socket.rollback_handle();
                (Box::new(socket), Some(Box::new(rollback)), None)
            }
            IrPlan::Connect(addr) => {
                let socket = platform::socket_ir::SocketIr::connect(addr);
                let rollback = socket.rollback_handle();
                (Box::new(socket), Some(Box::new(rollback)), None)
            }
            IrPlan::Relay(addr) => {
                let relay = platform::relay_ir::RelayIr::connect(addr);
                let rollback = relay.rollback_handle();
                let commander = relay.commander();
                (Box::new(relay), Some(Box::new(rollback)), Some(commander))
            }
        }
    }
}

/// Holds the whole D-pad (active low on port A) through early boot, then
/// defers to the real inputs. The boot ROM samples PA at cold start and a
/// fully-held D-pad selects its "Please Connect to PC" (USB) mode - the same
/// thing a player does to connect a real handheld.
struct ConnectModeBoot {
    inner: Box<dyn miuchiz::GpioInterfaceInternal>,
}

/// How long the D-pad stays held, in oscillator cycles (~4 s; the boot
/// decision happens within the first few million cycles).
const CONNECT_MODE_HOLD_CYCLES: u64 = 60_000_000;

impl miuchiz::GpioInterfaceInternal for ConnectModeBoot {
    fn get_inputs(&mut self, cycle: u64) -> miuchiz::GpioConnections {
        let mut connections = self.inner.get_inputs(cycle);
        if cycle < CONNECT_MODE_HOLD_CYCLES {
            for button in [
                miuchiz::MiuchizGpio::Up,
                miuchiz::MiuchizGpio::Down,
                miuchiz::MiuchizGpio::Left,
                miuchiz::MiuchizGpio::Right,
            ] {
                let (port, bit) = button.to_port();
                connections.connect(port, bit, false);
            }
        }
        connections
    }

    fn set_outputs(&mut self, state: miuchiz::GpioState, cycle: u64) {
        self.inner.set_outputs(state, cycle);
    }
}

/// Reads pairing commands from standard input while the emulator runs.
fn relay_console(commander: platform::relay_ir::RelayCommander) {
    use std::io::BufRead;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { return };
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("join") => match parts.next().and_then(emiu2_netplay::FriendCode::parse) {
                Some(code) => commander.join(code),
                None => eprintln!("Usage: join <6-character friend code>"),
            },
            Some("leave") => commander.leave(),
            Some("status") => {
                let code = commander
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "not assigned yet".into());
                eprintln!(
                    "IR relay: {}, {}. Your code: {code}",
                    if commander.connected() {
                        "connected"
                    } else {
                        "connecting..."
                    },
                    if commander.paired() {
                        "paired"
                    } else {
                        "not paired"
                    },
                );
            }
            Some(_) => eprintln!("Commands: join <code>, leave, status"),
            None => {}
        }
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
    let connect_mode = args.connect_mode;
    let savestate_file = args
        .savestate_file
        .unwrap_or_else(|| PathBuf::from(format!("{}.state", args.flash_file)));

    let ir_plan = match prepare_ir(&args.ir) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("{why}");
            return;
        }
    };

    // The USB cable: the internal half goes to the device, the external half
    // is shared by the discovery endpoint (always on, how host tools find
    // running emulators) and the optional explicit TCP endpoint.
    let (usb_host_port, usb_internal) = usb_interface::channel_pair();
    let usb_identity = std::path::Path::new(&args.flash_file)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "emiu2".to_string());
    let usb_cable = usb_socket::UsbCable::new(usb_host_port, usb_identity);
    // Held for the process lifetime; dropping it removes the endpoint file.
    let _usb_endpoint = match usb_socket::create_discovery_endpoint(usb_cable.clone()) {
        Ok(guard) => Some(guard),
        Err(why) => {
            eprintln!("Warning: could not create the USB discovery endpoint: {why}");
            None
        }
    };
    if let Some(addr) = args.usb_socket {
        println!("USB transaction socket listening on {addr}");
        let cable = usb_cable.clone();
        std::thread::spawn(move || {
            if let Err(why) = usb_socket::serve(&addr, cable) {
                eprintln!("USB socket server failed: {why}");
            }
        });
    }

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
            savestate_file,
            ir_plan,
            usb_internal,
            connect_mode,
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
    savestate_file: PathBuf,
    ir_plan: IrPlan,
    usb_internal: usb_interface::ChannelUsbInterface,
    connect_mode: bool,
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

    // The IR transports live on this thread; only the commander (used
    // by the stdin console) may leave it.
    let (ir_transceiver, ir_rollback, ir_commander) = ir_plan.start();
    if let Some(commander) = ir_commander {
        std::thread::spawn(move || relay_console(commander));
    }

    let io: Box<dyn miuchiz::GpioInterfaceInternal> = if connect_mode {
        Box::new(ConnectModeBoot {
            inner: Box::new(minifb_gpio),
        })
    } else {
        Box::new(minifb_gpio)
    };

    let mut handheld = match miuchiz::Handheld::new(
        &otp_data,
        &flash_data,
        Box::new(minifb_screen),
        io,
        Box::new(sender),
        ir_transceiver,
        Box::new(usb_internal),
    ) {
        Ok(handheld) => handheld,
        Err(why) => {
            eprintln!("Could not initialize the Miuchiz handheld device: {why}");
            return;
        }
    };
    // std::thread::sleep(std::time::Duration::from_secs(3));

    let mut rollback_driver = ir_rollback.map(rollback::RollbackDriver::new);

    // Wall-clock pacing anchor. Re-anchored whenever the emulated cycle
    // counter jumps (savestate load or IR rollback), so the machine
    // always runs at 1x from its current position instead of
    // fast-forwarding to catch up.
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

        if let Some(driver) = rollback_driver.as_mut() {
            if driver.run(&mut handheld) {
                anchor_time = std::time::Instant::now();
                anchor_cycles = handheld.mcu.core.cycles;
            }
        }

        screen.update_state();

        if screen.take_snapshot_request() {
            match std::fs::write(&savestate_file, handheld.snapshot()) {
                Ok(()) => println!("Saved savestate to {savestate_file:?}"),
                Err(why) => eprintln!("Failed to save state: {why}"),
            }
        }

        if screen.take_restore_request() {
            match std::fs::read(&savestate_file) {
                Ok(data) => match handheld.restore(&data) {
                    Ok(()) => {
                        anchor_time = std::time::Instant::now();
                        anchor_cycles = handheld.mcu.core.cycles;
                        println!("Loaded savestate from {savestate_file:?}");
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
