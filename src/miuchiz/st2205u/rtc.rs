use crate::state::{StateError, StateReader, StateWriter};

pub struct State {
    clock_frequency: u64,
    elapsed_ticks: u64,
    last_second_tick: u64,

    seconds: u8,
    minutes: u8,
    hours: u8,

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

    pub fn set_ticks(&mut self, ticks: u64) -> bool {
        let ticks_per_second = self.clock_frequency;
        let next_second_tick = self.last_second_tick + ticks_per_second;

        if ticks >= next_second_tick {
            self.last_second_tick = next_second_tick;
            return self.inc_second();
        }

        self.elapsed_ticks = ticks;
        false
    }

    fn inc_second(&mut self) -> bool {
        self.seconds += 1;
        if self.seconds >= 60 {
            self.seconds = 0;
            return self.inc_minute();
        }
        false
    }

    fn inc_minute(&mut self) -> bool {
        self.minutes += 1;

        let mut triggered = false;

        // Minute Interrupt (Bit 0)
        self.rctr.requests |= 0x01;
        if (self.rctr.enables & 0x01) != 0 {
            triggered = true;
        }

        if self.minutes >= 60 {
            self.minutes = 0;
            if self.inc_hour() {
                triggered = true;
            }
        }

        // Alarm Interrupt (Bit 3)
        if self.minutes == self.alarm_minutes && self.hours == self.alarm_hours {
            self.rctr.requests |= 0x08;
            if (self.rctr.enables & 0x08) != 0 {
                triggered = true;
            }
        }

        triggered
    }

    fn inc_hour(&mut self) -> bool {
        self.hours += 1;

        // Hour Interrupt (Bit 1)
        self.rctr.requests |= 0x02;
        let mut triggered = (self.rctr.enables & 0x02) != 0;

        if self.hours >= 24 {
            self.hours = 0;
            if self.inc_day() {
                triggered = true;
            }
        }
        triggered
    }

    fn inc_day(&mut self) -> bool {
        // Day Interrupt (Bit 2)
        self.rctr.requests |= 0x04;
        (self.rctr.enables & 0x04) != 0
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
            Rsel::AlarmMinutes => self.set_alarm_minutes(value),
            Rsel::AlarmHours => self.set_alarm_hours(value),
        }
    }

    pub fn save_state(&self, writer: &mut StateWriter) {
        writer.put_u64(self.elapsed_ticks);
        writer.put_u64(self.last_second_tick);
        writer.put_u8(self.seconds);
        writer.put_u8(self.minutes);
        writer.put_u8(self.hours);
        writer.put_u8(self.alarm_minutes);
        writer.put_u8(self.alarm_hours);
        writer.put_u8(self.rctr.selection.to_u8());
        writer.put_u8(self.rctr.enables);
        writer.put_u8(self.rctr.requests);
    }

    pub fn load_state(&mut self, reader: &mut StateReader) -> Result<(), StateError> {
        self.elapsed_ticks = reader.take_u64()?;
        self.last_second_tick = reader.take_u64()?;
        self.seconds = reader.take_u8()?;
        self.minutes = reader.take_u8()?;
        self.hours = reader.take_u8()?;
        self.alarm_minutes = reader.take_u8()?;
        self.alarm_hours = reader.take_u8()?;
        self.rctr.selection = Rsel::from_u8(reader.take_u8()?);
        self.rctr.enables = reader.take_u8()?;
        self.rctr.requests = reader.take_u8()?;
        Ok(())
    }
}

struct Rctr {
    selection: Rsel,
    enables: u8,  // *IEN
    requests: u8, // *IRQ
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

        (rsel << 5) | (self.requests & 0x0F)
    }

    pub fn write_u8(&mut self, value: u8) {
        self.selection = Rsel::from_u8(value >> 5);
        let new_enables = value & 0x0F;
        let rtc_clr = (value & 0x10) != 0;

        self.enables = new_enables;

        // If an interrupt is disabled, we clear the request (?)
        self.requests &= new_enables;

        if rtc_clr {
            // "RTC Clear" - write 1 to clear all RTC interrupt requests
            self.requests = 0;
        }
    }
}

impl Default for Rctr {
    fn default() -> Self {
        Self {
            selection: Rsel::default(),
            enables: 0,
            requests: 0,
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
    /// The inverse of `from_u8`, using the datasheet's RSEL codes.
    pub fn to_u8(&self) -> u8 {
        match self {
            Rsel::Seconds => 0b000,
            Rsel::Minutes => 0b001,
            Rsel::Hours => 0b010,
            Rsel::AlarmMinutes => 0b100,
            Rsel::AlarmHours => 0b101,
        }
    }

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
