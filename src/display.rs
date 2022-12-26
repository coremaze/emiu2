use std::cell::RefCell;
use std::sync::mpsc::{channel, Receiver};
use std::{error::Error, sync::mpsc::Sender};

use minifb::{Key, Window, WindowOptions};

pub trait Screen {
    fn set_pixels(&self, pixels: &[Pixel]);
}

#[derive(Clone, Copy)]
pub struct Pixel {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

pub struct MiniFbScreen {
    tx: Sender<MiniFBMessage>,
    rx: Receiver<MiniFBWorkerMessage>,
    closed: RefCell<bool>,
}

impl MiniFbScreen {
    pub fn open(title: &str, width: usize, height: usize, scale: usize) -> Self {
        let (worker_tx, worker_rx) = channel::<MiniFBWorkerMessage>();
        let (host_tx, host_rx) = channel::<MiniFBMessage>();
        let owned_title = title.to_owned();
        std::thread::spawn(move || {
            run_minifb_worker(owned_title, width, height, scale, worker_tx, host_rx)
        });

        Self {
            tx: host_tx,
            rx: worker_rx,
            closed: RefCell::new(false),
        }
    }

    pub fn close(&self) {
        self.tx.send(MiniFBMessage::Close).unwrap_or_else(|err| {
            println!("Couldn't send close message to display client: {err:?}");
        });
    }

    pub fn is_open(&self) -> bool {
        if let Ok(MiniFBWorkerMessage::Close(_result)) = self.rx.try_recv() {
            *self.closed.borrow_mut() = true;
        }
        !*self.closed.borrow()
    }
}

impl Drop for MiniFbScreen {
    fn drop(&mut self) {
        self.close();
    }
}

impl Screen for MiniFbScreen {
    fn set_pixels(&self, pixels: &[Pixel]) {
        self.tx.send(MiniFBMessage::UpdatePixels(pixels.to_vec()));
    }
}

enum MiniFBWorkerMessage {
    Close(Result<(), minifb::Error>),
}

enum MiniFBMessage {
    UpdatePixels(Vec<Pixel>),
    Close,
}

fn run_minifb_worker(
    title: String,
    width: usize,
    height: usize,
    scale: usize,
    tx: Sender<MiniFBWorkerMessage>,
    rx: Receiver<MiniFBMessage>,
) {
    let mut window = match Window::new(
        &title,
        width * scale,
        height * scale,
        WindowOptions::default(),
    ) {
        Ok(window) => window,
        Err(err) => {
            tx.send(MiniFBWorkerMessage::Close(Err(err)));
            return;
        }
    };

    // Limit to max ~60 fps update rate
    window.limit_update_rate(Some(std::time::Duration::from_micros(16600)));

    let mut buffer: Vec<u32> = vec![0; width * scale * height * scale];

    let mut pixel_update: Option<Vec<Pixel>> = None;
    let mut close = false;
    while !close {
        loop {
            if !window.is_open() {
                close = true;
                break;
            }

            match rx.try_recv() {
                Ok(MiniFBMessage::UpdatePixels(pixels)) => {
                    pixel_update = Some(pixels);
                }
                Ok(MiniFBMessage::Close) => close = true,
                Err(_) => break,
            }
        }

        if let Some(pixels) = &pixel_update {
            for (i, pixel) in pixels.iter().enumerate() {
                let buf_element = {
                    let mut e = 0u32;
                    e |= pixel.red as u32;
                    e <<= 8;
                    e |= pixel.green as u32;
                    e <<= 8;
                    e |= pixel.blue as u32;
                    e
                };

                let pixel_start_x = (i % width) * scale;
                let pixel_end_x = pixel_start_x + scale;
                let pixel_start_y = (i / width) * scale;
                let pixel_end_y = pixel_start_y + scale;

                // println!("Got pixel {pixel_start_x}-{pixel_end_x}, {pixel_start_y}-{pixel_end_y}");

                for x in pixel_start_x..pixel_end_x {
                    for y in pixel_start_y..pixel_end_y {
                        let index = (y * width * scale) + x;
                        if index < buffer.len() {
                            buffer[index] = buf_element;
                            // println!("setting {}", y * width + x);
                        }
                    }
                }
            }

            pixel_update = None;
        }

        if let Err(err) = window.update_with_buffer(&buffer, width * scale, height * scale) {
            eprintln!("Failed to update window: {err:?}");
            close = true;
        }
    }

    tx.send(MiniFBWorkerMessage::Close(Ok(())));
}
