/// Save state serialization.
///
/// Serializes the full `System` state so it can be stored in IndexedDB
/// (via `ps1-wasm`) and restored later. Uses `serde` + `bincode`.
///
/// Mirrors DuckStation's `save_state_version.h` concept (versioning added
/// here for forward compatibility).
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SaveState {
    pub version: u32,
    pub cpu_pc: u32,
    pub cpu_next_pc: u32,
    pub cpu_cp0: [u32; 32],
    pub cpu_regs: [u32; 32],
    pub cpu_hi: u32,
    pub cpu_lo: u32,
    pub gpu_regs: [u32; 16],
    pub spu_regs: Vec<u16>,
    pub ram_snapshot: Vec<u8>,       // compressed in the future
    pub vram_snapshot: Vec<u8>,
    pub dma_regs: [u32; 16],
    pub timers: [u32; 3],
    pub interrupt_stat: u32,
    pub interrupt_mask: u32,
    pub cdrom_sector: u32,
    pub controllers: Vec<(u16, i8, i8, i8, i8)>,
}

const SAVE_STATE_VERSION: u32 = 2;

impl SaveState {
    pub const CURRENT_VERSION: u32 = SAVE_STATE_VERSION;

    pub fn from_system(sys: &crate::system::System) -> Self {
        Self {
            version: SAVE_STATE_VERSION,
            cpu_pc: sys.cpu.pc,
            cpu_next_pc: sys.cpu.next_pc,
            cpu_cp0: sys.cpu.cp0,
            cpu_regs: sys.cpu.regs,
            cpu_hi: sys.cpu.hi,
            cpu_lo: sys.cpu.lo,
            gpu_regs: sys.gpu.regs,
            spu_regs: sys.spu.register_file.to_vec(),
            ram_snapshot: sys.memory.ram.as_slice().to_vec(),
            vram_snapshot: sys.memory.vram.clone(),
            dma_regs: sys.dma.registers,
            timers: [sys.timers.timer0, sys.timers.timer1, sys.timers.timer2],
            interrupt_stat: sys.interrupt.stat,
            interrupt_mask: sys.interrupt.mask,
            cdrom_sector: sys.cdrom.current_sector,
            controllers: sys.controllers.ports.iter().map(|p| (
                p.buttons.bits(),
                p.left_stick_x,
                p.left_stick_y,
                p.right_stick_x,
                p.right_stick_y,
            )).collect(),
        }
    }

    pub fn apply_to_system(&self, sys: &mut crate::system::System) {
        if self.version != SAVE_STATE_VERSION {
            // TODO: handle migrations between versions.
            return;
        }
        sys.cpu.set_pc(self.cpu_pc);
        sys.cpu.next_pc = self.cpu_next_pc;
        sys.cpu.cp0 = self.cpu_cp0;
        sys.cpu.regs = self.cpu_regs;
        sys.cpu.hi = self.cpu_hi;
        sys.cpu.lo = self.cpu_lo;
        sys.gpu.regs = self.gpu_regs;
        sys.spu.register_file.copy_from_slice(&self.spu_regs);
        sys.memory.ram.as_mut_slice().copy_from_slice(&self.ram_snapshot);
        sys.memory.vram.copy_from_slice(&self.vram_snapshot);
        sys.dma.registers = self.dma_regs;
        sys.timers.timer0 = self.timers[0];
        sys.timers.timer1 = self.timers[1];
        sys.timers.timer2 = self.timers[2];
        sys.interrupt.stat = self.interrupt_stat;
        sys.interrupt.mask = self.interrupt_mask;
        sys.cdrom.current_sector = self.cdrom_sector;
        for (i, (bits, lx, ly, rx, ry)) in self.controllers.iter().enumerate() {
            if i < 8 {
                sys.controllers.set_input(i, *bits, *lx, *ly, *rx, *ry);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::system::System;

    #[test]
    fn cpu_state_round_trips_through_a_save_state() {
        let mut a = System::new();
        a.cpu.set_pc(0x8000_1000);
        a.cpu.next_pc = 0x8000_2000; // branch in flight
        a.cpu.regs[5] = 0xDEAD_BEEF;
        a.cpu.hi = 1;
        a.cpu.lo = 2;
        a.cpu.cp0[12] = 0x0040_0001;
        a.cpu.cp0[14] = 0x8000_0F00;

        let bytes = a.save_state();
        let mut b = System::new();
        b.load_state(&bytes).unwrap();

        assert_eq!(b.cpu.pc, 0x8000_1000);
        assert_eq!(b.cpu.next_pc, 0x8000_2000);
        assert_eq!(b.cpu.regs[5], 0xDEAD_BEEF);
        assert_eq!((b.cpu.hi, b.cpu.lo), (1, 2));
        assert_eq!(b.cpu.cp0, a.cpu.cp0);
    }
}
