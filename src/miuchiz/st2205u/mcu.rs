use super::clock::Clock;
use super::interrupt::Interrupt;
use super::psg::PsgChannel;
use super::vector;
use super::wdc_65c02;
use super::wdc_65c02::HandlesInterrupt;
use super::St2205uAddressSpace;
use crate::audio::{AudioInterface, AudioSender};
use crate::gpio::Gpio;
use crate::memory::AddressSpace;

/// Representation of a ST2205U microcontroller.
///
/// This microcontroller is capable of, through the use of bank registers,
/// accessing a larger address space than the 65C02 core itself can, represented
/// by `A`.
///
/// This device also implements its own address space, which is addressible using
/// 16 bits, which is directly exposed to the underlying 65C02.
pub struct Mcu<'a, A: AddressSpace> {
    pub core: wdc_65c02::Core<St2205uAddressSpace<'a, A>>,
    pub audio_sender: AudioSender,
}

impl<'a, A: AddressSpace> Mcu<'a, A> {
    pub fn new(
        frequency: u64,
        address_space: A,
        io: &'a impl Gpio,
        mut audio_sender: AudioSender,
    ) -> Self {
        audio_sender.set_clock_rate(frequency);
        let mut mcu = Self {
            core: wdc_65c02::Core::new(
                frequency,
                St2205uAddressSpace::new(address_space, io, frequency),
            ),
            audio_sender,
        };

        mcu.reset();

        mcu
    }

    pub fn step(&mut self) {
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

        // Sample the state of the PSG and send it to the audio interface
        if self
            .audio_sender
            .needs_sample(self.core.oscillator_cycles())
        {
            let mix = self.core.address_space.psg.get_mix_f32();
            self.audio_sender.add_sample(mix);
        }

        let interrupt = self
            .core
            .address_space
            .interrupt
            .highest_priority_interrupt();

        if !self.core.flags.interrupt_disable && !self.core.interrupted() {
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

    pub fn reset(&mut self) {
        self.core.set_interrupted(true);
        let reset_vector = self.core.address_space.read_u16_le(vector::RESET.into());
        self.core.registers.pc = reset_vector;
        self.core.set_interrupted(false);
    }
}
