use super::gpio::GpioInterfaceInternal;
use super::interrupt::Interrupt;
use super::psg::PsgChannel;
use super::vector;
use super::wdc_65c02;
use super::wdc_65c02::HandlesInterrupt;
use super::St2205uAddressSpace;
use crate::audio::AudioInterface;
use crate::memory::AddressSpace;

/// How often GPIO inputs are polled, in instruction cycles (~1 ms of
/// emulated time). Inputs change at human timescales, so per-instruction
/// polling is wasted work; port reads between polls see at most this much
/// latency.
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

    /// Cached minimum of all peripherals' next event cycles (sysck)
    next_event_sysck: u64,
    /// Oscillator cycle at which the audio interface wants its next sample
    next_audio_oscx: u64,
    /// Instruction cycle of the next GPIO input poll
    next_gpio_poll_sysck: u64,
}

impl<M: AddressSpace> Mcu<M> {
    pub fn new(
        frequency: u64,
        address_space: M,
        io: Box<dyn GpioInterfaceInternal>,
        mut audio_sender: Box<dyn AudioInterface>,
    ) -> Self {
        audio_sender.set_clock_rate(frequency);
        let mut mcu = Self {
            core: wdc_65c02::Core::new(
                frequency,
                St2205uAddressSpace::new(address_space, io, frequency),
            ),
            audio_sender,
            next_event_sysck: 0,
            next_audio_oscx: 0,
            next_gpio_poll_sysck: 0,
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
    /// (interrupts are only asserted in `process_events`), so instructions
    /// are chained back-to-back with no per-instruction WAI/interrupt
    /// checks. Straight-line code additionally fetches sequentially by
    /// decode-cache key, skipping window resolution entirely; a streak
    /// breaks on any branch, 8K virtual boundary, or fetch invalidation.
    pub fn run<F: FnMut(&wdc_65c02::Core<St2205uAddressSpace<M>>)>(
        &mut self,
        target_sysck: u64,
        mut observer: F,
    ) {
        while self.core.cycles < target_sysck {
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

                if !self.core.flags.interrupt_disable && !self.core.interrupted() {
                    self.dispatch_interrupt();
                }
            }
        }
    }

    #[inline]
    pub fn step(&mut self) {
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

            if !self.core.flags.interrupt_disable && !self.core.interrupted() {
                self.dispatch_interrupt();
            }
        }
    }

    /// Advance every peripheral whose next event cycle has been reached, and
    /// recompute the cached minimum. Order matches the original
    /// per-instruction update order: RTC, base timer, timers, audio, GPIO.
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

        // Sample the state of the PSG and send it to the audio interface
        if oscx >= self.next_audio_oscx {
            if self.audio_sender.needs_sample(oscx) {
                let mix = self.core.address_space.psg.get_mix_f32();
                self.audio_sender.add_sample(mix);
            }
            self.next_audio_oscx = self.audio_sender.next_sample_cycle();
        }

        if sysck >= self.next_gpio_poll_sysck {
            let port_a_transition = self
                .core
                .address_space
                .gpio
                .update_gpio_and_detect_pa_transition();
            if port_a_transition {
                self.core
                    .address_space
                    .interrupt
                    .assert_interrupt(Interrupt::PortATransition);
            }
            self.next_gpio_poll_sysck = sysck + GPIO_POLL_INTERVAL_SYSCK;
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
            .min(self.next_gpio_poll_sysck);
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
