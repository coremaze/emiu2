# emiu2

## Overview

Emiu2 is a work-in-progress emulator for the [Miuchiz handheld devices](https://miuchiz.com/overview).

## Usage

Emiu2 requires a dump of a Miuchiz handheld device's OTP (One Time Programmable) memory as well as a dump of its flash memory. These dumps can be created using [Native-Miuchiz-Handheld-USB-Utilities](https://github.com/ChrisMiuchiz/Native-Miuchiz-Handheld-USB-Utilities). Existing images of both can be obtained from https://archive.miuchiz.com/root/handhelds/.

To start the emulator, run `emiu2 <OTP_FILE> <FLASH_FILE>`.

## Building

This software uses the typical Rust build system `cargo`. Get started with Rust at https://rustup.rs/.

Build a release version of emiu2 with `cargo build -r`, or run it directly from cargo with `cargo run -r -- <OTP_FILE> <FLASH_FILE>`.

## Demo

![](DEMO.gif)