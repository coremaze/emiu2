use super::gpio::GpioInterfaceInternal;
use super::interrupt::Interrupt;
use super::psg::PsgChannel;
use super::usb::UsbState;
use super::vector;
use super::wdc_65c02;
use super::wdc_65c02::HandlesInterrupt;
use super::St2205uAddressSpace;
use crate::audio::AudioInterface;
use crate::memory::AddressSpace;
use crate::snapshot::{SnapshotError, SnapshotReader, SnapshotWriter};
use crate::usb_interface::UsbInterfaceInternal;

/// How often GPIO inputs are polled, in instruction cycles (~1 ms of
/// emulated time). Inputs change at human timescales, so per-instruction
/// polling is wasted work; port reads between polls see at most this much
/// latency. Polls happen on a fixed cycle grid (multiples of the interval)
/// so their timing does not depend on when unrelated events fire; board
/// circuitry that needs finer timing (the IR receiver) schedules its own
/// events through `GpioInterfaceInternal::next_event_cycle`.
const GPIO_POLL_INTERVAL_SYSCK: u64 = 8000;

/// Representation of a ST2205U microcontroller.
///
/// This microcontroller is capable of, through the use of bank registers,
/// accessing a larger address space than the 65C02 core itself can.
///
/// This device also implements its own address space, which is addressible using
/// 16 bits, which is directly exposed to the underlying 65C02.
///
/// Peripherals are event-driven: each knows the cycle of its next state
/// change, and the MCU only touches a peripheral when that cycle has been
/// reached (checked once per instruction against the cached minimum). Events
/// are processed at the end of the instruction during which they fall, which
/// is exactly when a per-instruction polling implementation would process
/// them, so the observable timing is identical.
pub struct Mcu<M: AddressSpace> {
    pub core: wdc_65c02::Core<St2205uAddressSpace<M>>,
    pub audio_sender: Box<dyn AudioInterface>,
    /// Host-facing USB seam plus its transaction pacing (serviced in
    /// `process_events`).
    usb_link: UsbHostLink,

    /// Cached minimum of all peripherals' next event cycles (sysck)
    next_event_sysck: u64,
    /// Oscillator cycle at which the audio interface wants its next sample
    next_audio_oscx: u64,
    /// Instruction cycle of the next GPIO input poll (a multiple of
    /// `GPIO_POLL_INTERVAL_SYSCK`)
    next_gpio_poll_sysck: u64,
    /// Instruction cycle of the next USB link service (a multiple of
    /// `UsbHostLink::MIN_TXN_INTERVAL_CYCLES`)
    next_usb_service_sysck: u64,
}

/// The host-facing USB interface together with the state that paces it. The
/// device services one host transaction per service call, so without a floor the
/// number of firmware cycles that elapse between two transactions would depend
/// entirely on how the owning loop is scheduled (flat-out in tests, but paced
/// with idle gaps in the GUI) - and the firmware needs to make main-loop
/// progress between transactions (e.g. `service_scsi` staging a read response
/// after a command block, before the host reads it back). Real USB already
/// spaces transactions in time and the device keeps up at that rate on hardware;
/// enforcing the floor here reproduces that, making USB correctness independent
/// of the host thread's timing rather than racy.
struct UsbHostLink {
    interface: Box<dyn UsbInterfaceInternal>,
    /// `core.cycles` at the most recent transaction we serviced.
    last_txn_cycle: u64,
    /// `core.cycles` at the most recent cable-presence refresh.
    last_presence_cycle: u64,
}

impl UsbHostLink {
    /// Minimum CPU cycles between servicing two host transactions (~1 ms at
    /// 16 MHz, measured to be ~2x the firmware's actual need; a 64-byte
    /// full-speed bulk packet is ~50 µs ≈ 800 cycles, plus per-command work).
    /// Reused as the cable-presence poll cadence - far finer than the firmware's
    /// ~2 s USBCON poll, so plug/unplug is reflected promptly.
    const MIN_TXN_INTERVAL_CYCLES: u64 = 16_000;

    fn new(interface: Box<dyn UsbInterfaceInternal>) -> Self {
        Self {
            interface,
            last_txn_cycle: 0,
            last_presence_cycle: 0,
        }
    }

    /// Refresh the device's cable-present status and service at most one pending
    /// host transaction against the device SIE - both rate-limited (presence so a
    /// per-step dyn call isn't made on the hot path; transactions because the
    /// device handles one per service and the firmware needs main-loop progress
    /// between them). Transactions queue and are never dropped, only paced. Runs
    /// even while the CPU is inside an ISR: on hardware the SIE moves bulk data
    /// autonomously, and the firmware's bulk-IN pump spin-waits in its ISR for the
    /// host to drain the buffer, so the host read must be serviced during the ISR
    /// or the two deadlock.
    fn service(&mut self, now_cycle: u64, usb: &mut UsbState) {
        if now_cycle.wrapping_sub(self.last_presence_cycle) >= Self::MIN_TXN_INTERVAL_CYCLES {
            usb.set_host_connected(self.interface.is_connected());
            self.last_presence_cycle = now_cycle;
        }
        if now_cycle.wrapping_sub(self.last_txn_cycle) < Self::MIN_TXN_INTERVAL_CYCLES {
            return;
        }
        if let Some(txn) = self.interface.poll_transaction() {
            let response = usb.handle_transaction(txn);
            self.interface.respond(response);
            self.last_txn_cycle = now_cycle;
        }
    }
}

/// The next multiple of `interval` strictly after `now`. Keeping periodic
/// polls on this fixed grid makes their cycles independent of when the poll
/// code happens to run, so a restored savestate polls at the same cycles the
/// original timeline did.
fn next_grid_cycle(now: u64, interval: u64) -> u64 {
    (now / interval + 1) * interval
}

impl<M: AddressSpace> Mcu<M> {
    pub fn new(
        frequency: u64,
        address_space: M,
        io: Box<dyn GpioInterfaceInternal>,
        mut audio_sender: Box<dyn AudioInterface>,
        usb_interface: Box<dyn UsbInterfaceInternal>,
    ) -> Self {
        audio_sender.set_clock_rate(frequency);

        let mut mcu = Self {
            core: wdc_65c02::Core::new(
                frequency,
                St2205uAddressSpace::new(address_space, io, frequency),
            ),
            audio_sender,
            usb_link: UsbHostLink::new(usb_interface),
            next_event_sysck: 0,
            next_audio_oscx: 0,
            next_gpio_poll_sysck: 0,
            next_usb_service_sysck: 0,
        };

        mcu.reset();
        mcu.next_audio_oscx = mcu.audio_sender.next_sample_cycle();
        mcu.process_events();

        mcu
    }

    /// Run until `core.cycles` reaches `target_sysck`, calling `observer`
    /// with the pre-execution core state once per executed instruction
    /// (exactly the states a `step()` loop would observe).
    ///
    /// Between two peripheral events no interrupt can become pending
    /// (interrupts are only asserted in `process_events` and by GPIO
    /// register accesses, which set `events_dirty`), so instructions are
    /// chained back-to-back with no per-instruction WAI/interrupt checks.
    /// Straight-line code additionally fetches sequentially by decode-cache
    /// key, skipping window resolution entirely; a streak breaks on any
    /// branch, 8K virtual boundary, or fetch invalidation.
    pub fn run<F: FnMut(&wdc_65c02::Core<St2205uAddressSpace<M>>)>(
        &mut self,
        target_sysck: u64,
        mut observer: F,
    ) {
        while self.core.cycles < target_sysck {
            // Whether the most recently executed instruction began inside an
            // ISR. RTI clears `interrupted` while executing, and the
            // instruction it returns to must run before the next interrupt
            // is taken - otherwise a persistently-asserted IRQ (e.g. a USB
            // bulk-OUT IRQ that only clears once the firmware drains the
            // buffer) would re-vector after every RTI and starve the main
            // loop forever.
            let mut was_interrupted = self.core.interrupted();
            if self.core.waiting_for_interrupt {
                // Nothing can happen until a peripheral event fires
                self.core.cycles = (self.core.cycles + 1).max(self.next_event_sysck);
            } else if self.core.address_space.interrupt.pending()
                || self.core.address_space.events_dirty
            {
                // Interrupt dispatch must be rechecked after every
                // instruction: single-step until the state clears
                observer(&self.core);
                self.core.address_space.boundary_sysck = self.core.cycles;
                self.core.step();
            } else {
                let limit = self.next_event_sysck.min(target_sysck);
                let mut streak_key = usize::MAX;
                let mut generation = self.core.address_space.fetch_generation();
                loop {
                    observer(&self.core);
                    self.core.address_space.boundary_sysck = self.core.cycles;
                    was_interrupted = self.core.interrupted();

                    let pc = self.core.registers.pc;
                    let (fins, key) = if streak_key != usize::MAX {
                        match self.core.address_space.fetch_sequential(streak_key) {
                            Some(fins) => (fins, streak_key),
                            None => self.core.address_space.fetch_keyed(pc),
                        }
                    } else {
                        self.core.address_space.fetch_keyed(pc)
                    };
                    self.core.execute_fetched(&fins);

                    if self.core.cycles >= limit
                        || self.core.address_space.events_dirty
                        || self.core.waiting_for_interrupt
                    {
                        break;
                    }

                    // Continue the sequential streak only if execution fell
                    // through (no branch), stayed inside the same 8K bank
                    // window region, and no fetch mapping was invalidated
                    let expected_pc = pc.wrapping_add(fins.length as u16);
                    let new_generation = self.core.address_space.fetch_generation();
                    streak_key = if key != usize::MAX
                        && self.core.registers.pc == expected_pc
                        && (pc ^ expected_pc) & !0x1FFF == 0
                        && new_generation == generation
                    {
                        key + fins.length as usize
                    } else {
                        usize::MAX
                    };
                    generation = new_generation;
                }
            }

            if self.core.cycles >= self.next_event_sysck || self.core.address_space.events_dirty {
                self.process_events();
            }

            if self.core.address_space.interrupt.pending() {
                // Cancel WAI mode if an interrupt is pending
                self.core.waiting_for_interrupt = false;

                if !self.core.flags.interrupt_disable
                    && !self.core.interrupted()
                    && !was_interrupted
                {
                    self.dispatch_interrupt();
                }
            }
        }
    }

    #[inline]
    pub fn step(&mut self) {
        // See `run` for why the pre-instruction ISR state gates dispatch
        let was_interrupted = self.core.interrupted();
        if self.core.waiting_for_interrupt {
            // Nothing can happen until a peripheral event fires, so jump
            // straight to the next one (never less than one cycle forward).
            self.core.cycles = (self.core.cycles + 1).max(self.next_event_sysck);
        } else {
            // Peripheral register accesses during this instruction take
            // effect at its starting cycle boundary.
            self.core.address_space.boundary_sysck = self.core.cycles;
            self.core.step();
        }

        if self.core.cycles >= self.next_event_sysck || self.core.address_space.events_dirty {
            self.process_events();
        }

        if self.core.address_space.interrupt.pending() {
            // Cancel WAI mode if an interrupt is pending
            self.core.waiting_for_interrupt = false;

            if !self.core.flags.interrupt_disable && !self.core.interrupted() && !was_interrupted {
                self.dispatch_interrupt();
            }
        }
    }

    /// Advance every peripheral whose next event cycle has been reached, and
    /// recompute the cached minimum. Order matches the original
    /// per-instruction update order: RTC, base timer, timers, USB, audio,
    /// GPIO.
    fn process_events(&mut self) {
        let sysck = self.core.instruction_cycles();
        let oscx = self.core.oscillator_cycles();
        self.core.address_space.events_dirty = false;

        if self.core.address_space.rtc.advance(oscx) {
            self.core
                .address_space
                .interrupt
                .assert_interrupt(Interrupt::Rtc);
        }

        if self.core.address_space.base_timer.advance(oscx) {
            self.core
                .address_space
                .interrupt
                .assert_interrupt(Interrupt::BaseTimer);
        }

        let timers_int = self.core.address_space.timer.advance(sysck);

        for i in 0..4 {
            // If a timer interrupt is pending, assert the interrupt and save the current PSG sample
            if timers_int & (1 << i) != 0 {
                let interrupt = match i {
                    0 => Interrupt::Timer0,
                    1 => Interrupt::Timer1,
                    2 => Interrupt::Timer2,
                    3 => Interrupt::Timer3,
                    _ => unreachable!(),
                };
                let channel = match i {
                    0 => PsgChannel::Channel0,
                    1 => PsgChannel::Channel1,
                    2 => PsgChannel::Channel2,
                    3 => PsgChannel::Channel3,
                    _ => unreachable!(),
                };

                self.core
                    .address_space
                    .interrupt
                    .assert_interrupt(interrupt);

                // This gets the state of the audio, and it will be sent to the audio interface when the interface wants it
                self.core.address_space.psg.pop_current_sample(channel);
            }
        }

        // Service the host-facing USB interface (paced; see `UsbHostLink`),
        // then raise the USB interrupt line if any enabled source is
        // asserted. A newly-set IRQ flag is observed by the firmware's next
        // USBIRQ read; the interrupt is only *vectored* when not already in
        // an ISR (see `run`). Firmware writes to USB registers set
        // `events_dirty`, so a source they raise is asserted here right
        // after the writing instruction.
        if sysck >= self.next_usb_service_sysck {
            self.usb_link
                .service(sysck, &mut self.core.address_space.usb);
            self.next_usb_service_sysck =
                next_grid_cycle(sysck, UsbHostLink::MIN_TXN_INTERVAL_CYCLES);
        }
        if self.core.address_space.usb.pending_irq() {
            self.core
                .address_space
                .interrupt
                .assert_interrupt(Interrupt::Usb);
        }

        // Sample the state of the PSG and send it to the audio interface
        if oscx >= self.next_audio_oscx {
            if self.audio_sender.needs_sample(oscx) {
                let mix = self.core.address_space.psg.get_mix_f32();
                self.audio_sender.add_sample(mix);
            }
            self.next_audio_oscx = self.audio_sender.next_sample_cycle();
        }

        // Refresh GPIO on the periodic input-poll grid, and additionally
        // whenever board circuitry scheduled an event of its own (an IR
        // edge due for replay needs cycle-accurate delivery, not poll
        // granularity).
        if sysck >= self.next_gpio_poll_sysck
            || oscx >= self.core.address_space.gpio.next_io_event_cycle()
        {
            self.core.address_space.refresh_gpio(oscx);
            self.next_gpio_poll_sysck = next_grid_cycle(sysck, GPIO_POLL_INTERVAL_SYSCK);
        }

        // Oscillator-domain events fire at the first instruction cycle whose
        // oscillator cycle has reached them
        let osc_to_sysck = |oscx: u64| oscx.div_ceil(2);
        self.next_event_sysck = self
            .core
            .address_space
            .timer
            .next_event()
            .min(osc_to_sysck(
                self.core.address_space.base_timer.next_event(),
            ))
            .min(osc_to_sysck(self.core.address_space.rtc.next_event()))
            .min(osc_to_sysck(self.next_audio_oscx))
            .min(self.next_gpio_poll_sysck)
            .min(osc_to_sysck(
                self.core.address_space.gpio.next_io_event_cycle(),
            ))
            .min(self.next_usb_service_sysck);
    }

    fn dispatch_interrupt(&mut self) {
        let interrupt = self
            .core
            .address_space
            .interrupt
            .highest_priority_interrupt();

        if let Some(interrupt) = interrupt {
            self.core
                .address_space
                .interrupt
                .clear_interrupt_request(interrupt);
            self.core.address_space.set_interrupted(true);
            self.core.push_u16(self.core.registers.pc);
            self.core.push_u8(self.core.flags.to_u8());

            let interrupt_vector = match interrupt {
                Interrupt::Intx => vector::INTX.into(),
                Interrupt::Timer0 => vector::T0.into(),
                Interrupt::Timer1 => vector::T1.into(),
                Interrupt::Timer2 => vector::T2.into(),
                Interrupt::Timer3 => vector::T3.into(),
                Interrupt::PortATransition => vector::PT.into(),
                Interrupt::BaseTimer => vector::BT.into(),
                Interrupt::LcdBuffer => vector::LCD.into(),
                Interrupt::SpiTxEmpty => vector::STX.into(),
                Interrupt::SpiRxReady => vector::SRX.into(),
                Interrupt::UartTx => vector::UTX.into(),
                Interrupt::UartRx => vector::URX.into(),
                Interrupt::Usb => vector::USB.into(),
                Interrupt::Pcm => vector::PCM.into(),
                Interrupt::Rtc => vector::RTC.into(),
            };

            self.core.registers.pc = self.core.address_space.read_u16_le(interrupt_vector);
        }
    }

    pub fn snapshot(&self, writer: &mut SnapshotWriter) {
        self.core.snapshot(writer);
    }

    pub fn restore(&mut self, reader: &mut SnapshotReader) -> Result<(), SnapshotError> {
        self.core.restore(reader)?;
        // The instruction boundary is run-loop scratch state, not part of
        // the snapshot; left stale it would stamp register accesses made
        // before the next instruction (e.g. by the platform after a
        // rollback) with pre-restore cycles.
        self.core.address_space.boundary_sysck = self.core.cycles;
        // The restored cycle counter may be far from where playback left
        // off; let the audio sink resynchronize its sample cursor.
        self.audio_sender
            .clock_rewound(self.core.oscillator_cycles());
        self.next_audio_oscx = self.audio_sender.next_sample_cycle();
        // Periodic polls live on fixed cycle grids, so the restored
        // schedule matches what the original timeline used at this cycle.
        let sysck = self.core.instruction_cycles();
        self.next_gpio_poll_sysck = next_grid_cycle(sysck, GPIO_POLL_INTERVAL_SYSCK);
        self.next_usb_service_sysck = next_grid_cycle(sysck, UsbHostLink::MIN_TXN_INTERVAL_CYCLES);
        // Recompute the event schedule from the restored peripheral state
        self.process_events();
        Ok(())
    }

    pub fn reset(&mut self) {
        self.core.set_interrupted(true);
        let reset_vector = self.core.address_space.read_u16_le(vector::RESET.into());
        self.core.registers.pc = reset_vector;
        self.core.set_interrupted(false);
    }

    pub fn read_machine_area(&mut self, start: usize, size: usize) -> Vec<u8> {
        let end = start + size;
        let mut data = Vec::<u8>::with_capacity(size);

        for addr in start..end {
            data.push(self.core.address_space.machine_addr_space.read_u8(addr));
        }

        data
    }
}
