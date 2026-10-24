use super::*;

// ----- Instruction encoders -----

fn rtype(rs: u32, rt: u32, rd: u32, shamt: u32, funct: u32) -> u32 {
    (rs << 21) | (rt << 16) | (rd << 11) | (shamt << 6) | funct
}

fn itype(op: u32, rs: u32, rt: u32, imm: i32) -> u32 {
    (op << 26) | (rs << 21) | (rt << 16) | (imm as u32 & 0xFFFF)
}

fn jtype(op: u32, target: u32) -> u32 {
    (op << 26) | ((target >> 2) & 0x03FF_FFFF)
}

const NOP: u32 = 0;

fn addiu(rt: u32, rs: u32, imm: i32) -> u32 { itype(0x09, rs, rt, imm) }
fn addi(rt: u32, rs: u32, imm: i32) -> u32 { itype(0x08, rs, rt, imm) }
fn slti(rt: u32, rs: u32, imm: i32) -> u32 { itype(0x0A, rs, rt, imm) }
fn sltiu(rt: u32, rs: u32, imm: i32) -> u32 { itype(0x0B, rs, rt, imm) }
fn andi(rt: u32, rs: u32, imm: i32) -> u32 { itype(0x0C, rs, rt, imm) }
fn ori(rt: u32, rs: u32, imm: i32) -> u32 { itype(0x0D, rs, rt, imm) }
fn xori(rt: u32, rs: u32, imm: i32) -> u32 { itype(0x0E, rs, rt, imm) }
fn lui(rt: u32, imm: i32) -> u32 { itype(0x0F, 0, rt, imm) }

fn lb(rt: u32, off: i32, base: u32) -> u32 { itype(0x20, base, rt, off) }
fn lh(rt: u32, off: i32, base: u32) -> u32 { itype(0x21, base, rt, off) }
fn lwl(rt: u32, off: i32, base: u32) -> u32 { itype(0x22, base, rt, off) }
fn lw(rt: u32, off: i32, base: u32) -> u32 { itype(0x23, base, rt, off) }
fn lbu(rt: u32, off: i32, base: u32) -> u32 { itype(0x24, base, rt, off) }
fn lhu(rt: u32, off: i32, base: u32) -> u32 { itype(0x25, base, rt, off) }
fn lwr(rt: u32, off: i32, base: u32) -> u32 { itype(0x26, base, rt, off) }
fn sb(rt: u32, off: i32, base: u32) -> u32 { itype(0x28, base, rt, off) }
fn sh(rt: u32, off: i32, base: u32) -> u32 { itype(0x29, base, rt, off) }
fn swl(rt: u32, off: i32, base: u32) -> u32 { itype(0x2A, base, rt, off) }
fn sw(rt: u32, off: i32, base: u32) -> u32 { itype(0x2B, base, rt, off) }
fn swr(rt: u32, off: i32, base: u32) -> u32 { itype(0x2E, base, rt, off) }

fn beq(rs: u32, rt: u32, off: i32) -> u32 { itype(0x04, rs, rt, off) }
fn bne(rs: u32, rt: u32, off: i32) -> u32 { itype(0x05, rs, rt, off) }
fn blez(rs: u32, off: i32) -> u32 { itype(0x06, rs, 0, off) }
fn bgtz(rs: u32, off: i32) -> u32 { itype(0x07, rs, 0, off) }
fn bltz(rs: u32, off: i32) -> u32 { itype(0x01, rs, 0x00, off) }
fn bgez(rs: u32, off: i32) -> u32 { itype(0x01, rs, 0x01, off) }
fn bltzal(rs: u32, off: i32) -> u32 { itype(0x01, rs, 0x10, off) }
fn bgezal(rs: u32, off: i32) -> u32 { itype(0x01, rs, 0x11, off) }
fn j(target: u32) -> u32 { jtype(0x02, target) }
fn jal(target: u32) -> u32 { jtype(0x03, target) }
fn jr(rs: u32) -> u32 { rtype(rs, 0, 0, 0, 0x08) }
fn jalr(rd: u32, rs: u32) -> u32 { rtype(rs, 0, rd, 0, 0x09) }

fn sll(rd: u32, rt: u32, sh: u32) -> u32 { rtype(0, rt, rd, sh, 0x00) }
fn srl(rd: u32, rt: u32, sh: u32) -> u32 { rtype(0, rt, rd, sh, 0x02) }
fn sra(rd: u32, rt: u32, sh: u32) -> u32 { rtype(0, rt, rd, sh, 0x03) }
fn sllv(rd: u32, rt: u32, rs: u32) -> u32 { rtype(rs, rt, rd, 0, 0x04) }
fn srlv(rd: u32, rt: u32, rs: u32) -> u32 { rtype(rs, rt, rd, 0, 0x06) }
fn srav(rd: u32, rt: u32, rs: u32) -> u32 { rtype(rs, rt, rd, 0, 0x07) }
fn syscall() -> u32 { rtype(0, 0, 0, 0, 0x0C) }
fn brk() -> u32 { rtype(0, 0, 0, 0, 0x0D) }
fn mfhi(rd: u32) -> u32 { rtype(0, 0, rd, 0, 0x10) }
fn mthi(rs: u32) -> u32 { rtype(rs, 0, 0, 0, 0x11) }
fn mflo(rd: u32) -> u32 { rtype(0, 0, rd, 0, 0x12) }
fn mtlo(rs: u32) -> u32 { rtype(rs, 0, 0, 0, 0x13) }
fn mult(rs: u32, rt: u32) -> u32 { rtype(rs, rt, 0, 0, 0x18) }
fn multu(rs: u32, rt: u32) -> u32 { rtype(rs, rt, 0, 0, 0x19) }
fn div(rs: u32, rt: u32) -> u32 { rtype(rs, rt, 0, 0, 0x1A) }
fn divu(rs: u32, rt: u32) -> u32 { rtype(rs, rt, 0, 0, 0x1B) }
fn add(rd: u32, rs: u32, rt: u32) -> u32 { rtype(rs, rt, rd, 0, 0x20) }
fn addu(rd: u32, rs: u32, rt: u32) -> u32 { rtype(rs, rt, rd, 0, 0x21) }
fn sub(rd: u32, rs: u32, rt: u32) -> u32 { rtype(rs, rt, rd, 0, 0x22) }
fn subu(rd: u32, rs: u32, rt: u32) -> u32 { rtype(rs, rt, rd, 0, 0x23) }
fn and(rd: u32, rs: u32, rt: u32) -> u32 { rtype(rs, rt, rd, 0, 0x24) }
fn or(rd: u32, rs: u32, rt: u32) -> u32 { rtype(rs, rt, rd, 0, 0x25) }
fn xor(rd: u32, rs: u32, rt: u32) -> u32 { rtype(rs, rt, rd, 0, 0x26) }
fn nor(rd: u32, rs: u32, rt: u32) -> u32 { rtype(rs, rt, rd, 0, 0x27) }
fn slt(rd: u32, rs: u32, rt: u32) -> u32 { rtype(rs, rt, rd, 0, 0x2A) }
fn sltu(rd: u32, rs: u32, rt: u32) -> u32 { rtype(rs, rt, rd, 0, 0x2B) }

fn mfc0(rt: u32, rd: u32) -> u32 { (0x10 << 26) | (rt << 16) | (rd << 11) }
fn mtc0(rt: u32, rd: u32) -> u32 { (0x10 << 26) | (0x04 << 21) | (rt << 16) | (rd << 11) }
fn rfe() -> u32 { (0x10 << 26) | (0x10 << 21) | 0x10 }

// ----- Test machine -----

/// Address the test programs are loaded at (KSEG0).
const CODE: u32 = 0x8000_1000;
/// Scratch data area used by load/store tests (reachable as KUSEG with $zero).
const DATA: u32 = 0x400;

struct Machine {
    cpu: Interpreter,
    bus: Bus,
}

impl Machine {
    /// Load `program` at `CODE` and point the CPU at it, in kernel mode with
    /// the RAM exception vector selected.
    fn new(program: &[u32]) -> Self {
        let mut bus = Bus::new();
        for (i, word) in program.iter().enumerate() {
            bus.memory.ram.write32((CODE & 0x1FFF_FFFF) as usize + i * 4, *word);
        }
        let mut cpu = Interpreter::new();
        cpu.cp0[COP0_SR] = 0;
        cpu.set_pc(CODE);
        Self { cpu, bus }
    }

    fn step(&mut self) {
        self.cpu.step(&mut self.bus).unwrap();
    }

    fn run(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
    }

    fn reg(&self, r: usize) -> u32 {
        self.cpu.regs[r]
    }

    fn poke32(&mut self, addr: u32, value: u32) {
        self.bus.memory.ram.write32(addr as usize, value);
    }

    fn peek32(&self, addr: u32) -> u32 {
        self.bus.memory.ram.read32(addr as usize)
    }

    fn exc_code(&self) -> u32 {
        (self.cpu.cp0[COP0_CAUSE] >> 2) & 0x1F
    }

    fn epc(&self) -> u32 {
        self.cpu.cp0[COP0_EPC]
    }

    fn in_delay_slot_flag(&self) -> bool {
        self.cpu.cp0[COP0_CAUSE] & CAUSE_BD != 0
    }
}

// ----- Reset / basics -----

#[test]
fn reset_starts_at_bios_entry_point() {
    let mut bus = Bus::new();
    bus.memory.bios.as_mut_slice()[..4].copy_from_slice(&addiu(1, 0, 0x42).to_le_bytes());
    let mut cpu = Interpreter::new();

    assert_eq!(cpu.pc, RESET_VECTOR);
    cpu.step(&mut bus).unwrap();
    assert_eq!(cpu.regs[1], 0x42);
    assert_eq!(cpu.pc, RESET_VECTOR + 4);
}

#[test]
fn reset_restores_power_on_state() {
    let mut m = Machine::new(&[addiu(1, 0, 5)]);
    m.step();
    m.cpu.hi = 7;
    m.cpu.reset();
    assert_eq!(m.cpu.pc, RESET_VECTOR);
    assert_eq!(m.cpu.next_pc, RESET_VECTOR + 4);
    assert_eq!(m.cpu.regs, [0; 32]);
    assert_eq!(m.cpu.hi, 0);
    assert_eq!(m.cpu.cp0[COP0_SR], SR_RESET);
    assert_eq!(m.cpu.cp0[COP0_PRID], 2);
}

#[test]
fn register_zero_is_hardwired() {
    let mut m = Machine::new(&[addiu(0, 0, 5), lui(0, 0x1234), addu(1, 0, 0)]);
    m.run(3);
    assert_eq!(m.reg(0), 0);
    assert_eq!(m.reg(1), 0);
}

// ----- Branch delay slots -----

#[test]
fn jump_executes_delay_slot_before_transferring_control() {
    let mut m = Machine::new(&[
        j(CODE + 0x10),
        addiu(1, 0, 1), // delay slot: must run
        addiu(2, 0, 2), // skipped
        addiu(2, 0, 2), // skipped
        addiu(3, 0, 3), // target
    ]);
    m.run(3);
    assert_eq!(m.reg(1), 1);
    assert_eq!(m.reg(2), 0);
    assert_eq!(m.reg(3), 3);
    assert_eq!(m.cpu.pc, CODE + 0x14);
}

#[test]
fn jump_does_not_change_pc_until_after_the_delay_slot() {
    let mut m = Machine::new(&[j(CODE + 0x10), NOP]);
    m.step();
    assert_eq!(m.cpu.pc, CODE + 4);
    assert_eq!(m.cpu.next_pc, CODE + 0x10);
    m.step();
    assert_eq!(m.cpu.pc, CODE + 0x10);
}

#[test]
fn untaken_branch_still_runs_its_delay_slot_and_falls_through() {
    let mut m = Machine::new(&[
        beq(1, 2, 2),
        addiu(3, 0, 7), // delay slot
        addiu(4, 0, 8), // fall-through
    ]);
    m.cpu.regs[1] = 1;
    m.cpu.regs[2] = 2;
    m.run(3);
    assert_eq!(m.reg(3), 7);
    assert_eq!(m.reg(4), 8);
}

#[test]
fn taken_branch_runs_delay_slot_then_jumps_over_instructions() {
    let mut m = Machine::new(&[
        beq(0, 0, 2),
        addiu(1, 0, 1), // delay slot
        addiu(2, 0, 2), // skipped
        addiu(3, 0, 3), // target = delay slot + 2*4
    ]);
    m.run(3);
    assert_eq!(m.reg(1), 1);
    assert_eq!(m.reg(2), 0);
    assert_eq!(m.reg(3), 3);
}

#[test]
fn backward_branch_loop_uses_signed_offset() {
    let mut m = Machine::new(&[
        addiu(1, 0, 3),   // counter
        addiu(2, 2, 1),   // loop: r2++
        addiu(1, 1, -1),  //       r1--
        bne(1, 0, -3),    //       back to the loop start
        NOP,              //       delay slot
        addiu(3, 0, 9),   // after the loop
    ]);
    m.run(1 + 3 * 4 + 1);
    assert_eq!(m.reg(1), 0);
    assert_eq!(m.reg(2), 3);
    assert_eq!(m.reg(3), 9);
}

#[test]
fn jal_links_past_the_delay_slot_and_jr_returns() {
    let mut m = Machine::new(&[
        jal(CODE + 0x10),
        NOP,
        addiu(5, 0, 5),   // return lands here
        NOP,
        addiu(4, 0, 4),   // function
        jr(31),
        NOP,
    ]);
    m.run(6);
    assert_eq!(m.reg(31), CODE + 8);
    assert_eq!(m.reg(4), 4);
    assert_eq!(m.reg(5), 5);
}

#[test]
fn jalr_links_into_the_chosen_register() {
    let mut m = Machine::new(&[
        lui(1, 0x8000),
        ori(1, 1, 0x1018),
        jalr(2, 1),
        NOP,
        addiu(6, 0, 6), // skipped
        NOP,
        addiu(3, 0, 3), // target
    ]);
    m.run(5);
    assert_eq!(m.reg(2), CODE + 0x10);
    assert_eq!(m.reg(3), 3);
    assert_eq!(m.reg(6), 0);
}

#[test]
fn jalr_with_same_source_and_destination_jumps_to_the_old_value() {
    let mut m = Machine::new(&[jalr(1, 1), NOP, NOP, NOP, addiu(3, 0, 3)]);
    m.cpu.regs[1] = CODE + 0x10;
    m.run(3);
    assert_eq!(m.reg(1), CODE + 8);
    assert_eq!(m.reg(3), 3);
}

#[test]
fn conditional_branches_on_sign() {
    // (instruction, register value, taken?)
    let cases = [
        (bltz(1, 2), 0xFFFF_FFFFu32, true),
        (bltz(1, 2), 0, false),
        (bgez(1, 2), 0, true),
        (bgez(1, 2), 0x8000_0000, false),
        (bgtz(1, 2), 1, true),
        (bgtz(1, 2), 0, false),
        (blez(1, 2), 0, true),
        (blez(1, 2), 1, false),
    ];
    for (instr, value, taken) in cases {
        let mut m = Machine::new(&[instr, NOP, addiu(2, 0, 1), NOP, NOP]);
        m.cpu.regs[1] = value;
        m.run(3);
        // When taken, the addiu (3rd instruction) is skipped.
        assert_eq!(m.reg(2) == 0, taken, "instr {instr:#010x} value {value:#x}");
    }
}

#[test]
fn branch_and_link_writes_ra_even_when_not_taken() {
    let mut m = Machine::new(&[bltzal(1, 4), NOP]);
    m.cpu.regs[1] = 5;
    m.run(2);
    assert_eq!(m.reg(31), CODE + 8);
    assert_eq!(m.cpu.pc, CODE + 8);

    let mut m = Machine::new(&[bgezal(1, 4), NOP]);
    m.cpu.regs[1] = 5;
    m.run(2);
    assert_eq!(m.reg(31), CODE + 8);
    assert_eq!(m.cpu.pc, CODE + 4 + 16);
}

#[test]
fn misaligned_jump_target_faults_on_fetch() {
    let mut m = Machine::new(&[jr(1), NOP]);
    m.cpu.regs[1] = CODE + 2;
    m.run(3);
    assert_eq!(m.exc_code(), Exception::AddressErrorLoad as u32);
    assert_eq!(m.cpu.cp0[COP0_BADVADDR], CODE + 2);
    assert_eq!(m.epc(), CODE + 2);
    assert!(!m.in_delay_slot_flag());
}

// ----- Load delay slots -----

#[test]
fn loaded_register_is_visible_only_after_the_next_instruction() {
    let mut m = Machine::new(&[
        lw(1, DATA as i32, 0),
        addu(2, 1, 0), // still sees the old r1
        addu(3, 1, 0), // sees the loaded value
    ]);
    m.cpu.regs[1] = 5;
    m.poke32(DATA, 99);
    m.run(3);
    assert_eq!(m.reg(2), 5);
    assert_eq!(m.reg(3), 99);
    assert_eq!(m.reg(1), 99);
}

#[test]
fn write_in_the_load_delay_slot_cancels_the_load() {
    let mut m = Machine::new(&[lw(1, DATA as i32, 0), addiu(1, 0, 7), NOP]);
    m.poke32(DATA, 99);
    m.run(3);
    assert_eq!(m.reg(1), 7);
}

#[test]
fn load_into_r0_is_ignored() {
    let mut m = Machine::new(&[lw(0, DATA as i32, 0), NOP, addu(1, 0, 0)]);
    m.poke32(DATA, 99);
    m.run(3);
    assert_eq!(m.reg(0), 0);
    assert_eq!(m.reg(1), 0);
}

#[test]
fn load_delay_survives_a_branch() {
    let mut m = Machine::new(&[
        j(CODE + 0x10),
        lw(1, DATA as i32, 0), // load in the delay slot
        NOP,
        NOP,
        addu(2, 1, 0), // first instruction at the target: old r1
        addu(3, 1, 0), // new r1
    ]);
    m.cpu.regs[1] = 5;
    m.poke32(DATA, 99);
    m.run(4);
    assert_eq!(m.reg(2), 5);
    m.step();
    assert_eq!(m.reg(3), 99);
}

// ----- Loads and stores -----

#[test]
fn byte_loads_sign_and_zero_extend() {
    let mut m = Machine::new(&[
        lb(1, DATA as i32, 0),
        lbu(2, DATA as i32, 0),
        lb(3, DATA as i32 + 1, 0),
        NOP,
        NOP,
    ]);
    m.poke32(DATA, 0x1234_5680);
    m.run(5);
    assert_eq!(m.reg(1), 0xFFFF_FF80);
    assert_eq!(m.reg(2), 0x80);
    assert_eq!(m.reg(3), 0x56);
}

#[test]
fn halfword_loads_sign_and_zero_extend() {
    let mut m = Machine::new(&[
        lh(1, DATA as i32, 0),
        lhu(2, DATA as i32, 0),
        lh(3, DATA as i32 + 2, 0),
        NOP,
        NOP,
    ]);
    m.poke32(DATA, 0x1234_8680);
    m.run(5);
    assert_eq!(m.reg(1), 0xFFFF_8680);
    assert_eq!(m.reg(2), 0x8680);
    assert_eq!(m.reg(3), 0x1234);
}

#[test]
fn stores_write_the_requested_width() {
    let mut m = Machine::new(&[
        sw(1, DATA as i32, 0),
        sh(2, DATA as i32 + 4, 0),
        sb(3, DATA as i32 + 8, 0),
    ]);
    m.cpu.regs[1] = 0xAABB_CCDD;
    m.cpu.regs[2] = 0x1111_2222;
    m.cpu.regs[3] = 0x3333_3344;
    m.run(3);
    assert_eq!(m.peek32(DATA), 0xAABB_CCDD);
    assert_eq!(m.peek32(DATA + 4), 0x0000_2222);
    assert_eq!(m.peek32(DATA + 8), 0x0000_0044);
}

#[test]
fn misaligned_accesses_raise_address_errors() {
    // lw
    let mut m = Machine::new(&[lw(1, DATA as i32 + 1, 0)]);
    m.cpu.regs[1] = 0x55;
    m.step();
    assert_eq!(m.exc_code(), Exception::AddressErrorLoad as u32);
    assert_eq!(m.cpu.cp0[COP0_BADVADDR], DATA + 1);
    assert_eq!(m.epc(), CODE);
    assert_eq!(m.cpu.pc, EXCEPTION_VECTOR_RAM);
    assert_eq!(m.reg(1), 0x55);

    // lh
    let mut m = Machine::new(&[lh(1, DATA as i32 + 1, 0)]);
    m.step();
    assert_eq!(m.exc_code(), Exception::AddressErrorLoad as u32);

    // sw / sh
    for instr in [sw(1, DATA as i32 + 2, 0), sh(1, DATA as i32 + 1, 0)] {
        let mut m = Machine::new(&[instr]);
        m.cpu.regs[1] = 0xFFFF_FFFF;
        m.step();
        assert_eq!(m.exc_code(), Exception::AddressErrorStore as u32);
        assert_eq!(m.peek32(DATA), 0);
    }
}

#[test]
fn unaligned_load_pair_assembles_a_word() {
    // lwr rt, 1(base) ; lwl rt, 4(base)  loads the word at address DATA+1.
    let mut m = Machine::new(&[
        lwr(1, DATA as i32 + 1, 0),
        lwl(1, DATA as i32 + 4, 0),
        NOP,
    ]);
    m.cpu.regs[1] = 0xFFFF_FFFF;
    m.poke32(DATA, 0x0403_0201);
    m.poke32(DATA + 4, 0x0807_0605);
    m.run(3);
    assert_eq!(m.reg(1), 0x0504_0302);
}

#[test]
fn unaligned_load_pair_works_for_every_offset() {
    for offset in 0..4u32 {
        let mut m = Machine::new(&[
            lwr(1, (DATA + offset) as i32, 0),
            lwl(1, (DATA + offset + 3) as i32, 0),
            NOP,
        ]);
        let bytes = [0x10u8, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17];
        for (i, b) in bytes.iter().enumerate() {
            m.bus.memory.ram.write8(DATA as usize + i, *b);
        }
        m.cpu.regs[1] = 0xFFFF_FFFF;
        m.run(3);
        let o = offset as usize;
        let expected = u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        assert_eq!(m.reg(1), expected, "offset {offset}");
    }
}

#[test]
fn unaligned_store_pair_scatters_a_word() {
    let mut m = Machine::new(&[
        swr(1, DATA as i32 + 1, 0),
        swl(1, DATA as i32 + 4, 0),
    ]);
    m.cpu.regs[1] = 0xAABB_CCDD;
    m.run(2);
    assert_eq!(m.peek32(DATA), 0xBBCC_DD00);
    assert_eq!(m.peek32(DATA + 4), 0x0000_00AA);
}

#[test]
fn stores_do_not_modify_memory_while_the_cache_is_isolated() {
    let mut m = Machine::new(&[sw(1, DATA as i32, 0)]);
    m.cpu.cp0[COP0_SR] = SR_ISC;
    m.cpu.regs[1] = 0x1234;
    m.step();
    assert_eq!(m.peek32(DATA), 0);
}

// ----- ALU -----

#[test]
fn immediates_are_extended_correctly() {
    let mut m = Machine::new(&[
        addiu(1, 0, -1),      // sign-extended
        ori(2, 0, 0x8000),    // zero-extended
        andi(3, 1, 0x8000),   // zero-extended mask
        xori(4, 1, 0xFFFF),
        lui(5, 0x8123),
    ]);
    m.run(5);
    assert_eq!(m.reg(1), 0xFFFF_FFFF);
    assert_eq!(m.reg(2), 0x0000_8000);
    assert_eq!(m.reg(3), 0x0000_8000);
    assert_eq!(m.reg(4), 0xFFFF_0000);
    assert_eq!(m.reg(5), 0x8123_0000);
}

#[test]
fn logic_operations() {
    let mut m = Machine::new(&[and(3, 1, 2), or(4, 1, 2), xor(5, 1, 2), nor(6, 1, 2)]);
    m.cpu.regs[1] = 0xFF00_FF00;
    m.cpu.regs[2] = 0x0FF0_0FF0;
    m.run(4);
    assert_eq!(m.reg(3), 0x0F00_0F00);
    assert_eq!(m.reg(4), 0xFFF0_FFF0);
    assert_eq!(m.reg(5), 0xF0F0_F0F0);
    assert_eq!(m.reg(6), 0x000F_000F);
}

#[test]
fn set_less_than_distinguishes_signed_and_unsigned() {
    let mut m = Machine::new(&[
        slti(2, 1, 0),   // -1 < 0 signed
        sltiu(3, 1, 0),  // 0xFFFFFFFF < 0 unsigned
        sltiu(4, 0, -1), // 0 < 0xFFFFFFFF (immediate sign-extended)
        slt(5, 1, 0),
        sltu(6, 1, 0),
    ]);
    m.cpu.regs[1] = 0xFFFF_FFFF;
    m.run(5);
    assert_eq!(m.reg(2), 1);
    assert_eq!(m.reg(3), 0);
    assert_eq!(m.reg(4), 1);
    assert_eq!(m.reg(5), 1);
    assert_eq!(m.reg(6), 0);
}

#[test]
fn shifts_immediate_and_variable() {
    let mut m = Machine::new(&[
        sll(4, 1, 4),
        srl(5, 1, 4),
        sra(6, 1, 4),
        sllv(7, 1, 2),
        srlv(8, 1, 2),
        srav(9, 1, 2),
    ]);
    m.cpu.regs[1] = 0x8000_0001;
    m.cpu.regs[2] = 33; // only the low 5 bits count -> shift by 1
    m.run(6);
    assert_eq!(m.reg(4), 0x0000_0010);
    assert_eq!(m.reg(5), 0x0800_0000);
    assert_eq!(m.reg(6), 0xF800_0000);
    assert_eq!(m.reg(7), 0x0000_0002);
    assert_eq!(m.reg(8), 0x4000_0000);
    assert_eq!(m.reg(9), 0xC000_0000);
}

#[test]
fn trapping_arithmetic_raises_overflow_and_leaves_the_destination_alone() {
    for instr in [addi(2, 1, 1), add(2, 1, 1), sub(2, 3, 1)] {
        let mut m = Machine::new(&[instr]);
        m.cpu.regs[1] = 0x7FFF_FFFF;
        m.cpu.regs[2] = 0x55;
        m.cpu.regs[3] = 0x8000_0000; // INT_MIN - INT_MAX overflows
        m.step();
        assert_eq!(m.exc_code(), Exception::Overflow as u32, "instr {instr:#010x}");
        assert_eq!(m.epc(), CODE);
        assert_eq!(m.reg(2), 0x55);
        assert_eq!(m.cpu.pc, EXCEPTION_VECTOR_RAM);
    }
}

#[test]
fn non_trapping_arithmetic_wraps() {
    let mut m = Machine::new(&[addu(2, 1, 1), subu(3, 0, 1), addiu(4, 1, 1)]);
    m.cpu.regs[1] = 0x7FFF_FFFF;
    m.run(3);
    assert_eq!(m.reg(2), 0xFFFF_FFFE);
    assert_eq!(m.reg(3), 0x8000_0001);
    assert_eq!(m.reg(4), 0x8000_0000);
    assert_eq!(m.cpu.cp0[COP0_CAUSE], 0);
}

#[test]
fn signed_and_unsigned_multiply() {
    let mut m = Machine::new(&[mult(1, 2), mflo(3), mfhi(4), multu(5, 6), mflo(7), mfhi(8)]);
    m.cpu.regs[1] = (-2i32) as u32;
    m.cpu.regs[2] = 3;
    m.cpu.regs[5] = 0xFFFF_FFFF;
    m.cpu.regs[6] = 2;
    m.run(6);
    assert_eq!(m.reg(3), 0xFFFF_FFFA);
    assert_eq!(m.reg(4), 0xFFFF_FFFF);
    assert_eq!(m.reg(7), 0xFFFF_FFFE);
    assert_eq!(m.reg(8), 1);
}

#[test]
fn division_results() {
    let mut m = Machine::new(&[div(1, 2), mflo(3), mfhi(4), divu(5, 6), mflo(7), mfhi(8)]);
    m.cpu.regs[1] = (-7i32) as u32;
    m.cpu.regs[2] = 2;
    m.cpu.regs[5] = 0xFFFF_FFFF;
    m.cpu.regs[6] = 0x10;
    m.run(6);
    assert_eq!(m.reg(3) as i32, -3);
    assert_eq!(m.reg(4) as i32, -1);
    assert_eq!(m.reg(7), 0x0FFF_FFFF);
    assert_eq!(m.reg(8), 0xF);
}

#[test]
fn division_edge_cases_match_hardware() {
    // (dividend, divisor, expected lo, expected hi)
    let signed = [
        (5i32, 0i32, 0xFFFF_FFFFu32, 5u32),
        (-5, 0, 1, (-5i32) as u32),
        (i32::MIN, -1, 0x8000_0000, 0),
    ];
    for (a, b, lo, hi) in signed {
        let mut m = Machine::new(&[div(1, 2)]);
        m.cpu.regs[1] = a as u32;
        m.cpu.regs[2] = b as u32;
        m.step();
        assert_eq!((m.cpu.lo, m.cpu.hi), (lo, hi), "div {a} / {b}");
        assert_eq!(m.cpu.cp0[COP0_CAUSE], 0);
    }

    let mut m = Machine::new(&[divu(1, 2)]);
    m.cpu.regs[1] = 123;
    m.step();
    assert_eq!((m.cpu.lo, m.cpu.hi), (0xFFFF_FFFF, 123));
}

#[test]
fn hi_lo_moves() {
    let mut m = Machine::new(&[mthi(1), mtlo(2), mfhi(3), mflo(4)]);
    m.cpu.regs[1] = 0x1111;
    m.cpu.regs[2] = 0x2222;
    m.run(4);
    assert_eq!(m.reg(3), 0x1111);
    assert_eq!(m.reg(4), 0x2222);
}

// ----- Exceptions -----

#[test]
fn syscall_and_break_enter_the_handler() {
    let mut m = Machine::new(&[NOP, syscall()]);
    m.run(2);
    assert_eq!(m.exc_code(), Exception::Syscall as u32);
    assert_eq!(m.epc(), CODE + 4);
    assert_eq!(m.cpu.pc, EXCEPTION_VECTOR_RAM);
    assert_eq!(m.cpu.next_pc, EXCEPTION_VECTOR_RAM + 4);

    let mut m = Machine::new(&[brk()]);
    m.step();
    assert_eq!(m.exc_code(), Exception::Breakpoint as u32);
}

#[test]
fn exception_vector_depends_on_bev() {
    let mut m = Machine::new(&[syscall()]);
    m.cpu.cp0[COP0_SR] = SR_BEV;
    m.step();
    assert_eq!(m.cpu.pc, EXCEPTION_VECTOR_BIOS);
}

#[test]
fn exception_in_delay_slot_points_epc_at_the_branch() {
    let mut m = Machine::new(&[j(CODE + 0x10), syscall()]);
    m.run(2);
    assert_eq!(m.exc_code(), Exception::Syscall as u32);
    assert_eq!(m.epc(), CODE);
    assert!(m.in_delay_slot_flag());
    assert_eq!(m.cpu.pc, EXCEPTION_VECTOR_RAM);

    // BD is cleared by a later exception outside a delay slot.
    let mut m = Machine::new(&[j(CODE + 8), syscall(), syscall()]);
    m.run(2);
    assert!(m.in_delay_slot_flag());
    m.cpu.set_pc(CODE + 8);
    m.step();
    assert!(!m.in_delay_slot_flag());
}

#[test]
fn exception_pushes_the_mode_stack_and_rfe_pops_it() {
    let mut m = Machine::new(&[syscall()]);
    m.poke32(EXCEPTION_VECTOR_RAM & 0x1FFF_FFFF, rfe());
    m.cpu.cp0[COP0_SR] = 0b01_0101; // IEc, KUp/IEp patterns to see shifting
    m.step();
    assert_eq!(m.cpu.cp0[COP0_SR] & 0x3F, 0b01_0100);
    m.step(); // rfe
    assert_eq!(m.cpu.cp0[COP0_SR] & 0x3F, 0b01_0101);
}

#[test]
fn reserved_instructions_raise_an_exception() {
    for instr in [0x1F << 26, rtype(0, 0, 0, 0, 0x3F), 0x3C << 26] {
        let mut m = Machine::new(&[instr]);
        m.step();
        assert_eq!(m.exc_code(), Exception::ReservedInstruction as u32, "{instr:#010x}");
        assert_eq!(m.epc(), CODE);
    }
}

#[test]
fn unusable_coprocessors_report_their_number() {
    for (instr, cop) in [(0x11u32 << 26, 1u32), (0x13 << 26, 3), (0x12 << 26, 2)] {
        let mut m = Machine::new(&[instr]);
        m.step();
        assert_eq!(m.exc_code(), Exception::CoprocessorUnusable as u32);
        assert_eq!((m.cpu.cp0[COP0_CAUSE] >> 28) & 3, cop);
    }
}

// ----- COP2 / GTE -----

fn mtc2(rt: u32, rd: u32) -> u32 { (0x12 << 26) | (0x04 << 21) | (rt << 16) | (rd << 11) }
fn mfc2(rt: u32, rd: u32) -> u32 { (0x12 << 26) | (rt << 16) | (rd << 11) }
fn ctc2(rt: u32, rd: u32) -> u32 { (0x12 << 26) | (0x06 << 21) | (rt << 16) | (rd << 11) }
fn cfc2(rt: u32, rd: u32) -> u32 { (0x12 << 26) | (0x02 << 21) | (rt << 16) | (rd << 11) }
fn lwc2(rt: u32, off: i32, base: u32) -> u32 { itype(0x32, base, rt, off) }
fn swc2(rt: u32, off: i32, base: u32) -> u32 { itype(0x3A, base, rt, off) }
/// A GTE command word (bit 25 set).
fn gte_command(opcode: u32) -> u32 { (0x12 << 26) | (1 << 25) | opcode }

fn gte_machine(program: &[u32]) -> Machine {
    let mut m = Machine::new(program);
    m.cpu.cp0[COP0_SR] = SR_CU2;
    m
}

#[test]
fn gte_register_moves_go_both_ways() {
    // The BIOS's first GTE instructions: ctc2 into ZSF3 / ZSF4.
    let mut m = gte_machine(&[
        addiu(8, 0, 0x155),
        ctc2(8, 29),
        addiu(8, 0, 0x100),
        ctc2(8, 30),
        cfc2(2, 29),
        cfc2(3, 30),
        NOP,
    ]);
    m.run(7);
    assert_eq!(m.reg(2), 0x155);
    assert_eq!(m.reg(3), 0x100);
    assert_eq!(m.cpu.gte.control[29], 0x155);
}

#[test]
fn mtc2_and_mfc2_use_data_registers_with_a_load_delay() {
    let mut m = gte_machine(&[
        addiu(1, 0, -2),
        mtc2(1, 9), // IR1 = -2
        mfc2(2, 9),
        addu(3, 2, 0), // load delay slot: still the old r2
        addu(4, 2, 0), // sees the loaded value
    ]);
    m.cpu.regs[2] = 0x55;
    m.run(5);
    assert_eq!(m.reg(3), 0x55);
    assert_eq!(m.reg(4), 0xFFFF_FFFE, "IR1 reads back sign-extended");
}

#[test]
fn lwc2_and_swc2_move_words_between_memory_and_gte() {
    let mut m = gte_machine(&[
        lwc2(6, DATA as i32, 0),       // RGBC <- [DATA]
        swc2(6, DATA as i32 + 8, 0),   // [DATA+8] <- RGBC
        lwc2(15, DATA as i32 + 4, 0),  // SXYP push
        mfc2(5, 14),
        NOP,
    ]);
    m.poke32(DATA, 0x1122_3344);
    m.poke32(DATA + 4, 0x0005_0006);
    m.run(5);
    assert_eq!(m.cpu.gte.data[6], 0x1122_3344);
    assert_eq!(m.peek32(DATA + 8), 0x1122_3344);
    assert_eq!(m.reg(5), 0x0005_0006);
}

#[test]
fn lwc2_and_swc2_check_alignment() {
    let mut m = gte_machine(&[lwc2(6, DATA as i32 + 2, 0)]);
    m.step();
    assert_eq!(m.exc_code(), Exception::AddressErrorLoad as u32);

    let mut m = gte_machine(&[swc2(6, DATA as i32 + 1, 0)]);
    m.step();
    assert_eq!(m.exc_code(), Exception::AddressErrorStore as u32);
}

#[test]
fn gte_commands_run_and_their_results_are_readable() {
    // NCLIP on a counter-clockwise triangle: MAC0 = 100.
    let mut m = gte_machine(&[
        mtc2(1, 15),
        mtc2(2, 15),
        mtc2(3, 15),
        gte_command(0x06),
        mfc2(4, 24),
        NOP,
    ]);
    m.cpu.regs[1] = 0x0000_0000; // (0, 0)
    m.cpu.regs[2] = 0x0000_000A; // (10, 0)
    m.cpu.regs[3] = 0x000A_0000; // (0, 10)
    m.run(6);
    assert_eq!(m.reg(4), 100);
}

#[test]
fn gte_flag_register_is_visible_through_cfc2() {
    // Divide overflow: RTPS with H = 600 and a depth of 100.
    let mut m = gte_machine(&[
        ctc2(1, 58), // H
        mtc2(2, 1),  // VZ0
        gte_command(0x01 | (1 << 19)),
        cfc2(3, 63),
        NOP,
    ]);
    m.cpu.regs[1] = 600;
    m.cpu.regs[2] = 100;
    m.run(5);
    assert_eq!(m.reg(3) & (1 << 17), 1 << 17, "divide overflow");
    assert_eq!(m.reg(3) >> 31, 1, "summary bit");
}

#[test]
fn every_cop2_instruction_needs_cu2() {
    for instr in [mtc2(1, 9), mfc2(1, 9), ctc2(1, 9), cfc2(1, 9), gte_command(0x06), lwc2(1, 0, 0), swc2(1, 0, 0)] {
        let mut m = Machine::new(&[instr]); // SR = 0: CU2 clear
        m.step();
        assert_eq!(m.exc_code(), Exception::CoprocessorUnusable as u32, "{instr:#010x}");
        assert_eq!((m.cpu.cp0[COP0_CAUSE] >> 28) & 3, 2);
    }
}

#[test]
fn unknown_cop2_moves_are_reserved_instructions() {
    let mut m = gte_machine(&[(0x12 << 26) | (0x08 << 21)]); // BC2
    m.step();
    assert_eq!(m.exc_code(), Exception::ReservedInstruction as u32);
}

#[test]
fn gte_state_is_cleared_by_cpu_reset() {
    let mut m = gte_machine(&[mtc2(1, 9)]);
    m.cpu.regs[1] = 5;
    m.step();
    assert_eq!(m.cpu.gte.data[9], 5);
    m.cpu.reset();
    assert_eq!(m.cpu.gte.data[9], 0);
}

#[test]
fn bus_failures_are_fatal_errors_not_cpu_exceptions() {
    let mut m = Machine::new(&[lui(1, 0xBFA0), lw(2, 0, 1), NOP]);
    m.run(1);
    let result = m.cpu.step(&mut m.bus);
    assert!(matches!(result, Err(Ps1Error::MemoryOutOfRange(_))));
}

#[test]
fn user_mode_cannot_touch_kernel_segments() {
    // Fetching from KSEG0 in user mode.
    let mut m = Machine::new(&[NOP]);
    m.cpu.cp0[COP0_SR] = SR_KUC;
    m.step();
    assert_eq!(m.exc_code(), Exception::AddressErrorLoad as u32);
    assert_eq!(m.cpu.cp0[COP0_BADVADDR], CODE);

    // Loading from KSEG0 in user mode (code runs from KUSEG).
    let mut m = Machine::new(&[lui(2, 0x8000), lw(1, DATA as i32, 2)]);
    m.cpu.cp0[COP0_SR] = SR_KUC;
    m.cpu.set_pc(CODE & 0x1FFF_FFFF);
    m.run(2);
    assert_eq!(m.exc_code(), Exception::AddressErrorLoad as u32);
    assert_eq!(m.cpu.cp0[COP0_BADVADDR], 0x8000_0000 + DATA);
}

// ----- COP0 -----

#[test]
fn mtc0_and_mfc0_move_status_with_masking() {
    let mut m = Machine::new(&[mtc0(1, COP0_SR as u32), mfc0(2, COP0_SR as u32), NOP]);
    // Keep KUc/IEc clear so the CPU stays in kernel mode and can keep fetching.
    let value = 0xFFFF_FFFC;
    m.cpu.regs[1] = value;
    m.run(3);
    assert_eq!(m.cpu.cp0[COP0_SR], value & SR_WRITE_MASK);
    assert_ne!(value & SR_WRITE_MASK, value, "reserved bits must be dropped");
    assert_eq!(m.reg(2), value & SR_WRITE_MASK);
}

#[test]
fn mfc0_has_a_load_delay() {
    let mut m = Machine::new(&[
        mfc0(1, COP0_PRID as u32),
        addu(2, 1, 0), // old value
        addu(3, 1, 0), // PRId
    ]);
    m.cpu.regs[1] = 77;
    m.run(3);
    assert_eq!(m.reg(2), 77);
    assert_eq!(m.reg(3), 2);
}

#[test]
fn only_software_interrupt_bits_of_cause_are_writable() {
    let mut m = Machine::new(&[mtc0(1, COP0_CAUSE as u32)]);
    m.cpu.regs[1] = 0xFFFF_FFFF;
    m.step();
    assert_eq!(m.cpu.cp0[COP0_CAUSE], 0x0300);
}

#[test]
fn read_only_cop0_registers_ignore_writes() {
    let mut m = Machine::new(&[
        mtc0(1, COP0_EPC as u32),
        mtc0(1, COP0_BADVADDR as u32),
        mtc0(1, COP0_PRID as u32),
    ]);
    m.cpu.regs[1] = 0x1234;
    m.run(3);
    assert_eq!(m.cpu.cp0[COP0_EPC], 0);
    assert_eq!(m.cpu.cp0[COP0_BADVADDR], 0);
    assert_eq!(m.cpu.cp0[COP0_PRID], 2);
}

#[test]
fn cop0_is_unusable_from_user_mode_without_cu0() {
    let mut m = Machine::new(&[mfc0(1, COP0_SR as u32)]);
    m.cpu.cp0[COP0_SR] = SR_KUC;
    m.cpu.set_pc(CODE & 0x1FFF_FFFF);
    m.step();
    assert_eq!(m.exc_code(), Exception::CoprocessorUnusable as u32);
    assert_eq!((m.cpu.cp0[COP0_CAUSE] >> 28) & 3, 0);

    let mut m = Machine::new(&[mfc0(1, COP0_SR as u32), NOP]);
    m.cpu.cp0[COP0_SR] = SR_KUC | SR_CU0;
    m.cpu.set_pc(CODE & 0x1FFF_FFFF);
    m.run(2);
    assert_eq!(m.reg(1), SR_KUC | SR_CU0);
}

// ----- Interrupts -----

#[test]
fn interrupt_is_taken_before_the_next_instruction() {
    let mut m = Machine::new(&[addiu(1, 0, 1)]);
    m.cpu.cp0[COP0_SR] = SR_IEC | CAUSE_IP2;
    m.cpu.set_irq(true);
    m.step();
    assert_eq!(m.exc_code(), Exception::Interrupt as u32);
    assert_eq!(m.epc(), CODE);
    assert_eq!(m.reg(1), 0, "the interrupted instruction must not have run");
    assert_eq!(m.cpu.pc, EXCEPTION_VECTOR_RAM);
    // IEc is pushed off, so no re-entry.
    assert_eq!(m.cpu.cp0[COP0_SR] & SR_IEC, 0);
}

#[test]
fn masked_or_disabled_interrupts_are_ignored() {
    // Interrupts globally disabled.
    let mut m = Machine::new(&[addiu(1, 0, 1)]);
    m.cpu.cp0[COP0_SR] = CAUSE_IP2;
    m.cpu.set_irq(true);
    m.step();
    assert_eq!(m.reg(1), 1);

    // IM2 clear.
    let mut m = Machine::new(&[addiu(1, 0, 1)]);
    m.cpu.cp0[COP0_SR] = SR_IEC;
    m.cpu.set_irq(true);
    m.step();
    assert_eq!(m.reg(1), 1);

    // Line deasserted.
    let mut m = Machine::new(&[addiu(1, 0, 1)]);
    m.cpu.cp0[COP0_SR] = SR_IEC | CAUSE_IP2;
    m.cpu.set_irq(true);
    m.cpu.set_irq(false);
    m.step();
    assert_eq!(m.reg(1), 1);
}

#[test]
fn software_interrupts_use_cause_bits() {
    let mut m = Machine::new(&[mtc0(1, COP0_CAUSE as u32), NOP]);
    m.cpu.cp0[COP0_SR] = SR_IEC | 0x0100; // IM0
    m.cpu.regs[1] = 0x0100;
    m.run(2);
    assert_eq!(m.exc_code(), Exception::Interrupt as u32);
}

#[test]
fn interrupt_in_a_delay_slot_returns_to_the_branch() {
    let mut m = Machine::new(&[j(CODE + 0x10), addiu(1, 0, 1), NOP, NOP, NOP]);
    m.cpu.cp0[COP0_SR] = SR_IEC | CAUSE_IP2;
    m.step(); // the jump
    m.cpu.set_irq(true);
    m.step(); // interrupt fires before the delay slot executes
    assert_eq!(m.exc_code(), Exception::Interrupt as u32);
    assert_eq!(m.epc(), CODE, "EPC must point at the branch");
    assert!(m.in_delay_slot_flag());
    assert_eq!(m.reg(1), 0);
}

#[test]
fn interrupt_lets_a_pending_load_complete() {
    let mut m = Machine::new(&[lw(1, DATA as i32, 0), NOP]);
    m.poke32(DATA, 0xABCD);
    m.step(); // load issued
    m.cpu.cp0[COP0_SR] = SR_IEC | CAUSE_IP2;
    m.cpu.set_irq(true);
    m.step(); // interrupt; the load still lands
    assert_eq!(m.reg(1), 0xABCD);
}
