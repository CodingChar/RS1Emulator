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

fn main() {
    let mut bios = None;
    let mut disc = None;
    let mut steps: u64 = 100_000_000;
    let mut cpu_only = false;
    let mut frames: Option<u64> = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--bios" => bios = Some(PathBuf::from(value())),
            "--disc" => disc = Some(PathBuf::from(value())),
            "--cpu-only" => cpu_only = true,
            "--frames" => frames = Some(value().parse().expect("--frames")),
            "--steps" => steps = value().replace('_', "").parse().expect("--steps"),
            other => panic!("unknown argument {other}"),
        }
    }
    let bios = std::fs::read(bios.expect("--bios is required")).expect("read BIOS");

    let mut sys = System::new();
    sys.load_bios(&bios).expect("load BIOS");
    if let Some(cue) = disc {
        sys.insert_disc_image(open_disc(&cue).expect("open disc"));
    }
    sys.reset().expect("reset");

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
        for _ in 0..steps {
            sys.step().expect("step");
        }
    }
    sys.sync();
    let elapsed = start.elapsed().as_secs_f64();

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
