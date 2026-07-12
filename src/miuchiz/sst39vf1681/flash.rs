use std::cmp::PartialEq;

use crate::memory::{AddressSpace, ContentChange};
use crate::snapshot::{SnapshotError, SnapshotReader, SnapshotWriter};

const CHIP_CAPACITY: usize = 0x200000;
const SECTOR_SIZE: usize = 0x1000;
const BLOCK_SIZE: usize = 0x10000;

const BYTE_PROGRAM: [CommandWrite; 3] = [
    CommandWrite {
        address: 0xAAA,
        value: 0xAA,
    },
    CommandWrite {
        address: 0x555,
        value: 0x55,
    },
    CommandWrite {
        address: 0xAAA,
        value: 0xA0,
    },
];

const ERASE: [CommandWrite; 5] = [
    CommandWrite {
        address: 0xAAA,
        value: 0xAA,
    },
    CommandWrite {
        address: 0x555,
        value: 0x55,
    },
    CommandWrite {
        address: 0xAAA,
        value: 0x80,
    },
    CommandWrite {
        address: 0xAAA,
        value: 0xAA,
    },
    CommandWrite {
        address: 0x555,
        value: 0x55,
    },
];

enum ReadMode {
    Status { address: usize },
    Data,
}

#[derive(Copy, Clone, PartialEq)]
struct CommandWrite {
    address: usize,
    value: u8,
}

pub struct Flash {
    data: Box<[u8; CHIP_CAPACITY]>,
    read_mode: ReadMode,
    command_writes: RingBuf<6, CommandWrite>,
    /// The range of array contents changed by the most recent program/erase
    pending_change: ContentChange,
}

impl Flash {
    pub fn new(data: &[u8]) -> Result<Self, String> {
        let flash_box = Box::<[u8; CHIP_CAPACITY]>::try_from(data.to_vec().into_boxed_slice())
            .map_err(|_| "Flash invalid")?;

        Ok(Self {
            data: flash_box,
            read_mode: ReadMode::Data,
            command_writes: RingBuf::new(),
            pending_change: ContentChange::None,
        })
    }

    pub fn len() -> usize {
        CHIP_CAPACITY
    }

    fn sector_erase(&mut self, sector: usize) {
        for i in 0..SECTOR_SIZE {
            let addr = (sector * SECTOR_SIZE + i) % self.data.len();
            self.data[addr] = 0xFF;
        }
        self.pending_change = ContentChange::Range {
            start: (sector * SECTOR_SIZE) % self.data.len(),
            len: SECTOR_SIZE,
        };
    }

    fn block_erase(&mut self, block: usize) {
        for i in 0..BLOCK_SIZE {
            let addr = (block * BLOCK_SIZE + i) % self.data.len();
            self.data[addr] = 0xFF;
        }
        self.pending_change = ContentChange::Range {
            start: (block * BLOCK_SIZE) % self.data.len(),
            len: BLOCK_SIZE,
        };
    }

    fn chip_erase(&mut self) {
        self.data.fill(0xFF);
        self.pending_change = ContentChange::All;
    }

    fn byte_program(&mut self, address: usize, value: u8) {
        self.data[address % self.data.len()] = value;
        self.pending_change = ContentChange::Range {
            start: address % self.data.len(),
            len: 1,
        };
    }

    fn status_register(&self) -> u8 {
        0b1100_0000
    }

    pub fn data(&self) -> &[u8] {
        self.data.as_ref()
    }
}

impl AddressSpace for Flash {
    fn read_u8(&mut self, address: usize) -> u8 {
        if let ReadMode::Status {
            address: status_address,
        } = self.read_mode
        {
            // While an embedded program/erase is "in progress" the firmware polls
            // the operated address and reads the data-polling/toggle status there.
            // A read of any *other* address means the firmware has moved on (the
            // operation, which we model as instantaneous, has completed), so the
            // chip returns to read-array mode - otherwise a later array read of the
            // last-operated byte (e.g. the host reading back a written page) would
            // wrongly see the status register instead of the stored data.
            if address == status_address {
                return self.status_register();
            }
            self.read_mode = ReadMode::Data;
        }

        self.data[address % self.data.len()]
    }

    fn write_u8(&mut self, address: usize, value: u8) {
        let mut command_handled = true;
        if self.command_writes.ends_with(&ERASE) {
            if value == 0x50 {
                // println!("Sector erase {address:X}");
                self.sector_erase(address / SECTOR_SIZE);
            } else if value == 0x30 {
                // println!("Block erase {address:X}");
                self.block_erase(address / BLOCK_SIZE);
            } else if address == 0xAAA && value == 0x10 {
                // println!("Chip erase");
                self.chip_erase();
            } else {
                println!("Invalid erase command: {address:X} {value:02X}");
            }
        } else if self.command_writes.ends_with(&BYTE_PROGRAM) {
            // println!("Program byte {address:X} to {value:02X}");
            self.byte_program(address, value);
        } else {
            self.command_writes.push(CommandWrite { address, value });
            command_handled = false;
        }

        if command_handled {
            self.read_mode = ReadMode::Status { address };
            self.command_writes.clear();
        }
    }

    fn code_cache_key_range(&self, address: usize, len: usize) -> Option<usize> {
        let address = address % CHIP_CAPACITY;
        if address + len > CHIP_CAPACITY {
            // The range would wrap around the chip: keys not contiguous
            return None;
        }
        match self.read_mode {
            // Reads at the status address return the status register, not
            // array contents; a range containing it must not be cached
            ReadMode::Status {
                address: status_address,
            } if (address..address + len).contains(&status_address) => None,
            _ => Some(address),
        }
    }

    fn take_content_change(&mut self) -> ContentChange {
        std::mem::replace(&mut self.pending_change, ContentChange::None)
    }

    fn snapshot(&self, writer: &mut SnapshotWriter) {
        writer.put_bytes(self.data.as_ref());
        match self.read_mode {
            ReadMode::Data => writer.put_u8(0),
            ReadMode::Status { address } => {
                writer.put_u8(1);
                writer.put_u64(address as u64);
            }
        }
        writer.put_u8(self.command_writes.index as u8);
        for slot in &self.command_writes.data {
            match slot {
                None => writer.put_bool(false),
                Some(write) => {
                    writer.put_bool(true);
                    writer.put_u64(write.address as u64);
                    writer.put_u8(write.value);
                }
            }
        }
    }

    fn restore(&mut self, reader: &mut SnapshotReader) -> Result<(), SnapshotError> {
        reader.take_into(self.data.as_mut())?;
        self.read_mode = match reader.take_u8()? {
            0 => ReadMode::Data,
            1 => ReadMode::Status {
                address: reader.take_u64()? as usize,
            },
            _ => return Err(SnapshotError::Corrupt("flash read mode")),
        };
        let index = reader.take_u8()? as usize;
        if index >= self.command_writes.size() {
            return Err(SnapshotError::Corrupt("flash command ring index"));
        }
        self.command_writes.index = index;
        for slot in &mut self.command_writes.data {
            *slot = if reader.take_bool()? {
                Some(CommandWrite {
                    address: reader.take_u64()? as usize,
                    value: reader.take_u8()?,
                })
            } else {
                None
            };
        }
        // The array contents and read mode were both replaced; any cached
        // decodes over this chip are stale
        self.pending_change = ContentChange::All;
        Ok(())
    }
}

struct RingBuf<const N: usize, T: Copy + PartialEq> {
    data: [Option<T>; N],
    index: usize,
}

impl<const N: usize, T: Copy + PartialEq> RingBuf<N, T> {
    pub fn new() -> Self {
        Self {
            data: [None; N],
            index: 0,
        }
    }

    pub fn size(&self) -> usize {
        N
    }

    pub fn push(&mut self, value: T) {
        self.data[self.index] = Some(value);
        self.index = (self.index + 1) % N;
    }

    pub fn clear(&mut self) {
        self.data = [None; N];
        self.index = 0;
    }

    pub fn get_from_back(&self, mut n: usize) -> Option<T> {
        n = n % N;

        // self.index is incremented after writes, this gets the index of the
        // most recent element without underflowing
        let prev_buf_index = (self.index + N - 1) % N;

        let index = (N - n + prev_buf_index) % N;
        self.data[index]
    }

    pub fn ends_with(&self, other: &[T]) -> bool {
        if other.len() >= N {
            return false;
        }

        for (i, e) in other.iter().rev().enumerate() {
            let Some(buf_element) = self.get_from_back(i) else {
                return false;
            };
            if *e != buf_element {
                return false;
            }
        }

        true
    }
}
