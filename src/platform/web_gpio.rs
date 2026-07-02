use crate::miuchiz::{GpioConnections, GpioInterfaceInternal, GpioState, MiuchizButtonStates};

pub struct WasmGpioInterface {
    pub button_states: std::rc::Rc<std::cell::RefCell<MiuchizButtonStates>>,
}

impl WasmGpioInterface {
    pub fn new() -> Self {
        WasmGpioInterface {
            button_states: std::rc::Rc::new(
                std::cell::RefCell::new(MiuchizButtonStates::default()),
            ),
        }
    }
}

impl GpioInterfaceInternal for WasmGpioInterface {
    fn get_inputs(&mut self, _cycle: u64) -> GpioConnections {
        let state = self.button_states.borrow();
        state.to_gpio_connections()
    }

    fn set_outputs(&mut self, _state: GpioState, _cycle: u64) {}
}
