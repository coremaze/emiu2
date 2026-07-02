//! Machine savestate serialization.
//!
//! Savestates capture the deterministic emulated machine only: CPU,
//! RAM, peripheral registers, flash and LCD contents. Host-side objects
//! (screen, audio, buttons, IR transports) are never serialized; state
//! is restored in place so those connections stay attached.
//!
//! The format is a fixed field order per component with a leading magic
//! and version. All integers are little-endian.

pub const MAGIC: &[u8; 8] = b"EMIU2SAV";
pub const VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    /// The data ended before all fields could be read.
    UnexpectedEof,
    /// Data remained after all fields were read.
    TrailingData,
    /// The data does not start with the savestate magic.
    BadMagic,
    /// The savestate was written by an incompatible version.
    UnsupportedVersion(u16),
    /// A field held a value that no machine state can produce.
    Corrupt(&'static str),
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateError::UnexpectedEof => f.write_str("savestate data is truncated"),
            StateError::TrailingData => f.write_str("savestate data has trailing bytes"),
            StateError::BadMagic => f.write_str("not a savestate file"),
            StateError::UnsupportedVersion(version) => {
                write!(f, "unsupported savestate version {version}")
            }
            StateError::Corrupt(what) => write!(f, "corrupt savestate field: {what}"),
        }
    }
}

impl std::error::Error for StateError {}

#[derive(Default)]
pub struct StateWriter {
    buf: Vec<u8>,
}

impl StateWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reuses an existing allocation, clearing its contents.
    pub fn from_vec(mut buf: Vec<u8>) -> Self {
        buf.clear();
        Self { buf }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn put_u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    pub fn put_bool(&mut self, value: bool) {
        self.buf.push(value as u8);
    }

    pub fn put_u16(&mut self, value: u16) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub fn put_i16(&mut self, value: i16) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub fn put_u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub fn put_u64(&mut self, value: u64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub fn put_bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }
}

pub struct StateReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> StateReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn take_u8(&mut self) -> Result<u8, StateError> {
        let bytes = self.take_bytes(1)?;
        Ok(bytes[0])
    }

    pub fn take_bool(&mut self) -> Result<bool, StateError> {
        Ok(self.take_u8()? != 0)
    }

    pub fn take_u16(&mut self) -> Result<u16, StateError> {
        let bytes = self.take_bytes(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub fn take_i16(&mut self) -> Result<i16, StateError> {
        let bytes = self.take_bytes(2)?;
        Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub fn take_u32(&mut self) -> Result<u32, StateError> {
        let bytes = self.take_bytes(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn take_u64(&mut self) -> Result<u64, StateError> {
        let bytes = self.take_bytes(8)?;
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(array))
    }

    pub fn take_bytes(&mut self, len: usize) -> Result<&'a [u8], StateError> {
        let end = self.pos.checked_add(len).ok_or(StateError::UnexpectedEof)?;
        if end > self.data.len() {
            return Err(StateError::UnexpectedEof);
        }
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    /// Fills `out` from the stream; the counterpart of `put_bytes` for
    /// fixed-size buffers restored in place.
    pub fn take_into(&mut self, out: &mut [u8]) -> Result<(), StateError> {
        let bytes = self.take_bytes(out.len())?;
        out.copy_from_slice(bytes);
        Ok(())
    }

    /// Verifies every byte was consumed.
    pub fn finish(&self) -> Result<(), StateError> {
        if self.pos == self.data.len() {
            Ok(())
        } else {
            Err(StateError::TrailingData)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_round_trip() {
        let mut w = StateWriter::new();
        w.put_u8(0xAB);
        w.put_bool(true);
        w.put_u16(0x1234);
        w.put_i16(-1234);
        w.put_u32(0xDEADBEEF);
        w.put_u64(0x0123_4567_89AB_CDEF);
        w.put_bytes(&[1, 2, 3]);
        let bytes = w.into_bytes();

        let mut r = StateReader::new(&bytes);
        assert_eq!(r.take_u8().unwrap(), 0xAB);
        assert!(r.take_bool().unwrap());
        assert_eq!(r.take_u16().unwrap(), 0x1234);
        assert_eq!(r.take_i16().unwrap(), -1234);
        assert_eq!(r.take_u32().unwrap(), 0xDEADBEEF);
        assert_eq!(r.take_u64().unwrap(), 0x0123_4567_89AB_CDEF);
        let mut out = [0u8; 3];
        r.take_into(&mut out).unwrap();
        assert_eq!(out, [1, 2, 3]);
        r.finish().unwrap();
    }

    #[test]
    fn truncated_data_errors() {
        let mut r = StateReader::new(&[0x01]);
        assert_eq!(r.take_u16(), Err(StateError::UnexpectedEof));
    }

    #[test]
    fn trailing_data_errors() {
        let r = StateReader::new(&[0x01]);
        assert_eq!(r.finish(), Err(StateError::TrailingData));
    }
}
