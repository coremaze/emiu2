//! Diagnostic USB client for the emulator's `--usb-socket` bridge.
//!
//! This is a DEBUGGING TOOL, not a product: it exists to exercise the USB seam
//! end to end and show exactly what crosses the wire. It connects over TCP and
//! drives every layer itself - raw transactions, control transfers, USB Mass
//! Storage Bulk-Only Transport, standard SCSI, and the device's tunneled
//! flash read/write protocol - logging each transaction and response.
//!
//! Usage:
//!   1. Run the emulator with the bridge enabled (device must be in its USB
//!      mass-storage mode - a normal boot from a real flash image, or the boot
//!      ROM's connect mode with the D-pad held):
//!        emiu2 OTP.dat flash.bin --usb-socket 127.0.0.1:3240
//!   2. Run this client:
//!        cargo run --bin usb_client -- 127.0.0.1:3240 [page_hex] [--write]
//!      `--write` additionally exercises the (destructive) flash write+verify
//!      path at the given page (default 0); without it the run is read-only.
//!
//! Every transaction is printed: `->` is host->device, `<-` is the device's
//! response. NAKs (firmware not ready yet) are retried and summarised, not spammed.

use std::error::Error;
use std::thread::sleep;
use std::time::Duration;

use emiu2::miuchiz::{UsbResponse, UsbToken, UsbTransaction};
use emiu2::usb_socket::RemoteUsbDevice;

type R<T> = Result<T, Box<dyn Error>>;

const EP_CONTROL: u8 = 0;
const EP_BULK: u8 = 1;
const EP0_MAX: usize = 8;
const BULK_MAX: usize = 64;

// Tunneled flash protocol constants (from the reference libmiuchiz-usb).
const SECTOR_SIZE: usize = 512;
const PAGE_SIZE: usize = 0x1000; // 4096-byte flash page (8 sectors)
const SECTOR_SCSI_WRITE: u32 = 0x31; // command interface
const SECTOR_DATA_READ: u32 = 0x58; // read-response interface
const SECTOR_DATA_WRITE: u32 = 0x33; // write-data interface

/// Per-transaction NAK retry budget (each retry waits NAK_WAIT).
const NAK_RETRIES: u32 = 200_000;
const NAK_WAIT: Duration = Duration::from_micros(50);

// ---------------------------------------------------------------------------
// Transport: raw transactions with logging + NAK retry.
// ---------------------------------------------------------------------------

struct Bus {
    dev: RemoteUsbDevice,
    seq: u64,
}

impl Bus {
    fn new(dev: RemoteUsbDevice) -> Self {
        Self { dev, seq: 0 }
    }

    /// Submit one transaction, retrying while the device NAKs, and log it.
    fn transact(&mut self, txn: &UsbTransaction) -> R<UsbResponse> {
        self.seq += 1;
        let tok = match txn.token {
            UsbToken::Setup => "SETUP",
            UsbToken::In => "IN",
            UsbToken::Out => "OUT",
        };
        print!(
            "[{:>4}] -> EP{} {:<5} {:>3}B",
            self.seq,
            txn.endpoint,
            tok,
            txn.data.len()
        );
        if !txn.data.is_empty() {
            print!("  {}", hex_inline(&txn.data));
        }
        println!();

        let mut naks = 0u32;
        loop {
            match self.dev.transaction(txn)? {
                UsbResponse::Nak => {
                    naks += 1;
                    if naks > NAK_RETRIES {
                        println!("       <- NAK (gave up after {naks})");
                        return Ok(UsbResponse::Nak);
                    }
                    sleep(NAK_WAIT);
                }
                resp => {
                    let suffix = if naks > 0 {
                        format!("  (after {naks} NAK)")
                    } else {
                        String::new()
                    };
                    match &resp {
                        UsbResponse::Ack => println!("       <- ACK{suffix}"),
                        UsbResponse::Stall => println!("       <- STALL{suffix}"),
                        UsbResponse::Data(d) => {
                            println!("       <- DATA {:>3}B{suffix}  {}", d.len(), hex_inline(d))
                        }
                        UsbResponse::Nak => unreachable!(),
                    }
                    return Ok(resp);
                }
            }
        }
    }

    // --- control transfers --------------------------------------------------

    fn control_in(
        &mut self,
        bm_request_type: u8,
        b_request: u8,
        w_value: u16,
        w_index: u16,
        length: u16,
    ) -> R<Vec<u8>> {
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
        expect_ack(
            self.transact(&txn(EP_CONTROL, UsbToken::Setup, setup))?,
            "control SETUP",
        )?;

        let mut data = Vec::new();
        while data.len() < length as usize {
            match self.transact(&txn(EP_CONTROL, UsbToken::In, vec![]))? {
                UsbResponse::Data(chunk) => {
                    let short = chunk.len() < EP0_MAX;
                    data.extend_from_slice(&chunk);
                    if short {
                        break;
                    }
                }
                other => return Err(format!("control IN: unexpected {other:?}").into()),
            }
        }
        // Status stage: zero-length OUT.
        expect_ack(
            self.transact(&txn(EP_CONTROL, UsbToken::Out, vec![]))?,
            "control status",
        )?;
        Ok(data)
    }

    // --- bulk transfers -----------------------------------------------------

    fn bulk_out(&mut self, data: &[u8]) -> R<()> {
        for chunk in data.chunks(BULK_MAX) {
            expect_ack(
                self.transact(&txn(EP_BULK, UsbToken::Out, chunk.to_vec()))?,
                "bulk OUT",
            )?;
        }
        Ok(())
    }

    fn bulk_in(&mut self, want: usize) -> R<Vec<u8>> {
        let mut data = Vec::new();
        while data.len() < want {
            match self.transact(&txn(EP_BULK, UsbToken::In, vec![]))? {
                UsbResponse::Data(chunk) => {
                    let short = chunk.len() < BULK_MAX;
                    data.extend_from_slice(&chunk);
                    if short {
                        break;
                    }
                }
                other => return Err(format!("bulk IN: unexpected {other:?}").into()),
            }
        }
        Ok(data)
    }

    // --- USB Mass Storage Bulk-Only Transport -------------------------------

    /// Run one SCSI command: CBW -> optional data phase -> CSW. Returns the
    /// data-in bytes (empty for non-IN commands) and validates the CSW.
    fn scsi(&mut self, cdb: &[u8], data: ScsiData) -> R<Vec<u8>> {
        let (dir_in, dlen) = match &data {
            ScsiData::In(n) => (true, *n as u32),
            ScsiData::Out(d) => (false, d.len() as u32),
            ScsiData::None => (false, 0),
        };
        self.bulk_out(&build_cbw(0xC0DE_0000 | self.seq as u32, dlen, dir_in, cdb))?;

        let data_in = match data {
            ScsiData::In(n) => self.bulk_in(n)?,
            ScsiData::Out(d) => {
                self.bulk_out(&d)?;
                Vec::new()
            }
            ScsiData::None => Vec::new(),
        };

        let csw = self.bulk_in(13)?;
        check_csw(&csw)?;
        Ok(data_in)
    }

    // --- the device's tunneled flash protocol -------------------------------
    //
    // Reverse-engineered from the reference libmiuchiz-usb. Flash is addressed in
    // 0x1000-byte PAGES (0x200 of them = 2 MB). Each operation is bracketed by an
    // initiator (0x80) and terminator (0x81) command, and every command goes to
    // the command interface (sector 0x31) as a full 512-byte sector. Read data
    // comes back from sector 0x58 (prefixed with a 4-byte big-endian length);
    // write data goes to sector 0x33.

    /// Send a protocol command to the command interface (sector 0x31), padded to
    /// a full sector.
    fn send_command(&mut self, cmd: &[u8]) -> R<()> {
        let mut sector = vec![0u8; SECTOR_SIZE];
        sector[..cmd.len()].copy_from_slice(cmd);
        self.scsi(&scsi_write10(SECTOR_SCSI_WRITE, 1), ScsiData::Out(sector))?;
        Ok(())
    }

    /// Read one 0x1000-byte flash page via the tunneled protocol.
    fn read_page(&mut self, page: u32) -> R<Vec<u8>> {
        self.send_command(&[0x80])?; // initiator
        let mut cmd = vec![0x28]; // read
        cmd.extend_from_slice(&page.to_be_bytes());
        println!("  read page {page:#06x} (cmd: opcode=0x28 page={page})");
        self.send_command(&cmd)?;

        // Response = 4-byte big-endian length + page data; read rounded up to
        // whole sectors from the data-read interface (sector 0x58).
        let want = 4 + PAGE_SIZE;
        let blocks = (want as u32).div_ceil(SECTOR_SIZE as u32);
        let resp = self.scsi(
            &scsi_read10(SECTOR_DATA_READ, blocks as u16),
            ScsiData::In(blocks as usize * SECTOR_SIZE),
        )?;
        self.send_command(&[0x81])?; // terminator

        if resp.len() < 4 + PAGE_SIZE {
            return Err(format!("short page response ({} bytes)", resp.len()).into());
        }
        let prefix_len = u32::from_be_bytes([resp[0], resp[1], resp[2], resp[3]]);
        println!("  response length prefix = {prefix_len}");
        Ok(resp[4..4 + PAGE_SIZE].to_vec())
    }

    /// Write one 0x1000-byte flash page via the tunneled protocol.
    fn write_page(&mut self, page: u32, data: &[u8]) -> R<()> {
        if data.len() != PAGE_SIZE {
            return Err(format!("write_page needs exactly {PAGE_SIZE} bytes").into());
        }
        self.send_command(&[0x80])?; // initiator
        let mut cmd = vec![0x2A]; // write
        cmd.extend_from_slice(&page.to_be_bytes());
        cmd.extend_from_slice(&(PAGE_SIZE as u32).to_be_bytes());
        println!("  write page {page:#06x} (cmd: opcode=0x2A page={page} size={PAGE_SIZE})");
        self.send_command(&cmd)?;

        // Data to the data-write interface (sector 0x33): one full page.
        let blocks = (PAGE_SIZE as u32).div_ceil(SECTOR_SIZE as u32);
        self.scsi(
            &scsi_write10(SECTOR_DATA_WRITE, blocks as u16),
            ScsiData::Out(data.to_vec()),
        )?;
        self.send_command(&[0x81])?; // terminator
        Ok(())
    }
}

enum ScsiData {
    In(usize),
    Out(Vec<u8>),
    None,
}

// ---------------------------------------------------------------------------
// Diagnostic sequence.
// ---------------------------------------------------------------------------

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    // `--write` opts in to the (destructive) flash write+verify exercise.
    let do_write = raw.iter().any(|a| a == "--write");
    let mut positional: Vec<&String> = raw.iter().filter(|a| !a.starts_with("--")).collect();
    // An explicit TCP address is recognizable by its colon; without one, the
    // emulator is found through endpoint discovery.
    let addr = match positional.first() {
        Some(first) if first.contains(':') => Some(positional.remove(0).to_string()),
        _ => None,
    };
    let flash_page = positional
        .first()
        .and_then(|s| parse_u32(s).ok())
        .unwrap_or(0x0000);

    let dev = match &addr {
        Some(addr) => {
            println!("Connecting to USB bridge at {addr} ...");
            match RemoteUsbDevice::connect(addr) {
                Ok(dev) => dev,
                Err(why) => {
                    eprintln!(
                        "Could not connect: {why}\nIs the emulator running with --usb-socket {addr}?"
                    );
                    std::process::exit(1);
                }
            }
        }
        None => {
            let found = emiu2::usb_socket::discover();
            let Some(endpoint) = found.first() else {
                eprintln!(
                    "No running emulators discovered. Start emiu2 (ideally with \
                     --connect-mode), or pass an explicit host:port."
                );
                std::process::exit(1);
            };
            println!(
                "Discovered emulator \"{}\" at {}",
                endpoint.identity,
                endpoint.path.display()
            );
            match RemoteUsbDevice::connect_endpoint(&endpoint.path) {
                Ok(dev) => dev,
                Err(why) => {
                    eprintln!("Could not connect: {why}");
                    std::process::exit(1);
                }
            }
        }
    };
    println!("Connected to \"{}\"", dev.identity());
    let mut bus = Bus::new(dev);

    run_step("ENUMERATE: device descriptor", || {
        enumerate_device(&mut bus)
    });
    run_step("ENUMERATE: configuration descriptor", || {
        enumerate_config(&mut bus)
    });
    run_step("ENUMERATE: string descriptors", || {
        enumerate_strings(&mut bus)
    });
    run_step("MASS STORAGE: GET MAX LUN", || get_max_lun(&mut bus));
    run_step("SCSI: INQUIRY", || scsi_inquiry(&mut bus));
    run_step("SCSI: TEST UNIT READY", || {
        bus.scsi(&[0x00, 0, 0, 0, 0, 0], ScsiData::None)?;
        println!("  unit ready (CSW OK)");
        Ok(())
    });
    run_step("SCSI: READ CAPACITY", || scsi_read_capacity(&mut bus));
    run_step("SCSI: READ(10) sector 0", || {
        let data = bus.scsi(&scsi_read10(0, 1), ScsiData::In(512))?;
        println!("  read {} bytes:", data.len());
        hexdump(&data[..data.len().min(128)]);
        if data.len() > 128 {
            println!("  ... ({} more bytes)", data.len() - 128);
        }
        Ok(())
    });
    run_step("FLASH: tunneled page read", || {
        let data = bus.read_page(flash_page)?;
        println!(
            "  page {flash_page:#06x} = {} bytes (first 128 shown):",
            data.len()
        );
        hexdump(&data[..data.len().min(128)]);
        Ok(())
    });

    if do_write {
        run_step("FLASH: page write + read-back verify (--write)", || {
            // A recognizable, non-erased full-page pattern so a successful program
            // is obvious in the readback.
            let pattern: Vec<u8> = (0..PAGE_SIZE).map(|i| (i as u8) ^ 0xA5).collect();
            println!("  writing {PAGE_SIZE}-byte page {flash_page:#06x} (first 64 shown):");
            hexdump(&pattern[..64]);
            bus.write_page(flash_page, &pattern)?;

            let back = bus.read_page(flash_page)?;
            if back == pattern {
                println!("  WRITE VERIFIED: full {PAGE_SIZE}-byte page matches");
            } else {
                let diff = pattern
                    .iter()
                    .zip(back.iter())
                    .filter(|(a, b)| a != b)
                    .count()
                    + pattern.len().abs_diff(back.len());
                println!("  WRITE NOT VERIFIED: {diff} differing/missing bytes");
                println!("  read back (first 64 shown):");
                hexdump(&back[..back.len().min(64)]);
            }
            Ok(())
        });
    } else {
        println!("\n(skipping flash write exercise; pass --write to enable it)");
    }

    println!("\nDone.");
}

fn run_step(name: &str, f: impl FnOnce() -> R<()>) {
    println!("\n=== {name} ===");
    if let Err(why) = f() {
        println!("  !! step failed: {why}");
    }
}

fn enumerate_device(bus: &mut Bus) -> R<()> {
    let d = bus.control_in(0x80, 0x06, 0x0100, 0x0000, 18)?;
    if d.len() < 18 {
        return Err(format!("short device descriptor ({} bytes)", d.len()).into());
    }
    println!(
        "  bLength={} bcdUSB={:02x}{:02x} bMaxPacketSize0={}",
        d[0], d[3], d[2], d[7]
    );
    println!(
        "  idVendor={:#06x} idProduct={:#06x} bcdDevice={:02x}{:02x}",
        u16::from_le_bytes([d[8], d[9]]),
        u16::from_le_bytes([d[10], d[11]]),
        d[13],
        d[12]
    );
    println!(
        "  iManufacturer={} iProduct={} iSerial={} bNumConfigurations={}",
        d[14], d[15], d[16], d[17]
    );
    Ok(())
}

fn enumerate_config(bus: &mut Bus) -> R<()> {
    // First 4 bytes give wTotalLength; then fetch the whole thing.
    let head = bus.control_in(0x80, 0x06, 0x0200, 0x0000, 9)?;
    if head.len() < 4 {
        return Err("short config descriptor header".into());
    }
    let total = u16::from_le_bytes([head[2], head[3]]);
    let full = bus.control_in(0x80, 0x06, 0x0200, 0x0000, total)?;
    println!("  wTotalLength={total}, fetched {} bytes:", full.len());
    hexdump(&full);
    Ok(())
}

fn enumerate_strings(bus: &mut Bus) -> R<()> {
    for index in 1..=3u16 {
        match bus.control_in(0x80, 0x06, 0x0300 | index, 0x0409, 255) {
            Ok(s) if s.len() >= 2 => println!("  string[{index}] = {:?}", decode_usb_string(&s)),
            Ok(_) => println!("  string[{index}] = (empty)"),
            Err(why) => println!("  string[{index}] failed: {why}"),
        }
    }
    Ok(())
}

fn get_max_lun(bus: &mut Bus) -> R<()> {
    // Class request, device-to-host, interface: bmRequestType=0xA1, bRequest=0xFE.
    match bus.control_in(0xA1, 0xFE, 0x0000, 0x0000, 1) {
        Ok(v) if !v.is_empty() => println!("  max LUN = {}", v[0]),
        Ok(_) => println!("  (no data)"),
        Err(why) => println!("  GET MAX LUN not supported: {why}"),
    }
    Ok(())
}

fn scsi_inquiry(bus: &mut Bus) -> R<()> {
    let d = bus.scsi(&[0x12, 0, 0, 0, 36, 0], ScsiData::In(36))?;
    if d.len() >= 36 {
        let vendor = String::from_utf8_lossy(&d[8..16]);
        let product = String::from_utf8_lossy(&d[16..32]);
        let rev = String::from_utf8_lossy(&d[32..36]);
        println!("  vendor={vendor:?} product={product:?} rev={rev:?}");
    } else {
        println!("  short INQUIRY ({} bytes)", d.len());
        hexdump(&d);
    }
    Ok(())
}

fn scsi_read_capacity(bus: &mut Bus) -> R<()> {
    let d = bus.scsi(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], ScsiData::In(8))?;
    if d.len() >= 8 {
        let last_lba = u32::from_be_bytes([d[0], d[1], d[2], d[3]]);
        let block_len = u32::from_be_bytes([d[4], d[5], d[6], d[7]]);
        let blocks = last_lba as u64 + 1;
        println!(
            "  last LBA={last_lba} block size={block_len}  => {} blocks, {} bytes",
            blocks,
            blocks * block_len as u64
        );
    } else {
        hexdump(&d);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

fn txn(endpoint: u8, token: UsbToken, data: Vec<u8>) -> UsbTransaction {
    UsbTransaction {
        endpoint,
        token,
        data,
    }
}

fn expect_ack(resp: UsbResponse, what: &str) -> R<()> {
    match resp {
        UsbResponse::Ack => Ok(()),
        other => Err(format!("{what}: expected ACK, got {other:?}").into()),
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

fn check_csw(csw: &[u8]) -> R<()> {
    if csw.len() != 13 {
        return Err(format!("CSW length {} (expected 13)", csw.len()).into());
    }
    if &csw[0..4] != b"USBS" {
        return Err(format!("bad CSW signature {}", hex_inline(&csw[0..4])).into());
    }
    let residue = u32::from_le_bytes([csw[8], csw[9], csw[10], csw[11]]);
    match csw[12] {
        0 => Ok(()),
        status => Err(format!("CSW status {status:#x} (residue {residue})").into()),
    }
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

fn decode_usb_string(desc: &[u8]) -> String {
    // desc[0]=bLength, desc[1]=type(3), then UTF-16LE.
    let units: Vec<u16> = desc[2..]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

fn parse_u32(s: &str) -> R<u32> {
    let s = s.trim();
    let v = if let Some(hex) = s.strip_prefix("0x") {
        u32::from_str_radix(hex, 16)?
    } else {
        s.parse::<u32>()?
    };
    Ok(v)
}

fn hex_inline(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn hexdump(data: &[u8]) {
    for (i, row) in data.chunks(16).enumerate() {
        let hex: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
        let ascii: String = row
            .iter()
            .map(|&b| {
                if (0x20..0x7f).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        println!("  {:04x}  {:<47}  {}", i * 16, hex.join(" "), ascii);
    }
}
