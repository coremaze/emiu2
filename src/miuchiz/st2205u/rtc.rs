pub struct State {
    clock_frequency: u64,
    elapsed_ticks: u64,
    last_second_tick: u64,

    seconds: u8,
    minutes: u8,
    hours: u8,

    // Alarms are not implemented yet
    alarm_minutes: u8,
    alarm_hours: u8,

    rctr: Rctr,
}

impl State {
    pub fn new(oscx: u64) -> Self {
        Self {
            clock_frequency: oscx,
            elapsed_ticks: 0,
            last_second_tick: 0,
            seconds: 0,
            minutes: 0,
            hours: 0,
            alarm_minutes: 0,
            alarm_hours: 0,
            rctr: Rctr::default(),
        }
    }

    pub fn set_ticks(&mut self, ticks: u64) {
        let ticks_per_second = self.clock_frequency;
        let next_second_tick = self.last_second_tick + ticks_per_second;

        if ticks >= next_second_tick {
            self.last_second_tick = next_second_tick;
            self.inc_second();
        }

        self.elapsed_ticks = ticks;
    }

    fn inc_second(&mut self) {
        self.seconds += 1;
        if self.seconds >= 60 {
            self.seconds = 0;
            self.inc_minute();
        }
    }

    fn inc_minute(&mut self) {
        self.minutes += 1;
        if self.minutes >= 60 {
            self.minutes = 0;
            self.inc_hour();
        }
    }

    fn inc_hour(&mut self) {
        self.hours += 1;
        if self.hours >= 24 {
            self.hours = 0;
        }
    }

    pub fn get_seconds(&self) -> u8 {
        self.seconds
    }

    pub fn get_minutes(&self) -> u8 {
        self.minutes
    }

    pub fn get_hours(&self) -> u8 {
        self.hours
    }

    pub fn set_seconds(&mut self, seconds: u8) {
        self.seconds = seconds;
    }

    pub fn set_minutes(&mut self, minutes: u8) {
        self.minutes = minutes;
    }

    pub fn set_hours(&mut self, hours: u8) {
        self.hours = hours;
    }

    pub fn set_alarm_minutes(&mut self, minutes: u8) {
        self.alarm_minutes = minutes;
    }

    pub fn set_alarm_hours(&mut self, hours: u8) {
        self.alarm_hours = hours;
    }

    pub fn read_rtc(&self) -> u8 {
        match self.rctr.selection {
            Rsel::Seconds => self.get_seconds(),
            Rsel::Minutes => self.get_minutes(),
            Rsel::Hours => self.get_hours(),
            Rsel::AlarmMinutes => self.alarm_minutes,
            Rsel::AlarmHours => self.alarm_hours,
        }
    }

    pub fn read_rctr(&self) -> u8 {
        self.rctr.read_u8()
    }

    pub fn write_rctr(&mut self, value: u8) {
        self.rctr.write_u8(value);
    }

    pub fn write_rtc(&mut self, value: u8) {
        match self.rctr.selection {
            Rsel::Seconds => self.set_seconds(value),
            Rsel::Minutes => self.set_minutes(value),
            Rsel::Hours => self.set_hours(value),
            Rsel::AlarmMinutes => self.alarm_minutes = value,
            Rsel::AlarmHours => self.alarm_hours = value,
        }
    }
}

struct Rctr {
    selection: Rsel,
}

impl Rctr {
    pub fn read_u8(&self) -> u8 {
        let rsel = match self.selection {
            Rsel::Seconds => 0b000,
            Rsel::Minutes => 0b001,
            Rsel::Hours => 0b010,
            Rsel::AlarmMinutes => 0b100,
            Rsel::AlarmHours => 0b101,
        };

        rsel << 5
    }

    pub fn write_u8(&mut self, value: u8) {
        self.selection = Rsel::from_u8(value >> 5);
    }
}

impl Default for Rctr {
    fn default() -> Self {
        Self {
            selection: Rsel::default(),
        }
    }
}

pub enum Rsel {
    Seconds,
    Minutes,
    Hours,
    AlarmMinutes,
    AlarmHours,
}

impl Rsel {
    pub fn from_u8(value: u8) -> Self {
        // From datasheet:
        // Second counter (RSEL=000) : counter = 0~59
        // Minute counter (RSEL=001) : counter = 0~59
        // Hour counter (RSEL=010) : counter = 0~23
        // Alarm minute counter (RSEL=1x0) : counter = 0~59
        // Alarm hour counter (RSEL=1x1) : counter = 0~23

        match value & 0b111 {
            0b000 => Rsel::Seconds,
            0b001 => Rsel::Minutes,
            0b010 => Rsel::Hours,
            0b100 | 0b110 => Rsel::AlarmMinutes,
            0b101 | 0b111 => Rsel::AlarmHours,
            _ => unreachable!(),
        }
    }
}

impl Default for Rsel {
    fn default() -> Self {
        Rsel::Seconds
    }
}
