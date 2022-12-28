use crate::memory::AddressSpace;

const CHIP_CAPACITY: usize = 0x200000;

pub struct Flash {
    data: Box<[u8; CHIP_CAPACITY]>,
}

impl Flash {
    pub fn new(data: &[u8]) -> Result<Self, String> {
        let flash_box = Box::<[u8; CHIP_CAPACITY]>::try_from(data.to_vec().into_boxed_slice())
            .map_err(|_| "Flash invalid")?;

        Ok(Self { data: flash_box })
    }
}

impl AddressSpace for Flash {
    fn read_u8(&mut self, address: usize) -> u8 {
        self.data[address % self.data.len()]
    }

    fn write_u8(&mut self, address: usize, value: u8) {
        println!("Flash write {value:02X} to {address:X}");
    }
}
