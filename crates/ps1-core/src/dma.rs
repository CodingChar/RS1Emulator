use crate::cdrom::CdRom;
use crate::gpu::Gpu;
use crate::io::{extract, merge};
use crate::memory::ram::Ram;
use crate::spu::Spu;

// Channels.
pub const CHANNEL_MDEC_IN: usize = 0;
pub const CHANNEL_MDEC_OUT: usize = 1;
pub const CHANNEL_GPU: usize = 2;
pub const CHANNEL_CDROM: usize = 3;
pub const CHANNEL_SPU: usize = 4;
pub const CHANNEL_PIO: usize = 5;
pub const CHANNEL_OTC: usize = 6;

/// Index of the DMA control register (`0x1F8010F0`) in `registers`.
const DPCR: usize = 28;
/// Index of the DMA interrupt register (`0x1F8010F4`) in `registers`.
const DICR: usize = 29;
/// Power-on value of DPCR: channel priorities 0..6 in order.
const DPCR_RESET: u32 = 0x0765_4321;

const DICR_FORCE: u32 = 1 << 15;
const DICR_MASTER_ENABLE: u32 = 1 << 23;
const DICR_WRITABLE: u32 = 0x00FF_FFFF;
const DICR_FLAGS: u32 = 0x7F00_0000;
const DICR_IRQ: u32 = 1 << 31;

// Channel control register bits.
const CHCR_FROM_RAM: u32 = 1 << 0;
const CHCR_STEP_BACKWARD: u32 = 1 << 1;
const CHCR_SYNC_SHIFT: u32 = 9;
const CHCR_BUSY: u32 = 1 << 24;
const CHCR_TRIGGER: u32 = 1 << 28;

const SYNC_MANUAL: u32 = 0;
const SYNC_LINKED_LIST: u32 = 2;

/// DMA addresses are word-aligned offsets into the 2 MB of RAM.
const RAM_ADDRESS_MASK: u32 = 0x001F_FFFC;
/// End-of-list marker in a linked list / ordering table.
const LIST_END: u32 = 0x00FF_FFFF;
/// Upper bound on linked-list nodes, so a corrupt (cyclic) list cannot hang
/// the emulator. RAM holds at most 512K words.
const MAX_LIST_NODES: u32 = 0x8_0000;

/// DMA controller.
///
/// Transfers data between main RAM and GPU/SPU/CD-ROM without CPU intervention.
/// Mirrors DuckStation's `dma.cpp` / `dma.h`.
///
/// Register file at `0x1F801080`: seven channels of `MADR`, `BCR`, `CHCR`
/// (16 bytes each), then `DPCR` and `DICR`.
///
/// Transfers complete instantly when they start, instead of stalling the CPU
/// for as long as the real hardware would.
pub struct Dma {
    /// One 32-bit word per register, indexed by `(address - 0x1F801080) / 4`.
    pub registers: [u32; 32],
    /// The DMA interrupt line rose; the bus forwards it to the interrupt
    /// controller (see [`Dma::take_irq`]).
    irq: bool,
}

impl Dma {
    pub fn new() -> Self {
        let mut dma = Self { registers: [0u32; 32], irq: false };
        dma.reset();
        dma
    }

    pub fn reset(&mut self) {
        self.registers = [0u32; 32];
        self.registers[DPCR] = DPCR_RESET;
        self.irq = false;
    }

    /// Whether the DMA interrupt fired since the last call.
    pub fn take_irq(&mut self) -> bool {
        std::mem::take(&mut self.irq)
    }

    fn madr(&self, channel: usize) -> u32 {
        self.registers[channel * 4]
    }

    fn bcr(&self, channel: usize) -> u32 {
        self.registers[channel * 4 + 1]
    }

    fn chcr(&self, channel: usize) -> u32 {
        self.registers[channel * 4 + 2]
    }

    /// Register read; `off` is relative to `0x1F801080`.
    pub fn read(&self, off: u32, width: u32) -> u32 {
        extract(self.registers[((off >> 2) & 31) as usize], off, width)
    }

    /// Register write; `off` is relative to `0x1F801080`.
    ///
    /// Returns the channel whose control register was written, so the caller
    /// can start the transfer if it is ready (see [`Dma::should_run`]).
    pub fn write(&mut self, off: u32, width: u32, value: u32) -> Option<usize> {
        let index = ((off >> 2) & 31) as usize;
        let old = self.registers[index];

        if index == DICR {
            // Bits 24-30 are interrupt flags: writing 1 clears them. Bytes
            // outside the access count as 0 so they acknowledge nothing.
            let written = merge(old & DICR_WRITABLE, off, width, value);
            let flags = (old & DICR_FLAGS) & !(written & DICR_FLAGS);
            self.set_dicr((written & DICR_WRITABLE) | flags);
            return None;
        }

        let new = merge(old, off, width, value);
        // MADR is a 24-bit address; every other register is stored whole.
        self.registers[index] = if index < DPCR && index % 4 == 0 {
            new & 0x00FF_FFFF
        } else {
            new
        };
        (index < DPCR && index % 4 == 2).then_some(index / 4)
    }

    /// Store DICR with a fresh master flag, remembering a rising IRQ edge.
    fn set_dicr(&mut self, dicr: u32) {
        let enabled = (dicr >> 16) & 0x7F;
        let flagged = (dicr >> 24) & 0x7F;
        let irq = dicr & DICR_FORCE != 0
            || (dicr & DICR_MASTER_ENABLE != 0 && enabled & flagged != 0);

        let was_high = self.registers[DICR] & DICR_IRQ != 0;
        self.registers[DICR] = (dicr & !DICR_IRQ) | if irq { DICR_IRQ } else { 0 };
        if irq && !was_high {
            self.irq = true;
        }
    }

    /// Whether the channel has been started and may transfer now: busy, enabled
    /// in DPCR, and (for manual sync) triggered.
    pub fn should_run(&self, channel: usize) -> bool {
        let chcr = self.chcr(channel);
        if chcr & CHCR_BUSY == 0 {
            return false;
        }
        if self.registers[DPCR] & (1 << (channel * 4 + 3)) == 0 {
            return false;
        }
        match (chcr >> CHCR_SYNC_SHIFT) & 3 {
            // Devices are assumed to always be ready to send/receive data.
            SYNC_MANUAL => chcr & CHCR_TRIGGER != 0,
            _ => true,
        }
    }

    /// Run a started channel to completion.
    pub fn run_channel(
        &mut self,
        channel: usize,
        ram: &mut Ram,
        gpu: &mut Gpu,
        spu: &mut Spu,
        cdrom: &mut CdRom,
    ) {
        let chcr = self.chcr(channel);
        let sync = (chcr >> CHCR_SYNC_SHIFT) & 3;
        let bcr = self.bcr(channel);

        let end_address = if channel == CHANNEL_OTC {
            Self::clear_ordering_table(self.madr(channel), bcr & 0xFFFF, ram)
        } else if channel == CHANNEL_GPU && sync == SYNC_LINKED_LIST {
            Self::walk_linked_list(self.madr(channel), ram, gpu)
        } else {
            let words = if sync == SYNC_MANUAL {
                Self::block_length(bcr & 0xFFFF)
            } else {
                Self::block_length(bcr & 0xFFFF) * (bcr >> 16)
            };
            let step: u32 = if chcr & CHCR_STEP_BACKWARD != 0 { 4u32.wrapping_neg() } else { 4 };
            let from_ram = chcr & CHCR_FROM_RAM != 0;

            let mut address = self.madr(channel) & RAM_ADDRESS_MASK;
            for _ in 0..words {
                if from_ram {
                    let word = ram.read32(address as usize);
                    Self::write_device(channel, word, gpu, spu);
                } else {
                    let word = Self::read_device(channel, gpu, spu, cdrom);
                    ram.write32(address as usize, word);
                }
                address = address.wrapping_add(step) & RAM_ADDRESS_MASK;
            }
            if sync != SYNC_MANUAL {
                // The block counter counts down to zero.
                self.registers[channel * 4 + 1] = bcr & 0xFFFF;
            }
            address
        };

        self.registers[channel * 4] = end_address & 0x00FF_FFFF;
        self.registers[channel * 4 + 2] = chcr & !(CHCR_BUSY | CHCR_TRIGGER);
        self.signal_completion(channel);
    }

    /// A zero block length means 0x10000 words.
    fn block_length(n: u32) -> u32 {
        if n == 0 { 0x1_0000 } else { n }
    }

    /// Channel 6 builds an ordering table: each entry points at the previous
    /// word and the first one is the end marker. Returns the final address.
    fn clear_ordering_table(madr: u32, count: u32, ram: &mut Ram) -> u32 {
        let count = Self::block_length(count);
        let mut address = madr & RAM_ADDRESS_MASK;
        for i in 0..count {
            let entry = if i == count - 1 {
                LIST_END
            } else {
                address.wrapping_sub(4) & RAM_ADDRESS_MASK
            };
            ram.write32(address as usize, entry);
            address = address.wrapping_sub(4) & RAM_ADDRESS_MASK;
        }
        address
    }

    /// Feed a GPU command list to GP0. Each node is a header word (word count
    /// in the top byte, next node in the low 24 bits) followed by that many
    /// command words. Returns the address of the last header's successor.
    fn walk_linked_list(madr: u32, ram: &mut Ram, gpu: &mut Gpu) -> u32 {
        let mut node = madr & RAM_ADDRESS_MASK;
        for _ in 0..MAX_LIST_NODES {
            let header = ram.read32(node as usize);
            for i in 1..=(header >> 24) {
                let word = ram.read32((node.wrapping_add(i * 4) & RAM_ADDRESS_MASK) as usize);
                gpu.write(0, 4, word);
            }
            let next = header & 0x00FF_FFFF;
            // Bit 23 set means end of list (normally 0xFFFFFF).
            if next & 0x0080_0000 != 0 {
                return LIST_END;
            }
            node = next & RAM_ADDRESS_MASK;
        }
        node
    }

    fn write_device(channel: usize, word: u32, gpu: &mut Gpu, spu: &mut Spu) {
        match channel {
            CHANNEL_GPU => gpu.write(0, 4, word),
            CHANNEL_SPU => spu.dma_write_word(word),
            // TODO: MDEC input.
            CHANNEL_MDEC_IN | CHANNEL_MDEC_OUT | CHANNEL_CDROM | CHANNEL_PIO => {}
            _ => {}
        }
    }

    fn read_device(channel: usize, gpu: &mut Gpu, spu: &mut Spu, cdrom: &mut CdRom) -> u32 {
        match channel {
            CHANNEL_GPU => gpu.read(0, 4),
            CHANNEL_CDROM => cdrom.dma_read_word(),
            CHANNEL_SPU => spu.dma_read_word(),
            // TODO: MDEC output.
            _ => 0,
        }
    }

    /// Flag the channel in DICR and raise the DMA interrupt if enabled.
    fn signal_completion(&mut self, channel: usize) {
        let dicr = self.registers[DICR];
        if dicr & (1 << (16 + channel)) != 0 {
            self.set_dicr(dicr | (1 << (24 + channel)));
        }
    }
}

impl Default for Dma {
    fn default() -> Self {
        Self::new()
    }
}
