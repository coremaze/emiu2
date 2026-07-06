//! Device-facing USB interface - a peripheral seam like `Screen`, `AudioInterface`,
//! and `GpioInterfaceInternal`.
//!
//! The emulator holds a `Box<dyn UsbInterfaceInternal>` and services it from the
//! main step loop (see `Mcu::step`). Like the other peripherals, the trait is
//! oblivious to what's on the other side: nothing (an unplugged cable), an
//! in-process test driver, or a USB/IP server on its own thread. A concrete
//! impl may split into two halves connected by channels (`channel_pair`),
//! exactly as the minifb screen/GPIO and cpal audio interfaces do.
//!
//! The transactions that cross this seam are raw USB (the transaction-accurate
//! level - "what libusb sees"). Anything higher (enumeration, SCSI, the flash
//! protocol) belongs to whatever drives the external half, not to the emulator.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use crate::miuchiz::{UsbResponse, UsbTransaction};

/// The device-facing half of a USB connection. The emulator pulls one pending
/// host transaction per service tick and returns the device's response.
pub trait UsbInterfaceInternal {
    /// Non-blocking: the next host-issued transaction to service, if any.
    fn poll_transaction(&mut self) -> Option<UsbTransaction>;
    /// Deliver the device's response to the transaction returned by the most
    /// recent `poll_transaction` that yielded `Some`.
    fn respond(&mut self, response: UsbResponse);
    /// Whether a host is attached (the "cable" is plugged in). The device
    /// reflects this in the USBCON connect-status bit, which firmware polls to
    /// auto-detect USB without first bringing up the SIE.
    /// The null cable is never connected; the channel pair tracks its host port's
    /// `set_connected` state (the socket bridge toggles it per TCP client).
    fn is_connected(&self) -> bool;
}

/// A USB interface with no host attached - the device never sees a transaction
/// (an unplugged cable). The default for the GUI/web builds.
pub struct NullUsbInterface;

impl UsbInterfaceInternal for NullUsbInterface {
    fn poll_transaction(&mut self) -> Option<UsbTransaction> {
        None
    }
    fn respond(&mut self, _response: UsbResponse) {}
    fn is_connected(&self) -> bool {
        false
    }
}

/// Internal half of a channel-backed interface (held by the emulator).
pub struct ChannelUsbInterface {
    txn_rx: Receiver<UsbTransaction>,
    resp_tx: Sender<UsbResponse>,
    connected: Arc<AtomicBool>,
}

impl UsbInterfaceInternal for ChannelUsbInterface {
    fn poll_transaction(&mut self) -> Option<UsbTransaction> {
        self.txn_rx.try_recv().ok()
    }
    fn respond(&mut self, response: UsbResponse) {
        let _ = self.resp_tx.send(response);
    }
    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}

/// External half (the "host port"), held by whatever drives USB: an in-process
/// cooperative-poll driver, or a USB/IP server on its own thread. Both ends are
/// `Send`, so the external half can move to another thread.
pub struct UsbHostPort {
    txn_tx: Sender<UsbTransaction>,
    resp_rx: Receiver<UsbResponse>,
    connected: Arc<AtomicBool>,
}

impl UsbHostPort {
    /// Report whether a host is attached (the "cable" is plugged). Drives the
    /// device's USBCON connect-status bit. A fresh pair starts disconnected; the
    /// socket bridge raises it while the emulator's cable is plugged, and an
    /// in-process driver can raise it to exercise the connect-status bit.
    pub fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Relaxed);
    }

    /// The shared cable-presence flag itself, for owners that need to drive it
    /// while the port is locked away inside a session (see `UsbCable`).
    pub(crate) fn connected_flag(&self) -> Arc<AtomicBool> {
        self.connected.clone()
    }

    /// Queue a transaction for the device to service on its next step.
    pub fn submit(&self, txn: UsbTransaction) {
        let _ = self.txn_tx.send(txn);
    }

    /// Non-blocking response retrieval (for in-process cooperative polling,
    /// where the same thread also steps the emulator).
    pub fn try_response(&self) -> Option<UsbResponse> {
        self.resp_rx.try_recv().ok()
    }

    /// Blocking response retrieval (for a USB/IP server on its own thread, where
    /// the emulator is stepped elsewhere). `None` once the device is gone.
    pub fn response_blocking(&self) -> Option<UsbResponse> {
        self.resp_rx.recv().ok()
    }
}

/// Create an `(external host port, internal device interface)` pair. The shared
/// connect flag starts `false` (no host has declared presence yet); the driver
/// raises it - the socket bridge per TCP client (`serve_on`), or an in-process
/// host that wants to exercise the connect-status bit via `set_connected(true)`.
/// It only feeds the USBCON connect bit firmware polls; enumeration is driven by
/// the bus reset when firmware enables USB, independent of this flag.
pub fn channel_pair() -> (UsbHostPort, ChannelUsbInterface) {
    let (txn_tx, txn_rx) = channel();
    let (resp_tx, resp_rx) = channel();
    let connected = Arc::new(AtomicBool::new(false));
    (
        UsbHostPort {
            txn_tx,
            resp_rx,
            connected: connected.clone(),
        },
        ChannelUsbInterface {
            txn_rx,
            resp_tx,
            connected,
        },
    )
}
