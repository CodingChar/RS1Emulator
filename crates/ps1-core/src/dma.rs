use crate::io::{extract, merge};

/// Index of the DMA control register (`0x1F8010F0`) in `registers`.
const DPCR: usize = 28;
/// Index of the DMA interrupt register (`0x1F8010F4`) in `registers`.
const DICR: usize = 29;
/// Power-on value of DPCR: channel priorities 0..6 in order.
const DPCR_RESET: u32 = 0x0765_4321;

const DICR_FORCE: u32 = 1 << 15;
const DICR_MASTER_ENABLE: u32 = 1 << 23;
const DICR_WRITABLE: u32 = 0x00FF_FFFF;
const DICR_FLAGS: u32 = 0x7F00_0000;
const DICR_IRQ: u32 = 1 << 31;

/// DMA controller.
///
/// Transfers data between main RAM and GPU/SPU/CD-ROM without CPU intervention.
/// Mirrors DuckStation's `dma.cpp` / `dma.h`.
///
/// Register file at `0x1F801080`: seven channels of `MADR`, `BCR`, `CHCR`
/// (16 bytes each), then `DPCR` and `DICR`. The transfers themselves are not
/// emulated yet.
pub struct Dma {
    /// One 32-bit word per register, indexed by `(address - 0x1F801080) / 4`.
    pub registers: [u32; 32],
}

impl Dma {
    pub fn new() -> Self {
        let mut dma = Self { registers: [0u32; 32] };
        dma.reset();
        dma
    }

    pub fn reset(&mut self) {
        self.registers = [0u32; 32];
        self.registers[DPCR] = DPCR_RESET;
    }

    pub fn step(&mut self, _memory: &mut crate::memory::Memory) {
        // TODO: process DMA channel transfers (GPU, MDEC, SPU, CD-ROM, OTC).
    }

    /// Register read; `off` is relative to `0x1F801080`.
    pub fn read(&self, off: u32, width: u32) -> u32 {
        extract(self.registers[((off >> 2) & 31) as usize], off, width)
    }

    /// Register write; `off` is relative to `0x1F801080`.
    pub fn write(&mut self, off: u32, width: u32, value: u32) {
        let index = ((off >> 2) & 31) as usize;
        let old = self.registers[index];

        if index == DICR {
            // Bits 24-30 are interrupt flags: writing 1 clears them. Bytes
            // outside the access count as 0 so they acknowledge nothing.
            let written = merge(old & DICR_WRITABLE, off, width, value);
            let flags = (old & DICR_FLAGS) & !(written & DICR_FLAGS);
            self.registers[DICR] = Self::with_master_flag((written & DICR_WRITABLE) | flags);
            return;
        }

        let new = merge(old, off, width, value);
        // MADR is a 24-bit address; every other register is stored whole.
        self.registers[index] = if index < DPCR && index % 4 == 0 {
            new & 0x00FF_FFFF
        } else {
            new
        };
    }

    /// Bit 31 of DICR is read-only: force flag, or master enable with any
    /// enabled channel flagged.
    fn with_master_flag(dicr: u32) -> u32 {
        let enabled = (dicr >> 16) & 0x7F;
        let flagged = (dicr >> 24) & 0x7F;
        let irq = dicr & DICR_FORCE != 0
            || (dicr & DICR_MASTER_ENABLE != 0 && enabled & flagged != 0);
        (dicr & !DICR_IRQ) | if irq { DICR_IRQ } else { 0 }
    }
}

impl Default for Dma {
    fn default() -> Self {
        Self::new()
    }
}
