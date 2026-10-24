use crate::io::{extract, merge};

/// Mode bit 10: "no IRQ requested" (set after any write to the mode register).
const MODE_IRQ_OFF: u32 = 1 << 10;
/// Mode bits 11 and 12: reached-target / reached-0xFFFF, cleared on read.
const MODE_REACHED_FLAGS: u32 = 0x1800;

/// System timers (Timer 0, 1, 2) and timing events.
///
/// Registers at `0x1F801100` (timer 0), `0x1F801110` (timer 1) and
/// `0x1F801120` (timer 2): counter, mode and target, 16 bytes per timer.
/// The counters do not advance yet.
///
/// Mirrors DuckStation's `timers.cpp` / `timing_event.cpp`.
pub struct Timers {
    pub counter: [u32; 3],
    pub mode: [u32; 3],
    pub target: [u32; 3],
}

impl Timers {
    pub fn new() -> Self {
        Self { counter: [0; 3], mode: [0; 3], target: [0; 3] }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn step(&mut self, _interrupt: &mut crate::interrupt::InterruptController) {
        // TODO: decrement timers, raise IRQs on overflow.
    }

    /// Register read; `off` is relative to `0x1F801100`.
    pub fn read(&mut self, off: u32, width: u32) -> u32 {
        let timer = (off >> 4) as usize;
        if timer >= 3 {
            return 0;
        }
        let word = match (off >> 2) & 3 {
            0 => self.counter[timer],
            1 => {
                let mode = self.mode[timer];
                self.mode[timer] &= !MODE_REACHED_FLAGS;
                mode
            }
            2 => self.target[timer],
            _ => 0,
        };
        extract(word, off, width)
    }

    /// Register write; `off` is relative to `0x1F801100`.
    pub fn write(&mut self, off: u32, width: u32, value: u32) {
        let timer = (off >> 4) as usize;
        if timer >= 3 {
            return;
        }
        match (off >> 2) & 3 {
            0 => self.counter[timer] = merge(self.counter[timer], off, width, value) & 0xFFFF,
            1 => {
                let old = self.mode[timer];
                let new = merge(old, off, width, value) & 0x3FF;
                self.mode[timer] = new | MODE_IRQ_OFF | (old & MODE_REACHED_FLAGS);
                // Writing the mode restarts the counter.
                self.counter[timer] = 0;
            }
            2 => self.target[timer] = merge(self.target[timer], off, width, value) & 0xFFFF,
            _ => {}
        }
    }
}

impl Default for Timers {
    fn default() -> Self {
        Self::new()
    }
}
