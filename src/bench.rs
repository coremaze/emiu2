use std::time::Instant;

use crate::audio::AudioInterface;
use crate::miuchiz;
use crate::miuchiz::{GpioConnections, GpioInterfaceInternal, GpioState};
use crate::screen::{Pixel, Screen};

const AUDIO_SAMPLE_RATE: f64 = 44100.0;

struct NullScreen;

impl Screen for NullScreen {
    fn set_pixels(&self, _pixels: &[Pixel]) {}
}

struct NullGpio;

impl GpioInterfaceInternal for NullGpio {
    fn get_inputs(&mut self) -> GpioConnections {
        GpioConnections::default()
    }

    fn set_outputs(&mut self, _state: GpioState) {}
}

/// Consumes samples at the same cadence as the real audio backend so the
/// benchmark exercises the same PSG sampling work, but discards the samples.
struct NullAudio {
    clock_of_last_sample: f64,
    clocks_between_samples: f64,
    samples: u64,
}

impl NullAudio {
    fn new() -> Self {
        Self {
            clock_of_last_sample: 0.0,
            clocks_between_samples: 0.0,
            samples: 0,
        }
    }
}

impl AudioInterface for NullAudio {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64) {
        self.clocks_between_samples = emulated_clock_rate as f64 / AUDIO_SAMPLE_RATE;
    }

    fn needs_sample(&self, current_cycle: u64) -> bool {
        self.clock_of_last_sample + self.clocks_between_samples <= current_cycle as f64
    }

    fn add_sample(&mut self, _value: f32) {
        self.clock_of_last_sample += self.clocks_between_samples;
        self.samples += 1;
    }

    fn next_sample_cycle(&self) -> u64 {
        (self.clock_of_last_sample + self.clocks_between_samples).ceil() as u64
    }
}

/// FNV-1a, used to fingerprint execution so optimized implementations can be
/// checked for cycle-exact equivalence against the baseline.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf29ce484222325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

/// Run the emulator headless and flat-out for `seconds` of emulated time.
///
/// With `verify`, every executed instruction's registers and start cycle are
/// folded into a hash chain; two implementations that produce the same hash
/// executed the same instructions at the same cycles.
pub fn run(otp_data: &[u8], flash_data: &[u8], seconds: u64, verify: bool) {
    let mut handheld = match miuchiz::Handheld::new(
        otp_data,
        flash_data,
        Box::new(NullScreen),
        Box::new(NullGpio),
        Box::new(NullAudio::new()),
    ) {
        Ok(handheld) => handheld,
        Err(why) => {
            eprintln!("Could not initialize the Miuchiz handheld device: {why}");
            return;
        }
    };

    let cycles_per_second = handheld.mcu.core.cycles_per_second();
    let target_cycles = seconds * cycles_per_second;
    let mut steps = 0u64;
    let mut wai_steps = 0u64;
    let mut hasher = Fnv::new();

    let started = Instant::now();
    while handheld.mcu.core.cycles < target_cycles {
        if handheld.mcu.core.waiting_for_interrupt {
            wai_steps += 1;
        }
        if verify {
            let core = &handheld.mcu.core;
            if !core.waiting_for_interrupt {
                hasher.write(&core.registers.pc.to_le_bytes());
                hasher.write(&[
                    core.registers.a,
                    core.registers.x,
                    core.registers.y,
                    core.registers.sp,
                    core.flags.to_u8(),
                ]);
                hasher.write(&core.cycles.to_le_bytes());
            }
        }
        handheld.mcu.step();
        steps += 1;
    }
    let elapsed = started.elapsed();

    let host_seconds = elapsed.as_secs_f64();
    let emulated_seconds = handheld.mcu.core.cycles as f64 / cycles_per_second as f64;

    let mut ram_hasher = Fnv::new();
    ram_hasher.write(handheld.mcu.core.address_space.ram());

    println!("emulated_seconds: {emulated_seconds:.3}");
    println!("host_seconds:     {host_seconds:.3}");
    println!(
        "speed:            {:.2}x realtime",
        emulated_seconds / host_seconds
    );
    println!(
        "emulated_clock:   {:.3} MHz (target {:.3} MHz)",
        handheld.mcu.core.cycles as f64 / host_seconds / 1e6,
        cycles_per_second as f64 / 1e6,
    );
    println!(
        "steps:            {steps} ({:.2} M steps/host-sec)",
        steps as f64 / host_seconds / 1e6
    );
    println!(
        "instructions:     {} ({:.2} M instr/host-sec)",
        steps - wai_steps,
        (steps - wai_steps) as f64 / host_seconds / 1e6
    );
    println!(
        "final_state:      PC={:04X} cycles={}",
        handheld.mcu.core.registers.pc, handheld.mcu.core.cycles
    );
    println!("ram_hash:         {:016x}", ram_hasher.0);
    let [hits, misses, uncached] = handheld.mcu.core.address_space.fetch_stats;
    println!("fetches:          {hits} cached, {misses} decoded+cached, {uncached} uncacheable");
    if verify {
        println!("verify_hash:      {:016x}", hasher.0);
    }
}
