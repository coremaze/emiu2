use std::cell::RefCell;
use std::sync::mpsc::{channel, Receiver};
use std::{error::Error, sync::mpsc::Sender};

use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};

use crate::gpio::Button;

pub trait Screen {
    fn set_pixels(&self, pixels: &[Pixel]);
}

#[derive(Clone, Copy)]
pub struct Pixel {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Pixel {
    pub fn to_rgb_u32(&self) -> u32 {
        let mut e = 0u32;
        e |= self.red as u32;
        e <<= 8;
        e |= self.green as u32;
        e <<= 8;
        e |= self.blue as u32;
        e
    }
}

pub struct MiniFbScreen {
    tx: Sender<MiniFBMessage>,
    rx: Receiver<MiniFBWorkerMessage>,
    closed: RefCell<bool>,
    keys: RefCell<Vec<Key>>,
    buttons: RefCell<Vec<Button>>,
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
            keys: RefCell::new(Vec::<Key>::new()),
            buttons: RefCell::new(Vec::<Button>::new()),
        }
    }

    pub fn close(&self) {
        self.tx.send(MiniFBMessage::Close).unwrap_or_else(|err| {
            println!("Couldn't send close message to display client: {err:?}");
        });
    }

    fn update(&self) {
        loop {
            match self.rx.try_recv() {
                Ok(message) => match message {
                    MiniFBWorkerMessage::Keys(keys) => *self.keys.borrow_mut() = keys,
                    MiniFBWorkerMessage::Close(_result) => *self.closed.borrow_mut() = true,
                    MiniFBWorkerMessage::Buttons(buttons) => *self.buttons.borrow_mut() = buttons,
                },
                Err(_) => break,
            }
        }
    }

    pub fn is_open(&self) -> bool {
        self.update();
        !*self.closed.borrow()
    }

    pub fn is_button_pressed(&self, button: Button) -> bool {
        let keys = self.keys.borrow();
        let buttons = self.buttons.borrow();
        buttons.contains(&button)
            || match button {
                Button::Up => keys.contains(&Key::Up),
                Button::Down => keys.contains(&Key::Down),
                Button::Left => keys.contains(&Key::Left),
                Button::Right => keys.contains(&Key::Right),
                Button::Power => keys.contains(&Key::P),
                Button::Menu => keys.contains(&Key::Menu),
                Button::UpsideUp => false,
                Button::UpsideDown => false,
                Button::ScreenTopLeft => false,
                Button::ScreenTopRight => false,
                Button::ScreenBottomLeft => false,
                Button::ScreenBottomRight => false,
                Button::Action => keys.contains(&Key::A),
                Button::Mute => false,
            }
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
    Keys(Vec<Key>),
    Buttons(Vec<Button>),
    Close(Result<(), minifb::Error>),
}

enum MiniFBMessage {
    UpdatePixels(Vec<Pixel>),
    Close,
}

struct MiniFbWindowButton {
    pub position: (usize, usize),
    pub button: Button,
}

fn run_minifb_worker(
    title: String,
    width: usize,
    height: usize,
    scale: usize,
    tx: Sender<MiniFBWorkerMessage>,
    rx: Receiver<MiniFBMessage>,
) {
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
            button: Button::Up,
        },
        MiniFbWindowButton {
            position: (left_center.0, left_center.1 + 11 * scale),
            button: Button::Down,
        },
        MiniFbWindowButton {
            position: (left_center.0 + 11 * scale, left_center.1),
            button: Button::Right,
        },
        MiniFbWindowButton {
            position: (left_center.0 - 11 * scale, left_center.1),
            button: Button::Left,
        },
        MiniFbWindowButton {
            position: (right_center.0 - 5 * scale, right_center.1),
            button: Button::Action,
        },
        MiniFbWindowButton {
            position: (right_center.0 + 10 * scale, right_center.1 - 17 * scale),
            button: Button::Menu,
        },
        MiniFbWindowButton {
            position: (extra_player_width / 2 - button_radius - 1, button_radius),
            button: Button::ScreenTopLeft,
        },
        MiniFbWindowButton {
            position: (
                extra_player_width / 2 - button_radius - 1,
                height * scale - button_radius - 1,
            ),
            button: Button::ScreenBottomLeft,
        },
        MiniFbWindowButton {
            position: (
                player_width - extra_player_width / 2 + button_radius,
                button_radius,
            ),
            button: Button::ScreenTopRight,
        },
        MiniFbWindowButton {
            position: (
                player_width - extra_player_width / 2 + button_radius,
                height * scale - button_radius - 1,
            ),
            button: Button::ScreenBottomRight,
        },
        MiniFbWindowButton {
            position: (bottom_center.0 - 3 * button_radius, bottom_center.1),
            button: Button::Power,
        },
        MiniFbWindowButton {
            position: (bottom_center.0 + 3 * button_radius, bottom_center.1),
            button: Button::Mute,
        },
    ];

    let mut window = match Window::new(
        &title,
        player_width,
        player_height,
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

            match rx.try_recv() {
                Ok(MiniFBMessage::UpdatePixels(pixels)) => {
                    pixel_update = Some(pixels);
                }
                Ok(MiniFBMessage::Close) => close = true,
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

        let mut clicked_buttons = Vec::<Button>::new();
        for button in &buttons {
            let x1 = button.position.0 - button_radius;
            let x2 = button.position.0 + button_radius;
            let y1 = button.position.1 - button_radius;
            let y2 = button.position.1 + button_radius;

            let mut clicked = false;
            let mousedown = window.get_mouse_down(MouseButton::Left);
            let mousepos = window.get_mouse_pos(MouseMode::Discard);

            if mousedown {
                if let Some(pos) = mousepos {
                    let xpos = pos.0 as usize;
                    let ypos = pos.1 as usize;

                    if xpos >= x1 && xpos <= x2 && ypos >= y1 && ypos <= y2 {
                        clicked = true;
                    }
                }
            }

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

            if clicked {
                clicked_buttons.push(button.button);
            }
        }

        if let Err(err) = window.update_with_buffer(&player_buffer, player_width, player_height) {
            eprintln!("Failed to update window: {err:?}");
            close = true;
        }

        tx.send(MiniFBWorkerMessage::Keys(window.get_keys()));
        tx.send(MiniFBWorkerMessage::Buttons(clicked_buttons));
    }

    tx.send(MiniFBWorkerMessage::Close(Ok(())));
}
