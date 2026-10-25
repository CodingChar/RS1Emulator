//! Save states: the whole machine as bytes, so it can be stored (in IndexedDB
//! on the web) and restored later.
//!
//! The state is every piece of mutable device state serialized with `serde`
//! and `bincode`, behind a small header (magic, format version, BIOS
//! checksum). What is *not* in it: the BIOS and the disc, which must be the
//! same ones when the state is loaded (the header checks the BIOS; the disc
//! is the caller's business), and the output buffers (audio, frames).
//!
//! `StateRef` (writing, borrows) and `State` (reading, owns) must list the
//! same fields in the same order: bincode has no field names. The round-trip
//! tests catch any mismatch.
//!
//! Mirrors DuckStation's `save_state_version.h` concept.

use serde::{Deserialize, Serialize};

use crate::cdrom::CdRom;
use crate::controllers::sio::Sio0;
use crate::controllers::ControllerManager;
use crate::cpu::interpreter::Interpreter;
use crate::dma::Dma;
use crate::gpu::Gpu;
use crate::interrupt::InterruptController;
use crate::sio1::Sio1;
use crate::mdec::Mdec;
use crate::spu::Spu;
use crate::system::System;
use crate::timers::Timers;
use crate::types::{Ps1Error, Ps1Result};

/// Starts every save state.
const MAGIC: [u8; 4] = *b"RS1S";
/// Bumped whenever the layout of the state changes.
pub const SAVE_STATE_VERSION: u32 = 8;

#[derive(Serialize, Deserialize)]
struct Header {
    magic: [u8; 4],
    version: u32,
    /// A checksum of the BIOS the state was made with: the RAM holds the
    /// kernel it installed, so another BIOS cannot continue from it.
    bios_checksum: u32,
}

#[derive(Serialize)]
struct StateRef<'a> {
    header: Header,
    cpu: &'a Interpreter,
    gpu: &'a Gpu,
    spu: &'a Spu,
    cdrom: &'a CdRom,
    dma: &'a Dma,
    timers: &'a Timers,
    interrupt: &'a InterruptController,
    mdec: &'a Mdec,
    sio0: &'a Sio0,
    sio1: &'a Sio1,
    controllers: &'a ControllerManager,
    ram: &'a [u8],
    scratchpad: &'a [u8],
    mem_control: [u32; 9],
    ram_size: u32,
    cache_control: u32,
    post_code: u8,
    mdec_out_waiting: bool,
    cycles: u64,
    frame_cycles_remaining: u64,
    cycles_per_frame: u64,
    running: bool,
}

#[derive(Deserialize)]
struct State {
    /// Already checked by the early parse of [`Header`]; kept so the layouts match.
    #[allow(dead_code)]
    header: Header,
    cpu: Interpreter,
    gpu: Gpu,
    spu: Spu,
    cdrom: CdRom,
    dma: Dma,
    timers: Timers,
    interrupt: InterruptController,
    mdec: Mdec,
    sio0: Sio0,
    sio1: Sio1,
    controllers: ControllerManager,
    ram: Vec<u8>,
    scratchpad: Vec<u8>,
    mem_control: [u32; 9],
    ram_size: u32,
    cache_control: u32,
    post_code: u8,
    mdec_out_waiting: bool,
    cycles: u64,
    frame_cycles_remaining: u64,
    cycles_per_frame: u64,
    running: bool,
}

/// FNV-1a, enough to tell BIOS images apart.
fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811C_9DC5u32, |hash, byte| (hash ^ *byte as u32).wrapping_mul(0x0100_0193))
}

/// Serialize the machine. The devices are brought up to date first, so the
/// state holds exactly the instant the CPU is at.
pub fn save(sys: &mut System) -> Vec<u8> {
    sys.sync();
    let bus = &sys.bus;
    let state = StateRef {
        header: Header {
            magic: MAGIC,
            version: SAVE_STATE_VERSION,
            bios_checksum: checksum(bus.memory.bios.as_slice()),
        },
        cpu: &sys.cpu,
        gpu: &bus.gpu,
        spu: &bus.spu,
        cdrom: &bus.cdrom,
        dma: &bus.dma,
        timers: &bus.timers,
        interrupt: &bus.interrupt,
        mdec: &bus.mdec,
        sio0: &bus.sio0,
        sio1: &bus.sio1,
        controllers: &bus.controllers,
        ram: bus.memory.ram.as_slice(),
        scratchpad: &bus.scratchpad,
        mem_control: bus.mem_control,
        ram_size: bus.ram_size,
        cache_control: bus.cache_control,
        post_code: bus.post_code,
        mdec_out_waiting: bus.mdec_out_waiting,
        cycles: sys.cycles,
        frame_cycles_remaining: sys.frame_cycles_remaining,
        cycles_per_frame: sys.cycles_per_frame,
        running: sys.running,
    };
    bincode::serialize(&state).expect("save state serialization failed")
}

/// Restore a state made by [`save`]. Nothing changes if it is rejected.
pub fn load(sys: &mut System, data: &[u8]) -> Ps1Result<()> {
    let header: Header = bincode::deserialize(data).map_err(|e| Ps1Error::Io(format!("not a save state: {e}")))?;
    if header.magic != MAGIC {
        return Err(Ps1Error::Io("not a save state".into()));
    }
    if header.version != SAVE_STATE_VERSION {
        return Err(Ps1Error::SaveStateVersionMismatch);
    }
    if header.bios_checksum != checksum(sys.bus.memory.bios.as_slice()) {
        return Err(Ps1Error::Io("the save state was made with a different BIOS".into()));
    }
    let state: State =
        bincode::deserialize(data).map_err(|e| Ps1Error::Io(format!("damaged save state: {e}")))?;
    if state.ram.len() != sys.bus.memory.ram.as_slice().len() || state.scratchpad.len() != sys.bus.scratchpad.len() {
        return Err(Ps1Error::Io("damaged save state: wrong memory size".into()));
    }

    sys.cpu = state.cpu;
    let bus = &mut sys.bus;
    bus.gpu = state.gpu;
    bus.spu = state.spu;
    // The disc stays the one that is inserted.
    let (disc, total_sectors) = (bus.cdrom.disc.take(), bus.cdrom.total_sectors);
    bus.cdrom = state.cdrom;
    bus.cdrom.disc = disc;
    bus.cdrom.total_sectors = total_sectors;
    bus.dma = state.dma;
    bus.timers = state.timers;
    bus.interrupt = state.interrupt;
    bus.mdec = state.mdec;
    bus.sio0 = state.sio0;
    bus.sio1 = state.sio1;
    bus.controllers = state.controllers;
    // The cards come back as they were; make the frontend store them again.
    for port in 0..2 {
        if let Some(card) = bus.controllers.memory_card_mut(port) {
            card.mark_dirty();
        }
    }
    bus.memory.ram.as_mut_slice().copy_from_slice(&state.ram);
    bus.scratchpad.copy_from_slice(&state.scratchpad);
    bus.mem_control = state.mem_control;
    bus.ram_size = state.ram_size;
    bus.cache_control = state.cache_control;
    bus.post_code = state.post_code;
    bus.mdec_out_waiting = state.mdec_out_waiting;
    bus.restart_scheduling();
    sys.cycles = state.cycles;
    sys.frame_cycles_remaining = state.frame_cycles_remaining;
    sys.cycles_per_frame = state.cycles_per_frame;
    sys.running = state.running;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::system::System;

    /// A BIOS that loops forever touching registers, RAM and the scratchpad.
    fn busy_bios() -> Vec<u8> {
        let program: [u32; 12] = [
            0x3C08_8001, // lui   t0, 0x8001
            0x3C09_1F80, // lui   t1, 0x1F80
            0x2508_0001, // loop: addiu t0, t0, 1
            0xAD28_0000 | 0x0100, // sw    t0, 0x100(t1)   scratchpad
            0x8D2A_0100, // lw    t2, 0x100(t1)
            0x3C0B_8000, // lui   t3, 0x8000
            0xAD6A_0200, // sw    t2, 0x200(t3)           RAM
            0x8D6C_0200, // lw    t4, 0x200(t3)
            0x018A_6821, // addu  t5, t4, t2
            0x0800_0000 | (0x1FC0_0008 >> 2 & 0x03FF_FFFF), // j loop
            0,
            0,
        ];
        program.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    fn new_system() -> System {
        let mut sys = System::new();
        sys.load_bios(&busy_bios()).unwrap();
        sys.reset().unwrap();
        sys
    }

    #[test]
    fn a_restored_state_continues_exactly_like_the_original() {
        let mut original = new_system();
        original.run_cycles(100_000).unwrap();
        let bytes = original.save_state();

        original.run_cycles(250_000).unwrap();
        let expected = original.save_state();

        let mut restored = new_system();
        restored.load_state(&bytes).unwrap();
        restored.run_cycles(250_000).unwrap();
        assert_eq!(restored.save_state(), expected, "same bytes after the same run");
    }

    #[test]
    fn saving_does_not_change_the_machine() {
        let mut a = new_system();
        let mut b = new_system();
        a.run_cycles(50_000).unwrap();
        b.run_cycles(50_000).unwrap();
        let _ = a.save_state();
        a.run_cycles(50_000).unwrap();
        b.run_cycles(50_000).unwrap();
        assert_eq!(a.save_state(), b.save_state());
    }

    #[test]
    fn a_state_from_another_bios_is_rejected_and_changes_nothing() {
        let mut a = new_system();
        a.run_cycles(1000).unwrap();
        let bytes = a.save_state();

        let mut other = System::new();
        other.load_bios(&[0x34, 0x12, 0x00, 0x00]).unwrap();
        other.reset().unwrap();
        other.run_cycles(10).unwrap();
        let before = other.save_state();
        assert!(other.load_state(&bytes).is_err());
        assert_eq!(other.save_state(), before);
    }

    #[test]
    fn garbage_and_old_versions_are_rejected() {
        let mut sys = new_system();
        assert!(sys.load_state(b"definitely not a state").is_err());
        assert!(sys.load_state(&[]).is_err());
        let mut bytes = sys.save_state();
        bytes[4] = 5; // another format version
        assert_eq!(sys.load_state(&bytes), Err(crate::types::Ps1Error::SaveStateVersionMismatch));
        let truncated = &sys.save_state()[..1000];
        assert!(sys.load_state(truncated).is_err());
    }

    #[test]
    fn memory_cards_travel_with_the_state_and_are_marked_for_saving() {
        let mut a = new_system();
        a.insert_formatted_memory_card(0);
        let bytes = a.save_state();
        let mut b = new_system();
        b.load_state(&bytes).unwrap();
        assert_eq!(b.memory_card_data(0), a.memory_card_data(0));
        assert!(b.memory_card_dirty(0));
    }
}
