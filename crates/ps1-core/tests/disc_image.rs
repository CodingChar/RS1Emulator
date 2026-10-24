//! Sanity checks on a real disc image.
//!
//! The image is copyrighted and therefore never committed: it is expected in
//! the git-ignored `rom_test/` folder at the workspace root. When it is not
//! there (CI, fresh clones) these tests skip themselves.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

const RAW_SECTOR: u64 = 2352;

/// Find the first `.bin` under `<workspace>/rom_test`.
fn find_test_bin() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rom_test");
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("bin")) {
                return Some(path);
            }
        }
    }
    None
}

fn read_sector(file: &mut File, lba: u64) -> Vec<u8> {
    let mut sector = vec![0u8; RAW_SECTOR as usize];
    file.seek(SeekFrom::Start(lba * RAW_SECTOR)).unwrap();
    file.read_exact(&mut sector).unwrap();
    sector
}

#[test]
fn test_disc_is_a_raw_mode2_image_with_an_iso9660_volume() {
    let Some(bin) = find_test_bin() else {
        eprintln!("skipped: no disc image under rom_test/");
        return;
    };
    let mut file = File::open(&bin).unwrap();

    let len = file.metadata().unwrap().len();
    assert_eq!(len % RAW_SECTOR, 0, "not a whole number of 2352-byte sectors");

    let first = read_sector(&mut file, 0);
    assert_eq!(first[0], 0x00);
    assert!(first[1..11].iter().all(|b| *b == 0xFF), "missing sync pattern");
    assert_eq!(first[11], 0x00);

    // Primary volume descriptor: type 1, "CD001", at LBA 16 (user data at +24).
    let pvd = read_sector(&mut file, 16);
    assert_eq!(&pvd[24..30], b"\x01CD001");
}
