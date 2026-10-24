pub mod ram;
pub mod bios;

/// Main PS1 memory map.
///
/// The PS1 has:
/// - 2 MB main RAM
/// - 512 KB BIOS ROM (mirrored)
/// - 1 KB scratchpad
///
/// VRAM lives in the GPU and sound RAM in the SPU.
pub struct Memory {
    pub ram: ram::Ram,
    pub bios: bios::Bios,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            ram: ram::Ram::new(),
            bios: bios::Bios::new(),
        }
    }

    pub fn reset(&mut self) {
        self.ram.reset();
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}
