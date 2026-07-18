//! The emulator session: one background thread that owns the machine and
//! everything `!Send` around it (the audio stream, the IR transport), paced
//! to real time exactly like the dev frontend.
//!
//! The UI talks to it through three seams, all cheap and lock-light:
//!
//! - **buttons** flow UI -> emulator as a bitmask in an atomic (see
//!   [`crate::controls::gpio_bit`]); the GPIO interface rebuilds its
//!   connections only when the mask changes.
//! - **frames** flow emulator -> UI through [`FrameShared`], a small
//!   mutex-guarded RGB buffer with a sequence counter, plus an egui repaint
//!   request so the window wakes up.
//! - **commands** (reset, save now, shutdown) go through a channel and are
//!   handled between pacing bursts; **events** (saves, errors, the relay
//!   commander) come back the same way.
//!
//! The session also owns persistence of the machine itself: every
//! [`AUTOSAVE_INTERVAL`] (and once more at shutdown) it writes the snapshot,
//! a flash dump, and — when the screen isn't blank — a thumbnail into the
//! save directory. `save.toml` is deliberately left to the UI so two threads
//! never write the same file.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::StreamTrait;
use emiu2::audio::AudioInterface;
use emiu2::ir::{IrInterface, IrRollbackControl};
use emiu2::miuchiz::{self, GpioConnections, GpioInterfaceInternal, GpioState};
use emiu2::platform::{relay_ir, usb_socket};
use emiu2::rollback::RollbackDriver;
use emiu2::screen::{Pixel, Screen};
use emiu2::usb_interface;

use crate::controls::{gpio_bit, mask_to_states};
use crate::saves;

pub const LCD_WIDTH: usize = 98;
pub const LCD_HEIGHT: usize = 67;

/// How often the running machine is persisted.
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(15);

/// How long the boot buttons are held for "reboot to PC connection"
/// (Left+Menu through early boot; same as the dev frontend's
/// `--connect-mode`).
const CONNECT_MODE_HOLD_CYCLES: u64 = 60_000_000;

/// If the thread falls further behind real time than this (system sleep, a
/// debugger, a very slow disk write), skip ahead instead of fast-forwarding
/// the machine through the gap.
const MAX_CATCHUP: Duration = Duration::from_millis(500);

/// Everything a session needs to boot.
pub struct SessionSpec {
    pub save_dir: PathBuf,
    /// Reported to USB clients (the save's name).
    pub identity: String,
    pub otp: Vec<u8>,
    pub flash: Vec<u8>,
    /// Restored on boot when present; a fresh save cold-boots from flash.
    pub snapshot: Option<Vec<u8>>,
    pub usb_plugged: bool,
    /// IR relay `host:port`; `None` leaves the IR port disconnected.
    pub relay_addr: Option<String>,
    /// Hold Left+Menu through early boot so the device starts in its
    /// "Please Connect to PC" mode.
    pub connect_mode: bool,
}

pub enum EmuCmd {
    SaveNow,
    Shutdown,
}

pub enum EmuEvent {
    /// A save (auto or manual) hit disk.
    Saved,
    /// The IR relay control handle, sent once at startup when configured.
    Relay(relay_ir::RelayCommander),
    /// Something non-fatal went wrong (shown as a toast).
    Error(String),
}

/// The latest LCD frame, shared between the emulator and UI threads.
pub struct FrameShared {
    rgb: Mutex<Vec<u8>>,
    seq: AtomicU64,
}

impl FrameShared {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            rgb: Mutex::new(vec![0; LCD_WIDTH * LCD_HEIGHT * 3]),
            seq: AtomicU64::new(0),
        })
    }

    pub fn seq(&self) -> u64 {
        self.seq.load(Ordering::Acquire)
    }

    /// Copies the current frame out. Returns the sequence number it had.
    pub fn read_rgb(&self, out: &mut Vec<u8>) -> u64 {
        let rgb = self.rgb.lock().unwrap();
        out.clear();
        out.extend_from_slice(&rgb);
        self.seq.load(Ordering::Acquire)
    }
}

/// True when a frame is effectively a blank panel (sleep, boot, scene
/// transitions) — not worth keeping as a thumbnail.
pub fn frame_is_blank(rgb: &[u8]) -> bool {
    rgb.iter().all(|&channel| channel < 10)
}

/// The `Screen` given to the machine: publishes frames and wakes the UI.
struct SharedScreen {
    frame: Arc<FrameShared>,
    ctx: eframe::egui::Context,
}

impl Screen for SharedScreen {
    fn set_pixels(&self, pixels: &[Pixel]) {
        {
            let mut rgb = self.frame.rgb.lock().unwrap();
            for (chunk, pixel) in rgb.chunks_exact_mut(3).zip(pixels) {
                chunk[0] = pixel.red;
                chunk[1] = pixel.green;
                chunk[2] = pixel.blue;
            }
        }
        self.frame.seq.fetch_add(1, Ordering::Release);
        self.ctx.request_repaint();
    }
}

/// The GPIO interface: reads the UI's button mask, plus the temporary
/// boot-time hold used by "reboot to PC connection".
struct SharedGpio {
    buttons: Arc<AtomicU16>,
    /// Cycle number until which Left+Menu are forced down; 0 when inactive.
    boot_hold_until: Arc<AtomicU64>,
    cached_mask: Option<u16>,
    connections: GpioConnections,
}

impl GpioInterfaceInternal for SharedGpio {
    fn get_inputs(&mut self, cycle: u64) -> GpioConnections {
        let mut mask = self.buttons.load(Ordering::Relaxed);
        if cycle < self.boot_hold_until.load(Ordering::Relaxed) {
            mask |= gpio_bit(miuchiz::MiuchizGpio::Left) | gpio_bit(miuchiz::MiuchizGpio::Menu);
        }
        if self.cached_mask != Some(mask) {
            self.connections = mask_to_states(mask).to_gpio_connections();
            self.cached_mask = Some(mask);
        }
        self.connections.clone()
    }

    fn set_outputs(&mut self, _state: GpioState, _cycle: u64) {}
}

/// Audio sink used when no output device exists; the machine still runs.
struct SilentAudio;

impl AudioInterface for SilentAudio {
    fn set_clock_rate(&mut self, _emulated_clock_rate: u64) {}
    fn needs_sample(&self, _current_cycle: u64) -> bool {
        false
    }
    fn add_sample(&mut self, _value: f32) {}
    fn clock_rewound(&mut self, _current_cycle: u64) {}
}

/// A running emulator. Dropping it (or calling [`shutdown`](Self::shutdown))
/// saves and stops the thread.
pub struct EmuSession {
    pub buttons: Arc<AtomicU16>,
    pub frame: Arc<FrameShared>,
    pub cable: Arc<usb_socket::UsbCable>,
    pub events: Receiver<EmuEvent>,
    cmds: Sender<EmuCmd>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl EmuSession {
    pub fn start(spec: SessionSpec, ctx: eframe::egui::Context) -> Self {
        let buttons = Arc::new(AtomicU16::new(0));
        let frame = FrameShared::new();

        let (usb_host_port, usb_internal) = usb_interface::channel_pair();
        let cable = usb_socket::UsbCable::new(usb_host_port, spec.identity.clone());
        cable.set_plugged(spec.usb_plugged);

        let (cmds, cmd_rx) = channel();
        let (event_tx, events) = channel();

        let thread = {
            let buttons = buttons.clone();
            let frame = frame.clone();
            let cable = cable.clone();
            std::thread::Builder::new()
                .name("emulator".to_owned())
                .spawn(move || {
                    run_session(
                        spec,
                        ctx,
                        buttons,
                        frame,
                        cable,
                        usb_internal,
                        cmd_rx,
                        event_tx,
                    );
                })
                .expect("could not spawn the emulator thread")
        };

        Self {
            buttons,
            frame,
            cable,
            events,
            cmds,
            thread: Some(thread),
        }
    }

    pub fn send(&self, cmd: EmuCmd) {
        self.cmds.send(cmd).ok();
    }

    /// True once the emulator thread has exited (shutdown or crash).
    pub fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .map(|t| t.is_finished())
            .unwrap_or(true)
    }

    /// Stops the session, blocking until the final save has hit disk.
    pub fn shutdown(mut self) {
        self.send(EmuCmd::Shutdown);
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

impl Drop for EmuSession {
    fn drop(&mut self) {
        self.cmds.send(EmuCmd::Shutdown).ok();
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

/// Persists the machine into the save directory. `save.toml` is the UI's.
struct SaveWriter {
    dir: PathBuf,
    frame: Arc<FrameShared>,
    frame_scratch: Vec<u8>,
}

impl SaveWriter {
    fn save(&mut self, handheld: &mut miuchiz::Handheld) -> std::io::Result<()> {
        saves::write_atomic(&self.dir.join(saves::SNAPSHOT_FILE), &handheld.snapshot())?;
        saves::write_atomic(&self.dir.join(saves::FLASH_FILE), &handheld.make_flash_dump())?;

        // Refresh the thumbnail, but never replace a good one with a blank
        // panel (sleep, transitions): the gallery should show the game.
        self.frame.read_rgb(&mut self.frame_scratch);
        if !frame_is_blank(&self.frame_scratch) {
            let mut png = Vec::new();
            let encoder = image::codecs::png::PngEncoder::new(&mut png);
            use image::ImageEncoder;
            encoder
                .write_image(
                    &self.frame_scratch,
                    LCD_WIDTH as u32,
                    LCD_HEIGHT as u32,
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(|why| std::io::Error::new(std::io::ErrorKind::InvalidData, why))?;
            saves::write_atomic(&self.dir.join(saves::THUMB_FILE), &png)?;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn run_session(
    spec: SessionSpec,
    ctx: eframe::egui::Context,
    buttons: Arc<AtomicU16>,
    frame: Arc<FrameShared>,
    cable: Arc<usb_socket::UsbCable>,
    usb_internal: usb_interface::ChannelUsbInterface,
    cmd_rx: Receiver<EmuCmd>,
    events: Sender<EmuEvent>,
) {
    let report = |message: String| {
        eprintln!("{message}");
        events.send(EmuEvent::Error(message)).ok();
    };

    // Audio; the machine runs silently when no output device exists.
    // The cpal stream is `!Send` and must live (and die) on this thread.
    let mut _audio_stream = None;
    let audio: Box<dyn AudioInterface> = match emiu2::platform::cpal_audio::stream_setup_for() {
        Ok((stream, sender)) => {
            if let Err(why) = stream.play() {
                report(format!("Audio unavailable: {why}"));
                Box::new(SilentAudio)
            } else {
                _audio_stream = Some(stream);
                Box::new(sender)
            }
        }
        Err(why) => {
            report(format!("Audio unavailable: {why}"));
            Box::new(SilentAudio)
        }
    };

    // IR: the relay transport must be created on this thread; only its
    // commander crosses back to the UI.
    let (ir, ir_rollback): (Box<dyn IrInterface>, Option<Box<dyn IrRollbackControl>>) =
        match &spec.relay_addr {
            Some(addr) => {
                let relay = relay_ir::RelayIr::connect(addr.clone());
                events.send(EmuEvent::Relay(relay.commander())).ok();
                let rollback = relay.rollback_handle();
                (Box::new(relay), Some(Box::new(rollback)))
            }
            None => (Box::new(emiu2::ir::DisconnectedIr), None),
        };

    // The USB discovery endpoint lives exactly as long as this session.
    let _endpoint = match usb_socket::create_discovery_endpoint(cable.clone()) {
        Ok(guard) => Some(guard),
        Err(why) => {
            report(format!("USB discovery endpoint unavailable: {why}"));
            None
        }
    };

    let boot_hold_until = Arc::new(AtomicU64::new(0));
    let gpio = SharedGpio {
        buttons,
        boot_hold_until: boot_hold_until.clone(),
        cached_mask: None,
        connections: GpioConnections::default(),
    };
    let screen = SharedScreen {
        frame: frame.clone(),
        ctx,
    };

    let mut handheld = match miuchiz::Handheld::new(
        &spec.otp,
        &spec.flash,
        Box::new(screen),
        Box::new(gpio),
        audio,
        ir,
        Box::new(usb_internal),
    ) {
        Ok(handheld) => handheld,
        Err(why) => {
            report(format!("Could not start the device: {why}"));
            return;
        }
    };

    if let Some(snapshot) = &spec.snapshot {
        if let Err(why) = handheld.restore(snapshot) {
            report(format!(
                "Could not resume the last session ({why}); rebooting from flash"
            ));
        }
    }

    if spec.connect_mode {
        // get_inputs is handed oscillator cycles, so the deadline must be
        // in that clock (2x core.cycles).
        boot_hold_until.store(
            handheld.mcu.core.oscillator_cycles() + CONNECT_MODE_HOLD_CYCLES,
            Ordering::Relaxed,
        );
    }

    let mut rollback_driver = ir_rollback.map(RollbackDriver::new);
    let mut saver = SaveWriter {
        dir: spec.save_dir,
        frame,
        frame_scratch: Vec::new(),
    };

    // Wall-clock pacing anchor (same scheme as emiu2-dev's run loop): the
    // anchor stays fixed and the machine only ever steps up to the cycle
    // count real time has earned. Re-anchored whenever the emulated cycle
    // counter jumps (snapshot restore, IR rollback) or the machine falls
    // too far behind to honestly catch up.
    let mut anchor_time = Instant::now();
    let mut anchor_cycles = handheld.mcu.core.cycles;
    let mut last_autosave = Instant::now();

    let max_catchup_cycles =
        handheld.mcu.core.cycles_per_second() as u128 * MAX_CATCHUP.as_millis() / 1000;

    loop {
        let nanoseconds = anchor_time.elapsed().as_nanos();
        let mut cycles_required_so_far = anchor_cycles as u128
            + (nanoseconds * handheld.mcu.core.cycles_per_second() as u128) / 1_000_000_000;

        // The machine being far behind what real time earned means the
        // thread stalled (system sleep, a long disk write): skip ahead and
        // resume at 1x instead of fast-forwarding through the gap.
        if cycles_required_so_far > handheld.mcu.core.cycles as u128 + max_catchup_cycles {
            anchor_time = Instant::now();
            anchor_cycles = handheld.mcu.core.cycles;
            cycles_required_so_far = anchor_cycles as u128;
        }

        while (handheld.mcu.core.cycles as u128) < cycles_required_so_far {
            handheld.mcu.step();
        }

        if let Some(driver) = rollback_driver.as_mut() {
            if driver.run(&mut handheld) {
                anchor_time = Instant::now();
                anchor_cycles = handheld.mcu.core.cycles;
            }
        }

        let mut shutdown = false;
        loop {
            match cmd_rx.try_recv() {
                Ok(EmuCmd::SaveNow) => match saver.save(&mut handheld) {
                    Ok(()) => {
                        events.send(EmuEvent::Saved).ok();
                        last_autosave = Instant::now();
                    }
                    Err(why) => report(format!("Save failed: {why}")),
                },
                Ok(EmuCmd::Shutdown) => shutdown = true,
                Err(TryRecvError::Empty) => break,
                // The UI is gone; save and stop.
                Err(TryRecvError::Disconnected) => {
                    shutdown = true;
                    break;
                }
            }
        }
        if shutdown {
            break;
        }

        if last_autosave.elapsed() >= AUTOSAVE_INTERVAL {
            match saver.save(&mut handheld) {
                Ok(()) => {
                    events.send(EmuEvent::Saved).ok();
                }
                Err(why) => report(format!("Autosave failed: {why}")),
            }
            last_autosave = Instant::now();
        }

        std::thread::sleep(Duration::from_micros(300));
    }

    match saver.save(&mut handheld) {
        Ok(()) => {
            events.send(EmuEvent::Saved).ok();
        }
        Err(why) => report(format!("Final save failed: {why}")),
    }
}
