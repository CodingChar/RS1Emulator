/// PS1 BIOS ROM region (512 KB).
pub struct Bios {
    data: Vec<u8>,
}

const BIOS_SIZE: usize = 512 * 1024;

impl Bios {
    pub fn new() -> Self {
        Self {
            data: vec![0u8; BIOS_SIZE],
        }
    }

    pub fn capacity(&self) -> usize {
        BIOS_SIZE
    }

    pub fn read32(&self, offset: usize) -> u32 {
        let o = offset & (BIOS_SIZE - 4);
        u32::from_le_bytes([
            self.data[o],
            self.data[o + 1],
            self.data[o + 2],
            self.data[o + 3],
        ])
    }

    pub fn read16(&self, offset: usize) -> u16 {
        let o = offset & (BIOS_SIZE - 2);
        u16::from_le_bytes([self.data[o], self.data[o + 1]])
    }

    pub fn read8(&self, offset: usize) -> u8 {
        self.data[offset & (BIOS_SIZE - 1)]
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }
}
