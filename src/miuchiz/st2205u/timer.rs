// 12-bit counter; an increment past 0x0FFF overflows and triggers an interrupt
const COUNTER_MODULUS: u64 = 4096;

/// A hardware timer, modeled in closed form: instead of ticking every cycle,
/// the counter is defined by a reference point (`counter_base` at
/// `base_cycle`) plus the number of divisor multiples elapsed since. This
/// yields identical per-cycle behavior to a ticking implementation:
/// increments occur exactly at cycles that are multiples of the divisor.
pub struct TimerState {
    /// Counter value at `base_cycle`
    counter_base: u16,
    /// Cycle (sysck) at which the counter was last known exactly; increments
    /// are counted for divisor multiples strictly after this cycle
    base_cycle: u64,
    reload_value: u16,
    clock_select: u8,
    enabled: bool,
    auto_reload: bool,
    /// Cycle (sysck) of the next overflow, u64::MAX if the timer never
    /// overflows (disabled or unimplemented clock source)
    next_overflow: u64,
}

pub struct TimerBlocksState {
    t0: TimerState,
    t1: TimerState,
    t2: TimerState,
    t3: TimerState,
}

pub enum TimerIndex {
    T0,
    T1,
    T2,
    T3,
}

impl TimerBlocksState {
    pub fn new() -> Self {
        Self {
            t0: TimerState::new(),
            t1: TimerState::new(),
            t2: TimerState::new(),
            t3: TimerState::new(),
        }
    }

    fn timers(&mut self) -> [&mut TimerState; 4] {
        [&mut self.t0, &mut self.t1, &mut self.t2, &mut self.t3]
    }

    fn timer(&self, timer: TimerIndex) -> &TimerState {
        match timer {
            TimerIndex::T0 => &self.t0,
            TimerIndex::T1 => &self.t1,
            TimerIndex::T2 => &self.t2,
            TimerIndex::T3 => &self.t3,
        }
    }

    fn timer_mut(&mut self, timer: TimerIndex) -> &mut TimerState {
        match timer {
            TimerIndex::T0 => &mut self.t0,
            TimerIndex::T1 => &mut self.t1,
            TimerIndex::T2 => &mut self.t2,
            TimerIndex::T3 => &mut self.t3,
        }
    }

    /// Process all overflows up to and including `cycle` (sysck). Returns a
    /// bitmask of timers which overflowed, matching the per-instruction
    /// interrupt semantics of the ticking implementation.
    pub fn advance(&mut self, cycle: u64) -> u8 {
        let mut interrupts = 0;
        for (i, timer) in self.timers().into_iter().enumerate() {
            if timer.advance(cycle) {
                interrupts |= 1 << i;
            }
        }
        interrupts
    }

    /// The earliest cycle (sysck) at which any timer overflows
    pub fn next_event(&self) -> u64 {
        self.t0
            .next_overflow
            .min(self.t1.next_overflow)
            .min(self.t2.next_overflow)
            .min(self.t3.next_overflow)
    }

    pub fn read_txcl(&self, timer: TimerIndex, boundary: u64) -> u8 {
        (self.timer(timer).counter_at(boundary) & 0xFF) as u8
    }

    pub fn write_txcl(&mut self, timer: TimerIndex, value: u8, boundary: u64) {
        let timer = self.timer_mut(timer);
        timer.rebase(boundary);
        timer.counter_base = (timer.counter_base & 0x0F00) | value as u16;
        timer.reload_value = (timer.reload_value & 0x0F00) | value as u16;
        timer.recompute_next_overflow();
    }

    pub fn read_txch(&self, timer: TimerIndex, boundary: u64) -> u8 {
        let timer = self.timer(timer);
        let auto_reload = if timer.auto_reload { 0x80 } else { 0 };
        auto_reload | (timer.clock_select << 4) | ((timer.counter_at(boundary) >> 8) & 0x0F) as u8
    }

    pub fn write_txch(&mut self, timer: TimerIndex, value: u8, boundary: u64) {
        let timer = self.timer_mut(timer);
        // Capture the current counter before the clock source changes
        timer.rebase(boundary);
        timer.auto_reload = (value & 0x80) != 0;
        timer.clock_select = (value >> 4) & 0x07;
        timer.counter_base = (timer.counter_base & 0x00FF) | ((value as u16 & 0x0F) << 8);
        timer.reload_value = (timer.reload_value & 0x00FF) | ((value as u16 & 0x0F) << 8);
        timer.recompute_next_overflow();
    }

    pub fn read_tien(&self) -> u8 {
        (self.t0.enabled as u8) << 0
            | (self.t1.enabled as u8) << 1
            | (self.t2.enabled as u8) << 2
            | (self.t3.enabled as u8) << 3
    }

    pub fn write_tien(&mut self, value: u8, boundary: u64) {
        // todo: T4 is not implemented
        for (i, timer) in self.timers().into_iter().enumerate() {
            let enabled = value & (1 << i) != 0;
            if timer.enabled != enabled {
                // Capture the counter at the moment it stops/resumes ticking
                timer.rebase(boundary);
                timer.enabled = enabled;
                timer.recompute_next_overflow();
            }
        }
    }
}

impl TimerState {
    fn new() -> Self {
        Self {
            counter_base: 0,
            base_cycle: 0,
            reload_value: 0,
            clock_select: 0,
            enabled: false,
            auto_reload: false,
            next_overflow: u64::MAX,
        }
    }

    fn divisor(&self) -> Option<u64> {
        match self.clock_select {
            0 => Some(2),    // SYSCK/2
            1 => Some(4),    // SYSCK/4
            2 => Some(8),    // SYSCK/8
            3 => Some(32),   // SYSCK/32
            4 => Some(1024), // SYSCK/1024
            5 => Some(4096), // SYSCK/4096
            _ => None,       // BGRCK / external clock (not implemented)
        }
    }

    fn ticking_divisor(&self) -> Option<u64> {
        if self.enabled {
            self.divisor()
        } else {
            None
        }
    }

    fn counter_at(&self, cycle: u64) -> u16 {
        match self.ticking_divisor() {
            Some(d) => {
                let increments = cycle / d - self.base_cycle / d;
                ((self.counter_base as u64 + increments) % COUNTER_MODULUS) as u16
            }
            None => self.counter_base,
        }
    }

    /// Re-anchor the reference point at `cycle` without changing behavior
    fn rebase(&mut self, cycle: u64) {
        self.counter_base = self.counter_at(cycle);
        self.base_cycle = cycle;
    }

    fn recompute_next_overflow(&mut self) {
        self.next_overflow = match self.ticking_divisor() {
            Some(d) => {
                let increments_to_overflow = COUNTER_MODULUS - self.counter_base as u64;
                (self.base_cycle / d + increments_to_overflow) * d
            }
            None => u64::MAX,
        };
    }

    fn advance(&mut self, cycle: u64) -> bool {
        if cycle < self.next_overflow {
            return false;
        }

        while self.next_overflow <= cycle {
            self.counter_base = if self.auto_reload {
                self.reload_value & 0x0FFF
            } else {
                0
            };
            self.base_cycle = self.next_overflow;
            self.recompute_next_overflow();
        }

        true
    }
}
