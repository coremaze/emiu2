pub trait Screen {
    fn set_pixels(&self, pixels: &[Pixel]);
}

#[derive(Clone, Copy)]
pub struct Pixel {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Pixel {
    pub fn to_rgb_u32(&self) -> u32 {
        let mut e = 0u32;
        e |= self.red as u32;
        e <<= 8;
        e |= self.green as u32;
        e <<= 8;
        e |= self.blue as u32;
        e
    }
}
