use crate::interrupt::{InterruptController, IRQ_TIMER0};
use crate::io::{extract, merge};

// Mode register bits.
const MODE_SYNC_ENABLE: u32 = 1 << 0;
const MODE_SYNC_MODE_SHIFT: u32 = 1;
const MODE_RESET_AT_TARGET: u32 = 1 << 3;
const MODE_IRQ_AT_TARGET: u32 = 1 << 4;
const MODE_IRQ_AT_OVERFLOW: u32 = 1 << 5;
const MODE_IRQ_REPEAT: u32 = 1 << 6;
const MODE_IRQ_TOGGLE: u32 = 1 << 7;
const MODE_CLOCK_SOURCE_SHIFT: u32 = 8;
/// "No IRQ requested": set after any write to the mode register.
const MODE_IRQ_OFF: u32 = 1 << 10;
const MODE_REACHED_TARGET: u32 = 1 << 11;
const MODE_REACHED_OVERFLOW: u32 = 1 << 12;
/// Reached flags, cleared on read.
const MODE_REACHED_FLAGS: u32 = MODE_REACHED_TARGET | MODE_REACHED_OVERFLOW;
/// Bits the CPU can write (the rest is status).
const MODE_WRITABLE: u32 = 0x3FF;

/// The counters are 16 bits wide.
const COUNTER_OVERFLOW: u32 = 0x1_0000;

/// Lower bounds on the CPU cycles between two pulses of each clock source,
/// used to schedule the next possible timer event.
const MIN_CYCLES_PER_DOT: u32 = 2;
const MIN_CYCLES_PER_HBLANK: u32 = 2000;

/// Clock pulses elapsed since the previous call to [`Timers::tick`].
#[derive(Clone, Copy, Debug, Default)]
pub struct TimerClocks {
    /// System clock (CPU cycles).
    pub sysclk: u32,
    /// Pixel clock from the GPU.
    pub dotclock: u32,
    /// Horizontal blanks (one per scanline).
    pub hblank: u32,
}

/// System timers (Timer 0, 1, 2) and timing events.
///
/// Registers at `0x1F801100` (timer 0), `0x1F801110` (timer 1) and
/// `0x1F801120` (timer 2): counter, mode and target, 16 bytes per timer.
///
/// Clock sources: timer 0 sysclk or dot clock, timer 1 sysclk or hblank,
/// timer 2 sysclk or sysclk/8. Synchronisation with the blanking intervals
/// is only emulated for timer 2 (modes 0 and 3 stop the counter); timers 0
/// and 1 free-run regardless of their sync setting.
///
/// Mirrors DuckStation's `timers.cpp` / `timing_event.cpp`.
pub struct Timers {
    pub counter: [u32; 3],
    pub mode: [u32; 3],
    pub target: [u32; 3],
    /// Remainder of the sysclk/8 division for timer 2.
    div8_remainder: u32,
    /// With one-shot IRQs, whether the timer already fired since the last
    /// mode write.
    irq_done: [bool; 3],
}

impl Timers {
    pub fn new() -> Self {
        Self {
            counter: [0; 3],
            mode: [0; 3],
            target: [0; 3],
            div8_remainder: 0,
            irq_done: [false; 3],
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Whether the timer is held still by its sync mode.
    fn is_halted(&self, timer: usize) -> bool {
        let mode = self.mode[timer];
        let sync_mode = (mode >> MODE_SYNC_MODE_SHIFT) & 3;
        timer == 2 && mode & MODE_SYNC_ENABLE != 0 && (sync_mode == 0 || sync_mode == 3)
    }

    /// Pulses until the counter next reaches its target or overflows (at least 1).
    fn ticks_until_event(&self, timer: usize) -> u32 {
        let counter = self.counter[timer];
        let mut ticks = COUNTER_OVERFLOW - counter;
        if self.target[timer] > counter {
            ticks = ticks.min(self.target[timer] - counter);
        }
        ticks.max(1)
    }

    /// A lower bound on the CPU cycles until a timer reaches its target or
    /// overflows (`u32::MAX` when none can).
    pub fn cycles_until_event(&self) -> u32 {
        let mut next = u32::MAX;
        for timer in 0..3 {
            if self.is_halted(timer) {
                continue;
            }
            let source = (self.mode[timer] >> MODE_CLOCK_SOURCE_SHIFT) & 3;
            let cycles_per_pulse = match timer {
                0 if source & 1 != 0 => MIN_CYCLES_PER_DOT,
                1 if source & 1 != 0 => MIN_CYCLES_PER_HBLANK,
                2 if source & 2 != 0 => 8,
                _ => 1,
            };
            next = next.min((self.ticks_until_event(timer) - 1) * cycles_per_pulse);
        }
        next
    }

    /// Advance all timers by the given clock pulses.
    pub fn tick(&mut self, clocks: TimerClocks, interrupt: &mut InterruptController) {
        let sysclk_div8 = {
            self.div8_remainder += clocks.sysclk;
            let ticks = self.div8_remainder / 8;
            self.div8_remainder %= 8;
            ticks
        };

        for timer in 0..3 {
            let mode = self.mode[timer];
            let source = (mode >> MODE_CLOCK_SOURCE_SHIFT) & 3;
            let ticks = match timer {
                0 if source & 1 != 0 => clocks.dotclock,
                1 if source & 1 != 0 => clocks.hblank,
                2 if source & 2 != 0 => sysclk_div8,
                _ => clocks.sysclk,
            };

            // Timer 2 sync modes 0 and 3 halt the counter.
            if self.is_halted(timer) {
                continue;
            }

            // Far from the target and the overflow nothing happens: add up.
            if ticks < self.ticks_until_event(timer) {
                self.counter[timer] += ticks;
                continue;
            }

            for _ in 0..ticks {
                self.count(timer, interrupt);
            }
        }
    }

    /// Count one pulse on a timer.
    fn count(&mut self, timer: usize, interrupt: &mut InterruptController) {
        let mode = self.mode[timer];
        let mut counter = self.counter[timer] + 1;
        let mut fire = false;

        if counter >= COUNTER_OVERFLOW {
            counter = 0;
            self.mode[timer] |= MODE_REACHED_OVERFLOW;
            fire |= mode & MODE_IRQ_AT_OVERFLOW != 0;
        } else if counter == self.target[timer] {
            self.mode[timer] |= MODE_REACHED_TARGET;
            fire |= mode & MODE_IRQ_AT_TARGET != 0;
            if mode & MODE_RESET_AT_TARGET != 0 {
                counter = 0;
            }
        }
        self.counter[timer] = counter;

        if fire {
            self.request_irq(timer, interrupt);
        }
    }

    fn request_irq(&mut self, timer: usize, interrupt: &mut InterruptController) {
        let mode = self.mode[timer];
        // One-shot timers only fire once per mode write.
        if mode & MODE_IRQ_REPEAT == 0 && self.irq_done[timer] {
            return;
        }
        self.irq_done[timer] = true;

        if mode & MODE_IRQ_TOGGLE != 0 {
            // Toggle mode: the request line flips; the IRQ fires when it drops.
            self.mode[timer] ^= MODE_IRQ_OFF;
            if self.mode[timer] & MODE_IRQ_OFF == 0 {
                interrupt.raise(IRQ_TIMER0 + timer as u32);
            }
        } else {
            // Pulse mode: the request line only dips briefly, so it reads
            // back as "no request".
            interrupt.raise(IRQ_TIMER0 + timer as u32);
        }
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
                let new = merge(old, off, width, value) & MODE_WRITABLE;
                self.mode[timer] = new | MODE_IRQ_OFF | (old & MODE_REACHED_FLAGS);
                // Writing the mode restarts the counter and re-arms one-shot IRQs.
                self.counter[timer] = 0;
                self.irq_done[timer] = false;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interrupt::{IRQ_TIMER1, IRQ_TIMER2};

    fn sys(n: u32) -> TimerClocks {
        TimerClocks { sysclk: n, ..Default::default() }
    }

    /// Program a timer through its registers: (mode, target).
    fn setup(timers: &mut Timers, timer: u32, mode: u32, target: u32) {
        timers.write(timer * 16 + 8, 4, target);
        timers.write(timer * 16 + 4, 4, mode);
    }

    #[test]
    fn free_running_counter_follows_sysclk_and_wraps() {
        let mut timers = Timers::new();
        let mut irq = InterruptController::new();
        timers.tick(sys(100), &mut irq);
        assert_eq!(timers.read(0, 4), 100);

        timers.tick(sys(0x1_0000 - 100), &mut irq);
        assert_eq!(timers.read(0, 4), 0, "wrapped at 0x10000");
        assert_ne!(timers.read(4, 4) & MODE_REACHED_OVERFLOW, 0);
        assert_eq!(irq.stat, 0, "no IRQ unless enabled");
    }

    #[test]
    fn counter_resets_at_target_when_asked() {
        let mut timers = Timers::new();
        let mut irq = InterruptController::new();
        setup(&mut timers, 0, MODE_RESET_AT_TARGET, 10);
        timers.tick(sys(25), &mut irq);
        assert_eq!(timers.read(0, 4), 5);
        assert_ne!(timers.read(4, 4) & MODE_REACHED_TARGET, 0);
        assert_eq!(timers.read(4, 4) & MODE_REACHED_TARGET, 0, "flag clears on read");
    }

    #[test]
    fn counter_keeps_going_past_target_without_reset() {
        let mut timers = Timers::new();
        let mut irq = InterruptController::new();
        setup(&mut timers, 0, 0, 10);
        timers.tick(sys(25), &mut irq);
        assert_eq!(timers.read(0, 4), 25);
    }

    #[test]
    fn target_irq_is_one_shot_unless_repeat_is_set() {
        let mut timers = Timers::new();
        let mut irq = InterruptController::new();
        setup(&mut timers, 0, MODE_RESET_AT_TARGET | MODE_IRQ_AT_TARGET, 10);
        timers.tick(sys(10), &mut irq);
        assert_eq!(irq.stat, 1 << IRQ_TIMER0);

        irq.stat = 0;
        timers.tick(sys(30), &mut irq);
        assert_eq!(irq.stat, 0, "one-shot: no second IRQ");

        // Rewriting the mode re-arms it.
        setup(&mut timers, 0, MODE_RESET_AT_TARGET | MODE_IRQ_AT_TARGET, 10);
        timers.tick(sys(10), &mut irq);
        assert_eq!(irq.stat, 1 << IRQ_TIMER0);
    }

    #[test]
    fn repeat_mode_fires_on_every_target() {
        let mut timers = Timers::new();
        let mut irq = InterruptController::new();
        setup(
            &mut timers,
            1,
            MODE_RESET_AT_TARGET | MODE_IRQ_AT_TARGET | MODE_IRQ_REPEAT,
            10,
        );
        timers.tick(sys(10), &mut irq);
        assert_eq!(irq.stat, 1 << IRQ_TIMER1);
        irq.stat = 0;
        timers.tick(sys(10), &mut irq);
        assert_eq!(irq.stat, 1 << IRQ_TIMER1);
    }

    #[test]
    fn overflow_irq() {
        let mut timers = Timers::new();
        let mut irq = InterruptController::new();
        setup(&mut timers, 2, MODE_IRQ_AT_OVERFLOW, 0xFFFF);
        timers.tick(sys(0xFFFF), &mut irq);
        assert_eq!(irq.stat, 0);
        timers.tick(sys(1), &mut irq);
        assert_eq!(irq.stat, 1 << IRQ_TIMER2);
    }

    #[test]
    fn toggle_mode_fires_on_alternate_events() {
        let mut timers = Timers::new();
        let mut irq = InterruptController::new();
        setup(
            &mut timers,
            0,
            MODE_RESET_AT_TARGET | MODE_IRQ_AT_TARGET | MODE_IRQ_REPEAT | MODE_IRQ_TOGGLE,
            10,
        );
        // The line starts high ("no request"); the first event drops it.
        timers.tick(sys(10), &mut irq);
        assert_eq!(irq.stat, 1 << IRQ_TIMER0);
        irq.stat = 0;
        timers.tick(sys(10), &mut irq); // line goes back up: no IRQ
        assert_eq!(irq.stat, 0);
        timers.tick(sys(10), &mut irq);
        assert_eq!(irq.stat, 1 << IRQ_TIMER0);
    }

    #[test]
    fn timer0_can_count_the_dot_clock() {
        let mut timers = Timers::new();
        let mut irq = InterruptController::new();
        setup(&mut timers, 0, 1 << MODE_CLOCK_SOURCE_SHIFT, 0);
        let clocks = TimerClocks { sysclk: 1000, dotclock: 7, hblank: 3 };
        timers.tick(clocks, &mut irq);
        assert_eq!(timers.read(0, 4), 7);
    }

    #[test]
    fn timer1_can_count_hblanks() {
        let mut timers = Timers::new();
        let mut irq = InterruptController::new();
        setup(&mut timers, 1, 1 << MODE_CLOCK_SOURCE_SHIFT, 0);
        let clocks = TimerClocks { sysclk: 1000, dotclock: 7, hblank: 3 };
        timers.tick(clocks, &mut irq);
        assert_eq!(timers.read(16, 4), 3);
    }

    #[test]
    fn timer2_can_count_sysclk_divided_by_eight() {
        let mut timers = Timers::new();
        let mut irq = InterruptController::new();
        setup(&mut timers, 2, 2 << MODE_CLOCK_SOURCE_SHIFT, 0);
        timers.tick(sys(80), &mut irq);
        assert_eq!(timers.read(32, 4), 10);
        // The division remainder carries over between calls.
        timers.tick(sys(5), &mut irq);
        timers.tick(sys(3), &mut irq);
        assert_eq!(timers.read(32, 4), 11);
    }

    #[test]
    fn timer2_sync_modes_0_and_3_halt_the_counter() {
        for sync_mode in [0, 3] {
            let mut timers = Timers::new();
            let mut irq = InterruptController::new();
            setup(&mut timers, 2, MODE_SYNC_ENABLE | (sync_mode << MODE_SYNC_MODE_SHIFT), 0);
            timers.tick(sys(100), &mut irq);
            assert_eq!(timers.read(32, 4), 0, "sync mode {sync_mode}");
        }
        // Modes 1 and 2 free-run.
        let mut timers = Timers::new();
        let mut irq = InterruptController::new();
        setup(&mut timers, 2, MODE_SYNC_ENABLE | (1 << MODE_SYNC_MODE_SHIFT), 0);
        timers.tick(sys(100), &mut irq);
        assert_eq!(timers.read(32, 4), 100);
    }

    #[test]
    fn register_reads_and_writes() {
        let mut timers = Timers::new();
        timers.write(0x18, 4, 0x1_2345); // timer 1 target: 16 bits only
        assert_eq!(timers.read(0x18, 4), 0x2345);
        timers.write(0x10, 4, 0x77);
        timers.write(0x14, 4, 0x158);
        assert_eq!(timers.read(0x10, 4), 0, "mode write restarts the counter");
        assert_eq!(timers.read(0x14, 4) & 0x7FF, 0x158 | MODE_IRQ_OFF);
        assert_eq!(timers.read(0x30, 4), 0, "no fourth timer");
    }
}
