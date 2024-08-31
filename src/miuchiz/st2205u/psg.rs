use std::collections::VecDeque;

/// Programmable Sound Generator
pub struct State {
    psgc: Psgc,
    psg_states: [PsgModeState; 4],
    volumes: [PsgVolume; 4],
    multiplicator: Multiplicator,
}

pub struct Multiplicator {
    external_mull: u8,
    external_mulh: u8,

    last_mulh_was_1: bool,
    internal_mulh0: u8,
    internal_mulh1: u8,
    internal_mull: u8,
}

impl Multiplicator {
    pub fn new() -> Self {
        Self {
            external_mull: 0,
            external_mulh: 0,
            internal_mulh0: 0,
            internal_mulh1: 0,
            internal_mull: 0,
            last_mulh_was_1: false,
        }
    }

    pub fn write_mull(&mut self, value: u8) {
        self.internal_mull = value;

        let operand1 = u16::from(self.internal_mulh0) | u16::from(self.internal_mulh1) << 8;
        let operand2 = u16::from(self.internal_mull);
        let result = operand1 * operand2;
        self.external_mull = (result & 0xFF) as u8;
        self.external_mulh = ((result >> 8) & 0xFF) as u8;
    }

    pub fn write_mulh(&mut self, value: u8) {
        match self.last_mulh_was_1 {
            true => self.internal_mulh0 = value,
            false => self.internal_mulh1 = value,
        }
        self.last_mulh_was_1 = !self.last_mulh_was_1;
    }

    pub fn read_mull(&self) -> u8 {
        self.external_mull
    }

    pub fn read_mulh(&self) -> u8 {
        self.external_mulh
    }
}

pub enum PsgChannel {
    Channel0,
    Channel1,
    Channel2,
    Channel3,
}

impl State {
    pub fn new() -> Self {
        Self {
            psgc: Psgc::new(),
            psg_states: [
                PsgModeState::default(),
                PsgModeState::default(),
                PsgModeState::default(),
                PsgModeState::default(),
            ],
            volumes: [
                PsgVolume::new(),
                PsgVolume::new(),
                PsgVolume::new(),
                PsgVolume::new(),
            ],
            multiplicator: Multiplicator::new(),
        }
    }

    fn get_psg_state_mut(&mut self, channel: PsgChannel) -> &mut PsgModeState {
        &mut self.psg_states[match channel {
            PsgChannel::Channel0 => 0,
            PsgChannel::Channel1 => 1,
            PsgChannel::Channel2 => 2,
            PsgChannel::Channel3 => 3,
        }]
    }

    fn get_psg_state(&self, channel: PsgChannel) -> &PsgModeState {
        &self.psg_states[match channel {
            PsgChannel::Channel0 => 0,
            PsgChannel::Channel1 => 1,
            PsgChannel::Channel2 => 2,
            PsgChannel::Channel3 => 3,
        }]
    }

    fn get_volume(&self, channel: PsgChannel) -> &PsgVolume {
        &self.volumes[match channel {
            PsgChannel::Channel0 => 0,
            PsgChannel::Channel1 => 1,
            PsgChannel::Channel2 => 2,
            PsgChannel::Channel3 => 3,
        }]
    }

    fn get_volume_mut(&mut self, channel: PsgChannel) -> &mut PsgVolume {
        &mut self.volumes[match channel {
            PsgChannel::Channel0 => 0,
            PsgChannel::Channel1 => 1,
            PsgChannel::Channel2 => 2,
            PsgChannel::Channel3 => 3,
        }]
    }

    pub fn read_psgc(&self) -> u8 {
        self.psgc.read_psgc()
    }

    pub fn write_psgc(&mut self, value: u8) {
        self.psgc.write_psgc(value);
    }

    pub fn read_psgm(&self) -> u8 {
        let mut value = 0;
        for (i, state) in self.psg_states.iter().enumerate() {
            let mode = match state {
                PsgModeState::PcmDac { .. } => 0b00,
                PsgModeState::Tone { .. } => 0b01,
                PsgModeState::AdpcmDac { .. } => 0b11,
            };
            value |= mode << (i * 2);
        }
        value
    }

    pub fn write_psgm(&mut self, value: u8) {
        for i in 0..4 {
            let mode = (value >> (i * 2)) & 0b11;
            self.psg_states[i] = match mode {
                0b00 => PsgModeState::default_pcmdac(),
                0b01 => PsgModeState::Tone,
                0b11 => PsgModeState::default_adpcmdac(),
                _ => PsgModeState::default(),
            };
        }
    }

    pub fn write_psgxa(&mut self, channel: PsgChannel, value: u8) {
        match self.get_psg_state_mut(channel) {
            PsgModeState::AdpcmDac { fifo, .. } => {
                // ADPCM is differential, so the new value is calculated from the previous value
                let raw_value = (fifo.back().unwrap_or(&0) + i16::from(value)).clamp(-255, 256);
                fifo.push_back(raw_value);
            }
            PsgModeState::PcmDac { fifo, .. } => {
                fifo.push_back(value);
            }
            _ => todo!(),
        }
    }

    pub fn write_psgxb(&mut self, channel: PsgChannel, value: u8) {
        match self.get_psg_state_mut(channel) {
            PsgModeState::AdpcmDac { fifo, .. } => {
                let raw_value = (fifo.back().unwrap_or(&0) - i16::from(value)).clamp(-255, 256);
                fifo.push_back(raw_value);
            }
            _ => {
                // According to the datasheet, nothing happens if the channel is not ADPCM
            }
        }
    }

    pub fn read_psgxb(&self, channel: PsgChannel) -> u8 {
        match self.get_psg_state(channel) {
            PsgModeState::AdpcmDac { fifo, .. } => {
                if fifo.len() < 8 {
                    return 0b00100000 | (fifo.len() as u8);
                } else {
                    return fifo.len() as u8;
                }
            }
            PsgModeState::PcmDac { fifo, .. } => {
                if fifo.len() < 8 {
                    return 0b00100000 | (fifo.len() as u8);
                } else {
                    return fifo.len() as u8;
                }
            }
            _ => {}
        }
        return 0b00000000;
    }

    pub fn write_volx(&mut self, channel: PsgChannel, value: u8) {
        self.get_volume_mut(channel).set_u8(value);
    }

    pub fn read_volx(&self, channel: PsgChannel) -> u8 {
        self.get_volume(channel).get_u8()
    }

    pub fn read_mull(&self) -> u8 {
        self.multiplicator.read_mull()
    }

    pub fn read_mulh(&self) -> u8 {
        self.multiplicator.read_mulh()
    }

    pub fn write_mull(&mut self, value: u8) {
        self.multiplicator.write_mull(value);
    }

    pub fn write_mulh(&mut self, value: u8) {
        self.multiplicator.write_mulh(value);
    }

    pub fn pop_current_sample(&mut self, channel: PsgChannel) {
        match self.get_psg_state_mut(channel) {
            PsgModeState::AdpcmDac {
                fifo,
                current_sample,
            } => {
                let value = fifo.pop_front().unwrap_or(0);
                *current_sample = value;
            }
            PsgModeState::PcmDac {
                fifo,
                current_sample,
            } => {
                let value = fifo.pop_front().unwrap_or(0);
                *current_sample = value;
            }
            _ => {}
        }
    }

    pub fn get_mix_f32(&self) -> f32 {
        let adpcm_as_f32 = |current_sample: i16| -> f32 { current_sample as f32 / 256.0 };

        // I think this should be divided by 256.0, but that results in very loud sounds.
        // Maybe this is internally a 9 bit value like the ADPCM?
        let pcm_as_f32 = |current_sample: u8| -> f32 { (current_sample as f32 - 128.0) / 512.0 };

        let channel_as_f32 = |channel: PsgChannel| -> f32 {
            match self.get_psg_state(channel) {
                PsgModeState::AdpcmDac { current_sample, .. } => adpcm_as_f32(*current_sample),
                PsgModeState::PcmDac { current_sample, .. } => pcm_as_f32(*current_sample),
                PsgModeState::Tone => todo!(),
            }
        };

        let mixer0 = {
            let channel0 = channel_as_f32(PsgChannel::Channel0);
            let channel1 = channel_as_f32(PsgChannel::Channel1);
            let channel0_scaled = channel0 * self.get_volume(PsgChannel::Channel0).as_f32();
            let channel1_scaled = channel1 * self.get_volume(PsgChannel::Channel1).as_f32();

            (channel0_scaled + channel1_scaled) / 2.0
        };

        let mixer1 = {
            let channel2 = channel_as_f32(PsgChannel::Channel2);
            let channel3 = channel_as_f32(PsgChannel::Channel3);
            let channel2_scaled = channel2 * self.get_volume(PsgChannel::Channel2).as_f32();
            let channel3_scaled = channel3 * self.get_volume(PsgChannel::Channel3).as_f32();

            (channel2_scaled + channel3_scaled) / 2.0
        };

        let result = (mixer0 + mixer1) / 2.0;

        result
    }
}

#[derive(Debug)]
enum PsgModeState {
    PcmDac {
        fifo: VecDeque<u8>, // 8 bits
        current_sample: u8,
    },
    Tone,
    AdpcmDac {
        fifo: VecDeque<i16>, // 9 bits
        current_sample: i16,
    },
}

impl Default for PsgModeState {
    fn default() -> Self {
        Self::default_pcmdac()
    }
}

impl PsgModeState {
    pub fn default_pcmdac() -> Self {
        PsgModeState::PcmDac {
            fifo: VecDeque::with_capacity(16),
            current_sample: 128,
        }
    }

    pub fn default_adpcmdac() -> Self {
        PsgModeState::AdpcmDac {
            fifo: VecDeque::with_capacity(16),
            current_sample: 0,
        }
    }
}

// PSG Control
pub struct Psgc {
    mute: bool,
    // psgo not implemented
    pcmen: bool,
    p0en: bool,
    p1en: bool,
    p2en: bool,
    p3en: bool,
}

impl Psgc {
    pub fn new() -> Self {
        Self {
            mute: false,
            pcmen: false,
            p0en: false,
            p1en: false,
            p2en: false,
            p3en: false,
        }
    }

    pub fn read_psgc(&self) -> u8 {
        ((self.p3en as u8) << 7)
            | ((self.p2en as u8) << 6)
            | ((self.p1en as u8) << 5)
            | ((self.p0en as u8) << 4)
            | ((self.pcmen as u8) << 3)
            | ((self.mute as u8) << 0)
    }

    pub fn write_psgc(&mut self, value: u8) {
        self.mute = (value & 0b00000001) != 0;
        self.pcmen = (value & 0b00001000) != 0;
        self.p0en = (value & 0b00010000) != 0;
        self.p1en = (value & 0b00100000) != 0;
        self.p2en = (value & 0b01000000) != 0;
        self.p3en = (value & 0b10000000) != 0;
    }
}

struct PsgVolume {
    volume: u8,
}

impl PsgVolume {
    pub fn new() -> Self {
        Self { volume: 0 }
    }

    pub fn set_u8(&mut self, value: u8) {
        self.volume = value & 0b111111;
    }

    pub fn get_u8(&self) -> u8 {
        self.volume
    }

    pub fn as_f32(&self) -> f32 {
        self.volume as f32 / 63.0
    }
}
