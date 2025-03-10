<div align="center">
  <img src="web/emiu2.svg" alt="Emiu2 Logo" width="64" height="64">
  <h1>emiu2</h1>
</div>

## Overview

Emiu2 is an emulator for the [Miuchiz handheld devices](https://miuchiz.com/overview).

## Usage

Emiu2 requires a dump of a Miuchiz handheld device's OTP (One Time Programmable) memory as well as a dump of its flash memory. These dumps can be created using [Native-Miuchiz-Handheld-USB-Utilities](https://github.com/ChrisMiuchiz/Native-Miuchiz-Handheld-USB-Utilities). Existing images of both can be obtained from https://archive.miuchiz.com/root/handhelds/.

### Web

Emiu2 can also be run in the web browser and is available at [emiu2.miuchiz.com](https://emiu2.miuchiz.com). Select the OTP and flash files you'd like to load and click "Start Emulator".

### Desktop

To start the emulator, run `emiu2 <OTP_FILE> <FLASH_FILE>`. Run `emiu2 --help` for more options.

## Features

The implementation of the microcontroller itself is not complete or accurate, but with regard to the features the Miuchiz firmware uses, accuracy and support are extremely good.

At a high level, the emulator supports the following:
 - 65C02 CPU core
 - Memory bank mapping
 - Video
 - Audio
 - Flash
 - OTP (One Time Programmable memory)
 - GPIO

 It is possibly more useful to list the features which the Miuchiz firmware uses but which are not yet finished:
 - RTC interrupts (Used for the alarm clock ingame)
 - IR communication (Used to play or trade with other Miuchiz devices)
 - USB communication (Used to communicate with a PC)

## Building

This software uses the typical Rust build system `cargo`. Get started with Rust at https://rustup.rs/.

### Web

The hosted version of emiu2 is built with `wasm-pack`.

Install `wasm-pack` with `cargo install wasm-pack`.

Build a release version of emiu2 with `wasm-pack build --target web`.

### Desktop

Build a release version of emiu2 with `cargo build -r`, or run it directly from cargo with `cargo run -r -- <OTP_FILE> <FLASH_FILE>`.

## Demos

![](DEMO.gif)

A demonstration on a mobile device complete with audio is available on YouTube:

[![YouTube](http://i.ytimg.com/vi/EOrG064Emxc/hqdefault.jpg)](https://www.youtube.com/watch?v=EOrG064Emxc)