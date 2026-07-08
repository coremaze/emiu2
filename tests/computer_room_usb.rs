//! The in-game computer-room USB flow, end to end: the character walks up
//! to the in-game computer, ACTION starts the connect sequence (the game
//! calls the boot ROM's $4004 standalone-USB entry), the host connects and
//! transfers, ejects (the Miuchiz Sync log-off), the game resumes - and the
//! whole thing must work again on the next visit without restarting the
//! handheld.
//!
//! Needs the real OTP (firmware/OTP.dat) plus a game flash image and a
//! savestate positioned in front of the in-game computer, supplied through
//! environment variables (skipped when unset):
//!   EMIU2_CR_FLASH - path to the game flash image (e.g. Spike 1.09.03.dat)
//!   EMIU2_CR_STATE - path to the matching savestate
//! Optionally EMIU2_CR_SHOTS names a directory that receives PPM screen
//! captures at each checkpoint, for eyeballing the flow.

mod common;

use common::*;
use emiu2::ir::DisconnectedIr;
use emiu2::miuchiz::{
    GpioConnections, GpioInterfaceInternal, GpioState, Handheld, MiuchizButtonStates,
};
use emiu2::screen::{Pixel, Screen};
use emiu2::usb_interface::channel_pair;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// One emulated second (the ST2205U runs at 16 MHz).
const SECOND: u64 = 16_000_000;

/// GPIO whose ACTION button the test presses at will.
struct ScriptedGpio {
    action: Arc<AtomicBool>,
}

impl GpioInterfaceInternal for ScriptedGpio {
    fn get_inputs(&mut self, _cycle: u64) -> GpioConnections {
        let mut buttons = MiuchizButtonStates::default();
        buttons.action = self.action.load(Ordering::Relaxed);
        buttons.to_gpio_connections()
    }

    fn set_outputs(&mut self, _state: GpioState, _cycle: u64) {}
}

/// Keeps the most recent LCD frame so checkpoints can be captured.
#[derive(Clone)]
struct CapturedScreen(Arc<Mutex<Vec<Pixel>>>);

impl Screen for CapturedScreen {
    fn set_pixels(&self, pixels: &[Pixel]) {
        *self.0.lock().unwrap() = pixels.to_vec();
    }
}

const LCD_WIDTH: usize = 98;

/// Writes the current frame as a binary PPM into $EMIU2_CR_SHOTS (if set).
fn shot(screen: &CapturedScreen, name: &str) {
    let Some(dir) = std::env::var_os("EMIU2_CR_SHOTS") else {
        return;
    };
    let frame = screen.0.lock().unwrap().clone();
    if frame.is_empty() || frame.len() % LCD_WIDTH != 0 {
        eprintln!("shot {name}: no frame captured yet ({} px)", frame.len());
        return;
    }
    let height = frame.len() / LCD_WIDTH;
    let mut ppm = format!("P6\n{LCD_WIDTH} {height}\n255\n").into_bytes();
    for p in &frame {
        ppm.extend_from_slice(&[p.red, p.green, p.blue]);
    }
    let path = std::path::Path::new(&dir).join(format!("{name}.ppm"));
    std::fs::write(&path, ppm).expect("write screenshot");
    eprintln!("shot {name}: {}", path.display());
}

struct ComputerRoom {
    host: Host,
    action: Arc<AtomicBool>,
    screen: CapturedScreen,
}

impl ComputerRoom {
    fn step_seconds(&mut self, seconds: f64) {
        step_cycles(&mut self.host.handheld, (seconds * SECOND as f64) as u64);
    }

    /// A human-ish ACTION press: hold a quarter second, release.
    fn press_action(&mut self) {
        self.action.store(true, Ordering::Relaxed);
        self.step_seconds(0.25);
        self.action.store(false, Ordering::Relaxed);
    }

    /// Steps until USBEN reaches `want`, with a time budget in seconds.
    fn wait_usb_enabled(&mut self, want: bool, budget_seconds: u64, what: &str) {
        let start = self.host.handheld.mcu.core.cycles;
        while usb_enabled(&mut self.host.handheld) != want {
            let elapsed = self.host.handheld.mcu.core.cycles - start;
            let pc = self.host.handheld.mcu.core.registers.pc;
            let bank = prr(&mut self.host.handheld);
            assert!(
                elapsed < budget_seconds * SECOND,
                "{what}: USBEN never became {want} (pc={pc:04X}, prr={bank:04X})",
            );
            step_cycles(&mut self.host.handheld, 100_000);
        }
    }

    /// Steps until the program bank is the application firmware's.
    fn wait_back_in_game(&mut self, budget_seconds: u64, what: &str) {
        let start = self.host.handheld.mcu.core.cycles;
        while prr(&mut self.host.handheld) != PRR_APPLICATION {
            assert!(
                self.host.handheld.mcu.core.cycles - start < budget_seconds * SECOND,
                "{what}: never returned to the game (pc={:04X})",
                self.host.handheld.mcu.core.registers.pc,
            );
            step_cycles(&mut self.host.handheld, 100_000);
        }
    }

    /// The `miuchiz` utility's liveness probe: READ(10) of sector 0 must
    /// return a full sector with a passing CSW.
    fn probe(&mut self, what: &str) -> Vec<u8> {
        let sector = self.host.scsi(&scsi_read10(0, 1), SECTOR_SIZE, None);
        assert_eq!(sector.len(), SECTOR_SIZE, "{what}: probe sector length");
        sector
    }
}

fn load_inputs() -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let otp = std::fs::read(firmware_dir().join("OTP.dat")).ok()?;
    let flash = std::fs::read(std::env::var_os("EMIU2_CR_FLASH")?).ok()?;
    let state = std::fs::read(std::env::var_os("EMIU2_CR_STATE")?).ok()?;
    Some((otp, flash, state))
}

#[test]
fn computer_room_usb_survives_eject_and_reconnects() {
    let Some((otp, flash, state)) = load_inputs() else {
        eprintln!("EMIU2_CR_FLASH / EMIU2_CR_STATE not set (or files missing); skipping");
        return;
    };

    let action = Arc::new(AtomicBool::new(false));
    let screen = CapturedScreen(Arc::new(Mutex::new(Vec::new())));
    let (port, internal) = channel_pair();
    let mut handheld = Handheld::new(
        &otp,
        &flash,
        Box::new(screen.clone()),
        Box::new(ScriptedGpio {
            action: action.clone(),
        }),
        Box::new(NullAudio),
        Box::new(DisconnectedIr),
        Box::new(internal),
    )
    .expect("handheld construction");
    handheld
        .restore(&state)
        .expect("savestate restore (was it taken with a compatible build?)");

    // The player has the cable plugged the whole time.
    port.set_connected(true);

    let mut cr = ComputerRoom {
        host: Host { handheld, port },
        action,
        screen,
    };

    // Let the restored game settle, then walk up to the computer.
    cr.step_seconds(0.5);
    shot(&cr.screen, "01-before-action");

    // --- Session 1: ACTION -> connect sequence -> the ROM's standalone
    // USB mode comes up (the game calls the OTP's $4004 entry). ---
    cr.press_action();
    cr.wait_usb_enabled(true, 30, "session 1");
    cr.step_seconds(0.1);
    shot(&cr.screen, "02-session1-usb-up");
    let sector0 = cr.probe("session 1");

    // Log off: the eject. The ROM detaches and resumes the game.
    cr.host.eject();
    cr.wait_usb_enabled(false, 5, "session 1 eject teardown");
    cr.wait_back_in_game(35, "session 1 eject");
    shot(&cr.screen, "03-after-eject");

    // Give the kicked-out animation time to finish.
    cr.step_seconds(4.0);
    shot(&cr.screen, "04-back-in-room");

    // --- Session 2: the same flow must work again without a restart. ---
    cr.press_action();
    cr.wait_usb_enabled(true, 30, "session 2");
    cr.step_seconds(0.1);
    shot(&cr.screen, "05-session2-usb-up");
    let again = cr.probe("session 2");
    assert_eq!(sector0, again, "probe data differs between sessions");

    cr.host.eject();
    cr.wait_usb_enabled(false, 5, "session 2 eject teardown");
    cr.wait_back_in_game(35, "session 2 eject");
    cr.step_seconds(4.0);
    shot(&cr.screen, "06-after-second-eject");

    // --- Session 3: unplugging the cable mid-session must also kick the
    // character back out (the ~2 s cable poll notices). ---
    cr.press_action();
    cr.wait_usb_enabled(true, 30, "session 3");
    cr.probe("session 3");
    cr.host.port.set_connected(false);
    cr.wait_usb_enabled(false, 10, "session 3 unplug teardown");
    cr.wait_back_in_game(35, "session 3 unplug");
    shot(&cr.screen, "07-after-unplug");
}
