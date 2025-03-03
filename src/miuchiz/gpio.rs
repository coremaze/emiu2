use super::{st2205u::GpioPort, GpioConnections};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MiuchizGpio {
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

impl MiuchizGpio {
    pub fn to_port(&self) -> (GpioPort, u8) {
        match self {
            MiuchizGpio::Up => (GpioPort::PA, 0),
            MiuchizGpio::Down => (GpioPort::PA, 1),
            MiuchizGpio::Left => (GpioPort::PA, 2),
            MiuchizGpio::Right => (GpioPort::PA, 3),
            MiuchizGpio::Power => (GpioPort::PA, 4),
            MiuchizGpio::Menu => (GpioPort::PA, 5),
            MiuchizGpio::UpsideUp => (GpioPort::PA, 6),
            MiuchizGpio::UpsideDown => (GpioPort::PA, 7),
            MiuchizGpio::ScreenTopLeft => (GpioPort::PB, 0),
            MiuchizGpio::ScreenTopRight => (GpioPort::PB, 1),
            MiuchizGpio::ScreenBottomLeft => (GpioPort::PB, 2),
            MiuchizGpio::ScreenBottomRight => (GpioPort::PB, 3),
            MiuchizGpio::Action => (GpioPort::PB, 4),
            MiuchizGpio::Mute => (GpioPort::PB, 5),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MiuchizButtonStates {
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

impl MiuchizButtonStates {
    pub fn set(&mut self, button: MiuchizGpio, value: bool) {
        match button {
            MiuchizGpio::Up => self.up = value,
            MiuchizGpio::Down => self.down = value,
            MiuchizGpio::Left => self.left = value,
            MiuchizGpio::Right => self.right = value,
            MiuchizGpio::Power => self.power = value,
            MiuchizGpio::Menu => self.menu = value,
            MiuchizGpio::UpsideUp => self.upside_up = value,
            MiuchizGpio::UpsideDown => self.upside_down = value,
            MiuchizGpio::ScreenTopLeft => self.screen_top_left = value,
            MiuchizGpio::ScreenTopRight => self.screen_top_right = value,
            MiuchizGpio::ScreenBottomLeft => self.screen_bottom_left = value,
            MiuchizGpio::ScreenBottomRight => self.screen_bottom_right = value,
            MiuchizGpio::Action => self.action = value,
            MiuchizGpio::Mute => self.mute = value,
        }
    }

    pub fn to_gpio_connections(&self) -> GpioConnections {
        let mut connections = GpioConnections::default();

        // All buttons are low when active
        let (port, bit) = MiuchizGpio::Up.to_port();
        connections.connect(port, bit, !self.up);
        let (port, bit) = MiuchizGpio::Down.to_port();
        connections.connect(port, bit, !self.down);
        let (port, bit) = MiuchizGpio::Left.to_port();
        connections.connect(port, bit, !self.left);
        let (port, bit) = MiuchizGpio::Right.to_port();
        connections.connect(port, bit, !self.right);
        let (port, bit) = MiuchizGpio::Power.to_port();
        connections.connect(port, bit, !self.power);
        let (port, bit) = MiuchizGpio::Menu.to_port();
        connections.connect(port, bit, !self.menu);
        let (port, bit) = MiuchizGpio::UpsideUp.to_port();
        connections.connect(port, bit, !self.upside_up);
        let (port, bit) = MiuchizGpio::UpsideDown.to_port();
        connections.connect(port, bit, !self.upside_down);
        let (port, bit) = MiuchizGpio::ScreenTopLeft.to_port();
        connections.connect(port, bit, !self.screen_top_left);
        let (port, bit) = MiuchizGpio::ScreenTopRight.to_port();
        connections.connect(port, bit, !self.screen_top_right);
        let (port, bit) = MiuchizGpio::ScreenBottomLeft.to_port();
        connections.connect(port, bit, !self.screen_bottom_left);
        let (port, bit) = MiuchizGpio::ScreenBottomRight.to_port();
        connections.connect(port, bit, !self.screen_bottom_right);
        let (port, bit) = MiuchizGpio::Action.to_port();
        connections.connect(port, bit, !self.action);
        let (port, bit) = MiuchizGpio::Mute.to_port();
        connections.connect(port, bit, !self.mute);
        connections
    }
}
