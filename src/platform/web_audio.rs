use crate::audio::AudioInterface;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;
use wasm_bindgen_futures::JsFuture;
use web_sys::js_sys::Float32Array;
use web_sys::AudioContext;

pub struct WebAudio {
    context: AudioContext,
    emulated_clock_rate: u64,
    host_sample_rate: u32,
    clock_of_last_sample: f64,
    clocks_between_samples: f64,
    frame_size: usize,
    buffer: Vec<f32>,
    audio_queue: Rc<RefCell<VecDeque<f32>>>,
    script_processor: Option<web_sys::ScriptProcessorNode>,
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
        self.audio_queue.borrow_mut().extend(values);
    }

    pub fn create() -> Result<Self, String> {
        let context =
            AudioContext::new().map_err(|e| format!("Failed to create AudioContext: {:?}", e))?;
        let host_sample_rate = context.sample_rate() as u32;
        let buffer_size = 512;
        let audio_queue = Rc::new(RefCell::new(VecDeque::new()));
        let script_processor = context.create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(buffer_size, 0, 1)
            .map_err(|e| format!("Failed to create ScriptProcessorNode: {:?}", e))?;

        {
            let audio_queue_clone = Rc::clone(&audio_queue);
            let closure = Closure::wrap(Box::new(move |event: web_sys::AudioProcessingEvent| {
                let output_buffer = event.output_buffer().unwrap();
                let mut output = output_buffer.get_channel_data(0).unwrap();

                let mut samples = audio_queue_clone.borrow_mut();

                let mut output_array = web_sys::js_sys::Float32Array::new_with_length(buffer_size);
                for i in 0..buffer_size {
                    let sample = samples.pop_front().unwrap_or(0.0);
                    output_array.set_index(i as u32, sample);
                }

                // If the web browser falls more than 0.5 seconds behind, catch back up
                let max_samples_behind = (0.5 * host_sample_rate as f64) as usize;
                if samples.len() >= max_samples_behind {
                    samples.clear();
                }

                output_buffer
                    .copy_to_channel_with_f32_array_and_start_in_channel(&output_array, 0, 0)
                    .expect("Failed to copy audio data to output buffer");
            }) as Box<dyn FnMut(_)>);
            script_processor.set_onaudioprocess(Some(closure.as_ref().unchecked_ref()));
            closure.forget();
        }

        // Create a GainNode to control volume and ensure proper routing
        let gain_node = context
            .create_gain()
            .map_err(|e| format!("Failed to create GainNode: {:?}", e))?;
        // Set gain to 1.0 (adjust if necessary)
        gain_node.gain().set_value(1.0);

        // Connect the script processor to the gain node
        script_processor
            .connect_with_audio_node(&gain_node)
            .map_err(|e| format!("Failed to connect ScriptProcessorNode to GainNode: {:?}", e))?;

        // Connect the gain node to the audio destination
        gain_node
            .connect_with_audio_node(&context.destination())
            .map_err(|e| format!("Failed to connect GainNode to destination: {:?}", e))?;

        Ok(WebAudio {
            context,
            emulated_clock_rate: 1,
            host_sample_rate,
            clock_of_last_sample: 0.0,
            clocks_between_samples: 0.0,
            frame_size: buffer_size as usize,
            buffer: Vec::new(),
            audio_queue,
            script_processor: Some(script_processor),
        })
    }
}
