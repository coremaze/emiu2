pub trait AudioInterface {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64);
    fn needs_sample(&self, current_cycle: u64) -> bool;
    fn add_sample(&mut self, value: f32);

    /// Called when the emulated cycle counter jumps (a savestate load or
    /// rollback) so the sink can move its sample cursor along with it.
    fn clock_rewound(&mut self, _current_cycle: u64) {}
}
