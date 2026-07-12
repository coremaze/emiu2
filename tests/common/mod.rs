//! Shared harness for the firmware-backed USB integration tests: null
//! peripheral stubs, machine-state probes (USBEN, PRR), and a host-side USB
//! driver speaking enumeration, SCSI over Bulk-Only Transport, and the
//! tunneled flash protocol, with "wait" replaced by stepping the machine
//! (host and device share one thread).

// Each test target compiles this module independently and uses a different
// subset of it.
#![allow(dead_code)]

use emiu2::audio::AudioInterface;
use emiu2::memory::AddressSpace;
use emiu2::miuchiz::{Handheld, UsbResponse, UsbToken, UsbTransaction};
use emiu2::screen::{Pixel, Screen};
use emiu2::usb_interface::UsbHostPort;
use std::path::PathBuf;

pub struct NullScreen;

impl Screen for NullScreen {
    fn set_pixels(&self, _pixels: &[Pixel]) {}
}

pub struct NullAudio;

impl AudioInterface for NullAudio {
    fn set_clock_rate(&mut self, _emulated_clock_rate: u64) {}

    fn needs_sample(&self, _current_cycle: u64) -> bool {
        false
    }

    fn add_sample(&mut self, _value: f32) {}

    fn next_sample_cycle(&self) -> u64 {
        u64::MAX
    }
}

/// The repo's (non-distributable) firmware directory.
pub fn firmware_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("firmware")
}

/// ST2205U USBCON register: bit 7 is USBEN, the firmware-side "SIE is up"
/// switch. Reading it has no side effects.
pub const USBCON: usize = 0x0070;

pub fn usb_enabled(h: &mut Handheld) -> bool {
    h.mcu.core.address_space.read_u8(USBCON) & 0x80 != 0
}

/// ST2205U PRR register (banks the $4000-$7FFF code window). The boot ROM
/// runs with PRR = 0; the application firmware lives in PRR segment $0202
/// (flash offset $8000). Undefined PRRH bits read as 1, so mask before
/// comparing.
pub const PRRL: usize = 0x0032;
pub const PRRH: usize = 0x0033;
pub const PRRH_MASK: u8 = 0x8F;

/// The application firmware's program bank (see `boot_application` in the
/// OTP disassembly: far jump to $4000 in PRR segment $0202).
pub const PRR_APPLICATION: u16 = 0x0202;

pub fn prr(h: &mut Handheld) -> u16 {
    let lo = h.mcu.core.address_space.read_u8(PRRL);
    let hi = h.mcu.core.address_space.read_u8(PRRH) & PRRH_MASK;
    u16::from_le_bytes([lo, hi])
}

pub fn step_cycles(h: &mut Handheld, cycles: u64) {
    let target = h.mcu.core.cycles + cycles;
    while h.mcu.core.cycles < target {
        h.mcu.step();
    }
}

// ---------------------------------------------------------------------------
// Host-side USB driver.
// ---------------------------------------------------------------------------

pub const EP_CONTROL: u8 = 0;
pub const EP_BULK: u8 = 1;
pub const EP0_MAX: usize = 8;
pub const BULK_MAX: usize = 64;
pub const SECTOR_SIZE: usize = 512;
pub const PAGE_SIZE: usize = 0x1000;
pub const SECTOR_SCSI_WRITE: u32 = 0x31;
pub const SECTOR_DATA_READ: u32 = 0x58;

/// Cycles the device may spend before answering a submitted transaction at
/// all (it services one per ~16k-cycle interval).
pub const RESPONSE_CYCLE_BUDGET: u64 = 50_000_000;
/// NAK retry budget per transaction (each retry runs the machine further).
pub const NAK_LIMIT: u32 = 20_000;

pub struct Host {
    pub handheld: Handheld,
    pub port: UsbHostPort,
}

impl Host {
    pub fn transact(&mut self, endpoint: u8, token: UsbToken, data: Vec<u8>) -> UsbResponse {
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
                    "device stopped answering transactions ({txn:?}), pc={:04X}",
                    self.handheld.mcu.core.registers.pc
                );
            };
            match resp {
                UsbResponse::Nak => {
                    naks += 1;
                    assert!(
                        naks < NAK_LIMIT,
                        "NAK retry budget exhausted ({txn:?}), pc={:04X}",
                        self.handheld.mcu.core.registers.pc
                    );
                    // Give the firmware time to service its buffers.
                    step_cycles(&mut self.handheld, 20_000);
                }
                other => return other,
            }
        }
    }

    pub fn expect_ack(&mut self, endpoint: u8, token: UsbToken, data: Vec<u8>, what: &str) {
        match self.transact(endpoint, token, data) {
            UsbResponse::Ack => {}
            other => panic!("{what}: expected ACK, got {other:?}"),
        }
    }

    pub fn control_in(
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

    pub fn bulk_out(&mut self, data: &[u8]) {
        for chunk in data.chunks(BULK_MAX) {
            self.expect_ack(EP_BULK, UsbToken::Out, chunk.to_vec(), "bulk OUT");
        }
    }

    pub fn bulk_in(&mut self, want: usize) -> Vec<u8> {
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
    pub fn scsi(&mut self, cdb: &[u8], data_in_len: usize, data_out: Option<&[u8]>) -> Vec<u8> {
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
    pub fn send_command(&mut self, cmd: &[u8]) {
        let mut sector = vec![0u8; SECTOR_SIZE];
        sector[..cmd.len()].copy_from_slice(cmd);
        self.scsi(&scsi_write10(SECTOR_SCSI_WRITE, 1), 0, Some(&sector));
    }

    /// Read one 0x1000-byte flash page via the tunneled flash protocol.
    pub fn read_page(&mut self, page: u32) -> Vec<u8> {
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

    /// The eject: ask to read the disconnect block (page 0x200). The
    /// command sector is accepted in full (the ISR only queues the command
    /// once all 8 packets arrived), but the ROM parses it - and drops off
    /// the bus - before the host collects the CSW, so that read may fail;
    /// eject.c ignores everything from this point on.
    pub fn eject(&mut self) {
        self.send_command(&[0x80]); // initiator
        let mut cmd = vec![0x28]; // read page 0x200
        cmd.extend_from_slice(&0x200u32.to_be_bytes());
        let mut sector = vec![0u8; SECTOR_SIZE];
        sector[..cmd.len()].copy_from_slice(&cmd);
        self.bulk_out(&build_cbw(
            0xC0DE_0000,
            SECTOR_SIZE as u32,
            false,
            &scsi_write10(SECTOR_SCSI_WRITE, 1),
        ));
        self.bulk_out(&sector);
        match self.transact(EP_BULK, UsbToken::In, vec![]) {
            UsbResponse::Data(_) | UsbResponse::Detached => {}
            other => panic!("eject CSW: unexpected {other:?}"),
        }
    }
}

pub fn build_cbw(tag: u32, data_len: u32, dir_in: bool, cdb: &[u8]) -> Vec<u8> {
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

pub fn scsi_read10(lba: u32, blocks: u16) -> Vec<u8> {
    let l = lba.to_be_bytes();
    let b = blocks.to_be_bytes();
    vec![0x28, 0, l[0], l[1], l[2], l[3], 0, b[0], b[1], 0]
}

pub fn scsi_write10(lba: u32, blocks: u16) -> Vec<u8> {
    let l = lba.to_be_bytes();
    let b = blocks.to_be_bytes();
    vec![0x2A, 0, l[0], l[1], l[2], l[3], 0, b[0], b[1], 0]
}
