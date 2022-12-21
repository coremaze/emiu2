pub trait AddressSpace {
    // This uses &mut self because a read could possibly mutate the state of hardware
    fn read_u8(&mut self, address: usize) -> u8;
    fn write_u8(&mut self, address: usize, value: u8);
    fn read_u16_le(&mut self, address: usize) -> u16 {
        self.read_u8(address) as u16 | (self.read_u8(address + 1) as u16) << 8
    }
}
