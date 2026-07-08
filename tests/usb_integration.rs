//! End-to-end USB tests against real firmware, entirely in-process.
//!
//! The handheld is booted with the "cable" already plugged (the channel
//! pair's connect flag raised) and Left+Menu held by a scripted GPIO
//! interface. The firmware detects the host via the USBCON connect-status
//! bit and brings up the SIE; the test then acts as the host on the other
//! end of the channel pair, driving enumeration, SCSI over Bulk-Only
//! Transport, and the device's tunneled flash protocol - the same layers
//! `usb_client` exercises over TCP.
//!
//! Skipped when the firmware images are not present in `firmware/`
//! (they are not distributable).

mod common;

use common::*;
use emiu2::ir::DisconnectedIr;
use emiu2::miuchiz::{
    GpioConnections, GpioInterfaceInternal, GpioState, Handheld, MiuchizButtonStates, UsbResponse,
    UsbToken,
};
use emiu2::usb_interface::channel_pair;

/// Holds Left+Menu (active low on port A) from reset. The boot ROM samples
/// PA at cold start and Left+Menu selects its "Please Connect to PC" mode,
/// whose main loop brings up the SIE and services USB - the same thing a
/// player does to connect a real handheld. Released once the ROM is in that
/// loop. (A fully-held D-pad selects a different, factory-style USB mode
/// that speaks the same protocol but hangs forever after a host-commanded
/// eject instead of rebooting into the flash application.)
struct ConnectModeGpio;

/// How long Left+Menu stays held, in oscillator cycles (~4 s; the boot
/// decision happens within the first few million cycles).
const BUTTON_HOLD_CYCLES: u64 = 60_000_000;

impl GpioInterfaceInternal for ConnectModeGpio {
    fn get_inputs(&mut self, cycle: u64) -> GpioConnections {
        let mut buttons = MiuchizButtonStates::default();
        let held = cycle < BUTTON_HOLD_CYCLES;
        buttons.left = held;
        buttons.menu = held;
        buttons.to_gpio_connections()
    }

    fn set_outputs(&mut self, _state: GpioState, _cycle: u64) {}
}

fn load_firmware() -> Option<(Vec<u8>, Vec<u8>)> {
    let dir = firmware_dir();
    let otp = std::fs::read(dir.join("OTP.dat")).ok()?;
    let flash = std::fs::read(dir.join("Spike 1.02.dat")).ok()?;
    Some((otp, flash))
}

/// Cycle budget for the boot ROM to reach connect mode and bring up the
/// SIE (observed around 30M cycles; generous margin).
const USB_BRINGUP_BUDGET: u64 = 500_000_000;

/// Boots the real firmware into "Please Connect to PC" mode with the cable
/// plugged and waits for the ROM to bring the SIE up. `None` when the
/// firmware images are not present (the test should skip).
fn boot_connect_mode() -> Option<Host> {
    let (otp, flash) = match load_firmware() {
        Some(images) => images,
        None => {
            eprintln!("firmware images not present; skipping");
            return None;
        }
    };

    let (port, internal) = channel_pair();
    let handheld = Handheld::new(
        &otp,
        &flash,
        Box::new(NullScreen),
        Box::new(ConnectModeGpio),
        Box::new(NullAudio),
        Box::new(DisconnectedIr),
        Box::new(internal),
    )
    .expect("handheld construction");

    // The cable is plugged from the start; the connect-mode ROM polls the
    // USBCON connect-status bit to decide a host is really there.
    port.set_connected(true);

    let mut host = Host { handheld, port };
    while !usb_enabled(&mut host.handheld) {
        assert!(
            host.handheld.mcu.core.cycles < USB_BRINGUP_BUDGET,
            "firmware never enabled USB (USBEN) with a host attached"
        );
        step_cycles(&mut host.handheld, 100_000);
    }
    // Let the bus reset and the firmware's SIE setup settle.
    step_cycles(&mut host.handheld, 1_000_000);
    Some(host)
}

#[test]
fn usb_enumeration_and_flash_read_end_to_end() {
    let Some(mut host) = boot_connect_mode() else {
        return;
    };

    // Enumeration: the standard device descriptor, served by the firmware.
    let descriptor = host.control_in(0x80, 0x06, 0x0100, 0x0000, 18);
    assert_eq!(descriptor.len(), 18, "device descriptor length");
    assert_eq!(descriptor[0], 18, "bLength");
    assert_eq!(descriptor[1], 1, "bDescriptorType (device)");

    // SCSI INQUIRY over Bulk-Only Transport (bulk path both directions).
    // The boot ROM's response is a canned 11 bytes ending in "MGA".
    let inquiry = host.scsi(&[0x12, 0, 0, 0, 36, 0], 36, None);
    assert_eq!(
        inquiry,
        [0x00, 0x80, 0x00, 0x01, 0x1F, 0x00, 0x00, 0x00, b'M', b'G', b'A'],
        "INQUIRY response"
    );

    // The tunneled flash protocol returns the machine's actual flash.
    let expected = host.handheld.make_flash_dump()[..PAGE_SIZE].to_vec();
    let page = host.read_page(0);
    assert_eq!(page, expected, "tunneled page 0 differs from flash");
}

/// Cycle budget from the eject command to the SIE teardown (the ROM's main
/// loop parses the queued command within a few of its iterations).
const TEARDOWN_BUDGET: u64 = 50_000_000;
/// Cycle budget from the teardown to the flash-application handoff: the
/// ROM's ~2 s poll tick decides to reboot, then reinit_main waits another
/// half-second tick and commits state markers to flash (~3 s ≈ 48M cycles
/// at 16 MHz; generous margin).
const REBOOT_BUDGET: u64 = 500_000_000;

/// `miuchiz eject` = a tunneled read of flash page 0x200, which the boot ROM
/// treats as "disconnect": it tears down the SIE (the host sees the device
/// drop off the bus) and, in the player-facing Left+Menu connect mode, hands
/// off to the application firmware in flash. This is the behavior observed
/// on hardware; the factory-style all-D-pad USB mode instead hangs forever.
#[test]
fn eject_detaches_and_reboots_into_flash_application() {
    let Some(mut host) = boot_connect_mode() else {
        return;
    };
    assert_eq!(prr(&mut host.handheld), 0x0000, "boot ROM program bank");

    host.eject();

    // The main loop's parser hits the disconnect block and tears down the
    // SIE (check_disconnect_block clears USBEN).
    let start = host.handheld.mcu.core.cycles;
    while usb_enabled(&mut host.handheld) {
        assert!(
            host.handheld.mcu.core.cycles - start < TEARDOWN_BUDGET,
            "firmware never tore down USB after the eject command"
        );
        step_cycles(&mut host.handheld, 10_000);
    }

    // Off the bus: collecting the response now fails outright, the way a
    // real handheld vanishes from the host (eject.c ignores that failure).
    match host.transact(EP_BULK, UsbToken::In, vec![]) {
        UsbResponse::Detached => {}
        other => panic!("expected the device off the bus, got {other:?}"),
    }

    // Within its next ~2 s poll tick the ROM reboots into the application
    // firmware: the program bank switches to segment $0202.
    let start = host.handheld.mcu.core.cycles;
    while prr(&mut host.handheld) != PRR_APPLICATION {
        assert!(
            host.handheld.mcu.core.cycles - start < REBOOT_BUDGET,
            "ROM never handed off to the flash application after eject"
        );
        step_cycles(&mut host.handheld, 100_000);
    }

    // The game does not service USB; the device stays off the bus.
    assert!(
        !usb_enabled(&mut host.handheld),
        "USB came back after handoff"
    );
    match host.transact(EP_BULK, UsbToken::In, vec![]) {
        UsbResponse::Detached => {}
        other => panic!("expected the device to stay off the bus, got {other:?}"),
    }
}
