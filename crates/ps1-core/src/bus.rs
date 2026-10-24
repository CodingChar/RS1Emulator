use crate::cdrom::CdRom;
use crate::controllers::ControllerManager;
use crate::dma::Dma;
use crate::gpu::Gpu;
use crate::interrupt::InterruptController;
use crate::memory::Memory;
use crate::spu::Spu;
use crate::timers::Timers;
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

/// System bus: owns main memory and every memory-mapped device, and routes
/// CPU accesses to the right one.
///
/// In the PS1, the CPU talks to RAM, BIOS, GPU, SPU, CD-ROM, timers, etc.
/// through memory-mapped I/O. This struct centralizes those mappings.
pub struct Bus {
    pub memory: Memory,
    pub gpu: Gpu,
    pub spu: Spu,
    pub cdrom: CdRom,
    pub dma: Dma,
    pub timers: Timers,
    pub interrupt: InterruptController,
    pub controllers: ControllerManager,
}

impl Bus {
    pub fn new() -> Self {
        Self {
            memory: Memory::new(),
            gpu: Gpu::new(),
            spu: Spu::new(),
            cdrom: CdRom::new(),
            dma: Dma::new(),
            timers: Timers::new(),
            interrupt: InterruptController::new(),
            controllers: ControllerManager::new(),
        }
    }

    /// Reset every device and clear RAM/VRAM (the BIOS ROM is kept).
    pub fn reset(&mut self) {
        self.gpu.reset();
        self.spu.reset();
        self.cdrom.reset();
        self.dma.reset();
        self.timers.reset();
        self.interrupt.reset();
        self.memory.reset();
    }

    /// Advance all devices by one cycle.
    pub fn step_devices(&mut self) {
        self.dma.step(&mut self.memory);
        self.timers.step(&mut self.interrupt);
        self.gpu.step();
        self.spu.step();
        self.cdrom.step();
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
    pub fn read32(&mut self, addr: u32) -> Ps1Result<u32> {
        match Self::decode(addr) {
            Target::Ram(o) => Ok(self.memory.ram.read32(o)),
            Target::Bios(o) => Ok(self.memory.bios.read32(o)),
            // TODO: implement scratchpad
            Target::Scratchpad => Ok(0),
            Target::Unmapped => Err(Ps1Error::MemoryOutOfRange(addr)),
        }
    }

    /// Read a 16-bit halfword from the bus.
    pub fn read16(&mut self, addr: u32) -> Ps1Result<u16> {
        match Self::decode(addr) {
            Target::Ram(o) => Ok(self.memory.ram.read16(o)),
            Target::Bios(o) => Ok(self.memory.bios.read16(o)),
            Target::Scratchpad => Ok(0),
            Target::Unmapped => Err(Ps1Error::MemoryOutOfRange(addr)),
        }
    }

    /// Read a byte from the bus.
    pub fn read8(&mut self, addr: u32) -> Ps1Result<u8> {
        match Self::decode(addr) {
            Target::Ram(o) => Ok(self.memory.ram.read8(o)),
            Target::Bios(o) => Ok(self.memory.bios.read8(o)),
            Target::Scratchpad => Ok(0),
            Target::Unmapped => Err(Ps1Error::MemoryOutOfRange(addr)),
        }
    }

    /// Write a 32-bit word to the bus at the given virtual address.
    pub fn write32(&mut self, addr: u32, value: u32) -> Ps1Result<()> {
        match Self::decode(addr) {
            Target::Ram(o) => {
                self.memory.ram.write32(o, value);
                Ok(())
            }
            // The BIOS is ROM: writes are ignored.
            // TODO: scratchpad / I/O registers
            Target::Bios(_) | Target::Scratchpad => Ok(()),
            Target::Unmapped => Err(Ps1Error::MemoryOutOfRange(addr)),
        }
    }

    /// Write a 16-bit halfword to the bus.
    pub fn write16(&mut self, addr: u32, value: u16) -> Ps1Result<()> {
        match Self::decode(addr) {
            Target::Ram(o) => {
                self.memory.ram.write16(o, value);
                Ok(())
            }
            Target::Bios(_) | Target::Scratchpad => Ok(()),
            Target::Unmapped => Err(Ps1Error::MemoryOutOfRange(addr)),
        }
    }

    /// Write a byte to the bus.
    pub fn write8(&mut self, addr: u32, value: u8) -> Ps1Result<()> {
        match Self::decode(addr) {
            Target::Ram(o) => {
                self.memory.ram.write8(o, value);
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

    #[test]
    fn ram_is_visible_through_all_segments() {
        let mut bus = Bus::new();
        bus.write32(0x8000_0100, 0xDEAD_BEEF).unwrap();
        assert_eq!(bus.read32(0x0000_0100).unwrap(), 0xDEAD_BEEF); // KUSEG
        assert_eq!(bus.read32(0x8000_0100).unwrap(), 0xDEAD_BEEF); // KSEG0
        assert_eq!(bus.read32(0xA000_0100).unwrap(), 0xDEAD_BEEF); // KSEG1
    }

    #[test]
    fn ram_is_mirrored_in_the_first_8mb() {
        let mut bus = Bus::new();
        bus.write32(0x0000_0040, 0x1234_5678).unwrap();
        assert_eq!(bus.read32(0x0020_0040).unwrap(), 0x1234_5678);
        assert_eq!(bus.read32(0x0060_0040).unwrap(), 0x1234_5678);
    }

    #[test]
    fn sub_word_accesses_are_little_endian() {
        let mut bus = Bus::new();
        bus.write32(0x200, 0x4433_2211).unwrap();
        assert_eq!(bus.read8(0x200).unwrap(), 0x11);
        assert_eq!(bus.read8(0x203).unwrap(), 0x44);
        assert_eq!(bus.read16(0x200).unwrap(), 0x2211);
        assert_eq!(bus.read16(0x202).unwrap(), 0x4433);

        bus.write8(0x201, 0xAA).unwrap();
        bus.write16(0x202, 0xBBCC).unwrap();
        assert_eq!(bus.read32(0x200).unwrap(), 0xBBCC_AA11);
    }

    #[test]
    fn bios_is_reachable_from_kseg0_and_kseg1_and_is_read_only() {
        let mut bus = Bus::new();
        bus.memory.bios.as_mut_slice()[..4].copy_from_slice(&0xCAFE_F00Du32.to_le_bytes());
        assert_eq!(bus.read32(0xBFC0_0000).unwrap(), 0xCAFE_F00D);
        assert_eq!(bus.read32(0x9FC0_0000).unwrap(), 0xCAFE_F00D);
        assert_eq!(bus.read8(0xBFC0_0000).unwrap(), 0x0D);
        assert_eq!(bus.read16(0xBFC0_0002).unwrap(), 0xCAFE);

        bus.write32(0xBFC0_0000, 0).unwrap();
        assert_eq!(bus.read32(0xBFC0_0000).unwrap(), 0xCAFE_F00D);
    }

    #[test]
    fn unmapped_addresses_are_reported() {
        let mut bus = Bus::new();
        assert_eq!(
            bus.read32(0xBF80_1000),
            Err(Ps1Error::MemoryOutOfRange(0xBF80_1000))
        );
        assert!(bus.read32(0xFFFE_0130).is_err());
    }
}
