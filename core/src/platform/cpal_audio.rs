use std::collections::VecDeque;
use std::error::Error;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use cpal::StreamConfig;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, Sample, SizedSample,
};

use crate::audio::AudioInterface;

/// If the buffer falls further behind real time than this, drop the oldest
/// samples (down to [`TARGET_BUFFERED`]) instead of letting latency
/// accumulate forever.
const MAX_BUFFERED: f64 = 0.5;
/// What the buffer is trimmed back to after falling behind: enough cushion
/// that trimming doesn't cause an immediate underrun.
const TARGET_BUFFERED: f64 = 0.1;

struct AudioReceiver {
    audio_rx: Receiver<Vec<f32>>,
    buffer: VecDeque<f32>,
    last_sample: f32,
    max_buffered: usize,
    target_buffered: usize,
}

impl AudioReceiver {
    fn new(audio_rx: Receiver<Vec<f32>>, sample_rate: u32) -> Self {
        Self {
            audio_rx,
            buffer: VecDeque::new(),
            last_sample: 0.0,
            max_buffered: (sample_rate as f64 * MAX_BUFFERED) as usize,
            target_buffered: (sample_rate as f64 * TARGET_BUFFERED) as usize,
        }
    }

    fn update(&mut self) {
        while let Ok(values) = self.audio_rx.try_recv() {
            self.buffer.extend(values);
        }
        if self.buffer.len() > self.max_buffered {
            let excess = self.buffer.len() - self.target_buffered;
            self.buffer.drain(..excess);
        }
    }

    fn pop_value(&mut self) -> f32 {
        self.last_sample = self.buffer.pop_front().unwrap_or(self.last_sample);
        self.last_sample
    }
}

pub struct AudioSender {
    tx: Sender<Vec<f32>>,
    emulated_clock_rate: u64,
    host_sample_rate: u32,
    clock_of_last_sample: f64,
    clocks_between_samples: f64,
    frame_size: usize,
    buffer: Vec<f32>,
}

impl AudioInterface for AudioSender {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64) {
        self.emulated_clock_rate = emulated_clock_rate;
        self.clocks_between_samples =
            self.emulated_clock_rate as f64 / self.host_sample_rate as f64;
    }

    fn needs_sample(&self, current_cycle: u64) -> bool {
        let next_sample_cycle = self.clock_of_last_sample + self.clocks_between_samples;
        next_sample_cycle <= current_cycle as f64
    }

    fn add_sample(&mut self, value: f32) {
        self.buffer.push(value);
        self.clock_of_last_sample += self.clocks_between_samples;
        if self.buffer.len() >= self.frame_size {
            let values = std::mem::take(&mut self.buffer);
            self.tx.send(values).expect("Failed to send audio data");
        }
    }

    fn clock_rewound(&mut self, current_cycle: u64) {
        self.clock_of_last_sample = current_cycle as f64;
    }
}

pub fn stream_setup_for() -> Result<(cpal::Stream, AudioSender), Box<dyn Error>> {
    let host = cpal::default_host();

    let default_device = host.default_output_device();

    let mut last_err: Box<dyn Error> = "no audio output devices found".into();

    if let Some(device) = &default_device {
        match stream_for_device(device) {
            Ok(ok) => return Ok(ok),
            Err(why) => {
                eprintln!("Default audio output unusable ({why}); trying other devices");
                last_err = why;
            }
        }
    }

    // The default device can be broken while real ones work (e.g. ALSA's
    // `default` PCM routing to dmix on a card with no playback stream), so
    // walk the full device list before giving up.
    if let Ok(devices) = host.output_devices() {
        for device in devices {
            if Some(&device) == default_device.as_ref() || device.to_string() == "null" {
                continue;
            }
            match stream_for_device(&device) {
                Ok(ok) => {
                    eprintln!("Audio output: {device}");
                    return Ok(ok);
                }
                Err(why) => last_err = why,
            }
        }
    }

    Err(last_err)
}

fn stream_for_device(device: &cpal::Device) -> Result<(cpal::Stream, AudioSender), Box<dyn Error>> {
    let (config, sample_format) = select_config(device)?;
    let (tx, rx) = channel();

    let audio_sender = AudioSender {
        tx,
        emulated_clock_rate: 1,
        host_sample_rate: config.sample_rate,
        clock_of_last_sample: 0.0,
        clocks_between_samples: 0.0,
        // Send granularity only; the callback drains everything available,
        // so this doesn't need to match the device's callback size (WASAPI
        // callbacks are variable-sized regardless of the requested buffer).
        frame_size: 128,
        buffer: Vec::new(),
    };

    let stream = match sample_format {
        cpal::SampleFormat::F32 => make_stream::<f32>(device, &config, rx),
        cpal::SampleFormat::F64 => make_stream::<f64>(device, &config, rx),
        cpal::SampleFormat::I16 => make_stream::<i16>(device, &config, rx),
        cpal::SampleFormat::U16 => make_stream::<u16>(device, &config, rx),
        cpal::SampleFormat::I8 => make_stream::<i8>(device, &config, rx),
        cpal::SampleFormat::U8 => make_stream::<u8>(device, &config, rx),
        cpal::SampleFormat::I32 => make_stream::<i32>(device, &config, rx),
        cpal::SampleFormat::U32 => make_stream::<u32>(device, &config, rx),
        cpal::SampleFormat::I64 => make_stream::<i64>(device, &config, rx),
        cpal::SampleFormat::U64 => make_stream::<u64>(device, &config, rx),
        other => Err(format!("unsupported sample format {other:?}").into()),
    }?;

    // Start here rather than in the caller so a device that opens but can't
    // begin playback still falls through to the next candidate.
    stream.play()?;

    Ok((stream, audio_sender))
}

fn select_config(
    device: &cpal::Device,
) -> Result<(cpal::StreamConfig, cpal::SampleFormat), Box<dyn Error>> {
    // F32 is a preference, not a requirement: raw ALSA devices are often
    // S16-only.
    let supported_config = device
        .supported_output_configs()?
        .min_by_key(|config| match config.sample_format() {
            cpal::SampleFormat::F32 => 0,
            cpal::SampleFormat::I16 => 1,
            cpal::SampleFormat::U16 => 2,
            _ => 3,
        })
        .ok_or("No supported audio configuration found")?;

    let sample_format = supported_config.sample_format();

    // Choose sample rate closest to 44100
    let min_sample_rate = supported_config.min_sample_rate();
    let max_sample_rate = supported_config.max_sample_rate();

    let target_sample_rate = 44100;
    let sample_rate = if min_sample_rate >= target_sample_rate {
        min_sample_rate
    } else if max_sample_rate <= target_sample_rate {
        max_sample_rate
    } else {
        target_sample_rate
    };

    let config = supported_config.with_sample_rate(sample_rate);

    // Choose buffer size closest to 512 without going under
    let buffer_size = match config.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => {
            let target = 512;
            if *max < target {
                cpal::BufferSize::Fixed(*max)
            } else if *min > target {
                cpal::BufferSize::Fixed(*min)
            } else {
                cpal::BufferSize::Fixed(target)
            }
        }
        cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Default,
    };

    let output_config = StreamConfig {
        channels: config.channels(),
        sample_rate: config.sample_rate(),
        buffer_size,
    };

    Ok((output_config, sample_format))
}

fn make_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    audio_rx: Receiver<Vec<f32>>,
) -> Result<cpal::Stream, Box<dyn Error>>
where
    T: SizedSample + FromSample<f32>,
{
    let num_channels = config.channels as usize;
    let player = Arc::new(Mutex::new(AudioReceiver::new(audio_rx, config.sample_rate)));

    let err_fn = |err| eprintln!("Error building output sound stream: {}", err);

    Ok(device.build_output_stream(
        *config,
        move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
            process_frame(output, &player, num_channels)
        },
        err_fn,
        None,
    )?)
}

fn process_frame<SampleType>(
    output: &mut [SampleType],
    player: &Arc<Mutex<AudioReceiver>>,
    num_channels: usize,
) where
    SampleType: Sample + FromSample<f32>,
{
    let mut player = player.lock().expect("Failed to lock AudioReceiver");
    player.update();

    for frame in output.chunks_mut(num_channels) {
        let value = SampleType::from_sample(player.pop_value());
        frame.fill(value);
    }
}
