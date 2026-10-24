/// Interrupt controller.
///
/// Aggregates IRQs from DMA, GPU, SPU, CD-ROM, timers, and controllers,
/// then signals the CPU via COP0.
///
/// Mirrors DuckStation's `interrupt_controller.cpp`.
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

    pub fn step(&mut self, _cpu: &mut crate::cpu::interpreter::Interpreter) {
        // TODO: check pending IRQs against mask, set COP0 Cause/Status.
    }

    pub fn raise(&mut self, irq: u32) {
        self.stat |= 1 << irq;
    }

    pub fn acknowledge(&mut self, irq: u32) {
        self.stat &= !(1 << irq);
    }
}

impl Default for InterruptController {
    fn default() -> Self {
        Self::new()
    }
}
