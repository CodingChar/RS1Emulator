use crate::cdrom::CdRom;
use crate::controllers::ControllerManager;
use crate::dma::Dma;
use crate::gpu::Gpu;
use crate::interrupt::InterruptController;
use crate::io::{extract, merge, read_le, write_le, StubRegs};
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

const EXPANSION1_START: u32 = 0x1F00_0000;
const EXPANSION1_END: u32 = 0x1F7F_FFFF;
const SCRATCHPAD_START: u32 = 0x1F80_0000;
const SCRATCHPAD_END: u32 = 0x1F80_03FF;
/// Hardware registers, followed by expansion region 2.
const IO_START: u32 = 0x1F80_1000;
const IO_END: u32 = 0x1F80_3FFF;
const BIOS_START: u32 = 0x1FC0_0000;
const BIOS_END: u32 = 0x1FC7_FFFF;
/// Cache control register, only reachable through KSEG2.
const CACHE_CONTROL: u32 = 0xFFFE_0130;

// I/O register blocks (physical addresses).
const MEM_CONTROL1: u32 = 0x1F80_1000;
const MEM_CONTROL1_END: u32 = 0x1F80_1023;
const SIO: u32 = 0x1F80_1040;
const SIO_END: u32 = 0x1F80_105F;
const RAM_SIZE: u32 = 0x1F80_1060;
const RAM_SIZE_END: u32 = 0x1F80_1063;
const INTERRUPT: u32 = 0x1F80_1070;
const INTERRUPT_END: u32 = 0x1F80_1077;
const DMA: u32 = 0x1F80_1080;
const DMA_END: u32 = 0x1F80_10FF;
const TIMERS: u32 = 0x1F80_1100;
const TIMERS_END: u32 = 0x1F80_112F;
const CDROM: u32 = 0x1F80_1800;
const CDROM_END: u32 = 0x1F80_1803;
const GPU: u32 = 0x1F80_1810;
const GPU_END: u32 = 0x1F80_1817;
const MDEC: u32 = 0x1F80_1820;
const MDEC_END: u32 = 0x1F80_1827;
const SPU: u32 = 0x1F80_1C00;
const SPU_END: u32 = 0x1F80_1FFF;
const EXPANSION2: u32 = 0x1F80_2000;
const EXPANSION2_END: u32 = 0x1F80_207F;
/// The BIOS writes its boot progress ("POST") code here.
const POST_REGISTER: u32 = 0x1F80_2041;

/// Memory control registers at power-on: expansion 1/2 base addresses and the
/// access-timing registers of each region.
const MEM_CONTROL_RESET: [u32; 9] = [
    0x1F00_0000, // expansion 1 base
    0x1F80_2000, // expansion 2 base
    0x0013_243F, // expansion 1 delay/size
    0x0000_3022, // expansion 3 delay/size
    0x0013_243F, // BIOS ROM delay/size
    0x2009_31E1, // SPU delay/size
    0x0002_0843, // CD-ROM delay/size
    0x0007_0777, // expansion 2 delay/size
    0x0003_1125, // common delay
];
const RAM_SIZE_RESET: u32 = 0x0000_0B88;
/// JOY_STAT at power-on: transmitter ready.
const JOY_STAT_RESET: u32 = 0x0000_0005;
/// MDEC status at power-on: data-out FIFO empty, ready for commands.
const MDEC_STATUS_RESET: u32 = 0x8004_0000;

/// Keep at most this many unhandled accesses in `Bus::unhandled_io`.
const UNHANDLED_IO_LOG_LIMIT: usize = 256;

enum Target {
    Ram(usize),
    Bios(usize),
    Scratchpad(usize),
    /// A hardware register address (physical).
    Io(u32),
    Expansion1,
    CacheControl,
    Unmapped,
}

/// An access to an I/O address no device claims, kept for debugging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnhandledIo {
    pub addr: u32,
    /// Access size in bytes.
    pub width: u32,
    pub write: bool,
    pub value: u32,
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

    /// The 1 KB data cache used as fast RAM (`0x1F800000`).
    pub scratchpad: [u8; 1024],
    /// Memory control 1 registers (`0x1F801000..0x1F801024`).
    pub mem_control: [u32; 9],
    /// `RAM_SIZE` (`0x1F801060`).
    pub ram_size: u32,
    /// Cache control (`0xFFFE0130`).
    pub cache_control: u32,
    /// Last value written to the BIOS POST register.
    pub post_code: u8,

    /// Serial port registers (controllers/memory cards), not emulated yet.
    sio: StubRegs,
    /// Motion decoder registers, not emulated yet.
    mdec: StubRegs,

    /// First accesses to unclaimed I/O addresses (see `unhandled_io_count`).
    pub unhandled_io: Vec<UnhandledIo>,
    /// Total number of accesses to unclaimed I/O addresses.
    pub unhandled_io_count: u64,
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
            scratchpad: [0; 1024],
            mem_control: MEM_CONTROL_RESET,
            ram_size: RAM_SIZE_RESET,
            cache_control: 0,
            post_code: 0,
            sio: Self::idle_sio(),
            mdec: Self::idle_mdec(),
            unhandled_io: Vec::new(),
            unhandled_io_count: 0,
        }
    }

    fn idle_sio() -> StubRegs {
        StubRegs::new(0x20).with_u32(4, JOY_STAT_RESET)
    }

    fn idle_mdec() -> StubRegs {
        StubRegs::new(8).with_u32(4, MDEC_STATUS_RESET)
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
        self.scratchpad = [0; 1024];
        self.mem_control = MEM_CONTROL_RESET;
        self.ram_size = RAM_SIZE_RESET;
        self.cache_control = 0;
        self.post_code = 0;
        self.sio = Self::idle_sio();
        self.mdec = Self::idle_mdec();
        self.unhandled_io.clear();
        self.unhandled_io_count = 0;
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
            EXPANSION1_START..=EXPANSION1_END => Target::Expansion1,
            SCRATCHPAD_START..=SCRATCHPAD_END => Target::Scratchpad((phys & 0x3FF) as usize),
            IO_START..=IO_END => Target::Io(phys),
            BIOS_START..=BIOS_END => Target::Bios((phys - BIOS_START) as usize),
            CACHE_CONTROL..=0xFFFE_0133 => Target::CacheControl,
            _ => Target::Unmapped,
        }
    }

    /// Value read from an address nothing drives: all ones.
    fn open_bus(width: u32) -> u32 {
        match width {
            1 => 0xFF,
            2 => 0xFFFF,
            _ => 0xFFFF_FFFF,
        }
    }

    fn read(&mut self, addr: u32, width: u32) -> Ps1Result<u32> {
        Ok(match Self::decode(addr) {
            Target::Ram(o) => match width {
                1 => self.memory.ram.read8(o) as u32,
                2 => self.memory.ram.read16(o) as u32,
                _ => self.memory.ram.read32(o),
            },
            Target::Bios(o) => match width {
                1 => self.memory.bios.read8(o) as u32,
                2 => self.memory.bios.read16(o) as u32,
                _ => self.memory.bios.read32(o),
            },
            Target::Scratchpad(o) => read_le(&self.scratchpad, o, width),
            Target::Io(phys) => self.io_read(phys, width),
            Target::Expansion1 => Self::open_bus(width),
            Target::CacheControl => extract(self.cache_control, addr, width),
            Target::Unmapped => return Err(Ps1Error::MemoryOutOfRange(addr)),
        })
    }

    fn write(&mut self, addr: u32, width: u32, value: u32) -> Ps1Result<()> {
        match Self::decode(addr) {
            Target::Ram(o) => match width {
                1 => self.memory.ram.write8(o, value as u8),
                2 => self.memory.ram.write16(o, value as u16),
                _ => self.memory.ram.write32(o, value),
            },
            Target::Scratchpad(o) => write_le(&mut self.scratchpad, o, width, value),
            Target::Io(phys) => self.io_write(phys, width, value),
            Target::CacheControl => {
                self.cache_control = merge(self.cache_control, addr, width, value);
            }
            // The BIOS is ROM and expansion 1 has no device: writes are ignored.
            Target::Bios(_) | Target::Expansion1 => {}
            Target::Unmapped => return Err(Ps1Error::MemoryOutOfRange(addr)),
        }
        Ok(())
    }

    /// Read a 32-bit word from the bus at the given virtual address.
    pub fn read32(&mut self, addr: u32) -> Ps1Result<u32> {
        self.read(addr, 4)
    }

    /// Read a 16-bit halfword from the bus.
    pub fn read16(&mut self, addr: u32) -> Ps1Result<u16> {
        self.read(addr, 2).map(|v| v as u16)
    }

    /// Read a byte from the bus.
    pub fn read8(&mut self, addr: u32) -> Ps1Result<u8> {
        self.read(addr, 1).map(|v| v as u8)
    }

    /// Write a 32-bit word to the bus at the given virtual address.
    pub fn write32(&mut self, addr: u32, value: u32) -> Ps1Result<()> {
        self.write(addr, 4, value)
    }

    /// Write a 16-bit halfword to the bus.
    pub fn write16(&mut self, addr: u32, value: u16) -> Ps1Result<()> {
        self.write(addr, 2, value as u32)
    }

    /// Write a byte to the bus.
    pub fn write8(&mut self, addr: u32, value: u8) -> Ps1Result<()> {
        self.write(addr, 1, value as u32)
    }

    // ----- Memory-mapped I/O -----

    fn io_read(&mut self, phys: u32, width: u32) -> u32 {
        match phys {
            MEM_CONTROL1..=MEM_CONTROL1_END => {
                extract(self.mem_control[((phys - MEM_CONTROL1) >> 2) as usize], phys, width)
            }
            SIO..=SIO_END => self.sio.read(phys - SIO, width),
            RAM_SIZE..=RAM_SIZE_END => extract(self.ram_size, phys, width),
            INTERRUPT..=INTERRUPT_END => self.interrupt.read(phys - INTERRUPT, width),
            DMA..=DMA_END => self.dma.read(phys - DMA, width),
            TIMERS..=TIMERS_END => self.timers.read(phys - TIMERS, width),
            CDROM..=CDROM_END => self.cdrom.read(phys - CDROM, width),
            GPU..=GPU_END => self.gpu.read(phys - GPU, width),
            MDEC..=MDEC_END => self.mdec.read(phys - MDEC, width),
            SPU..=SPU_END => self.spu.read(phys - SPU, width),
            EXPANSION2..=EXPANSION2_END => Self::open_bus(width),
            _ => {
                self.log_unhandled(phys, width, false, 0);
                0
            }
        }
    }

    fn io_write(&mut self, phys: u32, width: u32, value: u32) {
        match phys {
            MEM_CONTROL1..=MEM_CONTROL1_END => {
                let i = ((phys - MEM_CONTROL1) >> 2) as usize;
                self.mem_control[i] = merge(self.mem_control[i], phys, width, value);
            }
            SIO..=SIO_END => self.sio.write(phys - SIO, width, value),
            RAM_SIZE..=RAM_SIZE_END => self.ram_size = merge(self.ram_size, phys, width, value),
            INTERRUPT..=INTERRUPT_END => self.interrupt.write(phys - INTERRUPT, width, value),
            DMA..=DMA_END => self.dma.write(phys - DMA, width, value),
            TIMERS..=TIMERS_END => self.timers.write(phys - TIMERS, width, value),
            CDROM..=CDROM_END => self.cdrom.write(phys - CDROM, width, value),
            GPU..=GPU_END => self.gpu.write(phys - GPU, width, value),
            MDEC..=MDEC_END => self.mdec.write(phys - MDEC, width, value),
            SPU..=SPU_END => self.spu.write(phys - SPU, width, value),
            POST_REGISTER => self.post_code = value as u8,
            EXPANSION2..=EXPANSION2_END => {}
            _ => self.log_unhandled(phys, width, true, value),
        }
    }

    fn log_unhandled(&mut self, addr: u32, width: u32, write: bool, value: u32) {
        self.unhandled_io_count += 1;
        if self.unhandled_io.len() < UNHANDLED_IO_LOG_LIMIT {
            self.unhandled_io.push(UnhandledIo { addr, width, write, value });
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
    use crate::interrupt::{IRQ_CDROM, IRQ_DMA, IRQ_VBLANK};

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
    fn addresses_outside_every_region_are_reported() {
        let mut bus = Bus::new();
        assert_eq!(
            bus.read32(0xBFA0_0000),
            Err(Ps1Error::MemoryOutOfRange(0xBFA0_0000))
        );
        assert!(bus.write32(0x1F90_0000, 0).is_err());
        assert!(bus.read32(0xFFFE_0200).is_err());
    }

    // ----- Scratchpad -----

    #[test]
    fn scratchpad_stores_data_at_every_width() {
        let mut bus = Bus::new();
        bus.write32(0x1F80_0000, 0x4433_2211).unwrap();
        assert_eq!(bus.read32(0x1F80_0000).unwrap(), 0x4433_2211);
        assert_eq!(bus.read8(0x1F80_0001).unwrap(), 0x22);
        assert_eq!(bus.read16(0x1F80_0002).unwrap(), 0x4433);

        bus.write16(0x1F80_03FE, 0xBEEF).unwrap();
        bus.write8(0x1F80_0010, 0x7F).unwrap();
        assert_eq!(bus.read16(0x1F80_03FE).unwrap(), 0xBEEF);
        assert_eq!(bus.read8(0x1F80_0010).unwrap(), 0x7F);
        // Last word of the 1 KB.
        bus.write32(0x1F80_03FC, 0xA5A5_5A5A).unwrap();
        assert_eq!(bus.read32(0x1F80_03FC).unwrap(), 0xA5A5_5A5A);
    }

    #[test]
    fn scratchpad_is_reachable_from_kuseg_and_kseg0_and_is_not_ram() {
        let mut bus = Bus::new();
        bus.write32(0x1F80_0020, 0x1111_2222).unwrap();
        assert_eq!(bus.read32(0x9F80_0020).unwrap(), 0x1111_2222);
        assert_eq!(bus.read32(0x0000_0020).unwrap(), 0);
    }

    // ----- Memory control -----

    #[test]
    fn memory_control_registers_have_power_on_values_and_read_back_writes() {
        let mut bus = Bus::new();
        assert_eq!(bus.read32(0x1F80_1000).unwrap(), 0x1F00_0000);
        assert_eq!(bus.read32(0x1F80_1004).unwrap(), 0x1F80_2000);
        assert_eq!(bus.read32(0x1F80_1010).unwrap(), 0x0013_243F);
        assert_eq!(bus.read32(0x1F80_1020).unwrap(), 0x0003_1125);

        bus.write32(0x1F80_1008, 0x1234_5678).unwrap();
        assert_eq!(bus.read32(0x1F80_1008).unwrap(), 0x1234_5678);
        bus.write16(0x1F80_1008, 0xAAAA).unwrap();
        assert_eq!(bus.read32(0x1F80_1008).unwrap(), 0x1234_AAAA);
    }

    #[test]
    fn ram_size_and_cache_control_registers() {
        let mut bus = Bus::new();
        assert_eq!(bus.read32(0x1F80_1060).unwrap(), 0x0B88);
        bus.write32(0x1F80_1060, 0x0000_0888).unwrap();
        assert_eq!(bus.read32(0x1F80_1060).unwrap(), 0x0888);

        bus.write32(0xFFFE_0130, 0x0001_E988).unwrap();
        assert_eq!(bus.read32(0xFFFE_0130).unwrap(), 0x0001_E988);
    }

    #[test]
    fn expansion_regions_have_no_device() {
        let mut bus = Bus::new();
        assert_eq!(bus.read32(0x1F00_0000).unwrap(), 0xFFFF_FFFF);
        assert_eq!(bus.read8(0x1F00_0084).unwrap(), 0xFF);
        bus.write32(0x1F00_0000, 0).unwrap(); // ignored
        assert_eq!(bus.read8(0x1F80_2000).unwrap(), 0xFF);
        assert_eq!(bus.unhandled_io_count, 0);
    }

    #[test]
    fn post_register_remembers_the_boot_progress_code() {
        let mut bus = Bus::new();
        bus.write8(0x1F80_2041, 0x0E).unwrap();
        assert_eq!(bus.post_code, 0x0E);
    }

    // ----- Interrupt controller -----

    #[test]
    fn interrupt_status_is_acknowledged_by_writing_zero_bits() {
        let mut bus = Bus::new();
        bus.interrupt.raise(IRQ_VBLANK);
        bus.interrupt.raise(IRQ_CDROM);
        assert_eq!(bus.read32(0x1F80_1070).unwrap(), 0b101);

        // Acknowledge only VBLANK: write 1s everywhere except that bit.
        bus.write32(0x1F80_1070, !1).unwrap();
        assert_eq!(bus.read32(0x1F80_1070).unwrap(), 0b100);
        // Writing 1 never raises or re-arms an interrupt.
        bus.write32(0x1F80_1070, 0xFFFF_FFFF).unwrap();
        assert_eq!(bus.read32(0x1F80_1070).unwrap(), 0b100);
        bus.write32(0x1F80_1070, 0).unwrap();
        assert_eq!(bus.read32(0x1F80_1070).unwrap(), 0);
    }

    #[test]
    fn partial_width_stat_writes_only_acknowledge_their_own_bytes() {
        let mut bus = Bus::new();
        bus.interrupt.raise(IRQ_CDROM); // bit 2, low byte
        bus.interrupt.raise(10); // bit 10, high byte
        bus.write8(0x1F80_1071, 0x00).unwrap(); // clears bits 8..15 only
        assert_eq!(bus.read32(0x1F80_1070).unwrap(), 0b100);
        bus.write16(0x1F80_1070, 0).unwrap();
        assert_eq!(bus.read32(0x1F80_1070).unwrap(), 0);
    }

    #[test]
    fn interrupt_mask_is_eleven_bits_and_drives_pending() {
        let mut bus = Bus::new();
        bus.write32(0x1F80_1074, 0xFFFF_FFFF).unwrap();
        assert_eq!(bus.read32(0x1F80_1074).unwrap(), 0x7FF);

        bus.write32(0x1F80_1074, 1 << IRQ_DMA).unwrap();
        bus.interrupt.raise(IRQ_CDROM);
        assert!(!bus.interrupt.pending());
        bus.interrupt.raise(IRQ_DMA);
        assert!(bus.interrupt.pending());
    }

    // ----- DMA -----

    #[test]
    fn dma_registers_have_power_on_values() {
        let mut bus = Bus::new();
        assert_eq!(bus.read32(0x1F80_10F0).unwrap(), 0x0765_4321);
        assert_eq!(bus.read32(0x1F80_10F4).unwrap(), 0);
    }

    #[test]
    fn dma_channel_registers_read_back_and_madr_is_24_bit() {
        let mut bus = Bus::new();
        // Channel 2 (GPU): MADR, BCR, CHCR.
        bus.write32(0x1F80_10A0, 0xFF12_3456).unwrap();
        bus.write32(0x1F80_10A4, 0x0010_0020).unwrap();
        bus.write32(0x1F80_10A8, 0x0100_0201).unwrap();
        assert_eq!(bus.read32(0x1F80_10A0).unwrap(), 0x0012_3456);
        assert_eq!(bus.read32(0x1F80_10A4).unwrap(), 0x0010_0020);
        assert_eq!(bus.read32(0x1F80_10A8).unwrap(), 0x0100_0201);
    }

    #[test]
    fn dicr_flags_are_cleared_by_writing_one_and_master_flag_is_computed() {
        let mut bus = Bus::new();
        // Enable channel 3 (bit 19) and the master enable (bit 23).
        bus.write32(0x1F80_10F4, (1 << 23) | (1 << 19)).unwrap();
        assert_eq!(bus.read32(0x1F80_10F4).unwrap(), (1 << 23) | (1 << 19));

        // Hardware sets flag bit 27 (channel 3); simulate it.
        bus.dma.registers[29] |= 1 << 27;
        bus.write32(0x1F80_10F4, (1 << 23) | (1 << 19)).unwrap(); // keeps the flag
        let dicr = bus.read32(0x1F80_10F4).unwrap();
        assert_ne!(dicr & (1 << 27), 0, "writing 0 leaves the flag alone");
        assert_ne!(dicr & (1 << 31), 0, "master flag set when an enabled channel is flagged");

        // Writing a 1 to the flag acknowledges it and drops the master flag.
        bus.write32(0x1F80_10F4, (1 << 23) | (1 << 19) | (1 << 27)).unwrap();
        let dicr = bus.read32(0x1F80_10F4).unwrap();
        assert_eq!(dicr & (1 << 27), 0);
        assert_eq!(dicr & (1 << 31), 0);
    }

    #[test]
    fn dicr_force_bit_sets_the_master_flag() {
        let mut bus = Bus::new();
        bus.write32(0x1F80_10F4, 1 << 15).unwrap();
        assert_ne!(bus.read32(0x1F80_10F4).unwrap() & (1 << 31), 0);
    }

    // ----- Timers -----

    #[test]
    fn timer_registers_read_back_and_mode_write_restarts_the_counter() {
        let mut bus = Bus::new();
        bus.write32(0x1F80_1118, 0x1234).unwrap(); // timer 1 target
        bus.write32(0x1F80_1110, 0x77).unwrap(); // timer 1 counter
        assert_eq!(bus.read32(0x1F80_1110).unwrap(), 0x77);
        bus.write32(0x1F80_1114, 0x0158).unwrap(); // timer 1 mode
        assert_eq!(bus.read32(0x1F80_1110).unwrap(), 0, "mode write restarts the counter");
        assert_eq!(bus.read32(0x1F80_1118).unwrap(), 0x1234);
        let mode = bus.read32(0x1F80_1114).unwrap();
        assert_eq!(mode & 0x7FF, 0x158 | (1 << 10));
    }

    #[test]
    fn timer_reached_flags_clear_on_mode_read() {
        let mut bus = Bus::new();
        bus.timers.mode[0] = 0x1800;
        assert_eq!(bus.read32(0x1F80_1104).unwrap() & 0x1800, 0x1800);
        assert_eq!(bus.read32(0x1F80_1104).unwrap() & 0x1800, 0);
    }

    // ----- GPU -----

    #[test]
    fn gpu_reports_ready_and_remembers_command_words() {
        let mut bus = Bus::new();
        let stat = bus.read32(0x1F80_1814).unwrap();
        assert_eq!(stat & 0x1C00_0000, 0x1C00_0000);
        bus.write32(0x1F80_1810, 0xE100_0000).unwrap();
        bus.write32(0x1F80_1814, 0x0300_0000).unwrap();
        assert_eq!(bus.gpu.regs[0], 0xE100_0000);
        assert_eq!(bus.gpu.regs[1], 0x0300_0000);
    }

    // ----- SPU -----

    #[test]
    fn spu_registers_read_back_at_every_width() {
        let mut bus = Bus::new();
        bus.write16(0x1F80_1D80, 0x3FFF).unwrap(); // main volume left
        assert_eq!(bus.read16(0x1F80_1D80).unwrap(), 0x3FFF);

        bus.write32(0x1F80_1C00, 0x2222_1111).unwrap(); // voice 0 volumes
        assert_eq!(bus.read16(0x1F80_1C00).unwrap(), 0x1111);
        assert_eq!(bus.read16(0x1F80_1C02).unwrap(), 0x2222);
        assert_eq!(bus.read32(0x1F80_1C00).unwrap(), 0x2222_1111);

        bus.write8(0x1F80_1C01, 0xAB).unwrap();
        assert_eq!(bus.read16(0x1F80_1C00).unwrap(), 0xAB11);
        assert_eq!(bus.read8(0x1F80_1C01).unwrap(), 0xAB);
    }

    #[test]
    fn spu_status_mirrors_the_control_register_and_is_read_only() {
        let mut bus = Bus::new();
        bus.write16(0x1F80_1DAA, 0xC0DF).unwrap(); // SPUCNT
        assert_eq!(bus.read16(0x1F80_1DAE).unwrap(), 0x001F); // SPUSTAT = low 6 bits
        bus.write16(0x1F80_1DAE, 0xFFFF).unwrap();
        assert_eq!(bus.read16(0x1F80_1DAE).unwrap(), 0x001F);
    }

    // ----- Stubbed devices -----

    #[test]
    fn serial_port_reports_transmitter_ready() {
        let mut bus = Bus::new();
        assert_eq!(bus.read32(0x1F80_1044).unwrap() & 5, 5);
    }

    #[test]
    fn cdrom_registers_report_an_idle_controller() {
        let mut bus = Bus::new();
        assert_eq!(bus.read8(0x1F80_1800).unwrap(), 0x18);
    }

    #[test]
    fn mdec_registers_report_a_ready_status() {
        let mut bus = Bus::new();
        assert_eq!(bus.read32(0x1F80_1824).unwrap(), 0x8004_0000);
    }

    // ----- Unhandled I/O -----

    #[test]
    fn unclaimed_io_addresses_are_logged_not_fatal() {
        let mut bus = Bus::new();
        assert_eq!(bus.read32(0x1F80_1500).unwrap(), 0);
        bus.write16(0x1F80_1502, 0xBEEF).unwrap();
        assert_eq!(bus.unhandled_io_count, 2);
        assert_eq!(
            bus.unhandled_io,
            vec![
                UnhandledIo { addr: 0x1F80_1500, width: 4, write: false, value: 0 },
                UnhandledIo { addr: 0x1F80_1502, width: 2, write: true, value: 0xBEEF },
            ]
        );
    }

    #[test]
    fn unhandled_io_log_is_bounded_but_keeps_counting() {
        let mut bus = Bus::new();
        for _ in 0..(UNHANDLED_IO_LOG_LIMIT + 10) {
            bus.read32(0x1F80_1500).unwrap();
        }
        assert_eq!(bus.unhandled_io.len(), UNHANDLED_IO_LOG_LIMIT);
        assert_eq!(bus.unhandled_io_count, (UNHANDLED_IO_LOG_LIMIT + 10) as u64);
    }

    #[test]
    fn reset_restores_device_registers_and_clears_the_log() {
        let mut bus = Bus::new();
        bus.write32(0x1F80_1008, 0).unwrap();
        bus.write32(0x1F80_0000, 0xFFFF_FFFF).unwrap();
        bus.read32(0x1F80_1500).unwrap();
        bus.reset();
        assert_eq!(bus.read32(0x1F80_1008).unwrap(), 0x0013_243F);
        assert_eq!(bus.read32(0x1F80_0000).unwrap(), 0);
        assert_eq!(bus.unhandled_io_count, 0);
    }
}
