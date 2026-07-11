use crate::snapshot::{SnapshotError, SnapshotReader, SnapshotWriter};

pub trait AddressSpace {
    // This uses &mut self because a read could possibly mutate the state of hardware
    fn read_u8(&mut self, address: usize) -> u8;
    fn write_u8(&mut self, address: usize, value: u8);
    fn read_u16_le(&mut self, address: usize) -> u16 {
        self.read_u8(address) as u16 | (self.read_u8(address + 1) as u16) << 8
    }

    /// Serializes the runtime-mutable state behind this address space.
    /// ROM-only spaces write nothing.
    fn snapshot(&self, writer: &mut SnapshotWriter);

    /// Restores state previously written by `snapshot`, in place.
    fn restore(&mut self, reader: &mut SnapshotReader) -> Result<(), SnapshotError>;
}
