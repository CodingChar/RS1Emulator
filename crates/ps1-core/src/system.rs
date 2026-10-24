use crate::bus::Bus;
use crate::controllers::memory_card::MemoryCard;
use crate::cpu::interpreter::Interpreter;
use crate::gpu::CPU_CYCLES_PER_FRAME;
use crate::save_state::SaveState;
use crate::types::Ps1Result;

/// Top-level PS1 emulation system.
///
/// Owns the CPU and the bus; the bus in turn owns main memory and every
/// memory-mapped device (GPU, SPU, CD-ROM, DMA, timers, interrupt controller,
/// controllers). `System` advances emulation one cycle at a time. This is the
/// moral equivalent of `core.cpp` in DuckStation.
pub struct System {
    pub cpu: Interpreter,
    pub bus: Bus,

    /// Total elapsed emulation cycles since power-on.
    pub cycles: u64,
    /// Cycles remaining in the current video frame.
    pub frame_cycles_remaining: u64,
    /// Target cycles per video frame.
    pub cycles_per_frame: u64,
    pub running: bool,
    pub bios_loaded: bool,
    pub disc_inserted: bool,
}

impl System {
    /// Create a new, uninitialized PS1 system.
    pub fn new() -> Self {
        Self {
            cpu: Interpreter::new(),
            bus: Bus::new(),
            cycles: 0,
            frame_cycles_remaining: 0,
            cycles_per_frame: CPU_CYCLES_PER_FRAME, // NTSC default
            running: false,
            bios_loaded: false,
            disc_inserted: false,
        }
    }

    /// Load the PS1 BIOS ROM (required to boot).
    pub fn load_bios(&mut self, data: &[u8]) -> Ps1Result<()> {
        let bios = &mut self.bus.memory.bios;
        if data.len() > bios.capacity() {
            return Err(crate::types::Ps1Error::Io(
                "BIOS image too large".into(),
            ));
        }
        bios.as_mut_slice()[..data.len()].copy_from_slice(data);
        self.bios_loaded = true;
        Ok(())
    }

    /// Insert a game disc image.
    pub fn insert_disc(&mut self, data: Vec<u8>) -> Ps1Result<()> {
        self.bus.cdrom.insert_image(data)?;
        self.disc_inserted = true;
        Ok(())
    }

    /// Plug a memory card image (exactly 128 KB) into slot 0 or 1.
    pub fn insert_memory_card(&mut self, port: usize, data: Vec<u8>) -> Ps1Result<()> {
        let card = MemoryCard::from_bytes(data)
            .ok_or_else(|| crate::types::Ps1Error::Io("a memory card image must be exactly 128 KB".into()))?;
        self.bus.controllers.insert_memory_card(port, card);
        Ok(())
    }

    /// Plug a freshly formatted, empty memory card into slot 0 or 1.
    pub fn insert_formatted_memory_card(&mut self, port: usize) {
        self.bus.controllers.insert_memory_card(port, MemoryCard::formatted());
    }

    /// The contents of the card in a slot, for saving it.
    pub fn memory_card_data(&self, port: usize) -> Option<Vec<u8>> {
        self.bus.controllers.memory_card(port).map(|card| card.data().to_vec())
    }

    /// Whether the console wrote to the card since `clear_memory_card_dirty`.
    pub fn memory_card_dirty(&self, port: usize) -> bool {
        self.bus.controllers.memory_card(port).is_some_and(|card| card.is_dirty())
    }

    pub fn clear_memory_card_dirty(&mut self, port: usize) {
        if let Some(card) = self.bus.controllers.memory_card_mut(port) {
            card.clear_dirty();
        }
    }

    /// Insert an already-parsed disc (for example one read lazily from a file).
    pub fn insert_disc_image(&mut self, disc: crate::disc::Disc) {
        self.bus.cdrom.insert_disc(disc);
        self.disc_inserted = true;
    }

    /// Power on / reset the system.
    pub fn reset(&mut self) -> Ps1Result<()> {
        if !self.bios_loaded {
            return Err(crate::types::Ps1Error::BiosNotLoaded);
        }
        self.cpu.reset();
        self.bus.reset();
        self.cycles = 0;
        self.frame_cycles_remaining = self.cycles_per_frame;
        self.running = true;
        Ok(())
    }

    /// Advance emulation by one CPU cycle.
    pub fn step(&mut self) -> Ps1Result<()> {
        if !self.running {
            return Ok(());
        }

        // Sample the interrupt controller into COP0 Cause.IP2, then execute
        // one CPU instruction.
        self.cpu.set_irq(self.bus.interrupt.pending());
        self.cpu.step(&mut self.bus)?;

        // Advance DMA, timers, GPU, SPU, CD-ROM in lockstep with CPU cycles.
        self.bus.step_devices();

        self.cycles += 1;
        if self.frame_cycles_remaining > 0 {
            self.frame_cycles_remaining -= 1;
        }

        Ok(())
    }

    /// Run exactly one video frame's worth of cycles.
    pub fn run_frame(&mut self) -> Ps1Result<()> {
        self.frame_cycles_remaining = self.cycles_per_frame;
        while self.frame_cycles_remaining > 0 {
            self.step()?;
        }
        Ok(())
    }

    /// Copy the current GPU framebuffer (RGBA8) for presentation.
    pub fn framebuffer(&self) -> Vec<u8> {
        self.bus.gpu.framebuffer()
    }

    /// Save the entire machine state (for save states).
    pub fn save_state(&self) -> Vec<u8> {
        let state = SaveState::from_system(self);
        bincode::serialize(&state).expect("save state serialization failed")
    }

    /// Load a previously saved machine state.
    pub fn load_state(&mut self, data: &[u8]) -> Ps1Result<()> {
        let state: SaveState = bincode::deserialize(data)
            .map_err(|e| crate::types::Ps1Error::Io(e.to_string()))?;
        state.apply_to_system(self);
        Ok(())
    }

    /// Set input state for a controller port.
    pub fn set_input(
        &mut self,
        port: usize,
        buttons: u16,
        left_stick_x: i8,
        left_stick_y: i8,
        right_stick_x: i8,
        right_stick_y: i8,
    ) {
        self.bus.controllers.set_input(
            port,
            buttons,
            left_stick_x,
            left_stick_y,
            right_stick_x,
            right_stick_y,
        );
    }
}

impl Default for System {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interrupt::IRQ_VBLANK;

    fn word(w: u32) -> [u8; 4] {
        w.to_le_bytes()
    }

    /// A BIOS image whose first instructions are `program`.
    fn bios_with(program: &[u32]) -> Vec<u8> {
        program.iter().flat_map(|w| word(*w)).collect()
    }

    #[test]
    fn reset_requires_a_bios() {
        let mut sys = System::new();
        assert_eq!(sys.reset(), Err(crate::types::Ps1Error::BiosNotLoaded));
    }

    #[test]
    fn program_in_the_bios_programs_the_interrupt_mask() {
        // lui   r1, 0x1F80
        // ori   r1, r1, 0x1074     ; I_MASK
        // addiu r2, r0, 1
        // sw    r2, 0(r1)
        let mut sys = System::new();
        sys.load_bios(&bios_with(&[
            0x3C01_1F80,
            0x3421_1074,
            0x2402_0001,
            0xAC22_0000,
        ]))
        .unwrap();
        sys.reset().unwrap();
        for _ in 0..4 {
            sys.step().unwrap();
        }
        assert_eq!(sys.bus.interrupt.mask, 1);
        assert_eq!(sys.cycles, 4);
    }

    #[test]
    fn pending_unmasked_interrupt_reaches_the_cpu() {
        let mut sys = System::new();
        sys.load_bios(&bios_with(&[0, 0, 0, 0])).unwrap();
        sys.reset().unwrap();
        // Interrupts enabled in COP0 with IM2 unmasked; BEV is set after reset.
        sys.cpu.cp0[12] |= 1 | (1 << 10);
        sys.bus.interrupt.mask = 1 << IRQ_VBLANK;

        // Nothing pending: the CPU keeps running through the BIOS.
        sys.step().unwrap();
        assert_eq!(sys.cpu.pc, 0xBFC0_0004);

        sys.bus.interrupt.raise(IRQ_VBLANK);
        sys.step().unwrap();
        assert_eq!(sys.cpu.pc, 0xBFC0_0180, "BEV exception vector");
        assert_eq!((sys.cpu.cp0[13] >> 2) & 0x1F, 0, "ExcCode = Interrupt");
    }
}
