use std::sync::mpsc::{channel, Receiver};
use std::sync::mpsc::{Sender, TryRecvError};

use minifb::{Key, MouseButton, MouseMode, Scale, ScaleMode, Window, WindowOptions};

use crate::miuchiz::{
    GpioConnections, GpioInterfaceInternal, GpioState, MiuchizButtonStates, MiuchizGpio,
};
use crate::screen::{Pixel, Screen};
use crate::ssc;

pub struct MiniFbGpioInterface;
impl MiniFbGpioInterface {
    pub fn create_interface() -> (MiniFbGpioExternalInterface, MiniFbGpioInternalInterface) {
        let (button_tx, button_rx) = ssc::SingleStateChannel::new::<MiuchizButtonStates>();
        let (gpio_leds_tx, gpio_leds_rx) = ssc::SingleStateChannel::new::<GpioState>();

        (
            MiniFbGpioExternalInterface {
                button_tx,
                gpio_leds_rx,
                gpio_leds: None,
            },
            MiniFbGpioInternalInterface {
                button_rx,
                connections: GpioConnections::default(),
                gpio_leds_tx,
                last_gpio_state: None,
            },
        )
    }
}

pub struct MiniFbGpioExternalInterface {
    button_tx: ssc::Sender<MiuchizButtonStates>,
    gpio_leds_rx: ssc::Receiver<GpioState>,
    gpio_leds: Option<GpioState>,
}

impl MiniFbGpioExternalInterface {
    fn set_buttons(&self, buttons: MiuchizButtonStates) {
        self.button_tx.send(buttons);
    }

    fn get_gpio_leds(&mut self) -> Option<GpioState> {
        if let Some(gpio_leds) = self.gpio_leds_rx.recv() {
            self.gpio_leds = Some(gpio_leds);
        }

        self.gpio_leds.clone()
    }
}

pub struct MiniFbGpioInternalInterface {
    button_rx: ssc::Receiver<MiuchizButtonStates>,
    connections: GpioConnections,
    gpio_leds_tx: ssc::Sender<GpioState>,
    last_gpio_state: Option<GpioState>,
}

impl GpioInterfaceInternal for MiniFbGpioInternalInterface {
    fn get_inputs(&mut self, _cycle: u64) -> GpioConnections {
        if let Some(buttons) = self.button_rx.recv() {
            self.connections = buttons.to_gpio_connections();
        }

        self.connections.clone()
    }

    fn set_outputs(&mut self, state: GpioState, _cycle: u64) {
        // Only send the state if it has changed or None.
        if Some(state.clone()) != self.last_gpio_state {
            self.gpio_leds_tx.send(state.clone());
            self.last_gpio_state = Some(state);
        }
    }
}

pub struct MiniFbScreen {
    tx: Sender<MiniFBMessage>,
    rx: Receiver<MiniFBMessage>,
    closed: bool,
}

impl MiniFbScreen {
    pub fn open(
        title: &str,
        scale: usize,
        show_gpio: bool,
    ) -> (
        Self,
        MiniFbGpioInternalInterface,
        Sender<Vec<Pixel>>,
        MiniFbWorker,
    ) {
        let (host_tx, worker_rx) = channel::<MiniFBMessage>();
        let (worker_tx, host_rx) = channel::<MiniFBMessage>();
        let (screen_tx, screen_rx) = channel::<Vec<Pixel>>();

        let (gpio_external, gpio_internal) = MiniFbGpioInterface::create_interface();

        // The window is NOT created here. On macOS, AppKit requires that all
        // window/menu creation happen on the main thread, so the caller must
        // drive `MiniFbWorker::run` from the main thread.
        let worker = MiniFbWorker {
            title: title.to_owned(),
            scale,
            show_gpio,
            gpio_external,
            screen_rx,
            worker_tx,
            worker_rx,
        };

        (
            Self {
                tx: host_tx,
                rx: host_rx,
                closed: false,
            },
            gpio_internal,
            screen_tx,
            worker,
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

/// Owns everything required to create and drive the minifb window. Because
/// macOS requires window creation on the main thread, `run` is intended to be
/// called from the main thread while the emulator runs on a background thread.
pub struct MiniFbWorker {
    title: String,
    scale: usize,
    show_gpio: bool,
    gpio_external: MiniFbGpioExternalInterface,
    screen_rx: Receiver<Vec<Pixel>>,
    worker_tx: Sender<MiniFBMessage>,
    worker_rx: Receiver<MiniFBMessage>,
}

impl MiniFbWorker {
    pub fn run(self) {
        run_minifb_worker(self);
    }
}

struct MiniFbWindowButton {
    pub position: (usize, usize),
    pub button: MiuchizGpio,
    pub key: Option<Key>,
}

fn run_minifb_worker(worker: MiniFbWorker) {
    let MiniFbWorker {
        title,
        scale,
        show_gpio,
        mut gpio_external,
        screen_rx,
        worker_tx,
        worker_rx,
    } = worker;

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
            button: MiuchizGpio::Up,
            key: Some(Key::Up),
        },
        MiniFbWindowButton {
            position: (left_center.0, left_center.1 + 11 * scale),
            button: MiuchizGpio::Down,
            key: Some(Key::Down),
        },
        MiniFbWindowButton {
            position: (left_center.0 + 11 * scale, left_center.1),
            button: MiuchizGpio::Right,
            key: Some(Key::Right),
        },
        MiniFbWindowButton {
            position: (left_center.0 - 11 * scale, left_center.1),
            button: MiuchizGpio::Left,
            key: Some(Key::Left),
        },
        MiniFbWindowButton {
            position: (right_center.0 - 5 * scale, right_center.1),
            button: MiuchizGpio::Action,
            key: Some(Key::A),
        },
        MiniFbWindowButton {
            position: (right_center.0 + 10 * scale, right_center.1 - 17 * scale),
            button: MiuchizGpio::Menu,
            key: Some(Key::Menu),
        },
        MiniFbWindowButton {
            position: (extra_player_width / 2 - button_radius - 1, button_radius),
            button: MiuchizGpio::ScreenTopLeft,
            key: None,
        },
        MiniFbWindowButton {
            position: (
                extra_player_width / 2 - button_radius - 1,
                height * scale - button_radius - 1,
            ),
            button: MiuchizGpio::ScreenBottomLeft,
            key: None,
        },
        MiniFbWindowButton {
            position: (
                player_width - extra_player_width / 2 + button_radius,
                button_radius,
            ),
            button: MiuchizGpio::ScreenTopRight,
            key: None,
        },
        MiniFbWindowButton {
            position: (
                player_width - extra_player_width / 2 + button_radius,
                height * scale - button_radius - 1,
            ),
            button: MiuchizGpio::ScreenBottomRight,
            key: None,
        },
        MiniFbWindowButton {
            position: (bottom_center.0 - 3 * button_radius, bottom_center.1),
            button: MiuchizGpio::Power,
            key: Some(Key::P),
        },
        MiniFbWindowButton {
            position: (bottom_center.0 + 3 * button_radius, bottom_center.1),
            button: MiuchizGpio::Mute,
            key: Some(Key::M),
        },
    ];

    let mut last_button_state: Option<MiuchizButtonStates> = None;

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

            match worker_rx.try_recv() {
                Ok(MiniFBMessage::Close) => close = true,
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    println!("Worker thread disconnected");
                }
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

        let mut button_state = MiuchizButtonStates {
            up: false,
            down: false,
            left: false,
            right: false,
            power: false,
            menu: false,
            screen_top_left: false,
            screen_top_right: false,
            screen_bottom_left: false,
            screen_bottom_right: false,
            action: false,
            mute: false,
            upside_up: false,
            upside_down: false,
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

        let off_gpio_pixel = Pixel {
            red: 0,
            green: 0,
            blue: 0,
        };

        let on_gpio_pixel = Pixel {
            red: 255,
            green: 255,
            blue: 255,
        };

        // Draw GPIO LEDs
        {
            if show_gpio {
                let gpio_led_size: usize = 2 * scale;
                let gpio_led_spacing: usize = 1 * scale;
                let x1 = 0 + gpio_led_spacing;
                let x2 = x1 + gpio_led_size;
                let y1 = player_height - gpio_led_size;
                let y2 = y1 + gpio_led_size;
                let gpio_leds = gpio_external.get_gpio_leds();

                if let Some(gpio_leds) = &gpio_leds {
                    for (gpio_index, &&gpio_set) in [
                        &gpio_leds.pl,
                        &gpio_leds.pf,
                        &gpio_leds.pe,
                        &gpio_leds.pd,
                        &gpio_leds.pc,
                        &gpio_leds.pb,
                        &gpio_leds.pa,
                    ]
                    .iter()
                    .enumerate()
                    {
                        for i in 0..8 {
                            let x1 = i * (gpio_led_size + gpio_led_spacing) + gpio_led_spacing;
                            let x2 = x1 + gpio_led_size;
                            let y1 = player_height
                                - (gpio_index + 1) * (gpio_led_size + gpio_led_spacing);
                            let y2 = y1 + gpio_led_size;
                            let pixel = if gpio_set & (1 << i) != 0 {
                                on_gpio_pixel
                            } else {
                                off_gpio_pixel
                            };
                            for x in x1..x2 {
                                for y in y1..y2 {
                                    let player_index = y * player_width + x;
                                    player_buffer[player_index] = pixel.to_rgb_u32();
                                }
                            }
                        }
                    }
                }
            }
        }

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
        if Some(button_state.clone()) != last_button_state {
            gpio_external.set_buttons(button_state.clone());
            last_button_state = Some(button_state);
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
