//! Speed benchmark: runs a BIOS (and optionally a disc) for a fixed number of
//! instructions and reports instructions per second, plus a checksum of the
//! final machine state so optimisations can be checked for identical behaviour.
//!
//! ```text
//! cargo run --release -p ps1-core --example bench --target <host triple> -- \
//!     --bios path/to/bios.bin [--disc game.cue] [--steps 100000000]
//! ```
//!
//! Other modes: `--synthetic alu|mem|branch` times endless loops instead of a
//! BIOS, `--press MASK@FIRST-LAST` holds buttons between two instructions,
//! `--frames N` runs whole video frames like the web frontend, and
//! `--roundtrip FIRST,SECOND` checks that a save state taken after FIRST
//! instructions makes a fresh machine continue exactly like the original.
//! Built with `--features profile` it also prints the share of time per area.
//!
//! Rendering options (what the web page offers): `--scale N` (internal
//! resolution), `--pgxp`, `--pgxp-textures`, `--true-color`, `--bilinear` and
//! `--widescreen`. `--shot FILE.png` writes the final picture, as the options
//! render it, so a run can be looked at.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ps1_core::disc::{Disc, FileSource, SectorSource};
use ps1_core::gpu::{RenderSettings, TextureFilter};
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

/// A checksum of the whole machine state: screen, registers, RAM, audio.
fn digest(sys: &mut System) -> u64 {
    sys.sync();
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
    fnv(&mut hash, sys.bus.memory.ram.as_slice());
    hash
}

/// Run from instruction `from` to `to`, applying the scripted button presses
/// (checked every 4096 instructions).
fn run_range(sys: &mut System, from: u64, to: u64, presses: &[(u16, u64, u64)]) {
    let mut held = 0u16;
    let mut done = from;
    while done < to {
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
        let chunk = (to - done).min(4096);
        sys.run_cycles(chunk).expect("run");
        done += chunk;
    }
}

fn main() {
    let mut synthetic: Option<String> = None;
    let mut bios = None;
    let mut disc = None;
    let mut steps: u64 = 100_000_000;
    let mut cpu_only = false;
    let mut frames: Option<u64> = None;
    let mut roundtrip: Option<(u64, u64)> = None;
    let mut render = RenderSettings::default();
    let mut shot: Option<PathBuf> = None;
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
            "--roundtrip" => {
                let text = value();
                let (first, second) = text.split_once(',').expect("--roundtrip FIRST,SECOND");
                roundtrip = Some((
                    first.replace('_', "").parse().expect("first"),
                    second.replace('_', "").parse().expect("second"),
                ));
            }
            "--frames" => frames = Some(value().parse().expect("--frames")),
            "--scale" => render.scale = value().parse().expect("--scale"),
            "--pgxp" => render.pgxp = true,
            "--pgxp-textures" => render.pgxp_textures = true,
            "--true-color" => render.true_color = true,
            "--bilinear" => render.texture_filter = TextureFilter::Bilinear,
            "--widescreen" => render.widescreen = true,
            "--shot" => shot = Some(PathBuf::from(value())),
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
    if let Some(cue) = &disc {
        sys.insert_disc_image(open_disc(cue).expect("open disc"));
    }
    sys.set_render_settings(render);
    sys.reset().expect("reset");
    if let Some(kind) = &synthetic {
        for (i, word) in synthetic_program(kind).iter().enumerate() {
            sys.bus.memory.ram.write32(0x1_0000 + i * 4, *word);
        }
    }

    if let Some((first, second)) = roundtrip {
        run_range(&mut sys, 0, first, &presses);
        let state = sys.save_state();
        println!("saved a state of {} bytes after {first} instructions", state.len());
        // Audio already produced is not part of a state; start both runs empty.
        sys.bus.spu.output_buffer.clear();
        run_range(&mut sys, first, first + second, &[]);
        let original = digest(&mut sys);

        let mut other = System::new();
        other.load_bios(&bios).expect("load BIOS");
        if let Some(cue) = &disc {
            other.insert_disc_image(open_disc(cue).expect("open disc"));
        }
        other.reset().expect("reset");
        other.load_state(&state).expect("load state");
        run_range(&mut other, first, first + second, &[]);
        let restored = digest(&mut other);
        let same = original == restored;
        println!(
            "original {original:016X}, restored {restored:016X}: {}",
            if same { "IDENTICAL" } else { "DIFFERENT" }
        );
        std::process::exit(if same { 0 } else { 1 });
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
        run_range(&mut sys, 0, steps, &presses);
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
    if let Some(path) = &shot {
        let (width, height, rgba) = sys.scaled_framebuffer();
        write_png(path, width, height, &rgba).expect("write the picture");
        println!("picture {width}x{height} written to {}", path.display());
    }
    let hash = digest(&mut sys);
    println!(
        "{steps} instructions in {elapsed:.2} s = {:.2} M instr/s ({:.0} % of real time)",
        steps as f64 / elapsed / 1e6,
        steps as f64 / elapsed / 33_868_800.0 * 100.0
    );
    println!("state checksum {hash:016X}");
}

// ----- PNG output (uncompressed, no dependencies) -----

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + *byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// Write RGBA pixels as a PNG using stored (uncompressed) deflate blocks.
fn write_png(path: &Path, width: usize, height: usize, rgba: &[u8]) -> Result<(), String> {
    let mut raw = Vec::with_capacity((width * 4 + 1) * height);
    for row in rgba.chunks_exact(width * 4) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65535).peekable();
    while let Some(block) = blocks.next() {
        zlib.push(blocks.peek().is_none() as u8);
        zlib.extend_from_slice(&(block.len() as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut header = Vec::new();
    header.extend_from_slice(&(width as u32).to_be_bytes());
    header.extend_from_slice(&(height as u32).to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    png_chunk(&mut png, b"IHDR", &header);
    png_chunk(&mut png, b"IDAT", &zlib);
    png_chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png).map_err(|e| format!("{}: {e}", path.display()))
}
