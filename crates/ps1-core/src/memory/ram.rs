/// 2 MB main RAM of the PS1.
pub struct Ram {
    data: Vec<u8>,
}

const RAM_SIZE: usize = 2 * 1024 * 1024; // 2 MB

impl Ram {
    pub fn new() -> Self {
        Self {
            data: vec![0u8; RAM_SIZE],
        }
    }

    pub fn reset(&mut self) {
        self.data.fill(0);
    }

    #[inline(always)]
    pub fn read32(&self, offset: usize) -> u32 {
        let o = offset & (RAM_SIZE - 4);
        u32::from_le_bytes(self.data[o..o + 4].try_into().unwrap())
    }

    #[inline(always)]
    pub fn write32(&mut self, offset: usize, value: u32) {
        let o = offset & (RAM_SIZE - 4);
        self.data[o..o + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[inline(always)]
    pub fn read16(&self, offset: usize) -> u16 {
        let o = offset & (RAM_SIZE - 2);
        u16::from_le_bytes(self.data[o..o + 2].try_into().unwrap())
    }

    #[inline(always)]
    pub fn write16(&mut self, offset: usize, value: u16) {
        let o = offset & (RAM_SIZE - 2);
        self.data[o..o + 2].copy_from_slice(&value.to_le_bytes());
    }

    pub fn read8(&self, offset: usize) -> u8 {
        self.data[offset & (RAM_SIZE - 1)]
    }

    pub fn write8(&mut self, offset: usize, value: u8) {
        self.data[offset & (RAM_SIZE - 1)] = value;
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }
}
