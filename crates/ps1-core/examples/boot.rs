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
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args { bios: None, disc: None, steps: 10_000_000 };
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
        Some(bios) => run_bios(bios, args.steps),
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

fn run_bios(path: &Path, steps: u64) {
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
        println!("hottest pcs in the last 100k instructions:");
        for (pc, n) in hot.iter().take(5) {
            println!("  {pc:#010X}  x{n}");
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
