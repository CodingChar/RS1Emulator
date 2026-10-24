pub mod ram;
pub mod bios;

/// Main PS1 memory map.
///
/// The PS1 has:
/// - 2 MB main RAM
/// - 512 KB BIOS ROM (mirrored)
/// - 1 KB scratchpad
/// - 1 MB VRAM (GPU)
/// - 512 KB SPU sound RAM
pub struct Memory {
    pub ram: ram::Ram,
    pub bios: bios::Bios,
    pub vram: Vec<u8>,
    pub spu_ram: Vec<u8>,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            ram: ram::Ram::new(),
            bios: bios::Bios::new(),
            vram: vec![0u8; 1024 * 1024], // 1 MB VRAM
            spu_ram: vec![0u8; 512 * 1024], // 512 KB SPU RAM
        }
    }

    pub fn reset(&mut self) {
        self.ram.reset();
        self.vram.fill(0);
        self.spu_ram.fill(0);
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}
