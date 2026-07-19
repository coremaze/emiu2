<div align="center">
  <img src="web/emiu2.svg" alt="Emiu2 Logo" width="64" height="64">
  <h1>emiu2</h1>
</div>

## Overview

Emiu2 is an emulator for the [Miuchiz handheld devices](https://miuchiz.com/overview).

### Web

Emiu2 can be run in the web browser and is available at [emiu2.miuchiz.com](https://emiu2.miuchiz.com). Select the OTP and flash files you'd like to load and click "Start Emulator".

### Desktop

Emiu2 provides a user-friendly interface intended for use on desktop operating systems as the `emiu2-desktop` binary.

### Dev (minimal)

The `emiu2-dev` binary can be useful for development or as a base for porting to other platforms.

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
 - RTC interrupts (Used for the alarm clock ingame)
 - IR communication (Used to play or trade with other Miuchiz devices)
 - USB communication (Used to communicate with a PC)

## Building

This software uses the typical Rust build system `cargo`. Get started with Rust at https://rustup.rs/.

### Web

The hosted version of emiu2 is built with `wasm-pack`.

Install `wasm-pack` with `cargo install wasm-pack`.

Build a release version of emiu2 with `wasm-pack build core --target web`.

### Desktop

Build a release version of emiu2 with `cargo build -r`, or run it directly from cargo with `cargo run -r -p emiu2-dev -- <OTP_FILE> <FLASH_FILE>`.

## Demos

![](DEMO.gif)

A demonstration on a mobile device complete with audio is available on YouTube:

[![YouTube](http://i.ytimg.com/vi/EOrG064Emxc/hqdefault.jpg)](https://www.youtube.com/watch?v=EOrG064Emxc)