#[cfg(target_arch = "wasm32")]
pub mod web_audio;
#[cfg(target_arch = "wasm32")]
pub mod web_gpio;
#[cfg(target_arch = "wasm32")]
pub mod web_screen;

#[cfg(not(target_arch = "wasm32"))]
pub mod cpal_audio;
#[cfg(not(target_arch = "wasm32"))]
pub mod minifb_screen_gpio;
