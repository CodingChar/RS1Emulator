use serde::{Deserialize, Serialize};
use crate::bus::Bus;
use crate::gte::Gte;
use crate::types::{Ps1Error, Ps1Result};

/// Address the CPU starts executing from after reset (start of the BIOS, KSEG1).
pub const RESET_VECTOR: u32 = 0xBFC0_0000;

const EXCEPTION_VECTOR_RAM: u32 = 0x8000_0080;
const EXCEPTION_VECTOR_BIOS: u32 = 0xBFC0_0180;

// COP0 register indices.
const COP0_BPC: usize = 3;
const COP0_BDA: usize = 5;
const COP0_JUMPDEST: usize = 6;
const COP0_DCIC: usize = 7;
const COP0_BADVADDR: usize = 8;
const COP0_BDAM: usize = 9;
const COP0_BPCM: usize = 11;
const COP0_SR: usize = 12;
const COP0_CAUSE: usize = 13;
const COP0_EPC: usize = 14;
const COP0_PRID: usize = 15;

// Status register bits.
const SR_IEC: u32 = 1 << 0;
const SR_KUC: u32 = 1 << 1;
const SR_ISC: u32 = 1 << 16;
const SR_BEV: u32 = 1 << 22;
const SR_CU0: u32 = 1 << 28;
const SR_CU2: u32 = 1 << 30;
/// Bits of SR that software can change.
const SR_WRITE_MASK: u32 = 0xF27F_FF3F;

// Cause register bits.
const CAUSE_IP_MASK: u32 = 0xFF00;
const CAUSE_IP2: u32 = 1 << 10;
const CAUSE_SOFT_IRQ_MASK: u32 = 0x0300;
const CAUSE_EXC_CODE_MASK: u32 = 0x7C;
const CAUSE_CE_MASK: u32 = 0x3000_0000;
const CAUSE_BD: u32 = 1 << 31;

const PRID_R3000A: u32 = 0x0000_0002;
/// SR value after reset: BEV and TS set.
const SR_RESET: u32 = 0x0060_0000;

/// COP0 exception codes (Cause.ExcCode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exception {
    Interrupt = 0,
    AddressErrorLoad = 4,
    AddressErrorStore = 5,
    BusErrorFetch = 6,
    BusErrorData = 7,
    Syscall = 8,
    Breakpoint = 9,
    ReservedInstruction = 10,
    CoprocessorUnusable = 11,
    Overflow = 12,
}

/// Why executing an instruction did not complete normally.
enum Trap {
    /// A MIPS exception: the CPU vectors to the exception handler.
    Exception {
        code: Exception,
        bad_vaddr: Option<u32>,
        cop: u8,
    },
    /// Any other emulator-level failure (boxed to keep `Trap` small).
    Fatal(Box<Ps1Error>),
}

impl Trap {
    fn exception(code: Exception) -> Self {
        Trap::Exception { code, bad_vaddr: None, cop: 0 }
    }

    fn address_error(code: Exception, addr: u32) -> Self {
        Trap::Exception { code, bad_vaddr: Some(addr), cop: 0 }
    }

    fn coprocessor_unusable(cop: u8) -> Self {
        Trap::Exception { code: Exception::CoprocessorUnusable, bad_vaddr: None, cop }
    }
}

impl From<Ps1Error> for Trap {
    fn from(e: Ps1Error) -> Self {
        Trap::Fatal(Box::new(e))
    }
}

type Exec = Result<(), Trap>;

/// MIPS R3000A CPU interpreter.
///
/// Executes PS1 CPU instructions one at a time. This is intentionally a simple
/// threaded interpreter; a dynamic recompiler (like DuckStation's) is a future
/// optimization and is not directly portable to WASM anyway.
///
/// Models the R3000A's pipeline visible behaviour:
/// - **branch delay slot**: the instruction after a branch/jump always runs
///   before control transfers (`pc`/`next_pc` pair);
/// - **load delay slot**: a loaded register only changes after the following
///   instruction has executed;
/// - **COP0 exceptions**, with the 3-deep KU/IE mode stack in `SR`.
#[derive(Serialize, Deserialize)]
pub struct Interpreter {
    /// General-purpose registers r0–r31 (r0 is hardwired to 0).
    pub regs: [u32; 32],
    /// Address of the next instruction to fetch.
    pub pc: u32,
    /// Address of the instruction after `pc` (differs from `pc + 4` while a
    /// branch is in flight).
    pub next_pc: u32,
    /// HI / LO registers for multiply/divide.
    pub hi: u32,
    pub lo: u32,
    /// Coprocessor 0 registers.
    pub cp0: [u32; 32],
    /// Coprocessor 2: the Geometry Transformation Engine.
    pub gte: Gte,
    /// How many times each exception code (Cause.ExcCode) was taken since
    /// reset, for debugging.
    pub exception_counts: [u64; 16],

    /// Address of the instruction being executed (for EPC).
    current_pc: u32,
    /// The instruction being executed sits in a branch delay slot.
    in_delay_slot: bool,
    /// The instruction just executed was a branch/jump.
    branch_pending: bool,
    /// Load issued by the previous instruction; lands after the current one.
    /// Register 0 stands for "no load" (loads into r0 are discarded anyway).
    pending_load: (usize, u32),
    /// Load issued by the current instruction; becomes `pending_load`.
    next_load: (usize, u32),
    /// Cycles until the multiply/divide unit has its result, counted from the
    /// start of the next instruction. Reading HI/LO before then stalls.
    hilo_busy: u32,
    /// The same for the GTE: cycles until the command it is running is done.
    gte_busy: u32,
    /// Stall cycles the last instruction added on top of its one cycle; the
    /// caller takes them with `take_extra_cycles` and passes them to the bus.
    #[serde(skip)]
    extra_cycles: u32,
    #[cfg(feature = "trace")]
    #[serde(skip)]
    trace: crate::cpu::trace::Trace,
}

impl Interpreter {
    pub fn new() -> Self {
        let mut cpu = Self {
            regs: [0u32; 32],
            pc: RESET_VECTOR,
            next_pc: RESET_VECTOR.wrapping_add(4),
            hi: 0,
            lo: 0,
            cp0: [0u32; 32],
            gte: Gte::new(),
            exception_counts: [0; 16],
            current_pc: RESET_VECTOR,
            in_delay_slot: false,
            branch_pending: false,
            pending_load: (0, 0),
            next_load: (0, 0),
            hilo_busy: 0,
            gte_busy: 0,
            extra_cycles: 0,
            #[cfg(feature = "trace")]
            trace: Default::default(),
        };
        cpu.cp0[COP0_SR] = SR_RESET;
        cpu.cp0[COP0_PRID] = PRID_R3000A;
        cpu
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Jump to `pc`, discarding any branch in flight.
    pub fn set_pc(&mut self, pc: u32) {
        self.pc = pc;
        self.next_pc = pc.wrapping_add(4);
        self.branch_pending = false;
    }

    /// Start a program: jump to `pc` with the global pointer and stack set,
    /// dropping any branch or load still in flight.
    pub fn start_program(&mut self, pc: u32, gp: u32, sp: u32) {
        self.set_pc(pc);
        self.pending_load = (0, 0);
        self.next_load = (0, 0);
        self.regs[28] = gp;
        self.regs[29] = sp;
        self.regs[30] = sp;
    }

    /// Drive the external interrupt line (Cause.IP2), fed by the interrupt
    /// controller.
    pub fn set_irq(&mut self, asserted: bool) {
        if asserted {
            self.cp0[COP0_CAUSE] |= CAUSE_IP2;
        } else {
            self.cp0[COP0_CAUSE] &= !CAUSE_IP2;
        }
    }

    /// The last executed instructions as `(address, word)`, oldest first.
    #[cfg(feature = "trace")]
    pub fn trace(&self) -> Vec<(u32, u32)> {
        self.trace.entries()
    }

    /// The stall cycles the last `step` added to its one cycle (a multiply or
    /// divide result read too early); the bus must be ticked for them as well.
    pub fn take_extra_cycles(&mut self) -> u32 {
        std::mem::take(&mut self.extra_cycles)
    }

    /// Charge the wait of a data read at `addr` (see [`RAM_READ_CYCLES`]). The
    /// multiply/divide unit and the GTE keep working meanwhile.
    #[inline(always)]
    fn charge_read(&mut self, addr: u32) {
        if Bus::physical(addr) < RAM_MIRROR_END {
            self.extra_cycles += RAM_READ_CYCLES;
            self.hilo_busy = self.hilo_busy.saturating_sub(RAM_READ_CYCLES);
            self.gte_busy = self.gte_busy.saturating_sub(RAM_READ_CYCLES);
        }
    }

    /// Execute one instruction.
    ///
    /// MIPS exceptions (overflow, syscall, address errors, interrupts...) are
    /// handled inside the emulated CPU and return `Ok`. Only emulator-level
    /// failures are returned as errors.
    pub fn step(&mut self, bus: &mut Bus) -> Ps1Result<()> {
        match self.step_inner(bus) {
            Ok(()) => Ok(()),
            Err(trap) => self.handle_trap(trap),
        }
    }

    /// Run `count` cycles, each instruction taking one (plus the stalls of a
    /// multiply or divide result read too early), followed by the same cycles of
    /// the devices (see `Bus::tick`), with the interrupt line sampled before each
    /// instruction. Returns how many cycles ran, which can exceed `count` by the
    /// stall of the last instruction, and why it stopped early, if it did.
    ///
    /// This is the same as calling [`Interpreter::step`] in a loop but keeps
    /// the whole interpreter in a single hot loop, which is what makes it fast.
    pub fn run(&mut self, bus: &mut Bus, count: u64) -> (u64, Ps1Result<()>) {
        let (executed, result, _) = self.run_loop::<false>(bus, count, 0);
        (executed, result)
    }

    /// Like [`Interpreter::run`], but stops (without executing it) when the
    /// next instruction is the one at `break_pc`. The last value says whether
    /// it stopped there.
    pub fn run_until(&mut self, bus: &mut Bus, count: u64, break_pc: u32) -> (u64, Ps1Result<()>, bool) {
        self.run_loop::<true>(bus, count, break_pc)
    }

    #[inline(always)]
    fn run_loop<const BREAKPOINT: bool>(
        &mut self,
        bus: &mut Bus,
        count: u64,
        break_pc: u32,
    ) -> (u64, Ps1Result<()>, bool) {
        let mut executed = 0u64;
        while executed < count {
            if BREAKPOINT && self.pc == break_pc {
                return (executed, Ok(()), true);
            }
            self.set_irq(bus.interrupt.pending());
            // (With the `trace` feature every instruction takes the exact path.)
            if !self.interrupt_pending() && !cfg!(feature = "trace") {
                // Nothing can change the interrupt line before the next device
                // event, so a whole batch runs without looking at it again.
                let limit = (count - executed).min(bus.cycles_to_deadline() as u64) as u32;
                let done = self.run_fast::<BREAKPOINT>(bus, limit, break_pc);
                let spent = done + self.take_extra_cycles();
                if spent > 0 {
                    bus.tick(spent);
                    executed += spent as u64;
                    if spent >= limit {
                        continue;
                    }
                }
                if BREAKPOINT && self.pc == break_pc {
                    continue;
                }
            }
            // One instruction the fast path does not handle (or an interrupt).
            if let Err(trap) = self.step_inner(bus) {
                if let Err(e) = self.handle_trap(trap) {
                    return (executed, Err(e), false);
                }
            }
            let spent = 1 + self.take_extra_cycles() + bus.take_dma_stall();
            bus.tick(spent);
            executed += spent as u64;
        }
        (executed, Ok(()), false)
    }

    #[inline(always)]
    fn step_inner(&mut self, bus: &mut Bus) -> Exec {
        self.current_pc = self.pc;
        self.in_delay_slot = self.branch_pending;
        self.branch_pending = false;
        self.pending_load = std::mem::replace(&mut self.next_load, (0, 0));

        let result = if self.interrupt_pending() {
            Err(Trap::exception(Exception::Interrupt))
        } else {
            self.fetch_and_execute(bus)
        };

        // The delayed load lands now, unless the instruction overwrote it.
        let (reg, value) = self.pending_load;
        if reg != 0 {
            self.regs[reg] = value;
            self.pending_load.0 = 0;
        }
        self.hilo_busy = self.hilo_busy.saturating_sub(1);
        self.gte_busy = self.gte_busy.saturating_sub(1);
        result
    }

    /// Deal with the outcome of an instruction that did not complete: take the
    /// MIPS exception, or report an emulator-level failure.
    #[cold]
    #[inline(never)]
    fn handle_trap(&mut self, trap: Trap) -> Ps1Result<()> {
        match trap {
            Trap::Exception { code, bad_vaddr, cop } => {
                self.raise_exception(code, bad_vaddr, cop);
                Ok(())
            }
            Trap::Fatal(e) => Err(*e),
        }
    }

    #[inline(always)]
    fn interrupt_pending(&self) -> bool {
        let sr = self.cp0[COP0_SR];
        sr & SR_IEC != 0 && sr & self.cp0[COP0_CAUSE] & CAUSE_IP_MASK != 0
    }

    #[inline(always)]
    fn fetch_and_execute(&mut self, bus: &mut Bus) -> Exec {
        let pc = self.pc;
        if pc & 3 != 0 || self.is_forbidden_address(pc) {
            return Err(Trap::address_error(Exception::AddressErrorLoad, pc));
        }
        let instr = bus.load(pc, 4).ok_or(Trap::exception(Exception::BusErrorFetch))?;
        #[cfg(feature = "trace")]
        self.trace.record(pc, instr);
        self.pc = self.next_pc;
        self.next_pc = self.next_pc.wrapping_add(4);
        self.execute(instr, bus)
    }

    /// Enter the exception handler.
    fn raise_exception(&mut self, code: Exception, bad_vaddr: Option<u32>, cop: u8) {
        self.exception_counts[code as usize] += 1;
        let sr = self.cp0[COP0_SR];
        let handler = if sr & SR_BEV != 0 {
            EXCEPTION_VECTOR_BIOS
        } else {
            EXCEPTION_VECTOR_RAM
        };

        // Push the KU/IE mode stack; the new mode is kernel with IRQs off.
        self.cp0[COP0_SR] = (sr & !0x3F) | ((sr << 2) & 0x3F);

        let mut cause = self.cp0[COP0_CAUSE] & !(CAUSE_EXC_CODE_MASK | CAUSE_CE_MASK | CAUSE_BD);
        cause |= (code as u32) << 2;
        if code == Exception::CoprocessorUnusable {
            cause |= (cop as u32) << 28;
        }
        if self.in_delay_slot {
            cause |= CAUSE_BD;
            self.cp0[COP0_EPC] = self.current_pc.wrapping_sub(4);
        } else {
            self.cp0[COP0_EPC] = self.current_pc;
        }
        self.cp0[COP0_CAUSE] = cause;

        if let Some(addr) = bad_vaddr {
            self.cp0[COP0_BADVADDR] = addr;
        }

        self.pc = handler;
        self.next_pc = handler.wrapping_add(4);
        self.branch_pending = false;
    }

    // ----- Register helpers -----

    /// Write a register immediately (also cancels a pending load to it).
    #[inline(always)]
    fn set_reg(&mut self, reg: usize, value: u32) {
        if reg != 0 {
            if self.pending_load.0 == reg {
                self.pending_load.0 = 0;
            }
            self.regs[reg] = value;
        }
    }

    /// Issue a load: the register changes after the next instruction.
    #[inline(always)]
    fn load_reg(&mut self, reg: usize, value: u32) {
        if reg != 0 {
            if self.pending_load.0 == reg {
                self.pending_load.0 = 0;
            }
            self.next_load = (reg, value);
        }
    }

    /// Current value of a register as seen by LWL/LWR, which can read the
    /// result of a load still in flight.
    fn reg_for_merge(&self, reg: usize) -> u32 {
        if reg != 0 && self.pending_load.0 == reg {
            self.pending_load.1
        } else {
            self.regs[reg]
        }
    }

    // ----- Memory access (alignment, privilege, cache isolation) -----

    #[inline(always)]
    fn is_forbidden_address(&self, addr: u32) -> bool {
        // User mode may only touch KUSEG.
        self.cp0[COP0_SR] & SR_KUC != 0 && addr & 0x8000_0000 != 0
    }

    #[inline(always)]
    fn read8(&self, bus: &mut Bus, addr: u32) -> Result<u8, Trap> {
        if self.is_forbidden_address(addr) {
            return Err(Trap::address_error(Exception::AddressErrorLoad, addr));
        }
        bus.load(addr, 1).map(|v| v as u8).ok_or(Trap::exception(Exception::BusErrorData))
    }

    #[inline(always)]
    fn read16(&self, bus: &mut Bus, addr: u32) -> Result<u16, Trap> {
        if addr & 1 != 0 || self.is_forbidden_address(addr) {
            return Err(Trap::address_error(Exception::AddressErrorLoad, addr));
        }
        bus.load(addr, 2).map(|v| v as u16).ok_or(Trap::exception(Exception::BusErrorData))
    }

    #[inline(always)]
    fn read32(&self, bus: &mut Bus, addr: u32) -> Result<u32, Trap> {
        if addr & 3 != 0 || self.is_forbidden_address(addr) {
            return Err(Trap::address_error(Exception::AddressErrorLoad, addr));
        }
        bus.load(addr, 4).ok_or(Trap::exception(Exception::BusErrorData))
    }

    /// Aligned word read used by LWL/LWR/SWL/SWR (no alignment fault).
    #[inline(never)]
    fn read32_unaligned_base(&self, bus: &mut Bus, addr: u32) -> Result<u32, Trap> {
        if self.is_forbidden_address(addr) {
            return Err(Trap::address_error(Exception::AddressErrorLoad, addr));
        }
        bus.load(addr & !3, 4).ok_or(Trap::exception(Exception::BusErrorData))
    }

    fn cache_isolated(&self) -> bool {
        self.cp0[COP0_SR] & SR_ISC != 0
    }

    #[inline(always)]
    fn write8(&self, bus: &mut Bus, addr: u32, value: u8) -> Exec {
        if self.is_forbidden_address(addr) {
            return Err(Trap::address_error(Exception::AddressErrorStore, addr));
        }
        if self.cache_isolated() {
            return Ok(());
        }
        Self::stored(bus.store(addr, 1, value as u32))
    }

    #[inline(always)]
    fn write16(&self, bus: &mut Bus, addr: u32, value: u16) -> Exec {
        if addr & 1 != 0 || self.is_forbidden_address(addr) {
            return Err(Trap::address_error(Exception::AddressErrorStore, addr));
        }
        if self.cache_isolated() {
            return Ok(());
        }
        Self::stored(bus.store(addr, 2, value as u32))
    }

    #[inline(always)]
    fn write32(&self, bus: &mut Bus, addr: u32, value: u32) -> Exec {
        if addr & 3 != 0 || self.is_forbidden_address(addr) {
            return Err(Trap::address_error(Exception::AddressErrorStore, addr));
        }
        if self.cache_isolated() {
            return Ok(());
        }
        Self::stored(bus.store(addr, 4, value))
    }

    fn stored(ok: bool) -> Exec {
        if ok { Ok(()) } else { Err(Trap::exception(Exception::BusErrorData)) }
    }

    /// Aligned word write used by SWL/SWR (no alignment fault).
    #[inline(never)]
    fn write32_unaligned_base(&self, bus: &mut Bus, addr: u32, value: u32) -> Exec {
        if self.is_forbidden_address(addr) {
            return Err(Trap::address_error(Exception::AddressErrorStore, addr));
        }
        if self.cache_isolated() {
            return Ok(());
        }
        Self::stored(bus.store(addr & !3, 4, value))
    }

    // ----- Branch helpers -----
    //
    // When an instruction executes, `pc` already points at its delay slot, so
    // relative targets are computed from `pc`.

    #[inline(always)]
    fn jump(&mut self, target: u32) {
        self.next_pc = target;
        self.branch_pending = true;
    }

    #[inline(always)]
    fn branch_if(&mut self, condition: bool, imm: u32) {
        if condition {
            self.next_pc = self.pc.wrapping_add(sign_extend16(imm) << 2);
        }
        // A delay slot follows even when the branch is not taken.
        self.branch_pending = true;
    }

    // ----- Instruction decode / execute -----

    #[inline(always)]
    fn execute(&mut self, instr: u32, bus: &mut Bus) -> Exec {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let imm = instr & 0xFFFF;
        let simm = sign_extend16(imm);

        match instr >> 26 {
            0x00 => self.execute_special(instr)?,
            0x01 => {
                // REGIMM: bit 0 of rt selects >= / <, bit 4 selects "and link".
                // Decoded this way (not as an exact match) as the hardware does.
                let is_bgez = rt & 1 != 0;
                let link = rt & 0x1E == 0x10;
                let negative = (self.regs[rs] as i32) < 0;
                let taken = negative != is_bgez;
                if link {
                    // The return address is written even if not taken.
                    let ret = self.next_pc;
                    self.set_reg(31, ret);
                }
                self.branch_if(taken, imm);
            }
            0x02 => {
                let target = (self.pc & 0xF000_0000) | ((instr & 0x03FF_FFFF) << 2);
                self.jump(target);
            }
            0x03 => {
                let target = (self.pc & 0xF000_0000) | ((instr & 0x03FF_FFFF) << 2);
                let ret = self.next_pc;
                self.set_reg(31, ret);
                self.jump(target);
            }
            0x04 => self.branch_if(self.regs[rs] == self.regs[rt], imm),
            0x05 => self.branch_if(self.regs[rs] != self.regs[rt], imm),
            0x06 => self.branch_if((self.regs[rs] as i32) <= 0, imm),
            0x07 => self.branch_if((self.regs[rs] as i32) > 0, imm),
            0x08 => {
                let value = (self.regs[rs] as i32)
                    .checked_add(simm as i32)
                    .ok_or_else(|| Trap::exception(Exception::Overflow))?;
                self.set_reg(rt, value as u32);
            }
            0x09 => self.set_reg(rt, self.regs[rs].wrapping_add(simm)),
            0x0A => self.set_reg(rt, ((self.regs[rs] as i32) < (simm as i32)) as u32),
            0x0B => self.set_reg(rt, (self.regs[rs] < simm) as u32),
            0x0C => self.set_reg(rt, self.regs[rs] & imm),
            0x0D => self.set_reg(rt, self.regs[rs] | imm),
            0x0E => self.set_reg(rt, self.regs[rs] ^ imm),
            0x0F => self.set_reg(rt, imm << 16),
            0x10 => self.execute_cop0(instr)?,
            0x11 => return Err(Trap::coprocessor_unusable(1)),
            0x12 => self.execute_cop2(instr)?,
            0x13 => return Err(Trap::coprocessor_unusable(3)),
            0x20 => {
                let addr = self.regs[rs].wrapping_add(simm);
                let value = self.read8(bus, addr)? as i8 as i32 as u32;
                self.charge_read(addr);
                self.load_reg(rt, value);
            }
            0x21 => {
                let addr = self.regs[rs].wrapping_add(simm);
                let value = self.read16(bus, addr)? as i16 as i32 as u32;
                self.charge_read(addr);
                self.load_reg(rt, value);
            }
            0x22 => {
                // LWL
                let addr = self.regs[rs].wrapping_add(simm);
                let word = self.read32_unaligned_base(bus, addr)?;
                self.charge_read(addr);
                let cur = self.reg_for_merge(rt);
                let value = match addr & 3 {
                    0 => (cur & 0x00FF_FFFF) | (word << 24),
                    1 => (cur & 0x0000_FFFF) | (word << 16),
                    2 => (cur & 0x0000_00FF) | (word << 8),
                    _ => word,
                };
                self.load_reg(rt, value);
            }
            0x23 => {
                let addr = self.regs[rs].wrapping_add(simm);
                let value = self.read32(bus, addr)?;
                self.charge_read(addr);
                self.load_reg(rt, value);
            }
            0x24 => {
                let addr = self.regs[rs].wrapping_add(simm);
                let value = self.read8(bus, addr)? as u32;
                self.charge_read(addr);
                self.load_reg(rt, value);
            }
            0x25 => {
                let addr = self.regs[rs].wrapping_add(simm);
                let value = self.read16(bus, addr)? as u32;
                self.charge_read(addr);
                self.load_reg(rt, value);
            }
            0x26 => {
                // LWR
                let addr = self.regs[rs].wrapping_add(simm);
                let word = self.read32_unaligned_base(bus, addr)?;
                self.charge_read(addr);
                let cur = self.reg_for_merge(rt);
                let value = match addr & 3 {
                    0 => word,
                    1 => (cur & 0xFF00_0000) | (word >> 8),
                    2 => (cur & 0xFFFF_0000) | (word >> 16),
                    _ => (cur & 0xFFFF_FF00) | (word >> 24),
                };
                self.load_reg(rt, value);
            }
            0x28 => {
                let addr = self.regs[rs].wrapping_add(simm);
                self.write8(bus, addr, self.regs[rt] as u8)?;
            }
            0x29 => {
                let addr = self.regs[rs].wrapping_add(simm);
                self.write16(bus, addr, self.regs[rt] as u16)?;
            }
            0x2A => {
                // SWL
                let addr = self.regs[rs].wrapping_add(simm);
                let mem = self.read32_unaligned_base(bus, addr)?;
                let reg = self.regs[rt];
                let value = match addr & 3 {
                    0 => (mem & 0xFFFF_FF00) | (reg >> 24),
                    1 => (mem & 0xFFFF_0000) | (reg >> 16),
                    2 => (mem & 0xFF00_0000) | (reg >> 8),
                    _ => reg,
                };
                self.write32_unaligned_base(bus, addr, value)?;
            }
            0x2B => {
                let addr = self.regs[rs].wrapping_add(simm);
                self.write32(bus, addr, self.regs[rt])?;
            }
            0x2E => {
                // SWR
                let addr = self.regs[rs].wrapping_add(simm);
                let mem = self.read32_unaligned_base(bus, addr)?;
                let reg = self.regs[rt];
                let value = match addr & 3 {
                    0 => reg,
                    1 => (mem & 0x0000_00FF) | (reg << 8),
                    2 => (mem & 0x0000_FFFF) | (reg << 16),
                    _ => (mem & 0x00FF_FFFF) | (reg << 24),
                };
                self.write32_unaligned_base(bus, addr, value)?;
            }
            // LWC0/LWC1/LWC3 and SWC0/SWC1/SWC3: coprocessor not present.
            0x30 | 0x38 => return Err(Trap::coprocessor_unusable(0)),
            0x31 | 0x39 => return Err(Trap::coprocessor_unusable(1)),
            0x33 | 0x3B => return Err(Trap::coprocessor_unusable(3)),
            0x32 => {
                // LWC2: load a word straight into a GTE data register.
                self.require_cop2()?;
                self.wait_for_gte();
                let addr = self.regs[rs].wrapping_add(simm);
                let value = self.read32(bus, addr)?;
                self.charge_read(addr);
                self.gte.write_data(rt, value);
            }
            0x3A => {
                // SWC2: store a GTE data register.
                self.require_cop2()?;
                self.wait_for_gte();
                let addr = self.regs[rs].wrapping_add(simm);
                self.write32(bus, addr, self.gte.read_data(rt))?;
            }
            _ => return Err(Trap::exception(Exception::ReservedInstruction)),
        }
        Ok(())
    }

    #[inline(always)]
    fn execute_special(&mut self, instr: u32) -> Exec {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let rd = ((instr >> 11) & 0x1F) as usize;
        let shamt = (instr >> 6) & 0x1F;
        let (a, b) = (self.regs[rs], self.regs[rt]);

        match instr & 0x3F {
            0x00 => self.set_reg(rd, b << shamt),
            0x02 => self.set_reg(rd, b >> shamt),
            0x03 => self.set_reg(rd, ((b as i32) >> shamt) as u32),
            0x04 => self.set_reg(rd, b << (a & 0x1F)),
            0x06 => self.set_reg(rd, b >> (a & 0x1F)),
            0x07 => self.set_reg(rd, ((b as i32) >> (a & 0x1F)) as u32),
            0x08 => self.jump(a),
            0x09 => {
                let ret = self.next_pc;
                self.set_reg(rd, ret);
                self.jump(a);
            }
            0x0C => return Err(Trap::exception(Exception::Syscall)),
            0x0D => return Err(Trap::exception(Exception::Breakpoint)),
            0x10 => {
                self.wait_for_hilo();
                self.set_reg(rd, self.hi);
            }
            0x11 => self.hi = a,
            0x12 => {
                self.wait_for_hilo();
                self.set_reg(rd, self.lo);
            }
            0x13 => self.lo = a,
            0x18 => {
                let prod = (a as i32 as i64) * (b as i32 as i64);
                self.lo = prod as u32;
                self.hi = (prod >> 32) as u32;
                // One more than the stall for the next instruction: this one's own cycle.
                self.hilo_busy = multiply_latency(a, true);
            }
            0x19 => {
                let prod = (a as u64) * (b as u64);
                self.lo = prod as u32;
                self.hi = (prod >> 32) as u32;
                self.hilo_busy = multiply_latency(a, false);
            }
            0x1A => {
                self.divide_signed(a as i32, b as i32);
                self.hilo_busy = DIVIDE_LATENCY;
            }
            0x1B => {
                self.divide_unsigned(a, b);
                self.hilo_busy = DIVIDE_LATENCY;
            }
            0x20 => {
                let value = (a as i32)
                    .checked_add(b as i32)
                    .ok_or_else(|| Trap::exception(Exception::Overflow))?;
                self.set_reg(rd, value as u32);
            }
            0x21 => self.set_reg(rd, a.wrapping_add(b)),
            0x22 => {
                let value = (a as i32)
                    .checked_sub(b as i32)
                    .ok_or_else(|| Trap::exception(Exception::Overflow))?;
                self.set_reg(rd, value as u32);
            }
            0x23 => self.set_reg(rd, a.wrapping_sub(b)),
            0x24 => self.set_reg(rd, a & b),
            0x25 => self.set_reg(rd, a | b),
            0x26 => self.set_reg(rd, a ^ b),
            0x27 => self.set_reg(rd, !(a | b)),
            0x2A => self.set_reg(rd, ((a as i32) < (b as i32)) as u32),
            0x2B => self.set_reg(rd, (a < b) as u32),
            _ => return Err(Trap::exception(Exception::ReservedInstruction)),
        }
        Ok(())
    }

    /// COP2 needs `SR.CU2`, even from kernel mode.
    fn require_cop2(&self) -> Exec {
        if self.cp0[COP0_SR] & SR_CU2 == 0 {
            return Err(Trap::coprocessor_unusable(2));
        }
        Ok(())
    }

    #[inline(never)]
    fn execute_cop2(&mut self, instr: u32) -> Exec {
        self.require_cop2()?;
        self.wait_for_gte();

        // Bit 25 set: a GTE command rather than a register move.
        if instr & (1 << 25) != 0 {
            let _area = crate::profile::enter(crate::profile::Area::Gte);
            self.gte.execute(instr & 0x01FF_FFFF);
            // The end of this instruction takes one cycle off before the next one starts.
            self.gte_busy = Gte::latency(instr);
            return Ok(());
        }

        let rt = ((instr >> 16) & 0x1F) as usize;
        let rd = ((instr >> 11) & 0x1F) as usize;
        match (instr >> 21) & 0x1F {
            // MFC2 / CFC2 are loads and have a load delay.
            0x00 => {
                let value = self.gte.read_data(rd);
                self.load_reg(rt, value);
            }
            0x02 => {
                let value = self.gte.read_control(rd);
                self.load_reg(rt, value);
            }
            0x04 => self.gte.write_data(rd, self.regs[rt]),
            0x06 => self.gte.write_control(rd, self.regs[rt]),
            _ => return Err(Trap::exception(Exception::ReservedInstruction)),
        }
        Ok(())
    }

    #[inline(never)]
    fn execute_cop0(&mut self, instr: u32) -> Exec {
        // COP0 is usable in kernel mode, or in user mode only if SR.CU0 is set.
        let sr = self.cp0[COP0_SR];
        if sr & SR_KUC != 0 && sr & SR_CU0 == 0 {
            return Err(Trap::coprocessor_unusable(0));
        }

        let rt = ((instr >> 16) & 0x1F) as usize;
        let rd = ((instr >> 11) & 0x1F) as usize;

        match (instr >> 21) & 0x1F {
            0x00 => {
                // MFC0
                match rd {
                    COP0_BPC | COP0_BDA | COP0_JUMPDEST | COP0_DCIC | COP0_BADVADDR
                    | COP0_BDAM | COP0_BPCM | COP0_SR | COP0_CAUSE | COP0_EPC | COP0_PRID => {
                        let value = self.cp0[rd];
                        self.load_reg(rt, value);
                    }
                    _ => return Err(Trap::exception(Exception::ReservedInstruction)),
                }
            }
            0x04 => {
                // MTC0
                let value = self.regs[rt];
                match rd {
                    COP0_BPC | COP0_BDA | COP0_JUMPDEST | COP0_DCIC | COP0_BDAM | COP0_BPCM => {
                        self.cp0[rd] = value;
                    }
                    COP0_SR => self.cp0[COP0_SR] = value & SR_WRITE_MASK,
                    COP0_CAUSE => {
                        // Only the two software interrupt bits are writable.
                        let cause = self.cp0[COP0_CAUSE];
                        self.cp0[COP0_CAUSE] =
                            (cause & !CAUSE_SOFT_IRQ_MASK) | (value & CAUSE_SOFT_IRQ_MASK);
                    }
                    // BadVAddr, EPC and PRId are read-only; writes are ignored.
                    COP0_BADVADDR | COP0_EPC | COP0_PRID => {}
                    _ => return Err(Trap::exception(Exception::ReservedInstruction)),
                }
            }
            0x10 if instr & 0x3F == 0x10 => {
                // RFE: pop the KU/IE mode stack.
                let sr = self.cp0[COP0_SR];
                self.cp0[COP0_SR] = (sr & !0x0F) | ((sr >> 2) & 0x0F);
            }
            _ => return Err(Trap::exception(Exception::ReservedInstruction)),
        }
        Ok(())
    }

    // ----- Multiply/divide helpers -----

    /// Stall until the GTE has finished its command (the CPU touches the GTE).
    #[inline(never)]
    fn wait_for_gte(&mut self) {
        self.extra_cycles += self.gte_busy;
        self.gte_busy = 0;
    }

    /// Stall until the multiply/divide unit has finished (HI/LO were read).
    #[inline(never)]
    fn wait_for_hilo(&mut self) {
        self.extra_cycles += self.hilo_busy;
        self.hilo_busy = 0;
    }

    // Results for the cases real hardware doesn't trap on, as documented in
    // the "CPU division by zero / overflow" notes.
    #[inline(never)]
    fn divide_signed(&mut self, a: i32, b: i32) {
        if b == 0 {
            self.lo = if a >= 0 { 0xFFFF_FFFF } else { 1 };
            self.hi = a as u32;
        } else if a == i32::MIN && b == -1 {
            self.lo = 0x8000_0000;
            self.hi = 0;
        } else {
            self.lo = (a / b) as u32;
            self.hi = (a % b) as u32;
        }
    }

    #[inline(never)]
    fn divide_unsigned(&mut self, a: u32, b: u32) {
        if b == 0 {
            self.lo = 0xFFFF_FFFF;
            self.hi = a;
        } else {
            self.lo = a / b;
            self.hi = a % b;
        }
    }
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

fn sign_extend16(imm: u32) -> u32 {
    imm as u16 as i16 as i32 as u32
}

/// Cycles a divide takes (R3000A, from the psx-spx timing notes).
pub(crate) const DIVIDE_LATENCY: u32 = 36;

/// End of main RAM and its mirrors, as a physical address.
pub(crate) const RAM_MIRROR_END: u32 = 0x80_0000;

/// Cycles a data read from main RAM takes on top of the instruction's own one.
///
/// The R3000A in the PS1 has no data cache, so every load that is not for the
/// scratchpad goes out to the DRAM, which answers a word in about 6 cycles
/// (psx-spx, "Memory Control"). Code that waits for something with a loop of
/// iterations, like the memory card routines of many games, is tuned for that
/// pace: at one cycle per load the loop would end several times too soon.
///
/// Writes are not charged (they go through the write buffer), nor are
/// instruction fetches (the instruction cache is not modelled and loops fit
/// in it), nor reads of the BIOS or of the I/O registers.
pub(crate) const RAM_READ_CYCLES: u32 = 5;

/// Cycles a multiply takes: 6, 9 or 13 depending on how many significant bits
/// `rs` has (for MULT the magnitude of the signed value counts).
#[inline(always)]
pub(crate) fn multiply_latency(rs: u32, signed: bool) -> u32 {
    let magnitude = if signed && (rs as i32) < 0 { !rs } else { rs };
    if magnitude < 0x800 {
        6
    } else if magnitude < 0x10_0000 {
        9
    } else {
        13
    }
}

mod fast;

#[cfg(test)]
mod tests;
