use console_error_panic_hook;
use wasm_bindgen::prelude::*;
use web_sys;

use crate::ir::DisconnectedIr;
use crate::miuchiz::{Handheld, MiuchizButtonStates, MiuchizGpio};
use crate::platform::web_audio::WebAudio;
use crate::platform::web_gpio::WasmGpioInterface;
use crate::platform::web_screen::{WasmScreen, WasmScreenInterface};

use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

thread_local! {
    static GLOBAL_GPIO_STATE: RefCell<Rc<RefCell<MiuchizButtonStates>>> = RefCell::new(Rc::new(RefCell::new(MiuchizButtonStates::default())));
    static GLOBAL_EMULATOR_STATE: RefCell<Option<Rc<RefCell<(Handheld, WasmScreen)>>>> = RefCell::new(None);
}

#[wasm_bindgen]
pub fn create_emulator_with_files(otp: Box<[u8]>, flash: Box<[u8]>) -> Result<(), JsValue> {
    // Set up panic hook for better error messages in the browser console
    console_error_panic_hook::set_once();

    // Use the uploaded file data
    let otp_data: Vec<u8> = otp.into();
    let flash_data: Vec<u8> = flash.into();

    let (screen, screen_tx) = WasmScreen::open();
    let screen_interface = WasmScreenInterface::new(screen_tx);

    let web_audio = WebAudio::create().map_err(|e| JsValue::from_str(&e))?;

    let wasm_gpio = WasmGpioInterface::new();
    let gpio_state = wasm_gpio.button_states.clone();

    GLOBAL_GPIO_STATE.with(|global_state| {
        *global_state.borrow_mut() = gpio_state;
    });

    let wasm_audio_interface = WebAudio::create().map_err(|e| JsValue::from_str(&e))?;
    wasm_audio_interface
        .play()
        .map_err(|e| JsValue::from_str(&e))?;

    // Initialize the handheld emulator using the provided interfaces
    let handheld = Handheld::new(
        &otp_data,
        &flash_data,
        Box::new(screen_interface),
        Box::new(wasm_gpio),
        Box::new(wasm_audio_interface),
        Box::new(DisconnectedIr),
    )
    .map_err(|e| JsValue::from_str(&format!("Failed to initialize handheld: {}", e)))?;

    // Wrap emulator state (handheld and screen) in an Rc<RefCell> for shared access in the animation loop
    let emulator_state = Rc::new(RefCell::new((handheld, screen)));
    GLOBAL_EMULATOR_STATE.with(|global| {
        *global.borrow_mut() = Some(emulator_state.clone());
    });

    Ok(())
}

#[wasm_bindgen]
pub fn start_driving_emulator() -> Result<(), JsValue> {
    let emulator_state = GLOBAL_EMULATOR_STATE.with(|state| {
        if let Some(emulator_state) = &*state.borrow() {
            Ok(emulator_state.clone())
        } else {
            Err(JsValue::from_str("Emulator state not found"))
        }
    });

    let emulator_state = emulator_state
        .map_err(|e| JsValue::from_str(&format!("Failed to get emulator state: {:?}", e)))?;

    let beginning = web_sys::window()
        .ok_or_else(|| JsValue::from_str("No window available"))?
        .performance()
        .ok_or_else(|| JsValue::from_str("Performance API not available"))?
        .now();

    // Set up simulation interval (1ms)
    let sim_state = emulator_state.clone();
    let sim_closure = Closure::wrap(Box::new(move || {
        let mut state = sim_state.borrow_mut();
        let now = web_sys::window()
            .and_then(|w| w.performance())
            .map(|p| p.now())
            .unwrap_or(beginning);

        let elapsed_ms = now - beginning;
        let nanoseconds = (elapsed_ms * 1_000_000.0) as u128;
        let cycles_per_second = state.0.mcu.core.cycles_per_second() as u128;
        let cycles_required_so_far = (nanoseconds * cycles_per_second) / 1_000_000_000;
        while (state.0.mcu.core.cycles as u128) < cycles_required_so_far {
            state.0.mcu.step();
        }
    }) as Box<dyn FnMut()>);

    // Set up the interval for simulation
    web_sys::window()
        .ok_or_else(|| JsValue::from_str("No window available"))?
        .set_interval_with_callback_and_timeout_and_arguments_0(
            sim_closure.as_ref().unchecked_ref(),
            1, // 1ms interval
        )
        .map_err(|e| JsValue::from_str(&format!("Failed to set simulation interval: {:?}", e)))?;
    sim_closure.forget(); // Prevent closure from being dropped

    // Set up the recursive animation frame loop (for rendering only)
    let sim_state = emulator_state.clone();
    let f: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
    {
        let f_clone = f.clone();
        *f.borrow_mut() = Some(Closure::wrap(Box::new(move || {
            // Update the screen only
            sim_state.borrow_mut().1.render_pixels();

            // Schedule the next frame
            if let Some(window) = web_sys::window() {
                if let Some(closure) = f_clone.borrow().as_ref() {
                    if let Err(e) = window.request_animation_frame(closure.as_ref().unchecked_ref())
                    {
                        web_sys::console::error_1(&e);
                    }
                }
            }
        }) as Box<dyn FnMut()>));
    }

    {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("No window available"))?;
        let request_result = {
            let f_borrow = f.borrow();
            if let Some(ref closure) = *f_borrow {
                window.request_animation_frame(closure.as_ref().unchecked_ref())
            } else {
                Err(JsValue::from_str("Closure not set"))
            }
        };
        request_result.map_err(|e| {
            JsValue::from_str(&format!("Failed to request animation frame: {:?}", e))
        })?;
    }

    web_sys::console::log_1(&JsValue::from_str(
        "Emulator started on wasm with uploaded files.",
    ));

    Ok(())
}

#[wasm_bindgen]
pub fn set_button_state(button: &str, state: bool) {
    GLOBAL_GPIO_STATE.with(|global_state_ref| {
        let global_state = global_state_ref.borrow();
        let mut button_state = global_state.borrow_mut();

        // Map the button name to MiuchizGpio and update the state
        let button_gpio = match button {
            "up" => Some(MiuchizGpio::Up),
            "down" => Some(MiuchizGpio::Down),
            "left" => Some(MiuchizGpio::Left),
            "right" => Some(MiuchizGpio::Right),
            "power" => Some(MiuchizGpio::Power),
            "menu" => Some(MiuchizGpio::Menu),
            "upside-up" => Some(MiuchizGpio::UpsideUp),
            "upside-down" => Some(MiuchizGpio::UpsideDown),
            "screen-top-left" => Some(MiuchizGpio::ScreenTopLeft),
            "screen-top-right" => Some(MiuchizGpio::ScreenTopRight),
            "screen-bottom-left" => Some(MiuchizGpio::ScreenBottomLeft),
            "screen-bottom-right" => Some(MiuchizGpio::ScreenBottomRight),
            "action" => Some(MiuchizGpio::Action),
            "mute" => Some(MiuchizGpio::Mute),
            _ => None,
        };

        if let Some(gpio_button) = button_gpio {
            button_state.set(gpio_button, state);
        }
    });
}

#[wasm_bindgen]
pub fn get_button_state(button: &str) -> bool {
    GLOBAL_GPIO_STATE.with(|global_state_ref| {
        let global_state = global_state_ref.borrow();
        let button_state = global_state.borrow();

        match button {
            "up" => button_state.up,
            "down" => button_state.down,
            "left" => button_state.left,
            "right" => button_state.right,
            "power" => button_state.power,
            "menu" => button_state.menu,
            "upside-up" => button_state.upside_up,
            "upside-down" => button_state.upside_down,
            "screen-top-left" => button_state.screen_top_left,
            "screen-top-right" => button_state.screen_top_right,
            "screen-bottom-left" => button_state.screen_bottom_left,
            "screen-bottom-right" => button_state.screen_bottom_right,
            "action" => button_state.action,
            "mute" => button_state.mute,
            _ => false,
        }
    })
}

#[wasm_bindgen]
pub fn get_flash_dump() -> web_sys::js_sys::Uint8Array {
    GLOBAL_EMULATOR_STATE.with(|state| {
        if let Some(emulator_state) = &*state.borrow() {
            let mut guard = emulator_state.borrow_mut();
            let dump = guard.0.make_flash_dump();
            web_sys::js_sys::Uint8Array::from(&dump[..]).into()
        } else {
            web_sys::js_sys::Uint8Array::new(&JsValue::from(0)).into()
        }
    })
}
