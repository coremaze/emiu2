use std::sync::mpsc::Sender;
use std::sync::mpsc::{channel, Receiver};

use minifb::{Key, MouseButton, MouseMode, Scale, ScaleMode, Window, WindowOptions};

use crate::gpio::{GpioButton, GpioButtonState, GpioInterface};
use crate::screen::{Pixel, Screen};

pub struct MiniFbScreen {
    tx: Sender<MiniFBMessage>,
    rx: Receiver<MiniFBMessage>,
    closed: bool,
}

impl MiniFbScreen {
    pub fn open(
        title: &str,
        scale: usize,
    ) -> (Self, Receiver<GpioButtonState>, Sender<Vec<Pixel>>) {
        let (host_tx, worker_rx) = channel::<MiniFBMessage>();
        let (worker_tx, host_rx) = channel::<MiniFBMessage>();
        let (gpio_tx, gpio_rx) = channel::<GpioButtonState>();
        let (screen_tx, screen_rx) = channel::<Vec<Pixel>>();

        let owned_title = title.to_owned();
        std::thread::spawn(move || {
            run_minifb_worker(owned_title, scale, gpio_tx, screen_rx, worker_tx, worker_rx)
        });

        (
            Self {
                tx: host_tx,
                rx: host_rx,
                closed: false,
            },
            gpio_rx,
            screen_tx,
        )
    }

    pub fn close(&self) {
        self.tx.send(MiniFBMessage::Close).ok();
    }

    pub fn update_state(&mut self) {
        match self.rx.try_recv() {
            Ok(message) => match message {
                MiniFBMessage::Close => {
                    self.closed = true;
                }
            },
            Err(_) => return,
        }
    }

    pub fn is_open(&self) -> bool {
        !self.closed
    }
}

impl Drop for MiniFbScreen {
    fn drop(&mut self) {
        self.close();
    }
}

enum MiniFBMessage {
    Close,
}

struct MiniFbWindowButton {
    pub position: (usize, usize),
    pub button: GpioButton,
    pub key: Option<Key>,
}

fn run_minifb_worker(
    title: String,
    scale: usize,
    gpio_tx: Sender<GpioButtonState>,
    screen_rx: Receiver<Vec<Pixel>>,
    worker_tx: Sender<MiniFBMessage>,
    worker_rx: Receiver<MiniFBMessage>,
) {
    let width = 98;
    let height = 67;

    let extra_player_width = width * scale;
    let extra_player_height = height / 2 * scale;
    let player_width = width * scale + extra_player_width;
    let player_height = height * scale + extra_player_height;

    let button_radius = scale * 5;

    let left_center = (extra_player_width / 4, player_height / 3);
    let right_center = (player_width - extra_player_width / 4, player_height / 3);
    let bottom_center = (player_width / 2, height * scale + extra_player_height / 2);

    let buttons = [
        MiniFbWindowButton {
            position: (left_center.0, left_center.1 - 11 * scale),
            button: GpioButton::Up,
            key: Some(Key::Up),
        },
        MiniFbWindowButton {
            position: (left_center.0, left_center.1 + 11 * scale),
            button: GpioButton::Down,
            key: Some(Key::Down),
        },
        MiniFbWindowButton {
            position: (left_center.0 + 11 * scale, left_center.1),
            button: GpioButton::Right,
            key: Some(Key::Right),
        },
        MiniFbWindowButton {
            position: (left_center.0 - 11 * scale, left_center.1),
            button: GpioButton::Left,
            key: Some(Key::Left),
        },
        MiniFbWindowButton {
            position: (right_center.0 - 5 * scale, right_center.1),
            button: GpioButton::Action,
            key: Some(Key::A),
        },
        MiniFbWindowButton {
            position: (right_center.0 + 10 * scale, right_center.1 - 17 * scale),
            button: GpioButton::Menu,
            key: Some(Key::Menu),
        },
        MiniFbWindowButton {
            position: (extra_player_width / 2 - button_radius - 1, button_radius),
            button: GpioButton::ScreenTopLeft,
            key: None,
        },
        MiniFbWindowButton {
            position: (
                extra_player_width / 2 - button_radius - 1,
                height * scale - button_radius - 1,
            ),
            button: GpioButton::ScreenBottomLeft,
            key: None,
        },
        MiniFbWindowButton {
            position: (
                player_width - extra_player_width / 2 + button_radius,
                button_radius,
            ),
            button: GpioButton::ScreenTopRight,
            key: None,
        },
        MiniFbWindowButton {
            position: (
                player_width - extra_player_width / 2 + button_radius,
                height * scale - button_radius - 1,
            ),
            button: GpioButton::ScreenBottomRight,
            key: None,
        },
        MiniFbWindowButton {
            position: (bottom_center.0 - 3 * button_radius, bottom_center.1),
            button: GpioButton::Power,
            key: Some(Key::P),
        },
        MiniFbWindowButton {
            position: (bottom_center.0 + 3 * button_radius, bottom_center.1),
            button: GpioButton::Mute,
            key: Some(Key::M),
        },
    ];

    let mut last_button_state = GpioButtonState {
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
    };

    let mut window = match Window::new(
        &title,
        player_width,
        player_height,
        WindowOptions {
            borderless: false,
            title: true,
            resize: false,
            scale: Scale::X1,
            scale_mode: ScaleMode::UpperLeft,
            topmost: false,
            transparency: false,
            none: false,
        },
    ) {
        Ok(window) => window,
        Err(err) => {
            eprintln!("Failed to create window: {err:?}");
            if let Err(err) = worker_tx.send(MiniFBMessage::Close) {
                eprintln!("Failed to send close message: {err:?}");
            }
            return;
        }
    };

    // Limit to max ~60 fps update rate
    window.set_target_fps(60);

    let mut screen_buffer = vec![0; width * height];

    let mut player_buffer = vec![0x00303050; player_width * player_height];
    let screen_pos = (extra_player_width / 2, 0);

    let mut pixel_update: Option<Vec<Pixel>> = None;
    let mut close = false;
    while !close {
        loop {
            if !window.is_open() {
                close = true;
                break;
            }

            match screen_rx.try_recv() {
                Ok(pixels) => {
                    pixel_update = Some(pixels);
                }
                Err(_) => break,
            }
        }

        // Update the screen buffer if there are new pixels
        if let Some(pixels) = &pixel_update {
            for (i, pixel) in pixels.iter().enumerate() {
                if i < screen_buffer.len() {
                    screen_buffer[i] = pixel.to_rgb_u32();
                }
            }

            pixel_update = None;
        }

        // Put the screen buffer on the player buffer
        for x in 0..width {
            for y in 0..height {
                let pixel = screen_buffer[y * width + x];
                for x2 in 0..scale {
                    for y2 in 0..scale {
                        let player_x = x * scale + x2 + screen_pos.0;
                        let player_y = y * scale + y2 + screen_pos.1;
                        let player_index = player_y * player_width + player_x;
                        player_buffer[player_index] = pixel;
                    }
                }
            }
        }

        let mut button_state = GpioButtonState {
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
        };
        let pressed_keys = window.get_keys();

        // Put the buttons on the player
        let clicked_pixel = Pixel {
            red: 255,
            green: 200,
            blue: 200,
        };

        let unclicked_pixel = Pixel {
            red: 200,
            green: 200,
            blue: 200,
        };

        let outline_pixel = Pixel {
            red: 0,
            green: 0,
            blue: 0,
        };

        for button in &buttons {
            let x1 = button.position.0 - button_radius;
            let x2 = button.position.0 + button_radius;
            let y1 = button.position.1 - button_radius;
            let y2 = button.position.1 + button_radius;

            let mousedown = window.get_mouse_down(MouseButton::Left);
            let mousepos = window.get_mouse_pos(MouseMode::Discard);

            // Check to see if the button is clicked.
            let clicked = {
                let mut c = false;
                if mousedown {
                    if let Some(pos) = mousepos {
                        let xpos = pos.0 as usize;
                        let ypos = pos.1 as usize;

                        if xpos >= x1 && xpos <= x2 && ypos >= y1 && ypos <= y2 {
                            c = true;
                        }
                    }
                }
                c
            };

            // Draw the button's box
            for x in x1..=x2 {
                for y in y1..=y2 {
                    let player_index = y * player_width + x;
                    if player_index < player_buffer.len() {
                        let pixel = if (x == x1 || x == x2) || (y == y1 || y == y2) {
                            outline_pixel
                        } else if clicked {
                            clicked_pixel
                        } else {
                            unclicked_pixel
                        };
                        player_buffer[player_index] = pixel.to_rgb_u32();
                    }
                }
            }

            // Set the button state if the button is clicked or the key is pressed.
            if clicked {
                button_state.set(button.button, true);
            } else if let Some(key) = button.key {
                if pressed_keys.contains(&key) {
                    button_state.set(button.button, true);
                }
            }
        }

        // Send the button state if it has changed.
        if button_state != last_button_state {
            if let Err(err) = gpio_tx.send(button_state.clone()) {
                eprintln!("Failed to send button state: {err:?}");
            }
            last_button_state = button_state;
        }

        // Paint the player buffer to the window
        if let Err(err) = window.update_with_buffer(&player_buffer, player_width, player_height) {
            eprintln!("Failed to update window: {err:?}");
            close = true;
        }
    }

    // Send the close message
    if let Err(err) = worker_tx.send(MiniFBMessage::Close) {
        eprintln!("Failed to send close message: {err:?}");
    }
}

pub struct MiniFbGpioInterface {
    receiver: Receiver<GpioButtonState>,
}

impl MiniFbGpioInterface {
    pub fn new(receiver: Receiver<GpioButtonState>) -> Self {
        Self { receiver }
    }
}

impl GpioInterface for MiniFbGpioInterface {
    fn get_updates(&self) -> Option<GpioButtonState> {
        self.receiver.try_recv().ok()
    }
}

pub struct MiniFbScreenInterface {
    tx: Sender<Vec<Pixel>>,
}

impl MiniFbScreenInterface {
    pub fn new(tx: Sender<Vec<Pixel>>) -> Self {
        Self { tx }
    }
}

impl Screen for MiniFbScreenInterface {
    fn set_pixels(&self, pixels: &[Pixel]) {
        if let Err(err) = self.tx.send(pixels.to_vec()) {
            eprintln!("Failed to send pixels: {err:?}");
        }
    }
}
