pub trait GpioInterface {
    fn get_updates(&self) -> Option<GpioButtonState>;
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub enum GpioButton {
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

#[derive(Clone, Debug, PartialEq)]
pub struct GpioButtonState {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub power: bool,
    pub menu: bool,
    pub upside_up: bool,
    pub upside_down: bool,
    pub screen_top_left: bool,
    pub screen_top_right: bool,
    pub screen_bottom_left: bool,
    pub screen_bottom_right: bool,
    pub action: bool,
    pub mute: bool,
}

impl GpioButtonState {
    pub fn set(&mut self, button: GpioButton, pressed: bool) {
        let b = match button {
            GpioButton::Up => &mut self.up,
            GpioButton::Down => &mut self.down,
            GpioButton::Left => &mut self.left,
            GpioButton::Right => &mut self.right,
            GpioButton::Power => &mut self.power,
            GpioButton::Menu => &mut self.menu,
            GpioButton::UpsideUp => &mut self.upside_up,
            GpioButton::UpsideDown => &mut self.upside_down,
            GpioButton::ScreenTopLeft => &mut self.screen_top_left,
            GpioButton::ScreenTopRight => &mut self.screen_top_right,
            GpioButton::ScreenBottomLeft => &mut self.screen_bottom_left,
            GpioButton::ScreenBottomRight => &mut self.screen_bottom_right,
            GpioButton::Action => &mut self.action,
            GpioButton::Mute => &mut self.mute,
        };
        *b = pressed;
    }

    pub fn get(&self, button: GpioButton) -> bool {
        match button {
            GpioButton::Up => self.up,
            GpioButton::Down => self.down,
            GpioButton::Left => self.left,
            GpioButton::Right => self.right,
            GpioButton::Power => self.power,
            GpioButton::Menu => self.menu,
            GpioButton::UpsideUp => self.upside_up,
            GpioButton::UpsideDown => self.upside_down,
            GpioButton::ScreenTopLeft => self.screen_top_left,
            GpioButton::ScreenTopRight => self.screen_top_right,
            GpioButton::ScreenBottomLeft => self.screen_bottom_left,
            GpioButton::ScreenBottomRight => self.screen_bottom_right,
            GpioButton::Action => self.action,
            GpioButton::Mute => self.mute,
        }
    }
}

impl Default for GpioButtonState {
    fn default() -> Self {
        Self {
            up: false,
            down: false,
            left: false,
            right: false,
            power: false,
            menu: false,
            upside_up: false,
            upside_down: false,
            screen_top_left: false,
            screen_top_right: false,
            screen_bottom_left: false,
            screen_bottom_right: false,
            action: false,
            mute: false,
        }
    }
}
