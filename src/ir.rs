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

    /// The receiver module's power supply changed state (PB6 on the
    /// Miuchiz; the firmware powers the receiver off while transmitting
    /// and between listen windows). Called only on change. Transports can
    /// use this to tell when the machine can and cannot hear.
    fn set_receiver_power(&mut self, _cycle: u64, _powered: bool) {}
}

/// What the platform's run loop should do next to keep a high-latency IR
/// link working, as decided by the transport.
///
/// The protocol: the transport asks for a snapshot whenever the receiver
/// powers on (the moment the machine's future becomes dependent on remote
/// input). When a frame arrives too late to be heard live, the transport
/// asks for a rollback; the run loop restores the snapshot and confirms,
/// and the transport replays the frame early in the restored listen
/// window. From the firmware's point of view the reply simply arrived in
/// time.
pub enum RollbackDirective {
    /// Nothing to do.
    Continue,
    /// The receiver just powered on: snapshot the machine and confirm
    /// with `snapshot_taken`.
    TakeSnapshot,
    /// A frame can only be heard by rewinding: restore the last snapshot
    /// and confirm with `rolled_back`.
    RollBack,
}

/// The run-loop side of rollback coordination. Obtained from transports
/// that support it (an emulator without one, or on a local link, never
/// pays for snapshots).
///
/// All cycle arguments are emulated oscillator cycles, the same clock
/// `IrInterface` uses.
pub trait IrRollbackControl {
    /// Called regularly (once per pacing quantum) by the run loop.
    fn poll(&mut self, now_cycle: u64) -> RollbackDirective;

    /// The run loop captured a snapshot; `cycle` is where a rollback will
    /// restore to.
    fn snapshot_taken(&mut self, cycle: u64);

    /// The run loop restored the snapshot: the machine is at
    /// `restored_cycle`, and the timeline from there to `abandoned_cycle`
    /// no longer happened.
    fn rolled_back(&mut self, restored_cycle: u64, abandoned_cycle: u64);
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
