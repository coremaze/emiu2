use super::reg::U8Register;

const TIMER_FREQUENCY: u64 = 8192;

pub struct State {
    /// The frequency of the clock source this timer receives
    input_clock_frequency: u64,

    /// The number of cycles at `clock_frequency` which have elapsed
    elapsed_ticks: u64,

    /// How many ticks have elapsed on this timer
    counter: u64,

    /// When, in terms of `input_clock_frequency`, should the next tick be
    next_counter_tick: u64,

    /// Used for BTREQ7
    btc: U8Register,

    /// Base timer enable
    bten: U8Register,

    // Base timer status
    btreq: U8Register,
}

impl State {
    pub fn new(clock_frequency: u64) -> Self {
        let mut timer = Self {
            input_clock_frequency: clock_frequency,
            elapsed_ticks: 0,
            counter: 0,
            next_counter_tick: 0,
            btc: U8Register::new(0b0000_0000, 0b1111_1111),
            bten: U8Register::new(0b0000_0000, 0b1111_1111),
            btreq: U8Register::new(0b0000_0000, 0b1111_1111),
        };
        timer.update_next_counter_tick();
        timer
    }

    pub fn set_elapsed_ticks(&mut self, ticks: u64) {
        self.elapsed_ticks = ticks;
    }

    fn update_next_counter_tick(&mut self) {
        self.next_counter_tick =
            ((self.counter + 1) * self.input_clock_frequency) / TIMER_FREQUENCY;
        // println!("Next timer at {}", self.next_counter_tick);
    }

    fn increment_counter(&mut self) {
        self.counter += 1;
        self.update_next_counter_tick();
    }

    fn btc(&self) -> u64 {
        self.counter % 8192
    }

    /// Update the state of the timer. Returns whether it should trigger an interrupt
    pub fn update(&mut self) -> bool {
        // Increase counter only once enough time has elapsed
        if self.elapsed_ticks < self.next_counter_tick {
            return false;
        }

        self.increment_counter();

        // if self.btc() == 0 {
        //     println!("1 Hz tick");
        // }

        let clock = self.btc();

        // Calculate which timers should have triggered

        // 2Hz
        let btreq0 = clock % (TIMER_FREQUENCY / 2) == 0;
        // 32 Hz
        let btreq1 = clock % (TIMER_FREQUENCY / 32) == 0;
        // 64 Hz
        let btreq2 = clock % (TIMER_FREQUENCY / 64) == 0;
        // 128 Hz
        let btreq3 = clock % (TIMER_FREQUENCY / 128) == 0;
        // 256 Hz
        let btreq4 = clock % (TIMER_FREQUENCY / 256) == 0;
        // 512 Hz
        let btreq5 = clock % (TIMER_FREQUENCY / 512) == 0;
        // 2048 Hz
        let btreq6 = clock % (TIMER_FREQUENCY / 2048) == 0;
        // 8192 Hz or BTC
        let btreq7 = if self.btc.get() == 0 {
            clock % (TIMER_FREQUENCY / 8192) == 0
        } else {
            clock % (TIMER_FREQUENCY / self.btc.get() as u64) == 0
        };

        // Put the new bits in place
        let mut btreq = 0;
        btreq |= (btreq0 as u8) << 0;
        btreq |= (btreq1 as u8) << 1;
        btreq |= (btreq2 as u8) << 2;
        btreq |= (btreq3 as u8) << 3;
        btreq |= (btreq4 as u8) << 4;
        btreq |= (btreq5 as u8) << 5;
        btreq |= (btreq6 as u8) << 6;
        btreq |= (btreq7 as u8) << 7;

        // Only use the bits which are enabled
        btreq &= self.bten.get();

        let assert_new_interrupt = btreq != 0;

        // Old bits should be kept too
        btreq |= self.btreq.get();

        self.btreq.set(btreq);

        assert_new_interrupt
    }
}

pub fn read_bten(state: &State) -> u8 {
    state.bten.get()
}

pub fn write_bten(state: &mut State, value: u8) {
    state.bten.set(value);
}

pub fn read_btreq(state: &State) -> u8 {
    state.btreq.get()
}

pub fn write_btreq(state: &mut State, value: u8) {
    // Writing to BTREQ should CLEAR a bit if 1 is written
    // In other words, only bits where `value` is 0 should be kept
    let mask = !value;

    state.btreq.set(state.btreq.get() & mask);
}

pub fn read_btc(state: &State) -> u8 {
    state.btc.get()
}

pub fn write_btc(state: &mut State, value: u8) {
    state.btc.set(value);
}
