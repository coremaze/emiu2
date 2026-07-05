// ---------------------------------------------------------------------------
// Transaction-accurate seam (the "USB cable"). The host issues one
// `UsbTransaction` at a time and the device answers with one `UsbResponse`.
// This synchronous boundary is the whole device/host interface.
// ---------------------------------------------------------------------------

// Internal-RAM addresses of the four USB endpoint buffers. When buffer access is
// enabled the SIE overlays these onto internal RAM; this module owns the layout,
// and the address-space router (`addr_space.rs`) uses the combined
// `BUFFER_START..=BUFFER_END` range to decide when to route here. One source of
// truth - keep the dispatch in `read_buffer`/`write_buffer` matched to these.
const BKO_START: u16 = 0x0200; // bulk-OUT (host -> device), 64 bytes
const BKO_END: u16 = 0x023F;
const BKI_START: u16 = 0x0240; // bulk-IN (device -> host), 64 bytes
const BKI_END: u16 = 0x027F;
const EP0_OUT_START: u16 = 0x0280; // control endpoint 0 OUT, 8 bytes
const EP0_OUT_END: u16 = 0x0287;
const EP0_IN_START: u16 = 0x0288; // control endpoint 0 IN, 8 bytes
const EP0_IN_END: u16 = 0x028F;
/// Combined buffer overlay range, used by the address-space router.
pub const BUFFER_START: u16 = BKO_START;
pub const BUFFER_END: u16 = EP0_IN_END;

/// USB packet identifier (token) - what kind of transaction the host issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbToken {
    /// 8-byte control SETUP packet (EP0 only).
    Setup,
    /// Host requests data from the device (device -> host).
    In,
    /// Host sends data to the device (host -> device).
    Out,
}

/// A single USB transaction issued by the host against one endpoint.
#[derive(Debug, Clone)]
pub struct UsbTransaction {
    /// Endpoint number: 0 = control (EP0), 1 = bulk. This chip has no others.
    pub endpoint: u8,
    pub token: UsbToken,
    /// Payload for `Setup`/`Out`; ignored for `In`.
    pub data: Vec<u8>,
}

/// The device's response to a transaction (the USB handshake + any IN data).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbResponse {
    /// SETUP/OUT accepted into the buffer.
    Ack,
    /// Not ready - firmware hasn't serviced the buffer yet; the host should retry.
    Nak,
    /// Endpoint halted, or the request is unsupported.
    Stall,
    /// IN data returned (0..=maxpacket bytes). An empty vec is a real
    /// zero-length packet (a transfer boundary), distinct from `Nak`.
    Data(Vec<u8>),
}

/// Endpoint numbers used in `UsbTransaction`.
const ENDPOINT_CONTROL: u8 = 0;
const ENDPOINT_BULK: u8 = 1;

/// Maximum packet sizes for this chip's endpoints.
const EP0_MAX_PACKET: usize = 8;

/// What the SIE decided to do with a SETUP packet on EP0.
enum SetupDisposition {
    /// Hand the request to the firmware via the EP0 OUT buffer, with this DRQ
    /// value pre-decoded into EP0CON (the firmware dispatches on it).
    Forward(u8),
    /// A no-data standard request the SIE completes itself (SET_ADDRESS,
    /// SET_CONFIGURATION, ...). Auto-ACK and answer the status stage.
    AutoAck,
    /// A standard data-IN request the SIE answers with constant bytes
    /// (GET_STATUS, GET_CONFIGURATION, ...).
    AutoIn(Vec<u8>),
}

/// EP0 control-transfer progress, so the SIE knows how to answer the IN/OUT
/// transactions that follow a SETUP.
enum Ep0Stage {
    /// No transfer in progress.
    Idle,
    /// A request was forwarded to firmware; IN/OUT are serviced via the
    /// firmware-armed buffers.
    Firmware,
    /// SIE-handled no-data request awaiting its status-stage IN (-> ZLP).
    AutoStatus,
    /// SIE-handled data-IN request; serve these bytes, then a status OUT.
    AutoDataIn(Vec<u8>),
}

/// Classify a SETUP packet: which requests the firmware sees vs. which the SIE
/// auto-handles. Confirmed against the boot ROM + datasheet.
/// The firmware only handles GET_DESCRIPTOR and class/vendor
/// requests; the SIE completes SET_ADDRESS / SET_CONFIGURATION / etc. itself.
fn classify_setup(setup: &[u8]) -> SetupDisposition {
    let bm_request_type = setup[0];
    let b_request = setup[1];
    let req_type = (bm_request_type >> 5) & 0x03; // 0 = standard, 1 = class, 2 = vendor

    if req_type != 0 {
        // Class/vendor requests go to the firmware as "non-standard" (DRQ 0b11).
        return SetupDisposition::Forward(0b11);
    }

    match b_request {
        0x06 => SetupDisposition::Forward(decode_drq(setup)), // GET_DESCRIPTOR
        // No-data standard "set" requests: completed by the SIE.
        0x05 // SET_ADDRESS
        | 0x09 // SET_CONFIGURATION
        | 0x03 // SET_FEATURE
        | 0x01 // CLEAR_FEATURE
        | 0x0B => SetupDisposition::AutoAck, // SET_INTERFACE
        // Data-IN standard requests answered with constants.
        0x00 => SetupDisposition::AutoIn(vec![0x00, 0x00]), // GET_STATUS [inferred]
        0x08 => SetupDisposition::AutoIn(vec![0x01]),       // GET_CONFIGURATION [inferred]
        0x0A => SetupDisposition::AutoIn(vec![0x00]),       // GET_INTERFACE [inferred]
        // Anything else: let the firmware decide (it will likely STALL it).
        _ => SetupDisposition::Forward(0b11),
    }
}

/// Decode the DRQ[1:0] field the hardware presents in EP0CON for a
/// GET_DESCRIPTOR request, keyed on the descriptor type (wValue high byte).
/// device=0b00, config=0b01, string=0b10, anything else=0b11 (datasheet
/// §18.2.5; this is the mapping the firmware's dispatch table expects).
fn decode_drq(setup: &[u8]) -> u8 {
    match setup.get(3) {
        Some(0x01) => 0b00, // DEVICE
        Some(0x02) => 0b01, // CONFIGURATION
        Some(0x03) => 0b10, // STRING
        _ => 0b11,          // non-standard
    }
}

pub struct UsbCon {
    usben: bool,
    pllrdy: bool,
    pllen: bool,
    pll1: bool,
    pll0: bool,
    rwake: bool,
    pull: bool,
}

impl UsbCon {
    pub fn new() -> Self {
        Self {
            usben: false,
            pllrdy: false,
            pllen: false,
            pll1: false,
            pll0: false,
            rwake: false,
            pull: false,
        }
    }

    pub fn read_u8(&self) -> u8 {
        (self.usben as u8) << 7
            | (self.pllrdy as u8) << 6  // PLLRDY is bit 6 when reading
            | (self.pllen as u8) << 5
            | (self.pll0 as u8) << 4
            | (self.rwake as u8) << 3
            | (self.pull as u8) << 2
    }

    pub fn write_u8(&mut self, value: u8) {
        self.usben = value & (1 << 7) != 0;
        // For writing, bit 6 is PLLEN (not PLLRDY which is read-only)
        self.pllen = value & (1 << 5) != 0 || value & (1 << 6) != 0;
        self.pll1 = value & (1 << 5) != 0;
        self.pll0 = value & (1 << 4) != 0;
        self.rwake = value & (1 << 3) != 0;
        self.pull = value & (1 << 2) != 0;

        // PLLRDY is set based on PLLEN status
        if self.pllen {
            self.pllrdy = true;
        } else {
            self.pllrdy = false;
        }
    }
}

pub struct UsbIen {
    bufen: bool,
    brien: bool,
    resien: bool,
    susien: bool,
    bkiien: bool,
    bkoien: bool,
    ep0ien: bool,
}

impl UsbIen {
    pub fn new() -> Self {
        Self {
            bufen: false,
            brien: true,
            resien: false,
            susien: false,
            bkiien: false,
            bkoien: false,
            ep0ien: false,
        }
    }

    pub fn read_u8(&self) -> u8 {
        (self.bufen as u8) << 7
            | (self.brien as u8) << 5
            | (self.resien as u8) << 4
            | (self.susien as u8) << 3
            | (self.bkiien as u8) << 2
            | (self.bkoien as u8) << 1
            | (self.ep0ien as u8) << 0
    }

    pub fn write_u8(&mut self, value: u8) {
        self.bufen = value & (1 << 7) != 0;
        self.brien = value & (1 << 5) != 0;
        self.resien = value & (1 << 4) != 0;
        self.susien = value & (1 << 3) != 0;
        self.bkiien = value & (1 << 2) != 0;
        self.bkoien = value & (1 << 1) != 0;
        self.ep0ien = value & (1 << 0) != 0;
    }
}

pub struct UsbIrq {
    brirq: bool,  // Bus Reset Interrupt
    resirq: bool, // Resume Interrupt
    susirq: bool, // Suspend Interrupt
    bkiirq: bool, // Bulk-IN Interrupt
    bkoirq: bool, // Bulk-OUT Interrupt
    ep0irq: bool, // Endpoint 0 Interrupt
}

impl UsbIrq {
    pub fn new() -> Self {
        Self {
            brirq: false,
            resirq: false,
            susirq: false,
            bkiirq: false,
            bkoirq: false,
            ep0irq: false,
        }
    }

    pub fn read_u8(&self) -> u8 {
        (self.brirq as u8) << 5
            | (self.resirq as u8) << 4
            | (self.susirq as u8) << 3
            | (self.bkiirq as u8) << 2
            | (self.bkoirq as u8) << 1
            | (self.ep0irq as u8) << 0
    }

    pub fn write_u8(&mut self, value: u8) {
        if value & (1 << 5) != 0 {
            self.brirq = false;
        }
        if value & (1 << 4) != 0 {
            self.resirq = false;
        }
        if value & (1 << 3) != 0 {
            self.susirq = false;
        }
        if value & (1 << 2) != 0 {
            self.bkiirq = false;
        }
        if value & (1 << 1) != 0 {
            self.bkoirq = false;
        }
        if value & (1 << 0) != 0 {
            self.ep0irq = false;
        }
    }

    pub fn clear_all(&mut self) {
        self.brirq = false;
        self.resirq = false;
        self.susirq = false;
        self.bkiirq = false;
        self.bkoirq = false;
        self.ep0irq = false;
    }

    pub fn trigger_bus_reset(&mut self) {
        self.clear_all();
        self.brirq = true;
    }

    // The endpoint interrupts are independent per-event sources - set the one
    // bit without disturbing the others (the firmware's ISR reads all pending
    // bits in one pass). Only the bus-event triggers above reset everything.
    pub fn trigger_bulk_in(&mut self) {
        self.bkiirq = true;
    }

    pub fn trigger_bulk_out(&mut self) {
        self.bkoirq = true;
    }

    pub fn trigger_ep0(&mut self) {
        self.ep0irq = true;
    }
}

pub struct UsbBfs {
    read_bki_needs_service: bool, // Bulk-IN Buffer Status (1 = empty, needs service)
    read_bko_needs_service: bool, // Bulk-OUT Buffer Status (1 = full, needs service)
    read_ep0in_needs_service: bool, // EP0 IN Buffer Status (1 = empty, needs service)
    read_ep0out_needs_service: bool, // EP0 OUT Buffer Status (1 = full, needs service)

    write_bki_needs_service: bool, // Bulk-IN Buffer Status (1 = empty, needs service)
    write_bko_needs_service: bool, // Bulk-OUT Buffer Status (1 = full, needs service)
    write_ep0in_needs_service: bool, // EP0 IN Buffer Status (1 = empty, needs service)
    write_ep0out_needs_service: bool, // EP0 OUT Buffer Status (1 = full, needs service)
}

impl UsbBfs {
    pub fn new() -> Self {
        Self {
            read_bki_needs_service: true,     // Initially empty (1)
            read_bko_needs_service: false,    // Initially not full (0)
            read_ep0in_needs_service: true,   // Initially empty (1)
            read_ep0out_needs_service: false, // Initially not full (0)

            write_bki_needs_service: false,    // Initially empty (1)
            write_bko_needs_service: false,    // Initially not full (0)
            write_ep0in_needs_service: false,  // Initially empty (1)
            write_ep0out_needs_service: false, // Initially not full (0)
        }
    }

    pub fn read_u8(&self) -> u8 {
        (self.read_bki_needs_service as u8) << 3
            | (self.read_bko_needs_service as u8) << 2
            | (self.read_ep0in_needs_service as u8) << 1
            | (self.read_ep0out_needs_service as u8) << 0
    }

    pub fn write_u8(&mut self, value: u8) {
        self.write_bki_needs_service = (value >> 3) & 1 != 0;
        self.write_bko_needs_service = (value >> 2) & 1 != 0;
        self.write_ep0in_needs_service = (value >> 1) & 1 != 0;
        self.write_ep0out_needs_service = (value >> 0) & 1 != 0;
    }
}

pub struct Ep0Con {
    stall: bool,  // Endpoint 0 STALL command bit
    flush: bool,  // Endpoint 0 buffer flush command bit (write-only)
    txzero: bool, // Send zero length data command bit (write-only)
    dir: bool,    // Endpoint 0 OUT buffer direction bit (0=OUT, 1=IN) (read-only)
    setup: bool,  // OUT package type flag (0=data, 1=setup) (read-only)
    drq1: bool,   // Descriptor request bit 1 (read-only)
    drq0: bool,   // Descriptor request bit 0 (read-only)
}

impl Ep0Con {
    pub fn new() -> Self {
        Self {
            stall: false,
            flush: false,
            txzero: false,
            dir: false,
            setup: false,
            drq1: false,
            drq0: false,
        }
    }

    pub fn read_u8(&self) -> u8 {
        (self.stall as u8) << 7
            | (self.dir as u8) << 3
            | (self.setup as u8) << 2
            | (self.drq1 as u8) << 1
            | (self.drq0 as u8) << 0
    }

    pub fn write_u8(&mut self, value: u8) {
        self.stall = value & (1 << 7) != 0;
        self.flush = value & (1 << 6) != 0;
        self.txzero = value & (1 << 5) != 0;

        // Writing 1 to flush clears all buffer states
        if self.flush {
            self.dir = false;
            self.setup = false;
            self.drq1 = false;
            self.drq0 = false;
            self.flush = false; // Auto-clear after use
        }
    }

    // Helper methods to set read-only status bits
    pub fn set_dir(&mut self, value: bool) {
        self.dir = value;
    }

    pub fn set_setup(&mut self, value: bool) {
        self.setup = value;
    }

    pub fn set_drq(&mut self, drq_value: u8) {
        self.drq1 = drq_value & 0x02 != 0;
        self.drq0 = drq_value & 0x01 != 0;
    }
}

pub struct BkCon {
    bki_stall: bool,  // Bulk-IN STALL command bit
    bki_flush: bool,  // Bulk-IN buffer flush command bit (write-only)
    bki_txzero: bool, // Bulk-IN send zero length data command bit (write-only)
    bko_stall: bool,  // Bulk-OUT STALL command bit
    bko_flush: bool,  // Bulk-OUT buffer flush command bit (write-only)
}

impl BkCon {
    pub fn new() -> Self {
        Self {
            bki_stall: false,
            bki_flush: false,
            bki_txzero: false,
            bko_stall: false,
            bko_flush: false,
        }
    }

    pub fn read_u8(&self) -> u8 {
        (self.bki_stall as u8) << 7 | (self.bko_stall as u8) << 3
    }

    pub fn write_u8(&mut self, value: u8) {
        self.bki_stall = value & (1 << 7) != 0;
        self.bki_flush = value & (1 << 6) != 0;
        self.bki_txzero = value & (1 << 5) != 0;
        self.bko_stall = value & (1 << 3) != 0;
        self.bko_flush = value & (1 << 2) != 0;
    }
}

// Buffer representation for USB endpoints
pub struct UsbBuffers {
    // The ST2205U uses 144 bytes of dedicated RAM for USB endpoints.
    //
    // Bulk-OUT is **double-buffered** (datasheet §18.1): two 64-byte slots that
    // the SIE fills as packets arrive while the firmware drains the other. The
    // firmware always sees the oldest unread packet at $0200; modeling this as a
    // single buffer makes the firmware's per-packet accounting drift and the host
    // NAK-wait per packet, which compounds into a write deadlock.
    bko_slots: [[u8; 64]; 2], // Bulk-OUT double buffer ($0200-$023F = current slot)
    bko_sizes: [usize; 2],
    bko_front: usize,        // slot currently presented at $0200 (oldest unread)
    bko_count: usize,        // occupied slots, 0..=2
    bki_buffer: [u8; 64],    // Bulk-IN Buffer ($0240-$027F)
    bki_size: usize, // the highest index of the BKI buffer that has been written to (plus 1)
    ep0_out_buffer: [u8; 8], // EP0 OUT Buffer ($0280-$0287)
    ep0_in_buffer: [u8; 8], // EP0 IN Buffer ($0288-$028F)
    // Buffer access is controlled by the BUFEN bit in USBIEN
    ep0_in_size: usize, // the highest index of the EP0 IN buffer that has been written to (plus 1)
}

impl UsbBuffers {
    pub fn new() -> Self {
        Self {
            bko_slots: [[0; 64]; 2],
            bko_sizes: [0; 2],
            bko_front: 0,
            bko_count: 0,
            bki_buffer: [0; 64],
            bki_size: 0,
            ep0_out_buffer: [0; 8],
            ep0_in_buffer: [0; 8],
            ep0_in_size: 0,
        }
    }

    /// Bulk-OUT double buffer: room for another received packet?
    fn bko_is_full(&self) -> bool {
        self.bko_count >= 2
    }

    fn bko_is_empty(&self) -> bool {
        self.bko_count == 0
    }

    /// Push a received bulk-OUT packet into the next free slot.
    fn bko_push(&mut self, data: &[u8]) {
        let slot = (self.bko_front + self.bko_count) % 2;
        let n = data.len().min(64);
        self.bko_slots[slot][..n].copy_from_slice(&data[..n]);
        self.bko_sizes[slot] = n;
        self.bko_count += 1;
    }

    /// The firmware finished with the front packet; advance to the next.
    fn bko_pop(&mut self) {
        if self.bko_count > 0 {
            self.bko_front = (self.bko_front + 1) % 2;
            self.bko_count -= 1;
        }
    }

    fn bko_read(&self, offset: usize) -> u8 {
        self.bko_slots[self.bko_front][offset]
    }

    fn bko_front_size(&self) -> usize {
        if self.bko_count > 0 {
            self.bko_sizes[self.bko_front]
        } else {
            0
        }
    }

    pub fn reset_bki_buffer(&mut self) {
        self.bki_buffer.fill(0);
        self.bki_size = 0;
    }

    pub fn write_bki_buffer(&mut self, index: usize, value: u8) {
        self.bki_buffer[index] = value;
        let len = index + 1;
        self.bki_size = self.bki_size.max(len);
    }

    pub fn reset_ep0_in_buffer(&mut self) {
        self.ep0_in_buffer.fill(0);
        self.ep0_in_size = 0;
    }

    pub fn write_ep0_in_buffer(&mut self, index: usize, value: u8) {
        self.ep0_in_buffer[index] = value;
        let len = index + 1;
        self.ep0_in_size = self.ep0_in_size.max(len);
    }
}

pub struct UsbState {
    usbcon: UsbCon,
    usbien: UsbIen,
    usbirq: UsbIrq,
    usbbfs: UsbBfs,
    ep0con: Ep0Con,
    ep0len: u8, // EP0 OUT Buffer Data Length Register (bits 3-0)
    bkcon: BkCon,
    bkolen: u8, // Bulk OUT Endpoint Data Length Register (bits 6-0)
    buffers: UsbBuffers,

    // --- transaction-model state (the new synchronous seam) ---
    /// Whether a USB host is attached (cable plugged). Surfaced in USBCON bit 1,
    /// the connect-status bit firmware polls (`and #$02`) to auto-detect USB
    /// before bringing up the SIE - see `read_usbcon`.
    /// Pushed in from the host seam (`Mcu`), since the SIE itself can't see VBUS.
    host_connected: bool,
    /// Current EP0 control-transfer stage (drives IN/OUT after a SETUP).
    ep0_stage: Ep0Stage,
    /// Firmware filled the EP0 IN buffer (wrote USBBFS EP0IN); ready to send.
    ep0_in_armed: bool,
    /// Firmware asked EP0 IN to send a zero-length packet (EP0CON TXZERO).
    ep0_in_zlp: bool,
    /// Firmware filled the bulk-IN buffer (wrote USBBFS BKI); ready to send.
    bki_armed: bool,
    /// Firmware asked bulk-IN to send a zero-length packet (BKCON TXZERO).
    bki_zlp: bool,
}

impl UsbState {
    pub fn new() -> Self {
        Self {
            usbcon: UsbCon::new(),
            usbien: UsbIen::new(),
            usbirq: UsbIrq::new(),
            usbbfs: UsbBfs::new(),
            ep0con: Ep0Con::new(),
            ep0len: 0,
            bkcon: BkCon::new(),
            bkolen: 0,
            buffers: UsbBuffers::new(),

            host_connected: false,
            ep0_stage: Ep0Stage::Idle,
            ep0_in_armed: false,
            ep0_in_zlp: false,
            bki_armed: false,
            bki_zlp: false,
        }
    }

    /// Reflect cable presence to the host seam. The SIE can't see VBUS itself, so
    /// `Mcu` pushes this in from the interface. This is purely the status firmware
    /// polls (USBCON bit 1) to decide whether to bring USB up; it does not by
    /// itself drive enumeration (the bus reset fires when firmware enables USB).
    pub fn set_host_connected(&mut self, connected: bool) {
        self.host_connected = connected;
    }

    pub fn read_usbcon(&self) -> u8 {
        // Bit 1 is an (datasheet-undocumented) USB-connected status bit: firmware
        // polls `USBCON & 0x02` to auto-detect a plugged cable before enabling the
        // SIE (55_main.s main()/periodic_poll). Reserved bits 0/1 in TABLE 18-3.
        self.usbcon.read_u8() | ((self.host_connected as u8) << 1)
    }

    pub fn write_usbcon(&mut self, value: u8) {
        let old_usben = self.usbcon.usben;
        self.usbcon.write_u8(value);
        println!(
            "usbcon: {:02x} ({:08b})",
            self.usbcon.read_u8(),
            self.usbcon.read_u8()
        );

        // If USBEN transitions from 0 to 1, enable BRIEN in USBIEN
        if !old_usben && self.usbcon.usben {
            // Enable Bus Reset interrupt when USB is enabled
            self.usbien.brien = true;
            println!("USBEN enabled, automatically enabling BRIEN");
        }

        // Enabling USB with the D+ pull-up asserted draws the host's bus reset.
        // (Firmware that gates on the connect-status bit only does this with a
        // cable present; the button-combo path enables USB unconditionally.)
        if self.usbcon.usben && self.usbcon.pull {
            self.usbirq.trigger_bus_reset();
        }
    }

    pub fn read_usbien(&self) -> u8 {
        self.usbien.read_u8()
    }

    pub fn write_usbien(&mut self, value: u8) {
        let old_bufen = self.usbien.bufen;
        self.usbien.write_u8(value);
        println!(
            "usbien: {:02x} ({:08b})",
            self.usbien.read_u8(),
            self.usbien.read_u8()
        );

        if old_bufen != self.usbien.bufen {
            println!(
                "USB buffer access {}",
                if self.usbien.bufen {
                    "enabled"
                } else {
                    "disabled"
                }
            );
        }
    }

    pub fn read_usbirq(&mut self) -> u8 {
        let value = self.usbirq.read_u8();
        // The EP0/bulk interrupts are per-event sources: the firmware's ISR reads
        // USBIRQ once into a snapshot and services from that, never writing their
        // CLR bits. So reading clears them (the buffer *status* in USBBFS is a
        // separate, level signal). The bus events (reset/resume/suspend) are
        // explicitly write-1-cleared and are left alone here.
        self.usbirq.ep0irq = false;
        self.usbirq.bkiirq = false;
        self.usbirq.bkoirq = false;
        value
    }

    pub fn write_usbirq(&mut self, value: u8) {
        self.usbirq.write_u8(value);
    }

    pub fn read_usbbfs(&self) -> u8 {
        self.usbbfs.read_u8()
    }

    pub fn write_usbbfs(&mut self, value: u8) {
        self.usbbfs.write_u8(value);

        // Writing an IN bit means the firmware filled that buffer (ready to send
        // on the next IN token); an OUT bit means the firmware consumed it. This
        // updates the buffer *status*; the per-event interrupts are edge sources
        // cleared when the firmware reads USBIRQ (see read_usbirq), so they are
        // not touched here.
        if self.usbbfs.write_ep0in_needs_service {
            self.ep0_in_armed = true;
            self.usbbfs.read_ep0in_needs_service = false; // IN buffer full, not free
        }
        if self.usbbfs.write_bki_needs_service {
            self.bki_armed = true;
            self.usbbfs.read_bki_needs_service = false; // IN buffer full, not free
        }
        if self.usbbfs.write_bko_needs_service {
            // Firmware released the consumed BKO packet by writing USBBFS_BKO;
            // advance the double buffer to the next slot. This is the *only* way
            // the bulk-OUT buffer advances - both the CBW/command path and the
            // write-data path (write_data_loop -> stage_response_header) release
            // each packet with an explicit USBBFS_BKO write.
            self.bko_advance();
        }
        if self.usbbfs.write_ep0out_needs_service {
            self.usbbfs.read_ep0out_needs_service = false; // firmware consumed EP0 OUT
        }
    }

    pub fn read_ep0con(&self) -> u8 {
        self.ep0con.read_u8()
    }

    pub fn write_ep0con(&mut self, value: u8) {
        self.ep0con.write_u8(value);

        if self.ep0con.txzero {
            // Arm a zero-length EP0 IN packet for the next IN token.
            self.ep0_in_zlp = true;
            self.ep0con.txzero = false;
        }
    }

    pub fn read_ep0len(&self) -> u8 {
        self.ep0len & 0x0F // Only lower 4 bits are valid
    }

    pub fn write_ep0len(&mut self, value: u8) {
        self.ep0len = value & 0x0F; // Max EP0 length is 15 (16 bytes)
    }

    pub fn read_bkcon(&self) -> u8 {
        self.bkcon.read_u8()
    }

    pub fn write_bkcon(&mut self, value: u8) {
        self.bkcon.write_u8(value);
        if self.bkcon.bki_txzero {
            // Arm a zero-length bulk-IN packet for the next IN token.
            self.bki_zlp = true;
            self.bkcon.bki_txzero = false;
        }
    }

    pub fn read_bkolen(&self) -> u8 {
        // Length of the current (front) bulk-OUT packet.
        (self.buffers.bko_front_size() as u8) & 0x7F
    }

    pub fn write_bkolen(&mut self, value: u8) {
        self.bkolen = value & 0x7F; // Max BKO length is 127 (128 bytes max, though we only support 64)
    }

    pub fn are_buffers_enabled(&self) -> bool {
        self.usbien.bufen
    }

    pub fn read_buffer(&mut self, address: u16) -> u8 {
        if !self.are_buffers_enabled() {
            return 0; // When buffer access is disabled, return 0
        }

        let value = match address {
            BKO_START..=BKO_END => self.buffers.bko_read((address - BKO_START) as usize),
            BKI_START..=BKI_END => self.buffers.bki_buffer[(address - BKI_START) as usize],
            EP0_OUT_START..=EP0_OUT_END => {
                self.buffers.ep0_out_buffer[(address - EP0_OUT_START) as usize]
            }
            EP0_IN_START..=EP0_IN_END => {
                self.buffers.ep0_in_buffer[(address - EP0_IN_START) as usize]
            }
            _ => 0, // Out of range
        };

        value
    }

    /// Advance the BKO double buffer past the consumed front packet and refresh
    /// the "buffer full" status. The bulk-OUT interrupt fires on packet arrival
    /// and is cleared when the firmware reads USBIRQ; but with double buffering a
    /// second packet can already be waiting behind the one just released. On
    /// hardware the "bulk-OUT data ready" condition persists while any received
    /// packet remains unread, so re-assert BKOIRQ here - otherwise the firmware,
    /// having merged two arrivals into one already-cleared interrupt, never gets
    /// woken to drain the trailing packet and the transfer stalls one chunk short.
    fn bko_advance(&mut self) {
        self.buffers.bko_pop();
        let pending = !self.buffers.bko_is_empty();
        self.usbbfs.read_bko_needs_service = pending;
        if pending {
            self.usbirq.trigger_bulk_out();
        }
    }

    pub fn write_buffer(&mut self, address: u16, value: u8) {
        if !self.are_buffers_enabled() {
            return; // When buffer access is disabled, ignore writes
        }

        match address {
            BKO_START..=BKO_END => {
                // Firmware writing into the (front) bulk-OUT slot; uncommon.
                let off = (address - BKO_START) as usize;
                let front = self.buffers.bko_front;
                self.buffers.bko_slots[front][off] = value;
            }
            BKI_START..=BKI_END => {
                self.buffers
                    .write_bki_buffer((address - BKI_START) as usize, value);
            }
            EP0_OUT_START..=EP0_OUT_END => {
                self.buffers.ep0_out_buffer[(address - EP0_OUT_START) as usize] = value;
            }
            EP0_IN_START..=EP0_IN_END => {
                self.buffers
                    .write_ep0_in_buffer((address - EP0_IN_START) as usize, value);
            }
            _ => {} // Out of range, ignore
        }
    }

    // -----------------------------------------------------------------------
    // Transaction-accurate seam. The host calls
    // `handle_transaction` once per USB transaction; the device answers inline.
    // -----------------------------------------------------------------------

    /// Handle one host-issued USB transaction and return the device's response.
    pub fn handle_transaction(&mut self, txn: UsbTransaction) -> UsbResponse {
        match txn.endpoint {
            ENDPOINT_CONTROL => match txn.token {
                UsbToken::Setup => self.ep0_setup(&txn.data),
                UsbToken::In => self.ep0_in(),
                UsbToken::Out => self.ep0_out(&txn.data),
            },
            ENDPOINT_BULK => match txn.token {
                UsbToken::In => self.bulk_in(),
                UsbToken::Out => self.bulk_out(&txn.data),
                UsbToken::Setup => UsbResponse::Stall, // SETUP is EP0-only
            },
            _ => UsbResponse::Stall,
        }
    }

    /// Is any enabled USB interrupt currently asserted? Pure predicate the CPU
    /// checks each step to decide whether to raise the USB interrupt line.
    pub fn pending_irq(&self) -> bool {
        (self.usbirq.brirq && self.usbien.brien)
            || (self.usbirq.resirq && self.usbien.resien)
            || (self.usbirq.susirq && self.usbien.susien)
            || (self.usbirq.bkiirq && self.usbien.bkiien)
            || (self.usbirq.bkoirq && self.usbien.bkoien)
            || (self.usbirq.ep0irq && self.usbien.ep0ien)
    }

    fn ep0_setup(&mut self, setup: &[u8]) -> UsbResponse {
        if setup.len() != EP0_MAX_PACKET {
            return UsbResponse::Stall;
        }
        // A new control transfer clears any prior EP0 stall condition and
        // flushes the IN buffer. A SETUP token aborts whatever came before, so
        // any IN packet the firmware armed but the host never collected (e.g. a
        // terminating ZLP left over when the host stopped at wLength) must not
        // survive into this transfer's data stage.
        self.ep0con.stall = false;
        self.ep0_in_armed = false;
        self.ep0_in_zlp = false;
        self.buffers.reset_ep0_in_buffer();

        match classify_setup(setup) {
            SetupDisposition::AutoAck => {
                // SET_ADDRESS and friends are acknowledged by the SIE; the device
                // doesn't enforce its assigned address, so nothing to record here.
                self.ep0_stage = Ep0Stage::AutoStatus;
                UsbResponse::Ack
            }
            SetupDisposition::AutoIn(mut bytes) => {
                let w_length = u16::from_le_bytes([setup[6], setup[7]]) as usize;
                bytes.truncate(w_length);
                self.ep0_stage = Ep0Stage::AutoDataIn(bytes);
                UsbResponse::Ack
            }
            SetupDisposition::Forward(drq) => {
                // Deliver the 8-byte SETUP packet to the firmware via EP0 OUT.
                self.buffers.ep0_out_buffer.copy_from_slice(setup);
                self.ep0len = EP0_MAX_PACKET as u8;
                self.ep0con.set_setup(true);
                self.ep0con.set_dir(false);
                self.ep0con.set_drq(drq);
                self.usbbfs.read_ep0out_needs_service = true; // OUT buffer full
                self.ep0_stage = Ep0Stage::Firmware;
                self.usbirq.trigger_ep0();
                UsbResponse::Ack
            }
        }
    }

    fn ep0_in(&mut self) -> UsbResponse {
        if self.ep0con.stall {
            return UsbResponse::Stall;
        }
        match self.ep0_stage {
            Ep0Stage::Idle => UsbResponse::Nak,
            Ep0Stage::AutoStatus => {
                // Status stage of a no-data control transfer: send a ZLP.
                self.ep0_stage = Ep0Stage::Idle;
                UsbResponse::Data(Vec::new())
            }
            Ep0Stage::AutoDataIn(_) => {
                // Serve the SIE-generated response in <=8-byte packets.
                if let Ep0Stage::AutoDataIn(bytes) = &mut self.ep0_stage {
                    let n = bytes.len().min(EP0_MAX_PACKET);
                    UsbResponse::Data(bytes.drain(..n).collect())
                } else {
                    unreachable!()
                }
            }
            Ep0Stage::Firmware => {
                if self.ep0_in_zlp {
                    self.ep0_in_zlp = false;
                    self.ep0_in_armed = false;
                    self.finish_ep0_in_packet();
                    UsbResponse::Data(Vec::new())
                } else if self.ep0_in_armed {
                    let len = self.buffers.ep0_in_size;
                    let chunk = self.buffers.ep0_in_buffer[..len].to_vec();
                    self.ep0_in_armed = false;
                    self.finish_ep0_in_packet();
                    UsbResponse::Data(chunk)
                } else {
                    // Firmware hasn't filled the IN buffer yet; host should retry.
                    UsbResponse::Nak
                }
            }
        }
    }

    fn ep0_out(&mut self, data: &[u8]) -> UsbResponse {
        if self.ep0con.stall {
            return UsbResponse::Stall;
        }
        match self.ep0_stage {
            Ep0Stage::Firmware => {
                // OUT data stage, or the status OUT of an IN control transfer
                // (which the firmware acks). Hand it to the firmware.
                let n = data.len().min(self.buffers.ep0_out_buffer.len());
                self.buffers.ep0_out_buffer[..n].copy_from_slice(&data[..n]);
                self.ep0len = n as u8;
                self.ep0con.set_setup(false);
                self.ep0con.set_dir(false);
                self.usbbfs.read_ep0out_needs_service = true;
                self.usbirq.trigger_ep0();
                UsbResponse::Ack
            }
            _ => {
                // Status stage (OUT ZLP) of a SIE-handled transfer: swallow it.
                self.ep0_stage = Ep0Stage::Idle;
                UsbResponse::Ack
            }
        }
    }

    /// After an EP0 IN packet leaves: the IN buffer is empty again, the last
    /// packet was IN data, and the firmware should run its continue-tx path.
    fn finish_ep0_in_packet(&mut self) {
        self.buffers.reset_ep0_in_buffer();
        self.usbbfs.read_ep0in_needs_service = true; // IN buffer free again
        self.ep0con.set_dir(true); // last EP0 packet was IN data
        self.usbirq.trigger_ep0();
    }

    fn bulk_in(&mut self) -> UsbResponse {
        if self.bkcon.bki_stall {
            return UsbResponse::Stall;
        }
        if self.bki_zlp {
            self.bki_zlp = false;
            self.bki_armed = false;
            self.finish_bulk_in_packet();
            UsbResponse::Data(Vec::new())
        } else if self.bki_armed {
            let len = self.buffers.bki_size;
            let chunk = self.buffers.bki_buffer[..len].to_vec();
            self.bki_armed = false;
            self.finish_bulk_in_packet();
            UsbResponse::Data(chunk)
        } else {
            // Host polled an empty bulk-IN buffer: just NAK and let it retry. The
            // "buffer empty" status stays asserted, but we must NOT re-raise BKIIRQ
            // on every poll - on hardware that interrupt fires once when the buffer
            // drains (see finish_bulk_in_packet), not per IN token. Re-firing it
            // makes the firmware pump ahead into the single bulk-IN buffer and
            // desync (the staged-response read stalls one chunk in).
            self.usbbfs.read_bki_needs_service = true;
            UsbResponse::Nak
        }
    }

    fn bulk_out(&mut self, data: &[u8]) -> UsbResponse {
        if self.bkcon.bko_stall {
            return UsbResponse::Stall;
        }
        // BKO is double-buffered: accept a packet as long as a slot is free, so
        // the host can stay one packet ahead of the firmware's draining.
        if self.buffers.bko_is_full() {
            return UsbResponse::Nak;
        }
        self.buffers.bko_push(data);
        self.usbbfs.read_bko_needs_service = true; // a packet is waiting
        self.usbirq.trigger_bulk_out();
        UsbResponse::Ack
    }

    /// After a bulk-IN packet leaves: the IN buffer is free again and the
    /// firmware should pump the next chunk (handle_bkiirq).
    fn finish_bulk_in_packet(&mut self) {
        self.buffers.reset_bki_buffer();
        self.usbbfs.read_bki_needs_service = true; // IN buffer free again
        self.usbirq.trigger_bulk_in();
    }
}
