//! Boots real games to a known screen. Slow (billions of instructions), so
//! the tests are `#[ignore]`d: run them with
//! `cargo test --release -p ps1-core --target <host> --test boot_games -- --ignored`.
//!
//! The BIOS and discs are copyrighted and live in the git-ignored `rom_test/`
//! folder; a missing file skips the test.

use std::collections::HashSet;
use std::path::PathBuf;

use ps1_core::disc::{Disc, FileSource, SectorSource};
use ps1_core::System;

fn rom_test() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rom_test")
}

/// The first `.cue` under `rom_test/` whose path contains `name`.
fn find_cue(name: &str) -> Option<PathBuf> {
    let mut stack = vec![rom_test()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("cue"))
                && path.to_string_lossy().contains(name)
            {
                return Some(path);
            }
        }
    }
    None
}

fn boot(game: &str, steps: u64) -> Option<System> {
    let bios = std::fs::read(rom_test().join("SCPH1001.BIN")).ok()?;
    let cue = find_cue(game)?;
    let text = std::fs::read_to_string(&cue).ok()?;
    let dir = cue.parent()?.to_path_buf();
    let disc = Disc::from_cue(&text, |name| {
        FileSource::open(&dir.join(name)).map(|f| Box::new(f) as Box<dyn SectorSource>).map_err(|e| e.to_string())
    })
    .ok()?;
    let mut sys = System::new();
    sys.load_bios(&bios).expect("BIOS image too large");
    sys.insert_disc_image(disc);
    sys.reset().expect("reset");
    for _ in 0..steps {
        sys.step().expect("the emulated CPU must not fail");
    }
    Some(sys)
}

/// Number of distinct colours on the displayed frame.
fn colours(sys: &System) -> usize {
    sys.framebuffer().chunks_exact(4).map(|p| u32::from_le_bytes([p[0], p[1], p[2], 0])).collect::<HashSet<_>>().len()
}

#[test]
#[ignore = "boots a real game for ~1.5 billion instructions"]
fn crash_bandicoot_reaches_the_title_menu() {
    let Some(sys) = boot("Crash Bandicoot", 1_500_000_000) else {
        eprintln!("skipped: no BIOS or Crash Bandicoot disc under rom_test/");
        return;
    };
    // The title screen is a lit 3D scene with text: thousands of colours.
    // A hung or blank boot shows a flat or black frame.
    let n = colours(&sys);
    assert!(n > 1000, "expected the title screen, found {n} colours");
}
