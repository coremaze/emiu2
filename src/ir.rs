/// A demodulated-envelope infrared link, as seen at the edge of the device.
///
/// The handheld transmits by gating a ~38kHz carrier on and off (Timer0
/// toggles PE0 through the ST2205U's TCO0 clocking output) and receives
/// through a demodulating receiver module whose active-low output feeds PE1
/// (INTX1). Implementations of this trait only deal in the demodulated
/// envelope: `true` means "carrier present". The carrier itself is never
/// synthesized.
///
/// Timestamps are in emulated oscillator cycles; `set_clock_rate` provides
/// the cycle rate so implementations can convert to time.
pub trait IrInterface {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64);

    /// The transmit envelope changed state. Called only on change.
    fn set_carrier(&mut self, cycle: u64, carrier: bool);

    /// Whether the receiver currently detects a carrier. Polled every few
    /// hundred cycles.
    fn carrier_detected(&mut self, cycle: u64) -> bool;
}

/// An IR interface with nothing on the other end. Transmissions go nowhere
/// and no carrier is ever received.
pub struct DisconnectedIr;

impl IrInterface for DisconnectedIr {
    fn set_clock_rate(&mut self, _emulated_clock_rate: u64) {}

    fn set_carrier(&mut self, _cycle: u64, _carrier: bool) {}

    fn carrier_detected(&mut self, _cycle: u64) -> bool {
        false
    }
}
