use crate::audio::AudioInterface;
use wasm_bindgen_futures::spawn_local;
use wasm_bindgen_futures::JsFuture;
use web_sys::AudioContext;

pub struct WebAudio {
    context: AudioContext,
    emulated_clock_rate: u64,
    host_sample_rate: u32,
    clock_of_last_sample: f64,
    clocks_between_samples: f64,
    frame_size: usize,
    buffer: Vec<f32>,
}

impl AudioInterface for WebAudio {
    /// Set the clock rate of the system so this audio driver knows how many cycles to wait between samples.
    fn set_clock_rate(&mut self, emulated_clock_rate: u64) {
        self.emulated_clock_rate = emulated_clock_rate;
        self.clocks_between_samples =
            self.emulated_clock_rate as f64 / self.host_sample_rate as f64;
    }

    /// Returns true if the audio driver is ready for a new sample.
    fn needs_sample(&self, current_cycle: u64) -> bool {
        let next_sample_cycle = self.clock_of_last_sample + self.clocks_between_samples;
        next_sample_cycle <= current_cycle as f64
    }

    /// Feed a sample into the audio driver.
    fn add_sample(&mut self, value: f32) {
        self.buffer.push(value);
        self.clock_of_last_sample += self.clocks_between_samples;
        if self.buffer.len() >= self.frame_size {
            self.add_frame(self.buffer.clone());
            self.buffer.clear();
        }
    }
}

impl WebAudio {
    pub fn play(&self) -> Result<(), String> {
        let promise = self
            .context
            .resume()
            .map_err(|e| format!("Error resuming AudioContext: {:?}", e))?;
        spawn_local(async move {
            if let Err(e) = JsFuture::from(promise).await {
                web_sys::console::error_1(&e);
            }
        });
        Ok(())
    }

    pub fn add_frame(&mut self, values: Vec<f32>) {
        // Create an AudioBuffer from the buffer samples (assume mono audio)
        let length = self.buffer.len() as u32;
        let sample_rate = self.host_sample_rate as f32;
        // Create a new AudioBuffer with 1 channel, the length, and sample_rate
        let audio_buffer = self
            .context
            .create_buffer(1, length, sample_rate)
            .expect("Failed to create AudioBuffer");

        // Get the channel data (Float32Array) for channel 0
        let channel_data = audio_buffer
            .get_channel_data(0)
            .expect("Failed to get channel data");

        // Copy samples into the AudioBuffer using copy_to_channel
        audio_buffer
            .copy_to_channel(self.buffer.as_slice(), 0)
            .expect("Failed to copy samples to AudioBuffer");

        // Create a BufferSource node
        let source = self
            .context
            .create_buffer_source()
            .expect("Failed to create BufferSource");
        source.set_buffer(Some(&audio_buffer));

        // Connect the source node to the destination (speakers)
        source
            .connect_with_audio_node(&self.context.destination())
            .expect("Failed to connect BufferSource");

        // Start playback immediately
        source.start().expect("Failed to start BufferSource");
    }

    pub fn create() -> Result<Self, String> {
        let context =
            AudioContext::new().map_err(|e| format!("Failed to create AudioContext: {:?}", e))?;
        let host_sample_rate = context.sample_rate() as u32;
        Ok(WebAudio {
            context,
            emulated_clock_rate: 1,
            host_sample_rate,
            clock_of_last_sample: 0.0,
            clocks_between_samples: 0.0,
            frame_size: 4096,
            buffer: Vec::new(),
        })
    }
}
