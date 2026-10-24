use crate::memory::Memory;
use crate::types::{Ps1Error, Ps1Result};

/// Mask applied to a virtual address to obtain the physical address, indexed
/// by the top three address bits: KUSEG (x4), KSEG0, KSEG1, KSEG2 (x2).
const REGION_MASK: [u32; 8] = [
    0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, // KUSEG
    0x7FFF_FFFF,                                         // KSEG0
    0x1FFF_FFFF,                                         // KSEG1
    0xFFFF_FFFF, 0xFFFF_FFFF,                            // KSEG2
];

const SCRATCHPAD_START: u32 = 0x1F80_0000;
const SCRATCHPAD_END: u32 = 0x1F80_03FF;
const BIOS_START: u32 = 0x1FC0_0000;
const BIOS_END: u32 = 0x1FC7_FFFF;

enum Target {
    Ram(usize),
    Bios(usize),
    Scratchpad,
    Unmapped,
}

/// System bus: routes CPU/memory/device accesses to the correct hardware.
///
/// In the PS1, the CPU talks to RAM, BIOS, GPU, SPU, CD-ROM, timers, etc.
/// through memory-mapped I/O. This struct centralizes those mappings.
pub struct Bus {
    // Future: register device address ranges here.
}

impl Bus {
    pub fn new() -> Self {
        Self {}
    }

    fn decode(addr: u32) -> Target {
        let phys = addr & REGION_MASK[(addr >> 29) as usize];
        match phys {
            // The 2 MB of main RAM are mirrored four times in the first 8 MB.
            0..=0x007F_FFFF => Target::Ram(phys as usize),
            SCRATCHPAD_START..=SCRATCHPAD_END => Target::Scratchpad,
            BIOS_START..=BIOS_END => Target::Bios((phys - BIOS_START) as usize),
            _ => Target::Unmapped,
        }
    }

    /// Read a 32-bit word from the bus at the given virtual address.
    pub fn read32(&self, addr: u32, memory: &Memory) -> Ps1Result<u32> {
        match Self::decode(addr) {
            Target::Ram(o) => Ok(memory.ram.read32(o)),
            Target::Bios(o) => Ok(memory.bios.read32(o)),
            // TODO: implement scratchpad
            Target::Scratchpad => Ok(0),
            Target::Unmapped => Err(Ps1Error::MemoryOutOfRange(addr)),
        }
    }

    /// Read a 16-bit halfword from the bus.
    pub fn read16(&self, addr: u32, memory: &Memory) -> Ps1Result<u16> {
        match Self::decode(addr) {
            Target::Ram(o) => Ok(memory.ram.read16(o)),
            Target::Bios(o) => Ok(memory.bios.read16(o)),
            Target::Scratchpad => Ok(0),
            Target::Unmapped => Err(Ps1Error::MemoryOutOfRange(addr)),
        }
    }

    /// Read a byte from the bus.
    pub fn read8(&self, addr: u32, memory: &Memory) -> Ps1Result<u8> {
        match Self::decode(addr) {
            Target::Ram(o) => Ok(memory.ram.read8(o)),
            Target::Bios(o) => Ok(memory.bios.read8(o)),
            Target::Scratchpad => Ok(0),
            Target::Unmapped => Err(Ps1Error::MemoryOutOfRange(addr)),
        }
    }

    /// Write a 32-bit word to the bus at the given virtual address.
    pub fn write32(&mut self, addr: u32, value: u32, memory: &mut Memory) -> Ps1Result<()> {
        match Self::decode(addr) {
            Target::Ram(o) => {
                memory.ram.write32(o, value);
                Ok(())
            }
            // The BIOS is ROM: writes are ignored.
            // TODO: scratchpad / I/O registers
            Target::Bios(_) | Target::Scratchpad => Ok(()),
            Target::Unmapped => Err(Ps1Error::MemoryOutOfRange(addr)),
        }
    }

    /// Write a 16-bit halfword to the bus.
    pub fn write16(&mut self, addr: u32, value: u16, memory: &mut Memory) -> Ps1Result<()> {
        match Self::decode(addr) {
            Target::Ram(o) => {
                memory.ram.write16(o, value);
                Ok(())
            }
            Target::Bios(_) | Target::Scratchpad => Ok(()),
            Target::Unmapped => Err(Ps1Error::MemoryOutOfRange(addr)),
        }
    }

    /// Write a byte to the bus.
    pub fn write8(&mut self, addr: u32, value: u8, memory: &mut Memory) -> Ps1Result<()> {
        match Self::decode(addr) {
            Target::Ram(o) => {
                memory.ram.write8(o, value);
                Ok(())
            }
            Target::Bios(_) | Target::Scratchpad => Ok(()),
            Target::Unmapped => Err(Ps1Error::MemoryOutOfRange(addr)),
        }
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Bus, Memory) {
        (Bus::new(), Memory::new())
    }

    #[test]
    fn ram_is_visible_through_all_segments() {
        let (mut bus, mut mem) = setup();
        bus.write32(0x8000_0100, 0xDEAD_BEEF, &mut mem).unwrap();
        assert_eq!(bus.read32(0x0000_0100, &mem).unwrap(), 0xDEAD_BEEF); // KUSEG
        assert_eq!(bus.read32(0x8000_0100, &mem).unwrap(), 0xDEAD_BEEF); // KSEG0
        assert_eq!(bus.read32(0xA000_0100, &mem).unwrap(), 0xDEAD_BEEF); // KSEG1
    }

    #[test]
    fn ram_is_mirrored_in_the_first_8mb() {
        let (mut bus, mut mem) = setup();
        bus.write32(0x0000_0040, 0x1234_5678, &mut mem).unwrap();
        assert_eq!(bus.read32(0x0020_0040, &mem).unwrap(), 0x1234_5678);
        assert_eq!(bus.read32(0x0060_0040, &mem).unwrap(), 0x1234_5678);
    }

    #[test]
    fn sub_word_accesses_are_little_endian() {
        let (mut bus, mut mem) = setup();
        bus.write32(0x200, 0x4433_2211, &mut mem).unwrap();
        assert_eq!(bus.read8(0x200, &mem).unwrap(), 0x11);
        assert_eq!(bus.read8(0x203, &mem).unwrap(), 0x44);
        assert_eq!(bus.read16(0x200, &mem).unwrap(), 0x2211);
        assert_eq!(bus.read16(0x202, &mem).unwrap(), 0x4433);

        bus.write8(0x201, 0xAA, &mut mem).unwrap();
        bus.write16(0x202, 0xBBCC, &mut mem).unwrap();
        assert_eq!(bus.read32(0x200, &mem).unwrap(), 0xBBCC_AA11);
    }

    #[test]
    fn bios_is_reachable_from_kseg0_and_kseg1_and_is_read_only() {
        let (mut bus, mut mem) = setup();
        mem.bios.as_mut_slice()[..4].copy_from_slice(&0xCAFE_F00Du32.to_le_bytes());
        assert_eq!(bus.read32(0xBFC0_0000, &mem).unwrap(), 0xCAFE_F00D);
        assert_eq!(bus.read32(0x9FC0_0000, &mem).unwrap(), 0xCAFE_F00D);
        assert_eq!(bus.read8(0xBFC0_0000, &mem).unwrap(), 0x0D);
        assert_eq!(bus.read16(0xBFC0_0002, &mem).unwrap(), 0xCAFE);

        bus.write32(0xBFC0_0000, 0, &mut mem).unwrap();
        assert_eq!(bus.read32(0xBFC0_0000, &mem).unwrap(), 0xCAFE_F00D);
    }

    #[test]
    fn unmapped_addresses_are_reported() {
        let (bus, mem) = setup();
        assert_eq!(
            bus.read32(0xBF80_1000, &mem),
            Err(Ps1Error::MemoryOutOfRange(0xBF80_1000))
        );
        assert!(bus.read32(0xFFFE_0130, &mem).is_err());
    }
}
