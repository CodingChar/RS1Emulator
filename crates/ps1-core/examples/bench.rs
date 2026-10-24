//! Speed benchmark: runs a BIOS (and optionally a disc) for a fixed number of
//! instructions and reports instructions per second, plus a checksum of the
//! final machine state so optimisations can be checked for identical behaviour.
//!
//! ```text
//! cargo run --release -p ps1-core --example bench --target <host triple> -- \
//!     --bios path/to/bios.bin [--disc game.cue] [--steps 100000000]
//! ```

use std::path::{Path, PathBuf};
use std::time::Instant;

use ps1_core::disc::{Disc, FileSource, SectorSource};
use ps1_core::System;

fn open_disc(cue: &Path) -> Result<Disc, String> {
    let text = std::fs::read_to_string(cue).map_err(|e| format!("{}: {e}", cue.display()))?;
    let dir = cue.parent().unwrap_or(Path::new(".")).to_path_buf();
    Disc::from_cue(&text, |name| {
        FileSource::open(&dir.join(name))
            .map(|f| Box::new(f) as Box<dyn SectorSource>)
            .map_err(|e| e.to_string())
    })
    .map_err(|e| e.to_string())
}

/// FNV-1a over a byte stream.
fn fnv(hash: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *hash ^= *b as u64;
        *hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
}

/// Small endless loops placed in RAM at 0x80010000, to measure the
/// interpreter on its own.
fn synthetic_program(kind: &str) -> Vec<u32> {
    match kind {
        // Pure ALU work: addiu, addu, xor, sll, then j loop with a nop in the slot.
        "alu" => vec![0x2508_0001, 0x0108_4821, 0x0128_5026, 0x000A_5880, 0x0800_4000, 0],
        // Loads and stores to RAM: lui, sw, lw, addiu, j, nop.
        "mem" => vec![0x3C0E_8002, 0xADC8_0000, 0x8DC9_0000, 0x2508_0001, 0x0800_4001, 0],
        // Taken and not-taken conditional branches: addiu, andi, beq, nop, bne, nop, j, nop.
        "branch" => vec![0x2508_0001, 0x3109_0001, 0x1120_0001, 0, 0x1520_0001, 0, 0x0800_4000, 0],
        other => panic!("unknown synthetic program {other}"),
    }
}

fn main() {
    let mut synthetic: Option<String> = None;
    let mut bios = None;
    let mut disc = None;
    let mut steps: u64 = 100_000_000;
    let mut cpu_only = false;
    let mut frames: Option<u64> = None;
    // (button mask, first step, last step): held between the two steps.
    let mut presses: Vec<(u16, u64, u64)> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--bios" => bios = Some(PathBuf::from(value())),
            "--disc" => disc = Some(PathBuf::from(value())),
            "--cpu-only" => cpu_only = true,
            "--synthetic" => synthetic = Some(value()),
            "--press" => {
                // MASK@FIRST-LAST, for example 0x0008@1300000000-1310000000
                let text = value();
                let (mask, range) = text.split_once('@').expect("--press MASK@FIRST-LAST");
                let (first, last) = range.split_once('-').expect("--press MASK@FIRST-LAST");
                let mask = u16::from_str_radix(mask.trim_start_matches("0x"), 16).expect("mask");
                presses.push((mask, first.parse().expect("first"), last.parse().expect("last")));
            }
            "--frames" => frames = Some(value().parse().expect("--frames")),
            "--steps" => steps = value().replace('_', "").parse().expect("--steps"),
            other => panic!("unknown argument {other}"),
        }
    }
    let bios = match &synthetic {
        // A BIOS that jumps to the RAM program: lui t0,0x8001; jr t0; nop.
        Some(_) => [0x3C08_8001u32, 0x0100_0008, 0].iter().flat_map(|w| w.to_le_bytes()).collect(),
        None => std::fs::read(bios.expect("--bios is required")).expect("read BIOS"),
    };

    let mut sys = System::new();
    sys.load_bios(&bios).expect("load BIOS");
    if let Some(cue) = disc {
        sys.insert_disc_image(open_disc(&cue).expect("open disc"));
    }
    sys.reset().expect("reset");
    if let Some(kind) = &synthetic {
        for (i, word) in synthetic_program(kind).iter().enumerate() {
            sys.bus.memory.ram.write32(0x1_0000 + i * 4, *word);
        }
    }

    #[cfg(feature = "profile")]
    let sampler = {
        use std::sync::atomic::{AtomicBool, Ordering};
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut histogram = [0u64; 16];
            while !flag.load(Ordering::Relaxed) {
                let area = ps1_core::profile::CURRENT.load(Ordering::Relaxed) as usize;
                histogram[area & 15] += 1;
                std::thread::sleep(std::time::Duration::from_micros(100));
            }
            histogram
        });
        (stop, handle)
    };
    let start = Instant::now();
    if cpu_only {
        // The interpreter and bus alone, without advancing any device: shows
        // how much of the time the devices take.
        for _ in 0..steps {
            sys.cpu.set_irq(sys.bus.interrupt.pending());
            sys.cpu.step(&mut sys.bus).expect("step");
        }
    } else if let Some(frames) = frames {
        // What the web frontend does: whole video frames.
        for _ in 0..frames {
            sys.run_frame().expect("frame");
        }
        steps = sys.cycles;
    } else {
        // Scripted button presses are checked every 4096 instructions.
        let mut held = 0u16;
        let mut done = 0u64;
        while done < steps {
            if !presses.is_empty() {
                let mask = presses
                    .iter()
                    .filter(|(_, first, last)| done >= *first && done < *last)
                    .fold(0, |m, (bits, _, _)| m | bits);
                if mask != held {
                    held = mask;
                    sys.set_input(0, held, 0, 0, 0, 0);
                }
            }
            let chunk = (steps - done).min(4096);
            sys.run_cycles(chunk).expect("run");
            done += chunk;
        }
    }
    sys.sync();
    let elapsed = start.elapsed().as_secs_f64();

    #[cfg(feature = "profile")]
    {
        use std::sync::atomic::Ordering;
        sampler.0.store(true, Ordering::Relaxed);
        let histogram = sampler.1.join().unwrap();
        let total: u64 = histogram.iter().sum();
        println!("time per area (sampled):");
        for (name, count) in ps1_core::profile::AREA_NAMES.iter().zip(histogram.iter()) {
            if *count * 200 >= total {
                println!("  {name:14} {:5.1} %", *count as f64 / total as f64 * 100.0);
            }
        }
    }
    let mut hash = 0xCBF2_9CE4_8422_2325u64;
    fnv(&mut hash, &sys.framebuffer());
    for reg in sys.cpu.regs {
        fnv(&mut hash, &reg.to_le_bytes());
    }
    fnv(&mut hash, &sys.cpu.pc.to_le_bytes());
    fnv(&mut hash, &sys.cpu.hi.to_le_bytes());
    fnv(&mut hash, &sys.cpu.lo.to_le_bytes());
    for reg in sys.cpu.cp0 {
        fnv(&mut hash, &reg.to_le_bytes());
    }
    for sample in &sys.bus.spu.output_buffer {
        fnv(&mut hash, &sample.to_le_bytes());
    }
    fnv(&mut hash, &sys.bus.memory.ram.as_slice().to_vec());
    println!(
        "{steps} instructions in {elapsed:.2} s = {:.2} M instr/s ({:.0} % of real time)",
        steps as f64 / elapsed / 1e6,
        steps as f64 / elapsed / 33_868_800.0 * 100.0
    );
    println!("state checksum {hash:016X}");
}
