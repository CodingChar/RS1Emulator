//! PS-X EXE: the executable format of PlayStation programs and homebrew.
//!
//! A 2 KB header (magic `PS-X EXE`, entry point, global pointer, load address
//! and size, bss, initial stack) followed by the program text. Programs are
//! started the way the BIOS shell starts them: the BIOS boots normally and,
//! when it reaches the shell (`0x80030000`), the executable replaces it. See
//! [`crate::system::System::load_exe`].

use crate::types::{Ps1Error, Ps1Result};

/// Size of the header; the text starts right after it.
const HEADER_SIZE: usize = 0x800;
const MAGIC: &[u8; 8] = b"PS-X EXE";
/// Stack pointer used when the header does not give one.
const DEFAULT_STACK: u32 = 0x801F_FF00;
/// Size of main RAM.
const RAM_SIZE: u32 = 2 * 1024 * 1024;

/// A parsed executable, ready to be copied into RAM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exe {
    pub pc: u32,
    pub gp: u32,
    /// Where the text goes (a virtual address).
    pub load_address: u32,
    pub text: Vec<u8>,
    /// A region to clear (the bss): address and size in bytes.
    pub clear: Option<(u32, u32)>,
    pub sp: u32,
}

fn word(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn invalid(message: &str) -> Ps1Error {
    Ps1Error::Io(format!("not a valid PS-X EXE: {message}"))
}

impl Exe {
    pub fn parse(data: &[u8]) -> Ps1Result<Exe> {
        if data.len() < HEADER_SIZE || &data[..8] != MAGIC {
            return Err(invalid("missing the PS-X EXE header"));
        }
        let size = word(data, 0x1C) as usize;
        let text = data
            .get(HEADER_SIZE..HEADER_SIZE + size)
            .ok_or_else(|| invalid("the file is shorter than its header says"))?;
        let load_address = word(data, 0x18);
        // The text must fit in RAM (the segment bits of the address are ignored).
        let offset = load_address & 0x1FFF_FFFF;
        if offset >= RAM_SIZE || offset as usize + size > RAM_SIZE as usize {
            return Err(invalid("the program does not fit in RAM"));
        }

        let stack_base = word(data, 0x30);
        let sp = if stack_base == 0 { DEFAULT_STACK } else { stack_base.wrapping_add(word(data, 0x34)) };
        let clear_size = word(data, 0x2C);
        Ok(Exe {
            pc: word(data, 0x10),
            gp: word(data, 0x14),
            load_address,
            text: text.to_vec(),
            clear: (clear_size != 0).then(|| (word(data, 0x28), clear_size)),
            sp,
        })
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A header for a program of `text` loaded at `load` and started at `pc`.
    pub fn exe_bytes(text: &[u8], load: u32, pc: u32, gp: u32, stack: (u32, u32)) -> Vec<u8> {
        let mut data = vec![0u8; HEADER_SIZE];
        data[..8].copy_from_slice(MAGIC);
        data[0x10..0x14].copy_from_slice(&pc.to_le_bytes());
        data[0x14..0x18].copy_from_slice(&gp.to_le_bytes());
        data[0x18..0x1C].copy_from_slice(&load.to_le_bytes());
        data[0x1C..0x20].copy_from_slice(&(text.len() as u32).to_le_bytes());
        data[0x30..0x34].copy_from_slice(&stack.0.to_le_bytes());
        data[0x34..0x38].copy_from_slice(&stack.1.to_le_bytes());
        data.extend_from_slice(text);
        data
    }

    #[test]
    fn a_header_is_parsed() {
        let data = exe_bytes(&[1, 2, 3, 4], 0x8001_0000, 0x8001_0004, 0x8002_0000, (0x801F_FF00, 0x10));
        let exe = Exe::parse(&data).unwrap();
        assert_eq!(exe.pc, 0x8001_0004);
        assert_eq!(exe.gp, 0x8002_0000);
        assert_eq!(exe.text, vec![1, 2, 3, 4]);
        assert_eq!(exe.sp, 0x801F_FF10);
        assert_eq!(exe.clear, None);
    }

    #[test]
    fn a_missing_stack_gets_the_default() {
        let data = exe_bytes(&[0; 4], 0x8001_0000, 0x8001_0000, 0, (0, 0));
        assert_eq!(Exe::parse(&data).unwrap().sp, 0x801F_FF00);
    }

    #[test]
    fn bad_files_are_rejected() {
        assert!(Exe::parse(b"too short").is_err());
        let mut data = exe_bytes(&[0; 4], 0x8001_0000, 0x8001_0000, 0, (0, 0));
        data[0] = b'X';
        assert!(Exe::parse(&data).is_err(), "wrong magic");
        let mut data = exe_bytes(&[0; 8], 0x8001_0000, 0x8001_0000, 0, (0, 0));
        data.truncate(HEADER_SIZE + 4);
        assert!(Exe::parse(&data).is_err(), "truncated text");
        let data = exe_bytes(&[0; 8], 0x8020_0000, 0x8020_0000, 0, (0, 0));
        assert!(Exe::parse(&data).is_err(), "outside RAM");
    }
}
