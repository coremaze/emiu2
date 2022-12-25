#[derive(Default, Clone)]
pub struct U8Register {
    val: u8,
    /// Bits which exist in this register
    mask: u8,
}

impl U8Register {
    pub fn new(value: u8, mask: u8) -> Self {
        let mut reg = Self::default();
        reg.set_mask(mask);
        reg.set(value);
        reg.apply_mask();
        reg
    }

    fn set_mask(&mut self, mask: u8) {
        self.mask = mask;
    }

    pub fn mask(&self) -> u8 {
        self.mask
    }

    pub fn set(&mut self, value: u8) {
        self.val = value;
        self.apply_mask();
    }

    pub fn get(&self) -> u8 {
        self.val
    }

    fn apply_mask(&mut self) {
        self.val &= self.mask;
    }
}

#[derive(Default, Clone)]
pub struct U16Register {
    /// Low byte of register
    l: U8Register,
    /// High byte of register
    h: U8Register,
}

impl U16Register {
    /// Creates a new U16Register with a value of `value`.
    /// `mask` determines which bits are enabled. Any read from non-enabled bits
    /// will be 0.
    pub fn new(value: u16, mask: u16) -> Self {
        let mut reg = Self::default();
        reg.set_mask_u16(mask);
        reg.set_u16(value);
        reg
    }

    pub fn u16(&self) -> u16 {
        (self.l.get() as u16) | ((self.h.get() as u16) << 8)
    }

    pub fn l(&self) -> u8 {
        self.l.get()
    }

    pub fn h(&self) -> u8 {
        self.h.get()
    }

    pub fn set_l(&mut self, value: u8) {
        self.l.set(value);
    }

    pub fn set_h(&mut self, value: u8) {
        self.h.set(value);
    }

    pub fn set_u16(&mut self, value: u16) {
        self.l.set((value & 0x00FF) as u8);
        self.h.set(((value & 0xFF00) >> 8) as u8);
    }

    fn set_mask_u16(&mut self, value: u16) {
        self.l.set_mask((value & 0x00FF) as u8);
        self.h.set_mask(((value & 0xFF00) >> 8) as u8);
    }

    pub fn mask_u16(&self) -> u16 {
        (self.l_mask() as u16) | ((self.h_mask() as u16) << 8)
    }

    pub fn l_mask(&self) -> u8 {
        self.l.mask()
    }

    pub fn h_mask(&self) -> u8 {
        self.h.mask()
    }
}
