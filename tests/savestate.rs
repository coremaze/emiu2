//! Savestate completeness and determinism tests.
//!
//! The core property: two runs of the same span from the same state
//! produce byte-identical savestates. This catches any machine state
//! that is mutated during emulation but missing from serialization.
//!
//! The firmware-based tests are skipped when the images are not present
//! in `firmware/` (they are not distributable).

use emiu2::audio::AudioInterface;
use emiu2::ir::DisconnectedIr;
use emiu2::miuchiz::{GpioConnections, GpioInterfaceInternal, GpioState, Handheld};
use emiu2::screen::{Pixel, Screen};
use emiu2::snapshot::SnapshotError;
use std::path::PathBuf;

struct NullScreen;

impl Screen for NullScreen {
    fn set_pixels(&self, _pixels: &[Pixel]) {}
}

struct NullGpio;

impl GpioInterfaceInternal for NullGpio {
    fn get_inputs(&mut self, _cycle: u64) -> GpioConnections {
        GpioConnections::default()
    }

    fn set_outputs(&mut self, _state: GpioState, _cycle: u64) {}
}

struct NullAudio;

impl AudioInterface for NullAudio {
    fn set_clock_rate(&mut self, _emulated_clock_rate: u64) {}

    fn needs_sample(&self, _current_cycle: u64) -> bool {
        false
    }

    fn add_sample(&mut self, _value: f32) {}
}

fn load_firmware() -> Option<(Vec<u8>, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("firmware");
    let otp = std::fs::read(dir.join("OTP.dat")).ok()?;
    let flash = std::fs::read(dir.join("Spike 1.02.dat")).ok()?;
    Some((otp, flash))
}

fn make_handheld(otp: &[u8], flash: &[u8]) -> Handheld {
    Handheld::new(
        otp,
        flash,
        Box::new(NullScreen),
        Box::new(NullGpio),
        Box::new(NullAudio),
        Box::new(DisconnectedIr),
    )
    .expect("handheld construction")
}

fn run_until_cycle(handheld: &mut Handheld, target: u64) {
    while handheld.mcu.core.cycles < target {
        handheld.mcu.step();
    }
}

/// Boot cycles before the first snapshot, and the length of the compared
/// span. Long enough to cross many base-timer/RTC ticks, interrupts,
/// LCD activity and PSG traffic during boot.
const BOOT_CYCLES: u64 = 4_000_000;
const SPAN_CYCLES: u64 = 2_000_000;

#[test]
fn same_span_from_same_state_is_byte_identical() {
    let Some((otp, flash)) = load_firmware() else {
        eprintln!("skipping: firmware images not present");
        return;
    };

    let mut handheld = make_handheld(&otp, &flash);
    run_until_cycle(&mut handheld, BOOT_CYCLES);
    let state_a = handheld.snapshot();
    let cycles_at_a = handheld.mcu.core.cycles;

    run_until_cycle(&mut handheld, cycles_at_a + SPAN_CYCLES);
    let state_b = handheld.snapshot();

    // Rewind the same machine and replay the span.
    handheld.restore(&state_a).expect("restore in place");
    assert_eq!(
        handheld.mcu.core.cycles, cycles_at_a,
        "cycle counter must rewind with the state"
    );
    run_until_cycle(&mut handheld, cycles_at_a + SPAN_CYCLES);
    let state_c = handheld.snapshot();
    assert!(
        state_b == state_c,
        "replaying the same span from a restored state diverged"
    );

    // A fresh machine restored from the same savestate must also replay
    // identically (the savestate-from-disk pattern) — even one built
    // with a blank OTP, since the savestate carries the OTP with it and
    // must survive the original files going missing.
    let blank_otp = vec![0u8; otp.len()];
    let mut fresh = make_handheld(&blank_otp, &flash);
    fresh.restore(&state_a).expect("restore into fresh");
    run_until_cycle(&mut fresh, cycles_at_a + SPAN_CYCLES);
    let state_d = fresh.snapshot();
    assert!(
        state_b == state_d,
        "a fresh machine restored from the savestate diverged"
    );
}

#[test]
fn roundtrip_without_stepping_preserves_every_byte() {
    // Works without firmware: format roundtrip on a blank machine.
    let otp = vec![0u8; 0x4000];
    let flash = vec![0u8; 0x200000];
    let mut handheld = make_handheld(&otp, &flash);

    let saved = handheld.snapshot();
    handheld.restore(&saved).expect("load own savestate");
    let saved_again = handheld.snapshot();
    assert!(saved == saved_again);
}

#[test]
fn bad_data_is_rejected() {
    let otp = vec![0u8; 0x4000];
    let flash = vec![0u8; 0x200000];
    let mut handheld = make_handheld(&otp, &flash);

    assert_eq!(
        handheld.restore(b"not a savestate"),
        Err(SnapshotError::BadMagic)
    );

    let mut truncated = handheld.snapshot();
    truncated.truncate(truncated.len() - 1);
    assert_eq!(
        handheld.restore(&truncated),
        Err(SnapshotError::UnexpectedEof)
    );

    let mut trailing = handheld.snapshot();
    trailing.push(0);
    assert_eq!(
        handheld.restore(&trailing),
        Err(SnapshotError::TrailingData)
    );
}
