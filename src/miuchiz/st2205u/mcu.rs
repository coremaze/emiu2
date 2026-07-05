use super::clock::Clock;
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

/// Representation of a ST2205U microcontroller.
///
/// This microcontroller is capable of, through the use of bank registers,
/// accessing a larger address space than the 65C02 core itself can.
///
/// This device also implements its own address space, which is addressible using
/// 16 bits, which is directly exposed to the underlying 65C02.
pub struct Mcu {
    pub core: wdc_65c02::Core<St2205uAddressSpace>,
    pub audio_sender: Box<dyn AudioInterface>,
    /// Host-facing USB seam plus its transaction pacing (serviced in `step`).
    usb_link: UsbHostLink,
}

/// The host-facing USB interface together with the state that paces it. The
/// device services one host transaction per `Mcu::step`, so without a floor the
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
    /// device handles one per `step` and the firmware needs main-loop progress
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

impl Mcu {
    pub fn new(
        frequency: u64,
        address_space: Box<dyn AddressSpace>,
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
        };

        mcu.reset();

        mcu
    }

    pub fn step(&mut self) {
        // Whether we were already inside an interrupt service routine at the
        // start of this step. RTI clears `interrupted` while executing (below),
        // and we must let the instruction it returns to run before taking the
        // next interrupt - otherwise a persistently-asserted IRQ (e.g. a USB
        // bulk-OUT IRQ that only clears once the firmware drains the buffer)
        // would re-vector after every RTI and starve the main loop forever.
        let was_interrupted = self.core.interrupted();

        self.core.step();
        self.core.address_space.set_clocks(
            self.core.oscillator_cycles(),
            self.core.instruction_cycles(),
        );

        if self.core.address_space.base_timer.update() {
            self.core
                .address_space
                .interrupt
                .assert_interrupt(Interrupt::BaseTimer);
        }

        let timers_int = self.core.address_space.timer.update();

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

        // Service the host-facing USB interface (paced; see `UsbHostLink`), then
        // raise the USB interrupt line if any enabled source is asserted. A
        // newly-set IRQ flag is observed by the firmware's next USBIRQ read; the
        // interrupt is only *vectored* when not already in an ISR (see below).
        let now_cycle = self.core.cycles;
        self.usb_link
            .service(now_cycle, &mut self.core.address_space.usb);
        if self.core.address_space.usb.pending_irq() {
            self.core
                .address_space
                .interrupt
                .assert_interrupt(Interrupt::Usb);
        }

        // Sample the state of the PSG and send it to the audio interface
        if self
            .audio_sender
            .needs_sample(self.core.oscillator_cycles())
        {
            let mix = self.core.address_space.psg.get_mix_f32();
            self.audio_sender.add_sample(mix);
        }

        let activity = self
            .core
            .address_space
            .update_gpio(self.core.oscillator_cycles());
        if activity.port_a_transition {
            self.core
                .address_space
                .interrupt
                .assert_interrupt(Interrupt::PortATransition);
        }
        if activity.intx {
            self.core
                .address_space
                .interrupt
                .assert_interrupt(Interrupt::Intx);
        }

        let interrupt = self
            .core
            .address_space
            .interrupt
            .highest_priority_interrupt();

        // Cancel WAI mode if an interrupt is pending
        if interrupt.is_some() && self.core.waiting_for_interrupt {
            self.core.waiting_for_interrupt = false;
        }

        if !self.core.flags.interrupt_disable && !self.core.interrupted() && !was_interrupted {
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
    }

    pub fn snapshot(&self, writer: &mut SnapshotWriter) {
        self.core.snapshot(writer);
    }

    pub fn restore(&mut self, reader: &mut SnapshotReader) -> Result<(), SnapshotError> {
        self.core.restore(reader)?;
        // The restored cycle counter may be far from where playback left
        // off; let the audio sink resynchronize its sample cursor.
        self.audio_sender
            .clock_rewound(self.core.oscillator_cycles());
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
