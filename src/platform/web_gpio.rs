use crate::gpio::GpioButtonState;
use crate::gpio::GpioInterface;

pub struct WasmGpioInterface {
    pub state: std::rc::Rc<std::cell::RefCell<crate::gpio::GpioButtonState>>,
}

impl WasmGpioInterface {
    pub fn new() -> Self {
        WasmGpioInterface {
            state: std::rc::Rc::new(std::cell::RefCell::new(GpioButtonState::default())),
        }
    }
}

impl GpioInterface for WasmGpioInterface {
    fn get_updates(&self) -> Option<GpioButtonState> {
        Some(self.state.borrow().clone())
    }
}
