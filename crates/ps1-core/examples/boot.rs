//! Host-side boot harness.
//!
//! Inspects a disc image and, when a BIOS is supplied, runs the emulated CPU
//! for a number of instructions and reports how far the boot got.
//!
//! ```text
//! cargo run -p ps1-core --example boot --target x86_64-pc-windows-msvc -- \
//!     --disc "rom_test/Resident Evil (USA)/Resident Evil (USA).cue" \
//!     --bios path/to/your/bios.bin --steps 20000000
//! ```
//!
//! Neither a BIOS nor a game is bundled with this repository; bring your own
//! dumps.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use ps1_core::System;

const RAW_SECTOR: u64 = 2352;
const SYNC: [u8; 12] = [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0];
/// Offset of the user data in a raw Mode 2 Form 1 sector (sync + header + subheader).
const MODE2_DATA_OFFSET: u64 = 24;
/// The ISO 9660 primary volume descriptor lives at LBA 16.
const PVD_LBA: u64 = 16;

struct Args {
    bios: Option<PathBuf>,
    disc: Option<PathBuf>,
    steps: u64,
    /// Addresses whose 32-bit contents are printed after the run.
    peek: Vec<u32>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args { bios: None, disc: None, steps: 10_000_000, peek: Vec::new() };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--bios" => args.bios = Some(PathBuf::from(value("--bios")?)),
            "--disc" => args.disc = Some(PathBuf::from(value("--disc")?)),
            "--steps" => {
                args.steps = value("--steps")?
                    .replace('_', "")
                    .parse()
                    .map_err(|e| format!("--steps: {e}"))?
            }
            "--peek" => {
                let text = value("--peek")?;
                let hex = text.trim_start_matches("0x");
                args.peek.push(u32::from_str_radix(hex, 16).map_err(|e| format!("--peek: {e}"))?)
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(args)
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\nusage: boot [--disc game.cue] [--bios bios.bin] [--steps N]");
            std::process::exit(2);
        }
    };

    if let Some(disc) = &args.disc {
        match inspect_disc(disc) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("disc: {e}");
                std::process::exit(1);
            }
        }
    }

    match &args.bios {
        Some(bios) => run_bios(bios, args.steps, &args.peek),
        None => println!("\nNo --bios given: skipping CPU boot (a BIOS dump is required)."),
    }
}

// ----- Disc inspection -----

/// Resolve the BIN file a CUE sheet points at, and its first track's mode.
fn read_cue(cue: &Path) -> Result<(PathBuf, String), String> {
    let text = std::fs::read_to_string(cue).map_err(|e| format!("{}: {e}", cue.display()))?;
    let mut file = None;
    let mut mode = None;
    for line in text.lines().map(str::trim) {
        if file.is_none() && line.starts_with("FILE") {
            let start = line.find('"').ok_or("malformed FILE line")? + 1;
            let end = line.rfind('"').ok_or("malformed FILE line")?;
            file = Some(line[start..end].to_string());
        } else if mode.is_none() && line.starts_with("TRACK") {
            mode = line.split_whitespace().nth(2).map(str::to_string);
        }
    }
    let file = file.ok_or("no FILE entry in the CUE sheet")?;
    let bin = cue.parent().unwrap_or(Path::new(".")).join(file);
    Ok((bin, mode.ok_or("no TRACK entry in the CUE sheet")?))
}

fn inspect_disc(cue: &Path) -> Result<(), String> {
    let (bin, mode) = read_cue(cue)?;
    println!("CUE:    {}", cue.display());
    println!("BIN:    {} (track 1: {mode})", bin.display());
    if mode != "MODE2/2352" {
        return Err(format!("unsupported track mode {mode}, expected MODE2/2352"));
    }

    let mut file = File::open(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    println!(
        "Size:   {len} bytes = {} sectors of {RAW_SECTOR} bytes",
        len / RAW_SECTOR
    );
    if len % RAW_SECTOR != 0 {
        return Err("image size is not a whole number of raw sectors".into());
    }

    let mut sector = vec![0u8; RAW_SECTOR as usize];
    file.read_exact(&mut sector).map_err(|e| e.to_string())?;
    println!("Sync:   {}", if sector[..12] == SYNC { "ok" } else { "MISSING" });

    file.seek(SeekFrom::Start(PVD_LBA * RAW_SECTOR)).map_err(|e| e.to_string())?;
    file.read_exact(&mut sector).map_err(|e| e.to_string())?;
    let pvd = &sector[MODE2_DATA_OFFSET as usize..];
    if &pvd[1..6] != b"CD001" || pvd[0] != 1 {
        return Err("no ISO 9660 primary volume descriptor at LBA 16".into());
    }
    let text = |range: std::ops::Range<usize>| {
        String::from_utf8_lossy(&pvd[range]).trim().to_string()
    };
    println!("System: {}", text(8..40));
    println!("Volume: {}", text(40..72));
    Ok(())
}

// ----- CPU boot -----

fn run_bios(path: &Path, steps: u64, peek: &[u32]) {
    let bios = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("bios: {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    println!("\nBIOS:   {} ({} bytes)", path.display(), bios.len());

    let mut sys = System::new();
    sys.load_bios(&bios).expect("BIOS image too large");
    sys.reset().expect("reset");

    let mut recent_pcs = vec![0u32; 16];
    let mut histogram: HashMap<u32, u64> = HashMap::new();
    let mut last_post = 0u8;
    let mut executed = 0u64;
    let mut failure = None;

    for i in 0..steps {
        let pc = sys.cpu.pc;
        recent_pcs[(i % 16) as usize] = pc;
        // Only the last 100k instructions feed the "where is it stuck" histogram.
        if i + 100_000 >= steps {
            *histogram.entry(pc).or_default() += 1;
        }
        if let Err(e) = sys.step() {
            failure = Some(e);
            break;
        }
        executed += 1;
        if sys.bus.post_code != last_post {
            last_post = sys.bus.post_code;
            println!("  [{executed:>10}] POST code {last_post:#04X}");
        }
    }

    println!("\nExecuted {executed} instructions.");
    println!("pc = {:#010X}", sys.cpu.pc);
    if let Some(e) = failure {
        println!("STOPPED: {e}");
        let start = (executed % 16) as usize;
        let trail: Vec<String> = (0..16)
            .map(|k| format!("{:#010X}", recent_pcs[(start + k) % 16]))
            .collect();
        println!("last pcs: {}", trail.join(" "));
    } else {
        let mut hot: Vec<_> = histogram.into_iter().collect();
        hot.sort_by(|a, b| b.1.cmp(&a.1));
        println!("hottest pcs in the last 100k instructions (by address):");
        let mut loop_body: Vec<_> = hot.iter().take(40).copied().collect();
        loop_body.sort_by_key(|(pc, _)| *pc);
        for (pc, n) in loop_body {
            let word = sys.bus.read32(pc).unwrap_or(0);
            println!("  {pc:#010X}  x{n:<6} {word:08X}  {}", disasm(pc, word));
        }
    }

    println!("registers:");
    for row in 0..8 {
        let cells: Vec<String> = (0..4)
            .map(|col| {
                let r = row * 4 + col;
                format!("{:>4}={:08X}", reg(r as u32), sys.cpu.regs[r])
            })
            .collect();
        println!("  {}", cells.join("  "));
    }
    println!(
        "  hi={:08X} lo={:08X} sr={:08X} cause={:08X} epc={:08X}",
        sys.cpu.hi, sys.cpu.lo, sys.cpu.cp0[12], sys.cpu.cp0[13], sys.cpu.cp0[14]
    );
    println!(
        "  I_STAT={:03X} I_MASK={:03X} scanline={}",
        sys.bus.interrupt.stat, sys.bus.interrupt.mask, sys.bus.gpu.scanline
    );
    for addr in peek {
        match sys.bus.read32(*addr) {
            Ok(v) => println!("  [{addr:#010X}] = {v:08X}"),
            Err(e) => println!("  [{addr:#010X}] = <{e}>"),
        }
    }
    println!(
        "I/O accesses with no device: {} ({} logged)",
        sys.bus.unhandled_io_count,
        sys.bus.unhandled_io.len()
    );
    for access in sys.bus.unhandled_io.iter().take(20) {
        println!(
            "  {} {} byte(s) at {:#010X}{}",
            if access.write { "write" } else { "read " },
            access.width,
            access.addr,
            if access.write { format!(" = {:#X}", access.value) } else { String::new() }
        );
    }
}

// ----- Disassembler (debugging aid) -----

const REG_NAMES: [&str; 32] = [
    "zero", "at", "v0", "v1", "a0", "a1", "a2", "a3", "t0", "t1", "t2", "t3", "t4", "t5", "t6",
    "t7", "s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "t8", "t9", "k0", "k1", "gp", "sp",
    "fp", "ra",
];

fn reg(n: u32) -> &'static str {
    REG_NAMES[(n & 31) as usize]
}

/// Render one R3000A instruction located at `pc`.
fn disasm(pc: u32, w: u32) -> String {
    let (rs, rt, rd, sh) = ((w >> 21) & 31, (w >> 16) & 31, (w >> 11) & 31, (w >> 6) & 31);
    let imm = w & 0xFFFF;
    let simm = imm as u16 as i16;
    let branch = pc.wrapping_add(4).wrapping_add(((simm as i32) << 2) as u32);
    let mem = |op: &str| format!("{op} {}, {simm}({})", reg(rt), reg(rs));
    let arith = |op: &str| format!("{op} {}, {}, {}", reg(rd), reg(rs), reg(rt));
    let imm_op = |op: &str| format!("{op} {}, {}, {imm:#x}", reg(rt), reg(rs));
    match w >> 26 {
        0x00 => match w & 0x3F {
            0x00 if w == 0 => "nop".into(),
            0x00 => format!("sll {}, {}, {sh}", reg(rd), reg(rt)),
            0x02 => format!("srl {}, {}, {sh}", reg(rd), reg(rt)),
            0x03 => format!("sra {}, {}, {sh}", reg(rd), reg(rt)),
            0x04 => format!("sllv {}, {}, {}", reg(rd), reg(rt), reg(rs)),
            0x06 => format!("srlv {}, {}, {}", reg(rd), reg(rt), reg(rs)),
            0x07 => format!("srav {}, {}, {}", reg(rd), reg(rt), reg(rs)),
            0x08 => format!("jr {}", reg(rs)),
            0x09 => format!("jalr {}, {}", reg(rd), reg(rs)),
            0x0C => "syscall".into(),
            0x0D => "break".into(),
            0x10 => format!("mfhi {}", reg(rd)),
            0x11 => format!("mthi {}", reg(rs)),
            0x12 => format!("mflo {}", reg(rd)),
            0x13 => format!("mtlo {}", reg(rs)),
            0x18 => format!("mult {}, {}", reg(rs), reg(rt)),
            0x19 => format!("multu {}, {}", reg(rs), reg(rt)),
            0x1A => format!("div {}, {}", reg(rs), reg(rt)),
            0x1B => format!("divu {}, {}", reg(rs), reg(rt)),
            0x20 => arith("add"),
            0x21 => arith("addu"),
            0x22 => arith("sub"),
            0x23 => arith("subu"),
            0x24 => arith("and"),
            0x25 => arith("or"),
            0x26 => arith("xor"),
            0x27 => arith("nor"),
            0x2A => arith("slt"),
            0x2B => arith("sltu"),
            _ => format!(".word {w:#010x}"),
        },
        0x01 => {
            let op = match rt {
                0x00 => "bltz",
                0x01 => "bgez",
                0x10 => "bltzal",
                0x11 => "bgezal",
                _ => return format!(".word {w:#010x}"),
            };
            format!("{op} {}, {branch:#010x}", reg(rs))
        }
        0x02 => format!("j {:#010x}", (pc.wrapping_add(4) & 0xF000_0000) | ((w & 0x03FF_FFFF) << 2)),
        0x03 => format!("jal {:#010x}", (pc.wrapping_add(4) & 0xF000_0000) | ((w & 0x03FF_FFFF) << 2)),
        0x04 => format!("beq {}, {}, {branch:#010x}", reg(rs), reg(rt)),
        0x05 => format!("bne {}, {}, {branch:#010x}", reg(rs), reg(rt)),
        0x06 => format!("blez {}, {branch:#010x}", reg(rs)),
        0x07 => format!("bgtz {}, {branch:#010x}", reg(rs)),
        0x08 => format!("addi {}, {}, {simm}", reg(rt), reg(rs)),
        0x09 => format!("addiu {}, {}, {simm}", reg(rt), reg(rs)),
        0x0A => format!("slti {}, {}, {simm}", reg(rt), reg(rs)),
        0x0B => format!("sltiu {}, {}, {simm}", reg(rt), reg(rs)),
        0x0C => imm_op("andi"),
        0x0D => imm_op("ori"),
        0x0E => imm_op("xori"),
        0x0F => format!("lui {}, {imm:#x}", reg(rt)),
        0x10 => match rs {
            0x00 => format!("mfc0 {}, cop0r{rd}", reg(rt)),
            0x04 => format!("mtc0 {}, cop0r{rd}", reg(rt)),
            0x10 => "rfe".into(),
            _ => format!(".word {w:#010x}"),
        },
        0x12 => format!("cop2 {:#09x}", w & 0x01FF_FFFF),
        0x20 => mem("lb"),
        0x21 => mem("lh"),
        0x22 => mem("lwl"),
        0x23 => mem("lw"),
        0x24 => mem("lbu"),
        0x25 => mem("lhu"),
        0x26 => mem("lwr"),
        0x28 => mem("sb"),
        0x29 => mem("sh"),
        0x2A => mem("swl"),
        0x2B => mem("sw"),
        0x2E => mem("swr"),
        0x32 => format!("lwc2 cop2r{rt}, {simm}({})", reg(rs)),
        0x3A => format!("swc2 cop2r{rt}, {simm}({})", reg(rs)),
        _ => format!(".word {w:#010x}"),
    }
}
