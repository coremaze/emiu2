use std::sync::mpsc::{channel, Receiver, Sender};

use crate::gpio::GpioButtonState;
use crate::gpio::GpioInterface;
use crate::screen::Pixel;
use crate::screen::Screen;
use wasm_bindgen::JsValue;

const WIDTH: u32 = 98;
const HEIGHT: u32 = 67;

pub struct WasmScreen {
    rx: Receiver<Vec<Pixel>>,
    pixels: Vec<Pixel>,
}

impl WasmScreen {
    pub fn open() -> (Self, Sender<Vec<Pixel>>) {
        let (screen_tx, screen_rx) = channel::<Vec<Pixel>>();
        (
            WasmScreen {
                rx: screen_rx,
                pixels: vec![
                    Pixel {
                        red: 0,
                        green: 0,
                        blue: 0,
                    };
                    (WIDTH * HEIGHT) as usize
                ],
            },
            screen_tx,
        )
    }

    fn update_pixels(&mut self) {
        while let Ok(pixels) = self.rx.try_recv() {
            self.pixels = pixels;
        }
    }

    pub fn render_pixels(&mut self) {
        self.update_pixels();
        let pixels = &self.pixels;

        if pixels.len() != (WIDTH * HEIGHT) as usize {
            web_sys::console::error_1(&JsValue::from_str(&format!(
                "Expected {} pixels but got {}.",
                WIDTH * HEIGHT,
                pixels.len()
            )));
            return;
        }

        let mut data = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for pixel in pixels.iter() {
            data.push(pixel.red);
            data.push(pixel.green);
            data.push(pixel.blue);
            data.push(255);
        }
        let image_data = match ImageData::new_with_u8_clamped_array_and_sh(
            Clamped(data.as_slice()),
            WIDTH,
            HEIGHT,
        ) {
            Ok(img) => img,
            Err(err) => {
                web_sys::console::error_1(&JsValue::from_str(&format!(
                    "Failed to create ImageData: {:?}",
                    err
                )));
                return;
            }
        };
        let window = match web_sys::window() {
            Some(w) => w,
            None => {
                web_sys::console::error_1(&JsValue::from_str("Window not available"));
                return;
            }
        };
        let document = match window.document() {
            Some(doc) => doc,
            None => {
                web_sys::console::error_1(&JsValue::from_str("Document not available"));
                return;
            }
        };
        let canvas_elem = match document.get_element_by_id("emulator-canvas") {
            Some(elem) => elem,
            None => {
                web_sys::console::error_1(&JsValue::from_str("Canvas element not found"));
                return;
            }
        };
        let canvas = match canvas_elem.dyn_into::<web_sys::HtmlCanvasElement>() {
            Ok(c) => c,
            Err(err) => {
                web_sys::console::error_1(&JsValue::from_str(&format!(
                    "Failed to cast element to canvas: {:?}",
                    err
                )));
                return;
            }
        };
        let ctx = match canvas.get_context("2d") {
            Ok(Some(ctx)) => ctx,
            _ => {
                web_sys::console::error_1(&JsValue::from_str(
                    "Failed to get 2d context from canvas",
                ));
                return;
            }
        };
        let context = match ctx.dyn_into::<web_sys::CanvasRenderingContext2d>() {
            Ok(ctx) => ctx,
            Err(err) => {
                web_sys::console::error_1(&JsValue::from_str(&format!(
                    "Failed to cast context: {:?}",
                    err
                )));
                return;
            }
        };
        context.save();
        if let Err(err) = context.scale(3.0, 3.0) {
            web_sys::console::error_1(&JsValue::from_str(&format!("Failed to scale: {:?}", err)));
        }
        if let Err(err) = context.put_image_data(&image_data, 0.0, 0.0) {
            web_sys::console::error_1(&JsValue::from_str(&format!(
                "Failed to put image data: {:?}",
                err
            )));
        }
        context.restore();
    }
}

pub struct WasmScreenInterface {
    tx: Sender<Vec<Pixel>>,
}

impl WasmScreenInterface {
    pub fn new(tx: Sender<Vec<Pixel>>) -> Self {
        WasmScreenInterface { tx }
    }
}

use wasm_bindgen::Clamped;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};
impl Screen for WasmScreenInterface {
    fn set_pixels(&self, pixels: &[Pixel]) {
        if let Err(err) = self.tx.send(pixels.to_vec()) {
            web_sys::console::error_1(&JsValue::from_str(&format!(
                "Failed to send pixels: {:?}",
                err
            )));
        }
    }
}
