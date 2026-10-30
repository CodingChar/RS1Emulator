//! The interpreter's fast path.
//!
//! [`Interpreter::run_fast`] executes the common instructions (ALU, branches,
//! jumps, and loads/stores to RAM, scratchpad and BIOS) in one tight loop with
//! the program counter and the delay-slot state held in local variables, and
//! the cycle bookkeeping done once per batch instead of once per instruction.
//!
//! It never raises an exception and never touches a device. Whenever an
//! instruction needs more than that (COP0/COP2, unaligned loads, I/O
//! registers, overflow, anything faulting) it stops *before* committing any
//! state, and the ordinary [`Interpreter::step_inner`] executes that one
//! instruction. The two paths therefore give identical results; the
//! checksum tests in `examples/bench.rs` check that over billions of
//! instructions.

use super::{multiply_latency, Interpreter, COP0_SR, DIVIDE_LATENCY, RAM_READ_CYCLES, SR_CU2, SR_ISC, SR_KUC};
use crate::bus::Bus;
use crate::gte::Gte;
use crate::io::{read_le, write_le};

const SCRATCHPAD_START: u32 = 0x1F80_0000;
const SCRATCHPAD_SIZE: u32 = 0x400;
const BIOS_START: u32 = 0x1FC0_0000;
const BIOS_SIZE: u32 = 0x8_0000;
const RAM_MIRROR_END: u32 = 0x80_0000;

#[inline(always)]
fn sign_extend16(imm: u32) -> u32 {
    imm as u16 as i16 as i32 as u32
}

impl Interpreter {
    /// Run up to `limit` instructions of the common kinds. Returns how many
    /// were executed; fewer than `limit` means the next instruction needs the
    /// slow path (it has not been started).
    ///
    /// The caller guarantees no interrupt is pending and that this CPU's
    /// delayed-load slot is empty (`pending_load.0 == 0`), which holds between
    /// instructions.
    // The state macros assign locals that the last instruction of a batch
    // never reads again; that is intended.
    #[allow(unused_assignments)]
    #[inline(never)]
    pub(super) fn run_fast<const BREAKPOINT: bool>(&mut self, bus: &mut Bus, limit: u32, break_pc: u32) -> u32 {
        let sr = self.cp0[COP0_SR];
        let user_mode = sr & SR_KUC != 0;
        let cache_isolated = sr & SR_ISC != 0;
        let cop2_usable = sr & SR_CU2 != 0;
        // PGXP tags follow words through registers and memory (see `crate::pgxp`).
        let pgxp = bus.pgxp.enabled();

        let mut pc = self.pc;
        let mut next_pc = self.next_pc;
        let mut branch_pending = self.branch_pending;
        // Load landing after the instruction being executed (register 0: none).
        let mut load_reg = 0usize;
        let mut load_value = 0u32;
        // Load issued by the instruction being executed.
        let (mut next_load_reg, mut next_load_value) = self.next_load;
        let mut count = 0u32;
        // Stall cycles added on top of one per instruction, and the cycle (from the
        // start of the batch) at which the multiply/divide unit has its result.
        let mut extra = 0u32;
        let mut busy_until = self.hilo_busy;
        let mut gte_until = self.gte_busy;
        // The 4 KB page of code being fetched from: translated once, not for
        // every instruction. `u32::MAX` never matches a page address.
        let mut code_page = u32::MAX;
        let mut code_phys = 0usize;
        let mut code_in_bios = false;
        // A wait-for-the-GPU loop seen once (see `poll_loop`): where it starts, when this
        // iteration began, and the registers it reads the status into and the mask from.
        let mut poll_head = u32::MAX;
        let mut poll_start = 0u32;
        let mut poll_regs = (0usize, 0usize);

        // Write a register now, cancelling a delayed load to it.
        macro_rules! set {
            ($reg:expr, $value:expr) => {{
                let reg = $reg;
                let value = $value;
                if reg != 0 {
                    if load_reg == reg {
                        load_reg = 0;
                    }
                    self.regs[reg] = value;
                }
            }};
        }
        // Issue a load: the register changes after the next instruction.
        macro_rules! load {
            ($reg:expr, $value:expr) => {{
                let reg = $reg;
                let value = $value;
                if reg != 0 {
                    if load_reg == reg {
                        load_reg = 0;
                    }
                    next_load_reg = reg;
                    next_load_value = value;
                }
            }};
        }
        // Start executing the instruction: nothing can fail from here on.
        macro_rules! commit {
            () => {{
                load_reg = next_load_reg;
                load_value = next_load_value;
                next_load_reg = 0;
                pc = next_pc;
                next_pc = next_pc.wrapping_add(4);
                branch_pending = false;
            }};
        }
        // The branch target is relative to the delay slot, which `pc` is now.
        macro_rules! branch_if {
            ($cond:expr, $imm:expr) => {{
                if $cond {
                    next_pc = pc.wrapping_add(sign_extend16($imm) << 2);
                }
                branch_pending = true;
            }};
        }
        macro_rules! jump {
            ($target:expr) => {{
                next_pc = $target;
                branch_pending = true;
            }};
        }
        // Reading HI/LO before the multiply/divide unit is done costs the wait. It is
        // cut short at the end of the batch (`limit`) so no device event is skipped.
        macro_rules! stall_for_hilo {
            () => {{
                let now = count + extra;
                if busy_until > now {
                    extra += (busy_until - now).min(limit - now - 1);
                }
                busy_until = 0;
            }};
        }
        // The same for the GTE: touching it before its command is done costs the wait.
        macro_rules! stall_for_gte {
            () => {{
                let now = count + extra;
                if gte_until > now {
                    extra += (gte_until - now).min(limit - now - 1);
                }
                gte_until = 0;
            }};
        }
        // A data read from main RAM waits for the DRAM (the exact path charges the same).
        macro_rules! charge_ram_read {
            ($phys:expr) => {{
                if $phys < RAM_MIRROR_END {
                    extra += RAM_READ_CYCLES;
                }
            }};
        }
        // Where a data address lives, or stop and let the slow path handle it.
        macro_rules! data_address {
            ($addr:expr, $align:expr) => {{
                let addr: u32 = $addr;
                if addr & $align != 0 || (user_mode && addr & 0x8000_0000 != 0) {
                    break;
                }
                Bus::physical(addr)
            }};
        }

        while count + extra < limit {
            if BREAKPOINT && pc == break_pc {
                break;
            }
            // ---- fetch
            if pc & 3 != 0 {
                break;
            }
            if pc & !0xFFF != code_page {
                if user_mode && pc & 0x8000_0000 != 0 {
                    break;
                }
                let phys = Bus::physical(pc);
                if phys < RAM_MIRROR_END {
                    code_phys = (phys & !0xFFF) as usize;
                    code_in_bios = false;
                } else if phys.wrapping_sub(BIOS_START) < BIOS_SIZE {
                    code_phys = ((phys - BIOS_START) & !0xFFF) as usize;
                    code_in_bios = true;
                } else {
                    break;
                }
                code_page = pc & !0xFFF;
            }
            let offset = code_phys | (pc & 0xFFF) as usize;
            let instr = if code_in_bios {
                bus.memory.bios.read32(offset)
            } else {
                bus.memory.ram.read32(offset)
            };

            let rs = ((instr >> 21) & 0x1F) as usize;
            let rt = ((instr >> 16) & 0x1F) as usize;
            let imm = instr & 0xFFFF;
            let simm = sign_extend16(imm);

            match instr >> 26 {
                0x00 => {
                    let rd = ((instr >> 11) & 0x1F) as usize;
                    let shamt = (instr >> 6) & 0x1F;
                    let (a, b) = (self.regs[rs], self.regs[rt]);
                    match instr & 0x3F {
                        0x00 => {
                            commit!();
                            set!(rd, b << shamt);
                        }
                        0x02 => {
                            commit!();
                            set!(rd, b >> shamt);
                        }
                        0x03 => {
                            commit!();
                            set!(rd, ((b as i32) >> shamt) as u32);
                        }
                        0x04 => {
                            commit!();
                            set!(rd, b << (a & 0x1F));
                        }
                        0x06 => {
                            commit!();
                            set!(rd, b >> (a & 0x1F));
                        }
                        0x07 => {
                            commit!();
                            set!(rd, ((b as i32) >> (a & 0x1F)) as u32);
                        }
                        0x08 => {
                            commit!();
                            jump!(a);
                        }
                        0x09 => {
                            commit!();
                            let ret = next_pc;
                            set!(rd, ret);
                            jump!(a);
                        }
                        0x10 => {
                            commit!();
                            stall_for_hilo!();
                            set!(rd, self.hi);
                        }
                        0x11 => {
                            commit!();
                            self.hi = a;
                        }
                        0x12 => {
                            commit!();
                            stall_for_hilo!();
                            set!(rd, self.lo);
                        }
                        0x13 => {
                            commit!();
                            self.lo = a;
                        }
                        0x18 => {
                            commit!();
                            let prod = (a as i32 as i64) * (b as i32 as i64);
                            self.lo = prod as u32;
                            self.hi = (prod >> 32) as u32;
                            busy_until = count + extra + multiply_latency(a, true);
                        }
                        0x19 => {
                            commit!();
                            let prod = (a as u64) * (b as u64);
                            self.lo = prod as u32;
                            self.hi = (prod >> 32) as u32;
                            busy_until = count + extra + multiply_latency(a, false);
                        }
                        0x1A => {
                            commit!();
                            self.divide_signed(a as i32, b as i32);
                            busy_until = count + extra + DIVIDE_LATENCY;
                        }
                        0x1B => {
                            commit!();
                            self.divide_unsigned(a, b);
                            busy_until = count + extra + DIVIDE_LATENCY;
                        }
                        0x20 => {
                            let Some(value) = (a as i32).checked_add(b as i32) else { break };
                            commit!();
                            set!(rd, value as u32);
                        }
                        0x21 => {
                            commit!();
                            set!(rd, a.wrapping_add(b));
                        }
                        0x22 => {
                            let Some(value) = (a as i32).checked_sub(b as i32) else { break };
                            commit!();
                            set!(rd, value as u32);
                        }
                        0x23 => {
                            commit!();
                            set!(rd, a.wrapping_sub(b));
                        }
                        0x24 => {
                            commit!();
                            set!(rd, a & b);
                        }
                        0x25 => {
                            commit!();
                            set!(rd, a | b);
                        }
                        0x26 => {
                            commit!();
                            set!(rd, a ^ b);
                        }
                        0x27 => {
                            commit!();
                            set!(rd, !(a | b));
                        }
                        0x2A => {
                            commit!();
                            set!(rd, ((a as i32) < (b as i32)) as u32);
                        }
                        0x2B => {
                            commit!();
                            set!(rd, (a < b) as u32);
                        }
                        // SYSCALL, BREAK and reserved functions raise exceptions.
                        _ => break,
                    }
                }
                0x01 => {
                    commit!();
                    // REGIMM: bit 0 of rt selects >= / <, bit 4 selects "and link".
                    let is_bgez = rt & 1 != 0;
                    let link = rt & 0x1E == 0x10;
                    let negative = (self.regs[rs] as i32) < 0;
                    let taken = negative != is_bgez;
                    if link {
                        // The return address is written even if not taken.
                        let ret = next_pc;
                        set!(31, ret);
                    }
                    branch_if!(taken, imm);
                }
                0x02 => {
                    commit!();
                    jump!((pc & 0xF000_0000) | ((instr & 0x03FF_FFFF) << 2));
                }
                0x03 => {
                    commit!();
                    let ret = next_pc;
                    set!(31, ret);
                    jump!((pc & 0xF000_0000) | ((instr & 0x03FF_FFFF) << 2));
                }
                0x04 => {
                    commit!();
                    branch_if!(self.regs[rs] == self.regs[rt], imm);
                }
                0x05 => {
                    commit!();
                    branch_if!(self.regs[rs] != self.regs[rt], imm);
                }
                0x06 => {
                    commit!();
                    branch_if!((self.regs[rs] as i32) <= 0, imm);
                }
                0x07 => {
                    commit!();
                    branch_if!((self.regs[rs] as i32) > 0, imm);
                }
                0x08 => {
                    let Some(value) = (self.regs[rs] as i32).checked_add(simm as i32) else { break };
                    commit!();
                    set!(rt, value as u32);
                }
                0x09 => {
                    commit!();
                    set!(rt, self.regs[rs].wrapping_add(simm));
                }
                0x0A => {
                    commit!();
                    set!(rt, ((self.regs[rs] as i32) < (simm as i32)) as u32);
                }
                0x0B => {
                    commit!();
                    set!(rt, (self.regs[rs] < simm) as u32);
                }
                0x0C => {
                    commit!();
                    set!(rt, self.regs[rs] & imm);
                }
                0x0D => {
                    commit!();
                    set!(rt, self.regs[rs] | imm);
                }
                0x0E => {
                    commit!();
                    set!(rt, self.regs[rs] ^ imm);
                }
                0x0F => {
                    commit!();
                    set!(rt, imm << 16);
                }
                // ---- loads
                0x20 | 0x24 => {
                    let phys = data_address!(self.regs[rs].wrapping_add(simm), 0);
                    let raw = if phys < RAM_MIRROR_END {
                        bus.memory.ram.read8(phys as usize)
                    } else if phys.wrapping_sub(SCRATCHPAD_START) < SCRATCHPAD_SIZE {
                        bus.scratchpad[(phys & 0x3FF) as usize]
                    } else if phys.wrapping_sub(BIOS_START) < BIOS_SIZE {
                        bus.memory.bios.read8((phys - BIOS_START) as usize)
                    } else {
                        break;
                    };
                    commit!();
                    charge_ram_read!(phys);
                    let value = if instr >> 26 == 0x20 { raw as i8 as i32 as u32 } else { raw as u32 };
                    load!(rt, value);
                }
                0x21 | 0x25 => {
                    let phys = data_address!(self.regs[rs].wrapping_add(simm), 1);
                    let raw = if phys < RAM_MIRROR_END {
                        bus.memory.ram.read16(phys as usize)
                    } else if phys.wrapping_sub(SCRATCHPAD_START) < SCRATCHPAD_SIZE {
                        read_le(&bus.scratchpad, (phys & 0x3FF) as usize, 2) as u16
                    } else if phys.wrapping_sub(BIOS_START) < BIOS_SIZE {
                        bus.memory.bios.read16((phys - BIOS_START) as usize)
                    } else {
                        break;
                    };
                    commit!();
                    charge_ram_read!(phys);
                    let value = if instr >> 26 == 0x21 { raw as i16 as i32 as u32 } else { raw as u32 };
                    load!(rt, value);
                }
                0x23 => {
                    let phys = data_address!(self.regs[rs].wrapping_add(simm), 3);
                    let value = if phys < RAM_MIRROR_END {
                        bus.memory.ram.read32(phys as usize)
                    } else if phys.wrapping_sub(SCRATCHPAD_START) < SCRATCHPAD_SIZE {
                        read_le(&bus.scratchpad, (phys & 0x3FF) as usize, 4)
                    } else if phys.wrapping_sub(BIOS_START) < BIOS_SIZE {
                        bus.memory.bios.read32((phys - BIOS_START) as usize)
                    } else if phys == Bus::GPU_STATUS {
                        // The wait-for-the-GPU loops of the games read this register over and over:
                        // answer from where the GPU will be, instead of leaving the batch.
                        if !BREAKPOINT && pc == poll_head && count + extra == poll_start + POLL_LOOP_CYCLES {
                            // The loop has come round once: the iterations that cannot end it
                            // are skipped in one go.
                            poll_head = u32::MAX;
                            count += POLL_LOOP_CYCLES * skippable_polls(bus, self.regs[poll_regs.1], self.regs[poll_regs.0], count + extra, limit);
                            // The batch ends where the turns that were skipped would have ended
                            // it: before this read, if they used up all the cycles.
                            if count + extra >= limit {
                                break;
                            }
                        } else if !BREAKPOINT && !code_in_bios && (pc & 0xFFF) <= 0x1000 - 4 * POLL_LOOP_WORDS as u32 {
                            if let Some(registers) = poll_loop(bus, offset, instr) {
                                poll_head = pc;
                                poll_start = count + extra;
                                poll_regs = registers;
                            }
                        }
                        bus.gpu_status_after(count + extra)
                    } else {
                        break;
                    };
                    commit!();
                    charge_ram_read!(phys);
                    load!(rt, value);
                    if pgxp && rt != 0 {
                        self.pgxp_regs[rt] = bus.pgxp.lookup(phys, value);
                    }
                }
                // ---- stores (RAM and scratchpad only; I/O goes the slow way)
                0x28 => {
                    if cache_isolated {
                        break;
                    }
                    let phys = data_address!(self.regs[rs].wrapping_add(simm), 0);
                    let value = self.regs[rt] as u8;
                    if phys < RAM_MIRROR_END {
                        commit!();
                        bus.memory.ram.write8(phys as usize, value);
                    } else if phys.wrapping_sub(SCRATCHPAD_START) < SCRATCHPAD_SIZE {
                        commit!();
                        bus.scratchpad[(phys & 0x3FF) as usize] = value;
                    } else {
                        break;
                    }
                }
                0x29 => {
                    if cache_isolated {
                        break;
                    }
                    let phys = data_address!(self.regs[rs].wrapping_add(simm), 1);
                    let value = self.regs[rt] as u16;
                    if phys < RAM_MIRROR_END {
                        commit!();
                        bus.memory.ram.write16(phys as usize, value);
                    } else if phys.wrapping_sub(SCRATCHPAD_START) < SCRATCHPAD_SIZE {
                        commit!();
                        write_le(&mut bus.scratchpad, (phys & 0x3FF) as usize, 2, value as u32);
                    } else {
                        break;
                    }
                }
                0x2B => {
                    if cache_isolated {
                        break;
                    }
                    let phys = data_address!(self.regs[rs].wrapping_add(simm), 3);
                    let value = self.regs[rt];
                    if phys < RAM_MIRROR_END {
                        commit!();
                        bus.memory.ram.write32(phys as usize, value);
                    } else if phys.wrapping_sub(SCRATCHPAD_START) < SCRATCHPAD_SIZE {
                        commit!();
                        write_le(&mut bus.scratchpad, (phys & 0x3FF) as usize, 4, value);
                    } else {
                        break;
                    }
                    if pgxp {
                        bus.pgxp.store(phys, self.pgxp_regs[rt].matching(value));
                    }
                }
                // ---- the GTE (COP2); without SR.CU2 these raise an exception
                0x12 => {
                    if !cop2_usable {
                        break;
                    }
                    let rd = ((instr >> 11) & 0x1F) as usize;
                    if instr & (1 << 25) != 0 {
                        // A GTE command rather than a register move.
                        commit!();
                        stall_for_gte!();
                        self.gte.execute(instr & 0x01FF_FFFF);
                        gte_until = count + extra + Gte::latency(instr);
                    } else {
                        match rs {
                            // MFC2 / CFC2 are loads and have a load delay.
                            0x00 => {
                                commit!();
                                stall_for_gte!();
                                let value = self.gte.read_data(rd);
                                load!(rt, value);
                                if pgxp && rt != 0 {
                                    self.pgxp_regs[rt] = self.gte.sxy_tag(rd, value);
                                }
                            }
                            0x02 => {
                                commit!();
                                stall_for_gte!();
                                let value = self.gte.read_control(rd);
                                load!(rt, value);
                            }
                            0x04 => {
                                commit!();
                                stall_for_gte!();
                                let value = self.regs[rt];
                                if pgxp {
                                    self.gte.write_data_tagged(rd, value, self.pgxp_regs[rt].matching(value));
                                } else {
                                    self.gte.write_data(rd, value);
                                }
                            }
                            0x06 => {
                                commit!();
                                stall_for_gte!();
                                self.gte.write_control(rd, self.regs[rt]);
                            }
                            _ => break,
                        }
                    }
                }
                // LWC2: a word straight into a GTE data register (no load delay).
                0x32 => {
                    if !cop2_usable {
                        break;
                    }
                    let phys = data_address!(self.regs[rs].wrapping_add(simm), 3);
                    let value = if phys < RAM_MIRROR_END {
                        bus.memory.ram.read32(phys as usize)
                    } else if phys.wrapping_sub(SCRATCHPAD_START) < SCRATCHPAD_SIZE {
                        read_le(&bus.scratchpad, (phys & 0x3FF) as usize, 4)
                    } else {
                        break;
                    };
                    commit!();
                    stall_for_gte!();
                    charge_ram_read!(phys);
                    if pgxp {
                        let tag = bus.pgxp.lookup(phys, value);
                        self.gte.write_data_tagged(rt, value, tag);
                    } else {
                        self.gte.write_data(rt, value);
                    }
                }
                // SWC2: store a GTE data register.
                0x3A => {
                    if !cop2_usable || cache_isolated {
                        break;
                    }
                    let phys = data_address!(self.regs[rs].wrapping_add(simm), 3);
                    if phys < RAM_MIRROR_END {
                        commit!();
                        stall_for_gte!();
                        bus.memory.ram.write32(phys as usize, self.gte.read_data(rt));
                    } else if phys.wrapping_sub(SCRATCHPAD_START) < SCRATCHPAD_SIZE {
                        commit!();
                        stall_for_gte!();
                        write_le(&mut bus.scratchpad, (phys & 0x3FF) as usize, 4, self.gte.read_data(rt));
                    } else {
                        break;
                    }
                    if pgxp {
                        bus.pgxp.store(phys, self.gte.sxy_tag(rt, self.gte.read_data(rt)));
                    }
                }
                // COP0, LWL/LWR/SWL/SWR, reserved opcodes...
                _ => break,
            }

            // The delayed load lands now, unless the instruction overwrote it.
            if load_reg != 0 {
                self.regs[load_reg] = load_value;
                load_reg = 0;
            }
            count += 1;
        }

        self.pc = pc;
        self.next_pc = next_pc;
        self.branch_pending = branch_pending;
        self.pending_load = (0, 0);
        self.next_load = (next_load_reg, next_load_value);
        self.hilo_busy = busy_until.saturating_sub(count + extra);
        self.gte_busy = gte_until.saturating_sub(count + extra);
        self.extra_cycles = extra;
        count
    }
}

/// Instructions (and cycles: none of them stalls) of the loop `poll_loop` recognises.
const POLL_LOOP_WORDS: usize = 5;
const POLL_LOOP_CYCLES: u32 = POLL_LOOP_WORDS as u32;

/// Whether the `lw` of GPUSTAT at `offset` (in RAM) starts the loop the libraries of the games
/// wait for the GPU with:
///
/// ```text
/// start: lw   rt, imm(rs)     ; GPUSTAT
///        nop
///        and  rt, rt, rm      ; rm holds the bits it waits for
///        beq  rt, zero, start
///        nop
/// ```
///
/// It returns `(rt, rm)`. Nothing else changes while it runs: the only thing that decides when it
/// ends is the GPU.
fn poll_loop(bus: &Bus, offset: usize, lw: u32) -> Option<(usize, usize)> {
    let word = |i: usize| bus.memory.ram.read32(offset + 4 * i);
    let (rt, rs) = (((lw >> 16) & 0x1F) as usize, ((lw >> 21) & 0x1F) as usize);
    let (and, branch) = (word(2), word(3));
    let (and_rs, and_rt, and_rd) = (((and >> 21) & 0x1F) as usize, ((and >> 16) & 0x1F) as usize, ((and >> 11) & 0x1F) as usize);
    let mask = if and_rs == rt { and_rt } else if and_rt == rt { and_rs } else { return None };
    let is_loop = lw >> 26 == 0x23
        && word(1) == 0
        && and >> 26 == 0
        && and & 0x3F == 0x24
        && and_rd == rt
        && branch >> 26 == 4
        && (branch >> 21) & 0x1F == rt as u32
        && (branch >> 16) & 0x1F == 0
        // Back to the `lw`: the branch is relative to its delay slot, 4 words after the `lw`.
        && branch & 0xFFFF == 0xFFFC
        && word(4) == 0;
    (is_loop && rt != 0 && rt != rs && rt != mask).then_some((rt, mask))
}

/// How many whole turns of the loop `poll_loop` found can be skipped when the next read of GPUSTAT
/// is `elapsed` cycles into the batch, which has `limit` cycles. The skipped turns are the ones
/// that read a GPU that is still busy (with a mask that does not see the bits that follow the
/// scanlines), so each would have stored 0 in `rt` and branched back, which is what `rt` holds
/// already: the registers do not change, only the time goes by.
fn skippable_polls(bus: &Bus, mask: u32, status_read: u32, elapsed: u32, limit: u32) -> u32 {
    // Bit 31 and bit 13 (odd/even line or field) change with the scanlines, not with the GPU's work.
    const FOLLOW_THE_BEAM: u32 = 1 << 31 | 1 << 13;
    if mask & FOLLOW_THE_BEAM != 0 || status_read != 0 || bus.gpu_status_after(elapsed) & mask != 0 {
        return 0;
    }
    let turns = match bus.gpu_idle_in(elapsed) {
        // The turn that reads the GPU idle ends the loop, if it sees what it waits for.
        Some(wait) if bus.gpu_status_after(elapsed + wait) & mask != 0 => wait.div_ceil(POLL_LOOP_CYCLES),
        // Nothing the GPU does ends the loop: it goes on until something else happens.
        _ => u32::MAX,
    };
    turns.min((limit - elapsed) / POLL_LOOP_CYCLES)
}
