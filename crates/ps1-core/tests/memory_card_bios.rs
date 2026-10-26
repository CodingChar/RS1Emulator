//! Writes to a memory card through the real BIOS, the way a game saves: the
//! BIOS memory card manager copies a save file from card 1 to card 2 (its
//! "COPY ALL" command), which makes the kernel read the directory and the
//! file and write them back to the other card with its card services.
//!
//! The manager is driven with scripted button presses; it waits for input, so
//! the presses are spaced generously. The check is on the cards: card 2 has to
//! end up with the same file, with a valid directory.
//!
//! Needs the BIOS dump in the git-ignored `rom_test/` folder (a missing one
//! skips the test) and boots it, so it is `#[ignore]`d: run it with
//! `cargo test --release -p ps1-core --target <host> --test memory_card_bios -- --ignored`.

use std::path::PathBuf;

use ps1_core::controllers::memory_card::{MemoryCard, FRAME_SIZE};
use ps1_core::System;

const CROSS: u16 = 0x4000;
const DOWN: u16 = 0x0040;

/// `(button, first cycle, last cycle)`: open the memory card manager, move the
/// cursor to COPY ALL, pick it, pick the card and answer YES to "Are you sure?".
const PRESSES: [(u16, u64, u64); 6] = [
    (CROSS, 500_000_000, 520_000_000),
    (DOWN, 700_000_000, 710_000_000),
    (DOWN, 760_000_000, 770_000_000),
    (CROSS, 820_000_000, 830_000_000),
    (CROSS, 900_000_000, 910_000_000),
    (CROSS, 980_000_000, 990_000_000),
];
/// When to stop: well after the last press, so the copy is done.
const END: u64 = 1_250_000_000;

/// The block (64 frames) the save file takes, and the directory frame that names it.
const FIRST_DATA_FRAME: usize = 64;
const DATA_FRAMES: usize = 64;
const DIRECTORY_FRAME: usize = 1;

fn xor(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |a, b| a ^ b)
}

fn frame_mut(data: &mut [u8], i: usize) -> &mut [u8] {
    &mut data[i * FRAME_SIZE..(i + 1) * FRAME_SIZE]
}

/// A formatted card holding one one-block save, laid out as the BIOS expects.
fn card_with_a_save() -> MemoryCard {
    let mut data = MemoryCard::formatted().data().to_vec();

    let entry = frame_mut(&mut data, DIRECTORY_FRAME);
    entry[0] = 0x51; // first (and last) block of a file
    entry[4..8].copy_from_slice(&0x2000u32.to_le_bytes());
    entry[8..10].copy_from_slice(&[0xFF, 0xFF]);
    entry[10..30].copy_from_slice(b"BASLUS-99999TESTSAVE");
    entry[127] = xor(&entry[..127]);

    let header = frame_mut(&mut data, FIRST_DATA_FRAME);
    header[..4].copy_from_slice(&[b'S', b'C', 0x11, 0x01]); // magic, one icon frame, one block
    header[4..12].copy_from_slice(&[0x82, 0x73, 0x82, 0x64, 0x82, 0x72, 0x82, 0x73]); // the title, "TEST", in Shift-JIS
    for colour in 1..16u16 {
        let level = colour * 2;
        let at = 0x60 + colour as usize * 2;
        header[at..at + 2].copy_from_slice(&(0x8000 | (level << 10) | (level << 5) | level).to_le_bytes());
    }
    for (i, byte) in frame_mut(&mut data, FIRST_DATA_FRAME + 1).iter_mut().enumerate() {
        *byte = ((i % 16) as u8) << 4 | (i % 16) as u8;
    }
    for i in FIRST_DATA_FRAME + 2..FIRST_DATA_FRAME + DATA_FRAMES {
        frame_mut(&mut data, i).fill(i as u8);
    }
    MemoryCard::from_bytes(data).expect("a whole card")
}

fn rom_test() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rom_test")
}

fn frame_of(card: &MemoryCard, i: usize) -> &[u8] {
    &card.data()[i * FRAME_SIZE..(i + 1) * FRAME_SIZE]
}

#[test]
#[ignore = "boots the BIOS and drives its menus for over a billion cycles"]
fn the_bios_copies_a_save_between_cards() {
    let Ok(bios) = std::fs::read(rom_test().join("SCPH1001.BIN")) else {
        eprintln!("skipped: no BIOS under rom_test/");
        return;
    };
    let source = card_with_a_save();
    let mut sys = System::new();
    sys.load_bios(&bios).expect("BIOS image too large");
    sys.insert_memory_card(0, source.data().to_vec()).unwrap();
    sys.insert_formatted_memory_card(1);
    sys.reset().expect("reset");
    sys.clear_memory_card_dirty(0);
    sys.clear_memory_card_dirty(1);

    while sys.cycles < END {
        let held = PRESSES.iter().filter(|(_, first, last)| (*first..*last).contains(&sys.cycles)).fold(0, |m, (b, _, _)| m | b);
        sys.set_input(0, held, 0, 0, 0, 0);
        sys.run_cycles(500_000).expect("the emulated CPU must not fail");
    }

    let from = sys.bus.controllers.memory_card(0).expect("card 1");
    let to = sys.bus.controllers.memory_card(1).expect("card 2");
    assert!(to.is_dirty(), "the BIOS wrote to card 2");

    let entry = frame_of(to, DIRECTORY_FRAME);
    assert_eq!(entry[0], 0x51, "the directory names the file");
    assert_eq!(&entry[4..8], &0x2000u32.to_le_bytes());
    assert_eq!(&entry[10..30], &frame_of(&source, DIRECTORY_FRAME)[10..30], "same file name");
    for i in 0..36 {
        let frame = frame_of(to, i);
        assert_eq!(xor(&frame[..127]), frame[127], "frame {i} of card 2 has a valid checksum");
    }
    for i in FIRST_DATA_FRAME..FIRST_DATA_FRAME + DATA_FRAMES {
        assert_eq!(frame_of(to, i), frame_of(&source, i), "frame {i} of the file was copied intact");
        assert_eq!(frame_of(from, i), frame_of(&source, i), "frame {i} of the source is untouched");
    }
}
