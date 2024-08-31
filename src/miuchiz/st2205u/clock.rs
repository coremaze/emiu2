use crate::memory::AddressSpace;

use super::St2205uAddressSpace;

pub trait Clock {
    fn set_clocks(&mut self, clocks: u64, sysck: u64);
}

impl<'a, A: AddressSpace> Clock for St2205uAddressSpace<'a, A> {
    fn set_clocks(&mut self, oscx: u64, sysck: u64) {
        self.base_timer.set_elapsed_ticks(oscx);
        self.timer.set_elapsed_ticks(sysck);
    }
}
