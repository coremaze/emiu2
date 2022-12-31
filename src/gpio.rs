use crate::display::MiniFbScreen;

pub trait Gpio {
    fn get_input(&self, bit: u32) -> bool;
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    Power,
    Menu,
    UpsideUp,
    UpsideDown,
    ScreenTopLeft,
    ScreenTopRight,
    ScreenBottomLeft,
    ScreenBottomRight,
    Action,
    Mute,
}

impl Gpio for MiniFbScreen {
    fn get_input(&self, bit: u32) -> bool {
        let button = match bit {
            0 => Button::Up,
            1 => Button::Down,
            2 => Button::Left,
            3 => Button::Right,
            4 => Button::Power,
            5 => Button::Menu,
            6 => Button::UpsideUp,
            7 => Button::UpsideDown,
            8 => Button::ScreenTopLeft,
            9 => Button::ScreenTopRight,
            10 => Button::ScreenBottomLeft,
            11 => Button::ScreenBottomRight,
            12 => Button::Action,
            13 => Button::Mute,
            _ => return false,
        };
        self.is_button_pressed(button)
    }
}
