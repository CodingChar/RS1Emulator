/// System timers (Timer 0, 1, 2) and timing events.
///
/// Mirrors DuckStation's `timers.cpp` / `timing_event.cpp`.
pub struct Timers {
    pub timer0: u32,
    pub timer1: u32,
    pub timer2: u32,
    pub mode: u32,
}

impl Timers {
    pub fn new() -> Self {
        Self { timer0: 0, timer1: 0, timer2: 0, mode: 0 }
    }

    pub fn reset(&mut self) {
        self.timer0 = 0;
        self.timer1 = 0;
        self.timer2 = 0;
        self.mode = 0;
    }

    pub fn step(&mut self, _interrupt: &mut crate::interrupt::InterruptController) {
        // TODO: decrement timers, raise IRQs on overflow.
    }
}

impl Default for Timers {
    fn default() -> Self {
        Self::new()
    }
}
