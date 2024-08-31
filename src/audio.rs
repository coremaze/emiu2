use std::collections::VecDeque;
use std::error::Error;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use cpal::StreamConfig;
use cpal::{
    traits::{DeviceTrait, HostTrait},
    FromSample, Sample, SizedSample,
};

pub trait AudioInterface {
    fn set_clock_rate(&mut self, emulated_clock_rate: u64);
    fn needs_sample(&self, current_cycle: u64) -> bool;
    fn add_sample(&mut self, value: f32);
}

struct AudioReceiver {
    audio_rx: Receiver<Vec<f32>>,
    buffer: VecDeque<f32>,
    last_sample: f32,
}

impl AudioReceiver {
    fn new(audio_rx: Receiver<Vec<f32>>) -> Self {
        Self {
            audio_rx,
            buffer: VecDeque::new(),
            last_sample: 0.0,
        }
    }

    fn update(&mut self) {
        if let Ok(values) = self.audio_rx.try_recv() {
            self.buffer.extend(values);
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
}

pub fn stream_setup_for() -> Result<(cpal::Stream, AudioSender), Box<dyn Error>> {
    let (_host, device, config) = host_device_setup()?;
    let (tx, rx) = channel();

    let audio_sender = AudioSender {
        tx,
        emulated_clock_rate: 1,
        host_sample_rate: config.sample_rate.0,
        clock_of_last_sample: 0.0,
        clocks_between_samples: 0.0,
        frame_size: match config.buffer_size {
            cpal::BufferSize::Fixed(size) => size as usize,
            cpal::BufferSize::Default => 64,
        },
        buffer: Vec::new(),
    };

    let stream = make_stream::<f32>(&device, &config.into(), rx)?;
    Ok((stream, audio_sender))
}

fn host_device_setup() -> Result<(cpal::Host, cpal::Device, cpal::StreamConfig), Box<dyn Error>> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("Default output device is not available")?;

    println!("Output device: {}", device.name()?);

    let output_config = StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(44100),
        buffer_size: cpal::BufferSize::Fixed(512),
    };

    println!("Default output config: {:?}", output_config);
    Ok((host, device, output_config))
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
    let player = Arc::new(Mutex::new(AudioReceiver::new(audio_rx)));

    let err_fn = |err| eprintln!("Error building output sound stream: {}", err);

    Ok(device.build_output_stream(
        config,
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
