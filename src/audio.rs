pub trait AudioInterface {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64);
    fn needs_sample(&self, current_cycle: u64) -> bool;
    fn add_sample(&mut self, value: f32);
}
