use crate::bus::Bus;
use crate::cpu::interpreter::Interpreter;
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
    /// Cycles remaining in the current video frame (NTSC ~ 3413, PAL ~ 4100).
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
            cycles_per_frame: 3413, // NTSC default
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
