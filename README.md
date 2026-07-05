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

### Savestates

On desktop, F5 saves the complete machine state and F9 restores it. The savestate file defaults to the flash image path with `.state` appended; override it with `--savestate-file`.

### Playing together

Miuchiz devices play and trade with each other over IR, and emiu2 can carry that link between two emulators in several ways.

**On one machine or LAN**, connect two emulators directly:

```sh
emiu2 OTP.dat flash1.dat --ir listen:5885
emiu2 OTP.dat flash2.dat --ir connect:127.0.0.1:5885
```

**Over the internet**, use the relay server and friend codes. Someone runs the relay on a reachable host:

```sh
cargo run -r -p emiu2-relay          # listens on port 5885
```

Each player then starts their emulator pointed at the relay:

```sh
emiu2 OTP.dat flash.dat --ir relay:relay.example.com:5885
```

On connecting, the terminal prints an ephemeral six-character friend code. Share it with the other player out of band; either of you types `join <code>` into the emulator's terminal to pair (`leave` unpairs, `status` shows the connection).

**In the browser**, click "Play with a Friend" beneath the emulator: the page connects to its site's relay automatically and shows your friend code; exchange and join codes the same way. Native and browser players can pair with each other.

High network latency is handled automatically. The Miuchiz firmware only listens for an IR reply for about 98ms, so on slow links the emulator snapshots itself whenever the firmware starts listening and invisibly rewinds to that point when a late reply arrives.

Hosting the web version's relay is one reverse-proxy rule: the browser client connects to `wss://<site>/relay` (`ws://` on plain-HTTP sites), so route that path's WebSocket upgrade to the relay's port. The relay speaks plain TCP and WebSocket on a single port and leaves TLS to the proxy. For development and self-hosting, a `?relay=ws://host:port` query parameter overrides the endpoint.

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
 - IR communication (Used to play or trade with other Miuchiz devices),
   including between emulators over the network with latency hiding

 It is possibly more useful to list the features which the Miuchiz firmware uses but which are not yet finished:
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