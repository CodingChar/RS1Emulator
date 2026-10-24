use crate::cdrom::CdRom;
use crate::controllers::sio::Sio0;
use crate::controllers::ControllerManager;
use crate::dma::{Dma, CHANNEL_MDEC_IN, CHANNEL_MDEC_OUT};
use crate::gpu::Gpu;
use crate::interrupt::{
    InterruptController, IRQ_CDROM, IRQ_CONTROLLER, IRQ_DMA, IRQ_GPU, IRQ_SPU, IRQ_VBLANK,
};
use crate::io::{extract, merge, read_le, write_le, StubRegs};
use crate::mdec::Mdec;
use crate::memory::Memory;
use crate::spu::Spu;
use crate::timers::{TimerClocks, Timers};
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

    /// Serial port 0: controllers and memory cards.
    pub sio0: Sio0,
    /// Serial port 1 (link cable) registers, not emulated.
    sio1: StubRegs,
    /// Motion decoder (FMV video decompression).
    pub mdec: Mdec,
    /// DMA channel 1 was started before the MDEC had produced enough data and
    /// finishes as soon as it has.
    mdec_out_waiting: bool,

    /// First accesses to unclaimed I/O addresses (see `unhandled_io_count`).
    pub unhandled_io: Vec<UnhandledIo>,
    /// Total number of accesses to unclaimed I/O addresses.
    pub unhandled_io_count: u64,

    /// CPU cycles run since the devices were last brought up to date.
    pending_cycles: u32,
    /// Cycles after which a device may have something to report.
    deadline: u32,
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
            sio0: Sio0::new(),
            sio1: Self::idle_sio1(),
            mdec: Mdec::new(),
            mdec_out_waiting: false,
            unhandled_io: Vec::new(),
            unhandled_io_count: 0,
            pending_cycles: 0,
            deadline: 1,
        }
        .with_deadline()
    }

    fn with_deadline(mut self) -> Self {
        self.deadline = self.compute_deadline();
        self
    }

    fn idle_sio1() -> StubRegs {
        StubRegs::new(0x10)
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
        self.sio0.reset();
        self.controllers.deselect();
        self.sio1 = Self::idle_sio1();
        self.mdec.reset();
        self.mdec_out_waiting = false;
        self.unhandled_io.clear();
        self.unhandled_io_count = 0;
        self.pending_cycles = 0;
        self.deadline = self.compute_deadline();
    }

    // ----- Device scheduling -----
    //
    // Devices do not run on every CPU cycle. The bus counts the cycles and
    // brings the devices up to date only when one of them may have something
    // to report (the `deadline`) or when the CPU talks to a device, so the
    // result is exactly what stepping them every cycle would give.

    /// The cycles after which the next device event can happen (at least 1).
    fn compute_deadline(&self) -> u32 {
        self.gpu
            .cycles_until_vblank()
            .min(self.timers.cycles_until_event(&self.gpu.video_timing()))
            .min(self.cdrom.cycles_until_event())
            .min(self.sio0.cycles_until_event())
            .min(self.spu.cycles_until_event())
            .max(1)
    }

    /// Account for `cycles` executed CPU cycles.
    #[inline]
    pub fn tick(&mut self, cycles: u32) {
        self.pending_cycles += cycles;
        if self.pending_cycles >= self.deadline {
            self.sync_devices();
        }
    }

    /// Bring every device up to date with the CPU.
    pub fn sync_devices(&mut self) {
        let cycles = std::mem::take(&mut self.pending_cycles);
        if cycles == 0 {
            return;
        }
        if cycles < self.deadline {
            // Nothing can happen yet: skip ahead and keep the (shorter) deadline.
            self.skip_devices(cycles);
            self.deadline -= cycles;
        } else {
            // The last cycle is where the event may happen: run it exactly.
            self.skip_devices(cycles - 1);
            self.step_devices();
            self.deadline = self.compute_deadline();
        }
    }

    /// Run the devices for `cycles` CPU cycles in which nothing happens.
    fn skip_devices(&mut self, cycles: u32) {
        if cycles == 0 {
            return;
        }
        let video = self.gpu.advance(cycles);
        if video.vblank_started {
            self.interrupt.raise(IRQ_VBLANK);
        }
        let clocks = TimerClocks {
            sysclk: cycles,
            dotclock: video.dot_ticks,
            hblank: video.hblanks,
        };
        self.timers.tick(clocks, &mut self.interrupt);
        self.cdrom.skip_cycles(cycles);
        self.sio0.skip_cycles(cycles);
        self.spu.skip_cycles(cycles);
    }

    /// Advance all devices by exactly one CPU cycle.
    pub fn step_devices(&mut self) {
        let video = self.gpu.advance(1);
        if video.vblank_started {
            self.interrupt.raise(IRQ_VBLANK);
        }
        let clocks = TimerClocks {
            sysclk: 1,
            dotclock: video.dot_ticks,
            hblank: video.hblanks,
        };
        self.timers.tick(clocks, &mut self.interrupt);

        self.gpu.step();
        self.forward_gpu_irq();
        self.spu.step(&mut self.cdrom);
        if self.spu.take_irq() {
            self.interrupt.raise(IRQ_SPU);
        }
        self.cdrom.step();
        self.forward_cdrom_irq();
        self.sio0.step(&mut self.controllers);
        self.forward_sio_irq();
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

    /// Read `width` bytes, or `None` when nothing is mapped at `addr`. This is
    /// what the CPU uses: RAM, by far the most common target, is checked first.
    #[inline]
    pub fn load(&mut self, addr: u32, width: u32) -> Option<u32> {
        let phys = addr & REGION_MASK[(addr >> 29) as usize];
        if phys < 0x0080_0000 {
            let ram = &self.memory.ram;
            let offset = phys as usize;
            return Some(match width {
                4 => ram.read32(offset),
                2 => ram.read16(offset) as u32,
                _ => ram.read8(offset) as u32,
            });
        }
        self.load_slow(addr, width)
    }

    /// Write `width` bytes; false when nothing is mapped at `addr`.
    #[inline]
    pub fn store(&mut self, addr: u32, width: u32, value: u32) -> bool {
        let phys = addr & REGION_MASK[(addr >> 29) as usize];
        if phys < 0x0080_0000 {
            let ram = &mut self.memory.ram;
            let offset = phys as usize;
            match width {
                4 => ram.write32(offset, value),
                2 => ram.write16(offset, value as u16),
                _ => ram.write8(offset, value as u8),
            }
            return true;
        }
        self.store_slow(addr, width, value)
    }

    fn read(&mut self, addr: u32, width: u32) -> Ps1Result<u32> {
        self.load(addr, width).ok_or(Ps1Error::MemoryOutOfRange(addr))
    }

    fn write(&mut self, addr: u32, width: u32, value: u32) -> Ps1Result<()> {
        if self.store(addr, width, value) {
            Ok(())
        } else {
            Err(Ps1Error::MemoryOutOfRange(addr))
        }
    }

    #[inline(never)]
    fn load_slow(&mut self, addr: u32, width: u32) -> Option<u32> {
        Some(match Self::decode(addr) {
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
            Target::Unmapped => return None,
        })
    }

    #[inline(never)]
    fn store_slow(&mut self, addr: u32, width: u32, value: u32) -> bool {
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
            Target::Unmapped => return false,
        }
        true
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

    /// Whether an access to this register depends on, or sets, the time
    /// (so the devices must be brought up to date first). The others hold
    /// state that only changes at the events the deadline already covers.
    fn is_timed(phys: u32) -> bool {
        match phys {
            TIMERS..=TIMERS_END | CDROM..=CDROM_END | SIO..=SIO_END => true,
            // GPUSTAT (scanline, field) and GP1 (video mode); not GP0/GPUREAD.
            GPU..=GPU_END => phys >= GPU + 4,
            _ => false,
        }
    }

    fn io_read(&mut self, phys: u32, width: u32) -> u32 {
        if !Self::is_timed(phys) {
            return self.io_read_synced(phys, width);
        }
        self.sync_devices();
        let value = self.io_read_synced(phys, width);
        // Reading a serial port or the drive can start something new.
        if matches!(phys, SIO..=SIO_END | CDROM..=CDROM_END) {
            self.deadline = self.compute_deadline();
        }
        value
    }

    fn io_read_synced(&mut self, phys: u32, width: u32) -> u32 {
        match phys {
            MEM_CONTROL1..=MEM_CONTROL1_END => {
                extract(self.mem_control[((phys - MEM_CONTROL1) >> 2) as usize], phys, width)
            }
            SIO..=SIO_END => match phys - SIO {
                off @ 0..=0x0F => self.sio0.read(off, width),
                off => self.sio1.read(off - 0x10, width),
            },
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
        let timed = Self::is_timed(phys);
        if timed {
            self.sync_devices();
        }
        self.io_write_synced(phys, width, value);
        if self.spu.take_irq() {
            self.interrupt.raise(IRQ_SPU);
        }
        if timed {
            self.deadline = self.compute_deadline();
        }
    }

    fn io_write_synced(&mut self, phys: u32, width: u32, value: u32) {
        match phys {
            MEM_CONTROL1..=MEM_CONTROL1_END => {
                let i = ((phys - MEM_CONTROL1) >> 2) as usize;
                self.mem_control[i] = merge(self.mem_control[i], phys, width, value);
            }
            SIO..=SIO_END => match phys - SIO {
                off @ 0..=0x0F => {
                    self.sio0.write(off, width, value, &mut self.controllers);
                    self.forward_sio_irq();
                }
                off => self.sio1.write(off - 0x10, width, value),
            },
            RAM_SIZE..=RAM_SIZE_END => self.ram_size = merge(self.ram_size, phys, width, value),
            INTERRUPT..=INTERRUPT_END => self.interrupt.write(phys - INTERRUPT, width, value),
            DMA..=DMA_END => {
                if let Some(channel) = self.dma.write(phys - DMA, width, value) {
                    self.run_dma(channel);
                }
                self.forward_dma_irq();
            }
            TIMERS..=TIMERS_END => self.timers.write(phys - TIMERS, width, value),
            CDROM..=CDROM_END => {
                self.cdrom.write(phys - CDROM, width, value);
                self.forward_cdrom_irq();
            }
            GPU..=GPU_END => {
                self.gpu.write(phys - GPU, width, value);
                self.forward_gpu_irq();
            }
            MDEC..=MDEC_END => self.mdec.write(phys - MDEC, width, value),
            SPU..=SPU_END => self.spu.write(phys - SPU, width, value),
            POST_REGISTER => self.post_code = value as u8,
            EXPANSION2..=EXPANSION2_END => {}
            _ => self.log_unhandled(phys, width, true, value),
        }
    }

    /// Start a DMA transfer if the channel is ready, and signal completion.
    fn run_dma(&mut self, channel: usize) {
        if !self.dma.should_run(channel) {
            return;
        }
        if channel == CHANNEL_MDEC_OUT
            && self.mdec.output_words() < self.dma.transfer_words(channel) as usize
        {
            // The decoder has not produced the data yet: wait for it.
            self.mdec_out_waiting = true;
            return;
        }
        self.dma.run_channel(
            channel,
            &mut self.memory.ram,
            &mut self.gpu,
            &mut self.spu,
            &mut self.cdrom,
            &mut self.mdec,
        );
        if channel == CHANNEL_MDEC_IN && self.mdec_out_waiting {
            self.mdec_out_waiting = false;
            self.run_dma(CHANNEL_MDEC_OUT);
            self.forward_dma_irq();
        }
    }

    fn forward_gpu_irq(&mut self) {
        if self.gpu.take_irq() {
            self.interrupt.raise(IRQ_GPU);
        }
    }

    fn forward_sio_irq(&mut self) {
        if self.sio0.take_irq() {
            self.interrupt.raise(IRQ_CONTROLLER);
        }
    }

    fn forward_cdrom_irq(&mut self) {
        if self.cdrom.take_irq() {
            self.interrupt.raise(IRQ_CDROM);
        }
    }

    fn forward_dma_irq(&mut self) {
        if self.dma.take_irq() {
            self.interrupt.raise(IRQ_DMA);
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
    fn gpu_status_reports_ready_and_tracks_commands() {
        let mut bus = Bus::new();
        let stat = bus.read32(0x1F80_1814).unwrap();
        assert_eq!(stat & 0x1C00_0000, 0x1C00_0000);
        assert_ne!(stat & (1 << 23), 0, "display starts disabled");

        bus.write32(0x1F80_1814, 0x0300_0000).unwrap(); // GP1: display on
        bus.write32(0x1F80_1810, 0xE100_0009).unwrap(); // GP0: texture page x = 9
        let stat = bus.read32(0x1F80_1814).unwrap();
        assert_eq!(stat & (1 << 23), 0);
        assert_eq!(stat & 0xF, 9);
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
    fn serial_port_1_registers_just_store_values() {
        let mut bus = Bus::new();
        bus.write16(0x1F80_105A, 0x1234).unwrap();
        assert_eq!(bus.read16(0x1F80_105A).unwrap(), 0x1234);
        assert_eq!(bus.read16(0x1F80_104A).unwrap(), 0, "SIO0 control is separate");
    }

    #[test]
    fn a_pad_poll_through_the_bus_raises_irq7() {
        let mut bus = Bus::new();
        bus.write16(0x1F80_1048, 0x000D).unwrap(); // MODE
        bus.write16(0x1F80_104E, 0x0088).unwrap(); // BAUD
        bus.write16(0x1F80_104A, 0x1003).unwrap(); // CTRL
        bus.write8(0x1F80_1040, 0x01).unwrap();

        for _ in 0..2000 {
            bus.step_devices();
        }
        assert_eq!(bus.read8(0x1F80_1040).unwrap(), 0xFF);
        assert_ne!(bus.interrupt.stat & (1 << IRQ_CONTROLLER), 0, "IRQ7 pending");
        assert_ne!(bus.read32(0x1F80_1044).unwrap() & (1 << 9), 0, "JOY_STAT IRQ flag");
    }

    #[test]
    fn bus_reset_releases_the_serial_port() {
        let mut bus = Bus::new();
        bus.write16(0x1F80_104A, 0x1003).unwrap();
        bus.write8(0x1F80_1040, 0x01).unwrap();
        bus.reset();
        assert_eq!(bus.read16(0x1F80_104A).unwrap(), 0);
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

    // ----- DMA transfers -----

    const DMA_BASE: u32 = 0x1F80_1080;

    fn dma_reg(channel: u32, reg: u32) -> u32 {
        DMA_BASE + channel * 0x10 + reg * 4
    }

    fn enable_dma_channel(bus: &mut Bus, channel: u32) {
        let dpcr = bus.read32(0x1F80_10F0).unwrap();
        bus.write32(0x1F80_10F0, dpcr | (1 << (channel * 4 + 3))).unwrap();
    }

    #[test]
    fn otc_dma_builds_an_ordering_table() {
        let mut bus = Bus::new();
        enable_dma_channel(&mut bus, 6);
        bus.write32(dma_reg(6, 0), 0x1000).unwrap(); // MADR: last entry
        bus.write32(dma_reg(6, 1), 4).unwrap(); // 4 entries
        bus.write32(dma_reg(6, 2), 0x1100_0002).unwrap(); // start + trigger, backward

        assert_eq!(bus.read32(0x1000).unwrap(), 0x0FFC);
        assert_eq!(bus.read32(0x0FFC).unwrap(), 0x0FF8);
        assert_eq!(bus.read32(0x0FF8).unwrap(), 0x0FF4);
        assert_eq!(bus.read32(0x0FF4).unwrap(), 0x00FF_FFFF, "end marker");
        assert_eq!(bus.read32(dma_reg(6, 2)).unwrap() & (1 << 24), 0, "no longer busy");
        assert_eq!(bus.read32(dma_reg(6, 2)).unwrap() & (1 << 28), 0, "trigger consumed");
        assert_eq!(bus.read32(dma_reg(6, 0)).unwrap(), 0x0FF0, "MADR after the last entry");
    }

    #[test]
    fn dma_does_nothing_until_the_channel_is_enabled() {
        let mut bus = Bus::new();
        bus.write32(dma_reg(6, 0), 0x1000).unwrap();
        bus.write32(dma_reg(6, 1), 4).unwrap();
        bus.write32(dma_reg(6, 2), 0x1100_0002).unwrap();
        assert_eq!(bus.read32(0x1000).unwrap(), 0);
        assert_ne!(bus.read32(dma_reg(6, 2)).unwrap() & (1 << 24), 0, "still busy");

        // Enabling afterwards does not retroactively start it, but a new write does.
        enable_dma_channel(&mut bus, 6);
        bus.write32(dma_reg(6, 2), 0x1100_0002).unwrap();
        assert_eq!(bus.read32(0x1000).unwrap(), 0x0FFC);
    }

    #[test]
    fn manual_sync_dma_waits_for_the_trigger_bit() {
        let mut bus = Bus::new();
        enable_dma_channel(&mut bus, 6);
        bus.write32(dma_reg(6, 0), 0x1000).unwrap();
        bus.write32(dma_reg(6, 1), 2).unwrap();
        bus.write32(dma_reg(6, 2), 0x0100_0002).unwrap(); // busy, no trigger
        assert_eq!(bus.read32(0x1000).unwrap(), 0);
        bus.write32(dma_reg(6, 2), 0x1100_0002).unwrap();
        assert_eq!(bus.read32(0x1000).unwrap(), 0x0FFC);
    }

    #[test]
    fn gpu_dma_block_mode_uploads_pixels_to_vram() {
        let mut bus = Bus::new();
        enable_dma_channel(&mut bus, 2);
        // A CPU-to-VRAM upload of a 4x2 block, plus a trailing NOP: 8 words.
        let words = [0xA000_0000u32, 0x0000_0010, 0x0002_0004, 0x0002_0001, 0x0004_0003, 0x0006_0005, 0x0008_0007, 0];
        for (i, word) in words.iter().enumerate() {
            bus.write32(0x2000 + i as u32 * 4, *word).unwrap();
        }
        bus.write32(dma_reg(2, 0), 0x2000).unwrap();
        bus.write32(dma_reg(2, 1), (2 << 16) | 4).unwrap(); // 2 blocks of 4 words
        bus.write32(dma_reg(2, 2), 0x0100_0201).unwrap(); // request sync, from RAM
        assert_eq!(bus.gpu.gp0_word_count, 8);
        for i in 0..4 {
            assert_eq!(bus.gpu.vram.get(16 + i, 0), 1 + i as u16, "row 0, pixel {i}");
            assert_eq!(bus.gpu.vram.get(16 + i, 1), 5 + i as u16, "row 1, pixel {i}");
        }
        assert_eq!(bus.read32(dma_reg(2, 2)).unwrap() & (1 << 24), 0);
        assert_eq!(bus.read32(dma_reg(2, 0)).unwrap(), 0x2020, "MADR advanced by 8 words");
        assert_eq!(bus.read32(dma_reg(2, 1)).unwrap() >> 16, 0, "block counter ran down");
    }


    #[test]
    fn gpu_dma_linked_list_follows_the_chain() {
        let mut bus = Bus::new();
        enable_dma_channel(&mut bus, 2);
        // A VRAM fill command split across two nodes: the GPU sees one word stream.
        // Node A: 2 words (command, position), next = 0x2100.
        bus.write32(0x2000, (2 << 24) | 0x2100).unwrap();
        bus.write32(0x2004, 0x0200_00FF).unwrap(); // fill, red
        bus.write32(0x2008, 0x0000_0010).unwrap(); // x = 16, y = 0
        // Node B: 1 word (size), end of list.
        bus.write32(0x2100, (1 << 24) | 0x00FF_FFFF).unwrap();
        bus.write32(0x2104, 0x0001_0010).unwrap(); // 16 x 1

        bus.write32(dma_reg(2, 0), 0x2000).unwrap();
        bus.write32(dma_reg(2, 2), 0x0100_0401).unwrap(); // linked-list sync, from RAM
        assert_eq!(bus.gpu.gp0_word_count, 3);
        assert_eq!(bus.gpu.vram.get(16, 0), 0x001F, "red in 15-bit");
        assert_eq!(bus.gpu.vram.get(31, 0), 0x001F);
        assert_eq!(bus.gpu.vram.get(32, 0), 0);
        assert_eq!(bus.read32(dma_reg(2, 2)).unwrap() & (1 << 24), 0);
    }


    #[test]
    fn gpu_dma_linked_list_survives_a_cycle() {
        let mut bus = Bus::new();
        enable_dma_channel(&mut bus, 2);
        bus.write32(0x2000, 0x2000).unwrap(); // empty node pointing at itself
        bus.write32(dma_reg(2, 0), 0x2000).unwrap();
        bus.write32(dma_reg(2, 2), 0x0100_0401).unwrap(); // must terminate
        assert_eq!(bus.read32(dma_reg(2, 2)).unwrap() & (1 << 24), 0);
    }

    #[test]
    fn gpu_dma_to_ram_downloads_vram() {
        let mut bus = Bus::new();
        enable_dma_channel(&mut bus, 2);
        bus.gpu.vram.set(8, 3, 0x1234);
        bus.gpu.vram.set(9, 3, 0x5678);
        bus.write32(0x3000, 0xFFFF_FFFF).unwrap();

        // GP0: VRAM-to-CPU transfer of a 2x1 block at (8, 3).
        bus.write32(0x1F80_1810, 0xC000_0000).unwrap();
        bus.write32(0x1F80_1810, 0x0003_0008).unwrap();
        bus.write32(0x1F80_1810, 0x0001_0002).unwrap();

        bus.write32(dma_reg(2, 0), 0x3000).unwrap();
        bus.write32(dma_reg(2, 1), (1 << 16) | 1).unwrap();
        bus.write32(dma_reg(2, 2), 0x0100_0200).unwrap(); // request sync, to RAM
        assert_eq!(bus.read32(0x3000).unwrap(), 0x5678_1234);
    }


    /// Queue a one-block 8-bit decode (DC only) at 0x2000 for MDEC input.
    fn stage_mdec_stream(bus: &mut Bus) {
        bus.write32(0x2000, 0x2800_0001).unwrap(); // decode, 8-bit, 1 word
        bus.write32(0x2004, 0xFE00_0020).unwrap(); // DC word + end of block
    }

    fn start_mdec_input(bus: &mut Bus) {
        bus.write32(dma_reg(0, 0), 0x2000).unwrap();
        bus.write32(dma_reg(0, 1), (1 << 16) | 2).unwrap();
        bus.write32(dma_reg(0, 2), 0x0100_0201).unwrap();
    }

    fn start_mdec_output(bus: &mut Bus) {
        bus.write32(dma_reg(1, 0), 0x3000).unwrap();
        bus.write32(dma_reg(1, 1), (1 << 16) | 16).unwrap();
        bus.write32(dma_reg(1, 2), 0x0100_0200).unwrap();
    }

    #[test]
    fn mdec_dma_decodes_input_and_returns_pixels() {
        let mut bus = Bus::new();
        enable_dma_channel(&mut bus, 0);
        enable_dma_channel(&mut bus, 1);
        stage_mdec_stream(&mut bus);
        start_mdec_input(&mut bus);
        assert_eq!(bus.mdec.output_words(), 16);
        start_mdec_output(&mut bus);
        for i in 0..16 {
            assert_eq!(bus.read32(0x3000 + i * 4).unwrap(), 0x8080_8080, "word {i}");
        }
        assert_eq!(bus.read32(dma_reg(1, 2)).unwrap() & (1 << 24), 0);
        assert_eq!(bus.read32(0x1F80_1824).unwrap() & (1 << 31), 1 << 31, "output drained");
    }

    #[test]
    fn mdec_output_dma_started_early_waits_for_the_decoder() {
        let mut bus = Bus::new();
        enable_dma_channel(&mut bus, 0);
        enable_dma_channel(&mut bus, 1);
        stage_mdec_stream(&mut bus);
        start_mdec_output(&mut bus);
        assert_ne!(bus.read32(dma_reg(1, 2)).unwrap() & (1 << 24), 0, "still busy");
        start_mdec_input(&mut bus);
        assert_eq!(bus.read32(dma_reg(1, 2)).unwrap() & (1 << 24), 0, "finished");
        assert_eq!(bus.read32(0x3000).unwrap(), 0x8080_8080);
    }

    #[test]
    fn spu_dma_copies_ram_into_sound_ram() {
        let mut bus = Bus::new();
        enable_dma_channel(&mut bus, 4);
        for i in 0..4u32 {
            bus.write32(0x4000 + i * 4, 0x0403_0201 + i * 0x0404_0404).unwrap();
        }
        bus.write16(0x1F80_1DA6, 0x100).unwrap(); // transfer address = 0x100 * 8
        bus.write32(dma_reg(4, 0), 0x4000).unwrap();
        bus.write32(dma_reg(4, 1), (1 << 16) | 4).unwrap();
        bus.write32(dma_reg(4, 2), 0x0100_0201).unwrap();
        assert_eq!(&bus.spu.sound_ram[0x800..0x808], &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn spu_dma_from_sound_ram_reads_back() {
        let mut bus = Bus::new();
        enable_dma_channel(&mut bus, 4);
        bus.spu.sound_ram[0x800..0x804].copy_from_slice(&[0xDD, 0xCC, 0xBB, 0xAA]);
        bus.write16(0x1F80_1DA6, 0x100).unwrap();
        bus.write32(dma_reg(4, 0), 0x4000).unwrap();
        bus.write32(dma_reg(4, 1), (1 << 16) | 1).unwrap();
        bus.write32(dma_reg(4, 2), 0x0100_0200).unwrap();
        assert_eq!(bus.read32(0x4000).unwrap(), 0xAABB_CCDD);
    }

    #[test]
    fn spu_manual_fifo_writes_advance_the_transfer_address() {
        let mut bus = Bus::new();
        bus.write16(0x1F80_1DA6, 0x10).unwrap();
        bus.write16(0x1F80_1DA8, 0x1234).unwrap();
        bus.write16(0x1F80_1DA8, 0x5678).unwrap();
        assert_eq!(&bus.spu.sound_ram[0x80..0x84], &[0x34, 0x12, 0x78, 0x56]);
    }

    #[test]
    fn dma_completion_sets_the_flag_and_raises_the_interrupt() {
        let mut bus = Bus::new();
        enable_dma_channel(&mut bus, 6);
        // Master enable + enable for channel 6 (bit 16 + 6).
        bus.write32(0x1F80_10F4, (1 << 23) | (1 << 22)).unwrap();
        bus.write32(dma_reg(6, 0), 0x1000).unwrap();
        bus.write32(dma_reg(6, 1), 4).unwrap();
        bus.write32(dma_reg(6, 2), 0x1100_0002).unwrap();

        let dicr = bus.read32(0x1F80_10F4).unwrap();
        assert_ne!(dicr & (1 << 30), 0, "channel 6 flag");
        assert_ne!(dicr & (1 << 31), 0, "master flag");
        assert_eq!(bus.interrupt.stat & (1 << IRQ_DMA), 1 << IRQ_DMA);
    }

    #[test]
    fn dma_completion_is_silent_when_the_channel_interrupt_is_disabled() {
        let mut bus = Bus::new();
        enable_dma_channel(&mut bus, 6);
        bus.write32(0x1F80_10F4, 1 << 23).unwrap(); // master enable only
        bus.write32(dma_reg(6, 0), 0x1000).unwrap();
        bus.write32(dma_reg(6, 1), 4).unwrap();
        bus.write32(dma_reg(6, 2), 0x1100_0002).unwrap();
        assert_eq!(bus.read32(0x1F80_10F4).unwrap() & 0xFF00_0000, 0);
        assert_eq!(bus.interrupt.stat, 0);
    }

    #[test]
    fn forcing_the_dma_interrupt_raises_it() {
        let mut bus = Bus::new();
        bus.write32(0x1F80_10F4, 1 << 15).unwrap();
        assert_eq!(bus.interrupt.stat & (1 << IRQ_DMA), 1 << IRQ_DMA);
    }
}
