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

/// A console with the BIOS and the disc of `game`, reset and ready to run.
fn console(game: &str) -> Option<System> {
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
    Some(sys)
}

fn boot(game: &str, steps: u64) -> Option<System> {
    let mut sys = console(game)?;
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

const START: u16 = 0x0008;
const DOWN: u16 = 0x0040;
const CROSS: u16 = 0x4000;

/// Resident Evil waits for the memory card with a loop of a fixed number of
/// iterations, while the BIOS reads the directory one sector every two frames.
/// The CPU has to be slow enough (see `RAM_READ_CYCLES`) for the loop to outlast
/// the 36 sectors, or the game gives up with ACCESS ERROR on a good card.
#[test]
#[ignore = "boots a real game for ~2.6 billion cycles"]
fn resident_evil_reads_a_formatted_card_from_its_load_game_screen() {
    let Some(mut sys) = console("Resident Evil") else {
        eprintln!("skipped: no BIOS or Resident Evil disc under rom_test/");
        return;
    };
    sys.insert_formatted_memory_card(0);
    sys.reset().expect("reset");

    // `(button, first cycle, last cycle)`: any button on the title screen opens the menu,
    // then down to LOAD GAME and cross.
    let presses = [(START, 1_430_000_000u64, 1_500_000_000u64), (DOWN, 1_750_000_000, 1_800_000_000), (CROSS, 1_900_000_000, 1_970_000_000)];
    while sys.cycles < 2_600_000_000 {
        let held = presses.iter().filter(|(_, first, last)| (*first..*last).contains(&sys.cycles)).fold(0, |m, (b, _, _)| m | b);
        sys.set_input(0, held, 0, 0, 0, 0);
        sys.run_cycles(500_000).expect("the emulated CPU must not fail");
    }

    // The BIOS reads the 36 sectors of the header and directory when the game starts the card
    // services, and the load game screen reads them again: a game that gave up part way would
    // leave the count short.
    let card = sys.bus.controllers.memory_card(0).expect("the card is inserted");
    assert!(card.sectors_read() >= 72, "only {} sectors read: the game gave up on the card", card.sectors_read());
    assert!(card.sectors_written() >= 1, "the BIOS writes a test sector when it brings the card up");
}
