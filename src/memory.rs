pub trait AddressSpace {
    // This uses &mut self because a read could possibly mutate the state of hardware
    fn read_u8(&mut self, address: usize) -> u8;
    fn write_u8(&mut self, address: usize, value: u8);
    fn read_u16_le(&mut self, address: usize) -> u16 {
        self.read_u8(address) as u16 | (self.read_u8(address + 1) as u16) << 8
    }

    /// A stable cache key for decoding instructions from
    /// `[address, address + len)`, or None if any read in the range
    /// currently has side effects or transient values and must not be
    /// cached. Keys must be alias-free (equal keys ⇔ same underlying byte)
    /// and contiguous across the range. The conservative default is that
    /// nothing may be cached.
    fn code_cache_key_range(&self, _address: usize, _len: usize) -> Option<usize> {
        None
    }

    /// Which cache keys the most recent write invalidated, if any. Calling
    /// this clears the pending change. A change must be reported whenever
    /// key contents changed or `code_cache_key_range` answers may have
    /// changed (e.g. entering a mode where reads return status values).
    fn take_content_change(&mut self) -> ContentChange {
        ContentChange::None
    }
}

/// A change to memory contents underlying previously-issued cache keys
pub enum ContentChange {
    None,
    /// Keys in `[start, start + len)` changed
    Range {
        start: usize,
        len: usize,
    },
    /// Everything changed
    All,
}
