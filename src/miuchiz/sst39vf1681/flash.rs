use std::cmp::PartialEq;
use std::fmt::Display;

use crate::memory::AddressSpace;

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
}

impl Flash {
    pub fn new(data: &[u8]) -> Result<Self, String> {
        let flash_box = Box::<[u8; CHIP_CAPACITY]>::try_from(data.to_vec().into_boxed_slice())
            .map_err(|_| "Flash invalid")?;

        Ok(Self {
            data: flash_box,
            read_mode: ReadMode::Data,
            command_writes: RingBuf::new(),
        })
    }

    fn sector_erase(&mut self, sector: usize) {
        for i in 0..SECTOR_SIZE {
            let addr = (sector * SECTOR_SIZE + i) % self.data.len();
            self.data[addr] = 0xFF;
        }
    }

    fn block_erase(&mut self, block: usize) {
        for i in 0..BLOCK_SIZE {
            let addr = (block * BLOCK_SIZE + i) % self.data.len();
            self.data[addr] = 0xFF;
        }
    }

    fn chip_erase(&mut self) {
        self.data.fill(0xFF);
    }

    fn byte_program(&mut self, address: usize, value: u8) {
        self.data[address % self.data.len()] = value;
    }

    fn status_register(&self) -> u8 {
        0b1100_0000
    }
}

impl AddressSpace for Flash {
    fn read_u8(&mut self, address: usize) -> u8 {
        if let ReadMode::Status {
            address: status_address,
        } = self.read_mode
        {
            if address == status_address {
                return self.status_register();
            }
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
            let Some(buf_element) = self.get_from_back(i) else { return false };
            if *e != buf_element {
                return false;
            }
        }

        true
    }
}
