pub struct TimerState {
    counter: u16, // 12-bit counter
    reload_value: u16,
    clock_select: u8,
    enabled: bool,
    auto_reload: bool,
}

pub struct TimerBlocksState {
    t0: TimerState,
    t1: TimerState,
    t2: TimerState,
    t3: TimerState,
    elapsed_ticks: u64,
}

impl TimerBlocksState {
    pub fn new() -> Self {
        Self {
            t0: TimerState::new(),
            t1: TimerState::new(),
            t2: TimerState::new(),
            t3: TimerState::new(),
            elapsed_ticks: 0,
        }
    }

    pub fn set_elapsed_ticks(&mut self, ticks: u64) {
        self.elapsed_ticks = ticks;
    }

    pub fn update(&mut self) -> u8 {
        let mut interrupts = 0;

        for (i, timer) in [&mut self.t0, &mut self.t1, &mut self.t2, &mut self.t3]
            .iter_mut()
            .enumerate()
        {
            if !timer.enabled {
                continue;
            }

            let should_increment = match timer.clock_select {
                0 => self.elapsed_ticks % 2 == 0,    // SYSCK/2
                1 => self.elapsed_ticks % 4 == 0,    // SYSCK/4
                2 => self.elapsed_ticks % 8 == 0,    // SYSCK/8
                3 => self.elapsed_ticks % 32 == 0,   // SYSCK/32
                4 => self.elapsed_ticks % 1024 == 0, // SYSCK/1024
                5 => self.elapsed_ticks % 4096 == 0, // SYSCK/4096
                6 => false,                          // BGRCK (not implemented in this example)
                7 => false, // External clock (not implemented in this example)
                _ => false,
            };

            if should_increment {
                let updated_counter16 = timer.counter + 1;
                let updated_counter12 = updated_counter16 & 0x0FFF;
                let overflowed = updated_counter12 != updated_counter16; // 12-bit overflow
                timer.counter = updated_counter12;

                if overflowed {
                    interrupts |= 1 << i;
                    if timer.auto_reload {
                        timer.counter = timer.reload_value;
                    } else {
                        timer.counter = 0;
                    }
                }
            }
        }

        interrupts
    }

    pub fn read_txcl(&self, timer: usize) -> u8 {
        let timer = match timer {
            0 => &self.t0,
            1 => &self.t1,
            2 => &self.t2,
            3 => &self.t3,
            _ => panic!("Invalid timer"),
        };
        (timer.counter & 0xFF) as u8
    }

    pub fn write_txcl(&mut self, timer: usize, value: u8) {
        let timer = match timer {
            0 => &mut self.t0,
            1 => &mut self.t1,
            2 => &mut self.t2,
            3 => &mut self.t3,
            _ => panic!("Invalid timer"),
        };
        timer.counter = (timer.counter & 0xFF00) | value as u16;
        timer.reload_value = (timer.reload_value & 0xFF00) | value as u16;
    }

    pub fn read_txch(&self, timer: usize) -> u8 {
        let timer = match timer {
            0 => &self.t0,
            1 => &self.t1,
            2 => &self.t2,
            3 => &self.t3,
            _ => panic!("Invalid timer"),
        };
        let auto_reload = if timer.auto_reload { 0x80 } else { 0 };
        auto_reload | (timer.clock_select << 4) | ((timer.counter >> 8) & 0x0F) as u8
    }

    pub fn write_txch(&mut self, timer: usize, value: u8) {
        let timer = match timer {
            0 => &mut self.t0,
            1 => &mut self.t1,
            2 => &mut self.t2,
            3 => &mut self.t3,
            _ => panic!("Invalid timer"),
        };
        timer.auto_reload = (value & 0x80) != 0;
        timer.clock_select = (value >> 4) & 0x07;
        timer.counter = (timer.counter & 0x00FF) | ((value as u16 & 0x0F) << 8);
        timer.reload_value = (timer.reload_value & 0x00FF) | ((value as u16 & 0x0F) << 8);
    }

    pub fn read_tien(&self) -> u8 {
        (self.t0.enabled as u8) << 0
            | (self.t1.enabled as u8) << 1
            | (self.t2.enabled as u8) << 2
            | (self.t3.enabled as u8) << 3
    }

    pub fn write_tien(&mut self, value: u8) {
        self.t0.enabled = (value & 0b00000001) != 0;
        self.t1.enabled = (value & 0b00000010) != 0;
        self.t2.enabled = (value & 0b00000100) != 0;
        self.t3.enabled = (value & 0b00001000) != 0;
        // todo: T4 is not implemented
    }
}

impl TimerState {
    fn new() -> Self {
        Self {
            counter: 0,
            reload_value: 0,
            clock_select: 0,
            enabled: false,
            auto_reload: false,
        }
    }
}
