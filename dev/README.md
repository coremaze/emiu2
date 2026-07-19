## emiu2-dev usage

`emiu2-dev` requires a dump of a Miuchiz handheld device's OTP (One Time Programmable) memory as well as a dump of its flash memory. These dumps can be created using [Native-Miuchiz-Handheld-USB-Utilities](https://github.com/ChrisMiuchiz/Native-Miuchiz-Handheld-USB-Utilities). Existing images of both can be obtained from https://archive.miuchiz.com/root/handhelds/.

### Desktop

To start the emulator, run `emiu2-dev <OTP_FILE> <FLASH_FILE>`. Run `emiu2-dev --help` for more options.

### Savestates

On desktop, F5 saves the complete machine state and F9 restores it. The savestate file defaults to the flash image path with `.state` appended; override it with `--savestate-file`.

### USB

The emulated device has a working USB port, and the cable is its own piece of state: press **U** to plug it in or unplug it (or start with `--usb-plugged`). Firmware sees the cable through the USBCON connect-status bit the moment it's plugged - stable, whether or not any host software is talking - just like a real cable left in the socket.

Host tools discover running emulators the way they discover real handhelds: each desktop instance publishes an endpoint in emiu2's runtime directory under the shared [Miuchiz Reborn path policy](https://github.com/coremaze/Miuchiz-Reborn-Paths) (`$XDG_RUNTIME_DIR/miuchiz-reborn/emiu2` on Linux; reroot everything with `MIUCHIZ_REBORN_HOME`, or override just this directory with `EMIU2_USB_DIR`), and [Native-Miuchiz-Handheld-USB-Utilities](https://github.com/ChrisMiuchiz/Native-Miuchiz-Handheld-USB-Utilities) built with the emulator backend lists them alongside physical devices, so `miuchiz dump-flash`, `load-flash`, and friends work on an emulator unchanged. Tools can only reach the device while the cable is plugged.

Like a real Miuchiz, the device only answers USB in its "Please Connect to PC" mode; start the emulator with `--connect-mode` to boot straight into it (this implies a plugged cable).

### Playing together

Miuchiz devices play and trade with each other over IR, and emiu2 can carry that link between two emulators in several ways.

**On one machine or LAN**, connect two emulators directly:

```sh
emiu2-dev OTP.dat flash1.dat --ir listen:5885
emiu2-dev OTP.dat flash2.dat --ir connect:127.0.0.1:5885
```

**Over the internet**, use the relay server and friend codes. Someone runs the relay on a reachable host:

```sh
cargo run -r -p emiu2-relay          # listens on port 5885
```

Each player then starts their emulator pointed at the relay:

```sh
emiu2-dev OTP.dat flash.dat --ir relay:relay.example.com:5885
```

On connecting, the terminal prints an ephemeral six-character friend code. Share it with the other player out of band; either of you types `join <code>` into the emulator's terminal to pair (`leave` unpairs, `status` shows the connection).

**In the browser**, click "Play with a Friend" beneath the emulator: the page connects to its site's relay automatically and shows your friend code; exchange and join codes the same way. Native and browser players can pair with each other.

High network latency is handled automatically. The Miuchiz firmware only listens for an IR reply for about 98ms, so on slow links the emulator snapshots itself whenever the firmware starts listening and invisibly rewinds to that point when a late reply arrives.

Hosting the web version's relay is one reverse-proxy rule: the browser client connects to `wss://<site>/relay` (`ws://` on plain-HTTP sites), so route that path's WebSocket upgrade to the relay's port. The relay speaks plain TCP and WebSocket on a single port and leaves TLS to the proxy. For development and self-hosting, a `?relay=ws://host:port` query parameter overrides the endpoint.