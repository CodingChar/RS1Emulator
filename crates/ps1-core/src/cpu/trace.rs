//! Execution trace, compiled in with the `trace` feature.
//!
//! Keeps the last [`CAPACITY`] executed instructions (address and word) in a
//! ring buffer. With the feature on the interpreter runs every instruction
//! through its exact path, so nothing is skipped; it is much slower and meant
//! for debugging (`examples/boot.rs --trace N`).

/// How many instructions the trace remembers.
pub const CAPACITY: usize = 1 << 14;

pub struct Trace {
    entries: Vec<(u32, u32)>,
    next: usize,
}

impl Default for Trace {
    fn default() -> Self {
        Self { entries: Vec::with_capacity(CAPACITY), next: 0 }
    }
}

impl Trace {
    #[inline]
    pub fn record(&mut self, pc: u32, word: u32) {
        if self.entries.len() < CAPACITY {
            self.entries.push((pc, word));
        } else {
            self.entries[self.next] = (pc, word);
        }
        self.next = (self.next + 1) % CAPACITY;
    }

    /// The remembered instructions, oldest first.
    pub fn entries(&self) -> Vec<(u32, u32)> {
        if self.entries.len() < CAPACITY {
            return self.entries.clone();
        }
        let (newer, older) = self.entries.split_at(self.next);
        older.iter().chain(newer).copied().collect()
    }
}
