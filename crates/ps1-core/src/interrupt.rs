use serde::{Deserialize, Serialize};
use crate::io::{extract, merge};

/// Interrupt sources, as bit numbers in `I_STAT` / `I_MASK`.
pub const IRQ_VBLANK: u32 = 0;
pub const IRQ_GPU: u32 = 1;
pub const IRQ_CDROM: u32 = 2;
pub const IRQ_DMA: u32 = 3;
pub const IRQ_TIMER0: u32 = 4;
pub const IRQ_TIMER1: u32 = 5;
pub const IRQ_TIMER2: u32 = 6;
pub const IRQ_CONTROLLER: u32 = 7;
pub const IRQ_SIO: u32 = 8;
pub const IRQ_SPU: u32 = 9;
pub const IRQ_LIGHTPEN: u32 = 10;

/// Only 11 interrupt lines exist.
const IRQ_MASK: u32 = 0x7FF;

/// Interrupt controller.
///
/// Aggregates IRQs from DMA, GPU, SPU, CD-ROM, timers, and controllers,
/// then signals the CPU via COP0.
///
/// Registers: `I_STAT` at `0x1F801070`, `I_MASK` at `0x1F801074`.
///
/// Mirrors DuckStation's `interrupt_controller.cpp`.
#[derive(Serialize, Deserialize)]
pub struct InterruptController {
    pub stat: u32,
    pub mask: u32,
}

impl InterruptController {
    pub fn new() -> Self {
        Self { stat: 0, mask: 0 }
    }

    pub fn reset(&mut self) {
        self.stat = 0;
        self.mask = 0;
    }

    /// Whether any unmasked interrupt is pending (drives COP0 Cause.IP2).
    pub fn pending(&self) -> bool {
        self.stat & self.mask != 0
    }

    pub fn raise(&mut self, irq: u32) {
        self.stat |= 1 << irq;
    }

    pub fn acknowledge(&mut self, irq: u32) {
        self.stat &= !(1 << irq);
    }

    /// Register read; `off` is relative to `0x1F801070`.
    pub fn read(&self, off: u32, width: u32) -> u32 {
        let word = match off & !3 {
            0 => self.stat,
            4 => self.mask,
            _ => 0,
        };
        extract(word, off, width)
    }

    /// Register write; `off` is relative to `0x1F801070`.
    pub fn write(&mut self, off: u32, width: u32, value: u32) {
        match off & !3 {
            // I_STAT: a 0 bit acknowledges the interrupt, a 1 bit leaves it
            // alone. Bytes not covered by the access are left untouched.
            0 => self.stat &= merge(0xFFFF_FFFF, off, width, value),
            4 => self.mask = merge(self.mask, off, width, value) & IRQ_MASK,
            _ => {}
        }
    }
}

impl Default for InterruptController {
    fn default() -> Self {
        Self::new()
    }
}
