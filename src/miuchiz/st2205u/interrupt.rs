use super::reg::U16Register;
use super::wdc_65c02::HandlesInterrupt;

#[derive(Debug)]
pub struct State {
    ireq: U16Register,
    shadow_ireq: U16Register, // Exists to prevent interrupts from firing continuously if the interrupt is not disabled by the program
    iena: U16Register,

    interrupted: bool,
}

#[derive(Debug, Copy, Clone)]
pub enum Interrupt {
    Intx,
    Timer0,
    Timer1,
    Timer2,
    Timer3,
    PortATransition,
    BaseTimer,
    LcdBuffer,
    SpiTxEmpty,
    SpiRxReady,
    UartTx,
    UartRx,
    Usb,
    Pcm,
    Rtc,
}

impl State {
    pub fn new() -> Self {
        Self {
            ireq: U16Register::new(0b0000_0000_0000_0000, 0b1101_1111_1111_1111),
            shadow_ireq: U16Register::new(0b0000_0000_0000_0000, 0b1101_1111_1111_1111),
            iena: U16Register::new(0b0000_0000_0000_0000, 0b1101_1111_1111_1111),
            interrupted: false,
        }
    }

    pub fn assert_interrupt(&mut self, irq: Interrupt) {
        let bit = match irq {
            Interrupt::Intx => 0,
            Interrupt::Timer0 => 1,
            Interrupt::Timer1 => 2,
            Interrupt::Timer2 => 3,
            Interrupt::Timer3 => 4,
            Interrupt::PortATransition => 5,
            Interrupt::BaseTimer => 6,
            Interrupt::LcdBuffer => 7,
            Interrupt::SpiTxEmpty => 8,
            Interrupt::SpiRxReady => 9,
            Interrupt::UartTx => 10,
            Interrupt::UartRx => 11,
            Interrupt::Usb => 12,
            Interrupt::Pcm => 14,
            Interrupt::Rtc => 15,
        };

        let mask = 1u16 << bit;

        // Check if the interrupt is enabled before asserting
        if self.iena.u16() & mask != 0 {
            // It is now the executor's responsibility to check this register
            self.ireq.set_u16(self.ireq.u16() | mask);
            self.shadow_ireq.set_u16(self.shadow_ireq.u16() | mask);
        }
    }

    pub fn highest_priority_interrupt(&self) -> Option<Interrupt> {
        // if self.shadow_ireq.u16() != 0 {
        //     dbg!(&self);
        // }
        for i in 0..16 {
            if self.shadow_ireq.u16() & (1 << i) != 0 {
                return Some(match i {
                    0 => Interrupt::Intx,
                    1 => Interrupt::Timer0,
                    2 => Interrupt::Timer1,
                    3 => Interrupt::Timer2,
                    4 => Interrupt::Timer3,
                    5 => Interrupt::PortATransition,
                    6 => Interrupt::BaseTimer,
                    7 => Interrupt::LcdBuffer,
                    8 => Interrupt::SpiTxEmpty,
                    9 => Interrupt::SpiRxReady,
                    10 => Interrupt::UartTx,
                    11 => Interrupt::UartRx,
                    12 => Interrupt::Usb,
                    14 => Interrupt::Pcm,
                    15 => Interrupt::Rtc,
                    _ => unreachable!(),
                });
            }
        }
        None
    }

    pub fn clear_interrupt_request(&mut self, irq: Interrupt) {
        let bit = match irq {
            Interrupt::Intx => 0,
            Interrupt::Timer0 => 1,
            Interrupt::Timer1 => 2,
            Interrupt::Timer2 => 3,
            Interrupt::Timer3 => 4,
            Interrupt::PortATransition => 5,
            Interrupt::BaseTimer => 6,
            Interrupt::LcdBuffer => 7,
            Interrupt::SpiTxEmpty => 8,
            Interrupt::SpiRxReady => 9,
            Interrupt::UartTx => 10,
            Interrupt::UartRx => 11,
            Interrupt::Usb => 12,
            Interrupt::Pcm => 14,
            Interrupt::Rtc => 15,
        };

        let mask = 1u16 << bit;

        self.shadow_ireq.set_u16(self.shadow_ireq.u16() & !mask);
    }
}

impl HandlesInterrupt for State {
    fn set_interrupted(&mut self, interrupted: bool) {
        self.interrupted = interrupted;
    }

    fn interrupted(&self) -> bool {
        self.interrupted
    }
}

pub fn read_ireql(state: &State) -> u8 {
    state.ireq.l()
}

pub fn read_ireqh(state: &State) -> u8 {
    state.ireq.h()
}

pub fn read_ienal(state: &State) -> u8 {
    state.iena.l()
}

pub fn read_ienah(state: &State) -> u8 {
    state.iena.h()
}

pub fn write_ireql(state: &mut State, value: u8) {
    // Bits set to 1 indicate do nothing
    // Bits set to 0 indicate clear irq
    let ireql = state.ireq.l();
    state.ireq.set_l(ireql & value);
}

pub fn write_ireqh(state: &mut State, value: u8) {
    // Bits set to 1 indicate do nothing
    // Bits set to 0 indicate clear irq
    let ireqh = state.ireq.h();
    state.ireq.set_h(ireqh & value);
}

pub fn write_ienal(state: &mut State, value: u8) {
    state.iena.set_l(value);
}

pub fn write_ienah(state: &mut State, value: u8) {
    state.iena.set_h(value);
}
