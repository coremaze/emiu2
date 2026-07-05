//! End-to-end USB test against real firmware, entirely in-process.
//!
//! The handheld is booted with the "cable" already plugged (the channel
//! pair's connect flag raised) and the power button pressed by a scripted
//! GPIO interface. The firmware detects the host via the USBCON
//! connect-status bit and brings up the SIE; the test then acts as the
//! host on the other end of the channel pair, driving enumeration, SCSI
//! over Bulk-Only Transport, and the device's tunneled flash protocol -
//! the same layers `usb_client` exercises over TCP.
//!
//! Skipped when the firmware images are not present in `firmware/`
//! (they are not distributable).

use emiu2::audio::AudioInterface;
use emiu2::ir::DisconnectedIr;
use emiu2::memory::AddressSpace;
use emiu2::miuchiz::{
    GpioConnections, GpioInterfaceInternal, GpioState, Handheld, MiuchizButtonStates, UsbResponse,
    UsbToken, UsbTransaction,
};
use emiu2::screen::{Pixel, Screen};
use emiu2::usb_interface::{channel_pair, UsbHostPort};
use std::path::PathBuf;

struct NullScreen;

impl Screen for NullScreen {
    fn set_pixels(&self, _pixels: &[Pixel]) {}
}

struct NullAudio;

impl AudioInterface for NullAudio {
    fn set_clock_rate(&mut self, _emulated_clock_rate: u64) {}

    fn needs_sample(&self, _current_cycle: u64) -> bool {
        false
    }

    fn add_sample(&mut self, _value: f32) {}
}

/// Holds the whole D-pad (all four directions, active low on port A) from
/// reset. The boot ROM samples PA at cold start and a fully-held D-pad
/// selects its "Please Connect to PC" mode, whose main loop brings up the
/// SIE and services USB - the same thing a player does to connect a real
/// handheld. Released once the ROM is in that loop.
struct ConnectModeGpio;

/// How long the D-pad stays held, in oscillator cycles (~4 s; the boot
/// decision happens within the first few million cycles).
const DPAD_HOLD_CYCLES: u64 = 60_000_000;

impl GpioInterfaceInternal for ConnectModeGpio {
    fn get_inputs(&mut self, cycle: u64) -> GpioConnections {
        let mut buttons = MiuchizButtonStates::default();
        let held = cycle < DPAD_HOLD_CYCLES;
        buttons.up = held;
        buttons.down = held;
        buttons.left = held;
        buttons.right = held;
        buttons.to_gpio_connections()
    }

    fn set_outputs(&mut self, _state: GpioState, _cycle: u64) {}
}

fn load_firmware() -> Option<(Vec<u8>, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("firmware");
    let otp = std::fs::read(dir.join("OTP.dat")).ok()?;
    let flash = std::fs::read(dir.join("Spike 1.02.dat")).ok()?;
    Some((otp, flash))
}

/// ST2205U USBCON register: bit 7 is USBEN, the firmware-side "SIE is up"
/// switch. Reading it has no side effects.
const USBCON: usize = 0x0070;

fn usb_enabled(h: &mut Handheld) -> bool {
    h.mcu.core.address_space.read_u8(USBCON) & 0x80 != 0
}

fn step_cycles(h: &mut Handheld, cycles: u64) {
    let target = h.mcu.core.cycles + cycles;
    while h.mcu.core.cycles < target {
        h.mcu.step();
    }
}

// ---------------------------------------------------------------------------
// Host-side driver: usb_client's transport, with "wait" replaced by
// stepping the machine (host and device share this thread).
// ---------------------------------------------------------------------------

const EP_CONTROL: u8 = 0;
const EP_BULK: u8 = 1;
const EP0_MAX: usize = 8;
const BULK_MAX: usize = 64;
const SECTOR_SIZE: usize = 512;
const PAGE_SIZE: usize = 0x1000;
const SECTOR_SCSI_WRITE: u32 = 0x31;
const SECTOR_DATA_READ: u32 = 0x58;

/// Cycles the device may spend before answering a submitted transaction at
/// all (it services one per ~16k-cycle interval).
const RESPONSE_CYCLE_BUDGET: u64 = 50_000_000;
/// NAK retry budget per transaction (each retry runs the machine further).
const NAK_LIMIT: u32 = 20_000;

struct Host {
    handheld: Handheld,
    port: UsbHostPort,
}

impl Host {
    fn transact(&mut self, endpoint: u8, token: UsbToken, data: Vec<u8>) -> UsbResponse {
        let txn = UsbTransaction {
            endpoint,
            token,
            data,
        };
        let mut naks = 0u32;
        loop {
            self.port.submit(txn.clone());
            let start = self.handheld.mcu.core.cycles;
            let resp = loop {
                step_cycles(&mut self.handheld, 4_000);
                if let Some(resp) = self.port.try_response() {
                    break resp;
                }
                assert!(
                    self.handheld.mcu.core.cycles - start < RESPONSE_CYCLE_BUDGET,
                    "device stopped answering transactions ({txn:?})"
                );
            };
            match resp {
                UsbResponse::Nak => {
                    naks += 1;
                    assert!(naks < NAK_LIMIT, "NAK retry budget exhausted ({txn:?})");
                    // Give the firmware time to service its buffers.
                    step_cycles(&mut self.handheld, 20_000);
                }
                other => return other,
            }
        }
    }

    fn expect_ack(&mut self, endpoint: u8, token: UsbToken, data: Vec<u8>, what: &str) {
        match self.transact(endpoint, token, data) {
            UsbResponse::Ack => {}
            other => panic!("{what}: expected ACK, got {other:?}"),
        }
    }

    fn control_in(
        &mut self,
        bm_request_type: u8,
        b_request: u8,
        w_value: u16,
        w_index: u16,
        length: u16,
    ) -> Vec<u8> {
        let setup = vec![
            bm_request_type,
            b_request,
            w_value as u8,
            (w_value >> 8) as u8,
            w_index as u8,
            (w_index >> 8) as u8,
            length as u8,
            (length >> 8) as u8,
        ];
        self.expect_ack(EP_CONTROL, UsbToken::Setup, setup, "control SETUP");

        let mut data = Vec::new();
        while data.len() < length as usize {
            match self.transact(EP_CONTROL, UsbToken::In, vec![]) {
                UsbResponse::Data(chunk) => {
                    let short = chunk.len() < EP0_MAX;
                    data.extend_from_slice(&chunk);
                    if short {
                        break;
                    }
                }
                other => panic!("control IN: unexpected {other:?}"),
            }
        }
        // Status stage: zero-length OUT.
        self.expect_ack(EP_CONTROL, UsbToken::Out, vec![], "control status");
        data
    }

    fn bulk_out(&mut self, data: &[u8]) {
        for chunk in data.chunks(BULK_MAX) {
            self.expect_ack(EP_BULK, UsbToken::Out, chunk.to_vec(), "bulk OUT");
        }
    }

    fn bulk_in(&mut self, want: usize) -> Vec<u8> {
        let mut data = Vec::new();
        while data.len() < want {
            match self.transact(EP_BULK, UsbToken::In, vec![]) {
                UsbResponse::Data(chunk) => {
                    // A short packet ends the data phase (the device may
                    // supply less than the host asked for - its INQUIRY
                    // response, for instance, is a canned 11 bytes).
                    let short = chunk.len() < BULK_MAX;
                    data.extend_from_slice(&chunk);
                    if short {
                        break;
                    }
                }
                other => panic!("bulk IN: unexpected {other:?}"),
            }
        }
        data
    }

    /// One SCSI command over Bulk-Only Transport: CBW -> optional data
    /// phase -> CSW (validated). Returns the data-in bytes.
    fn scsi(&mut self, cdb: &[u8], data_in_len: usize, data_out: Option<&[u8]>) -> Vec<u8> {
        let (dir_in, dlen) = match data_out {
            Some(out) => (false, out.len() as u32),
            None => (true, data_in_len as u32),
        };
        self.bulk_out(&build_cbw(0xC0DE_0000, dlen, dir_in, cdb));

        let data_in = match data_out {
            Some(out) => {
                self.bulk_out(out);
                Vec::new()
            }
            None => self.bulk_in(data_in_len),
        };

        let csw = self.bulk_in(13);
        assert_eq!(csw.len(), 13, "CSW length");
        assert_eq!(&csw[0..4], b"USBS", "CSW signature");
        assert_eq!(csw[12], 0, "CSW status");
        data_in
    }

    /// Send a tunneled-protocol command to the command interface
    /// (sector 0x31), padded to a full sector.
    fn send_command(&mut self, cmd: &[u8]) {
        let mut sector = vec![0u8; SECTOR_SIZE];
        sector[..cmd.len()].copy_from_slice(cmd);
        self.scsi(&scsi_write10(SECTOR_SCSI_WRITE, 1), 0, Some(&sector));
    }

    /// Read one 0x1000-byte flash page via the tunneled flash protocol.
    fn read_page(&mut self, page: u32) -> Vec<u8> {
        self.send_command(&[0x80]); // initiator
        let mut cmd = vec![0x28]; // read
        cmd.extend_from_slice(&page.to_be_bytes());
        self.send_command(&cmd);

        // Response = 4-byte big-endian length + page data, read rounded up
        // to whole sectors from the data-read interface (sector 0x58).
        let want = 4 + PAGE_SIZE;
        let blocks = (want as u32).div_ceil(SECTOR_SIZE as u32);
        let resp = self.scsi(
            &scsi_read10(SECTOR_DATA_READ, blocks as u16),
            blocks as usize * SECTOR_SIZE,
            None,
        );
        self.send_command(&[0x81]); // terminator

        assert!(
            resp.len() >= 4 + PAGE_SIZE,
            "short page response ({} bytes)",
            resp.len()
        );
        resp[4..4 + PAGE_SIZE].to_vec()
    }
}

fn build_cbw(tag: u32, data_len: u32, dir_in: bool, cdb: &[u8]) -> Vec<u8> {
    let mut cbw = Vec::with_capacity(31);
    cbw.extend_from_slice(b"USBC");
    cbw.extend_from_slice(&tag.to_le_bytes());
    cbw.extend_from_slice(&data_len.to_le_bytes());
    cbw.push(if dir_in { 0x80 } else { 0x00 });
    cbw.push(0x00); // LUN
    cbw.push(cdb.len() as u8);
    cbw.extend_from_slice(cdb);
    cbw.resize(31, 0);
    cbw
}

fn scsi_read10(lba: u32, blocks: u16) -> Vec<u8> {
    let l = lba.to_be_bytes();
    let b = blocks.to_be_bytes();
    vec![0x28, 0, l[0], l[1], l[2], l[3], 0, b[0], b[1], 0]
}

fn scsi_write10(lba: u32, blocks: u16) -> Vec<u8> {
    let l = lba.to_be_bytes();
    let b = blocks.to_be_bytes();
    vec![0x2A, 0, l[0], l[1], l[2], l[3], 0, b[0], b[1], 0]
}

/// Cycle budget for the boot ROM to reach connect mode and bring up the
/// SIE (observed around 30M cycles; generous margin).
const USB_BRINGUP_BUDGET: u64 = 500_000_000;

#[test]
fn usb_enumeration_and_flash_read_end_to_end() {
    let Some((otp, flash)) = load_firmware() else {
        eprintln!("firmware images not present; skipping");
        return;
    };

    let (port, internal) = channel_pair();
    let handheld = Handheld::new(
        &otp,
        &flash,
        Box::new(NullScreen),
        Box::new(ConnectModeGpio),
        Box::new(NullAudio),
        Box::new(DisconnectedIr),
        Box::new(internal),
    )
    .expect("handheld construction");

    // The cable is plugged from the start; the connect-mode ROM polls the
    // USBCON connect-status bit to decide a host is really there.
    port.set_connected(true);

    let mut host = Host { handheld, port };
    while !usb_enabled(&mut host.handheld) {
        assert!(
            host.handheld.mcu.core.cycles < USB_BRINGUP_BUDGET,
            "firmware never enabled USB (USBEN) with a host attached"
        );
        step_cycles(&mut host.handheld, 100_000);
    }
    // Let the bus reset and the firmware's SIE setup settle.
    step_cycles(&mut host.handheld, 1_000_000);

    // Enumeration: the standard device descriptor, served by the firmware.
    let descriptor = host.control_in(0x80, 0x06, 0x0100, 0x0000, 18);
    assert_eq!(descriptor.len(), 18, "device descriptor length");
    assert_eq!(descriptor[0], 18, "bLength");
    assert_eq!(descriptor[1], 1, "bDescriptorType (device)");

    // SCSI INQUIRY over Bulk-Only Transport (bulk path both directions).
    // The boot ROM's response is a canned 11 bytes ending in "MGA".
    let inquiry = host.scsi(&[0x12, 0, 0, 0, 36, 0], 36, None);
    assert_eq!(
        inquiry,
        [0x00, 0x80, 0x00, 0x01, 0x1F, 0x00, 0x00, 0x00, b'M', b'G', b'A'],
        "INQUIRY response"
    );

    // The tunneled flash protocol returns the machine's actual flash.
    let expected = host.handheld.make_flash_dump()[..PAGE_SIZE].to_vec();
    let page = host.read_page(0);
    assert_eq!(page, expected, "tunneled page 0 differs from flash");
}
