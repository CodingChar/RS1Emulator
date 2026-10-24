/// DMA controller.
///
/// Transfers data between main RAM and GPU/SPU/CD-ROM without CPU intervention.
/// Mirrors DuckStation's `dma.cpp` / `dma.h`.
pub struct Dma {
    pub registers: [u32; 16],
}

impl Dma {
    pub fn new() -> Self {
        Self { registers: [0u32; 16] }
    }

    pub fn reset(&mut self) {
        self.registers = [0u32; 16];
    }

    pub fn step(&mut self, _bus: &mut crate::bus::Bus, _memory: &mut crate::memory::Memory) {
        // TODO: process DMA channel transfers (GPU, MDEC, SPU, CD-ROM, OTC).
    }
}

impl Default for Dma {
    fn default() -> Self {
        Self::new()
    }
}
