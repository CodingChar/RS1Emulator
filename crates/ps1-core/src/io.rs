//! Helpers for memory-mapped hardware registers.
//!
//! The CPU can access most I/O registers as 8, 16 or 32 bits. Devices keep
//! their registers as 32-bit words and use [`extract`] / [`merge`] to serve
//! partial-width accesses at any byte offset.

/// Read `width` bytes (1, 2 or 4) at byte offset `off` out of a 32-bit register.
pub fn extract(word: u32, off: u32, width: u32) -> u32 {
    let shift = (off & 3) * 8;
    match width {
        1 => (word >> shift) & 0xFF,
        2 => (word >> shift) & 0xFFFF,
        _ => word,
    }
}

/// Merge a write of `width` bytes at byte offset `off` into a 32-bit register.
pub fn merge(old: u32, off: u32, width: u32, value: u32) -> u32 {
    let shift = (off & 3) * 8;
    match width {
        1 => (old & !(0xFF << shift)) | ((value & 0xFF) << shift),
        2 => (old & !(0xFFFF << shift)) | ((value & 0xFFFF) << shift),
        _ => value,
    }
}

/// Read a little-endian value of `width` bytes at `off` from a byte buffer.
/// Out-of-range reads return 0.
pub fn read_le(bytes: &[u8], off: usize, width: u32) -> u32 {
    let width = width as usize;
    if off + width > bytes.len() {
        return 0;
    }
    bytes[off..off + width]
        .iter()
        .enumerate()
        .fold(0u32, |acc, (i, b)| acc | ((*b as u32) << (8 * i)))
}

/// Write the low `width` bytes of `value` little-endian at `off`.
/// Out-of-range writes are ignored.
pub fn write_le(bytes: &mut [u8], off: usize, width: u32, value: u32) {
    let width = width as usize;
    if off + width > bytes.len() {
        return;
    }
    for i in 0..width {
        bytes[off + i] = (value >> (8 * i)) as u8;
    }
}

/// A block of plain read/write registers for a device that is not emulated
/// yet: writes are remembered and read back, and nothing else happens.
pub struct StubRegs {
    bytes: Vec<u8>,
}

impl StubRegs {
    pub fn new(size: usize) -> Self {
        Self { bytes: vec![0; size] }
    }

    /// Preload a 32-bit register value (builder style).
    pub fn with_u32(mut self, off: usize, value: u32) -> Self {
        write_le(&mut self.bytes, off, 4, value);
        self
    }

    pub fn read(&self, off: u32, width: u32) -> u32 {
        read_le(&self.bytes, off as usize, width)
    }

    pub fn write(&mut self, off: u32, width: u32, value: u32) {
        write_le(&mut self.bytes, off as usize, width, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_and_merge_handle_every_width() {
        let word = 0x4433_2211;
        assert_eq!(extract(word, 0, 4), word);
        assert_eq!(extract(word, 2, 2), 0x4433);
        assert_eq!(extract(word, 1, 1), 0x22);

        assert_eq!(merge(word, 0, 4, 0xAABB_CCDD), 0xAABB_CCDD);
        assert_eq!(merge(word, 2, 2, 0xAABB), 0xAABB_2211);
        assert_eq!(merge(word, 3, 1, 0xEE), 0xEE33_2211);
    }

    #[test]
    fn stub_registers_read_back_what_was_written() {
        let mut regs = StubRegs::new(8).with_u32(4, 0x8004_0000);
        assert_eq!(regs.read(4, 4), 0x8004_0000);
        regs.write(0, 2, 0x1234);
        assert_eq!(regs.read(0, 4), 0x0000_1234);
        assert_eq!(regs.read(1, 1), 0x12);
        // Out of range is harmless.
        regs.write(100, 4, 1);
        assert_eq!(regs.read(100, 4), 0);
    }
}
