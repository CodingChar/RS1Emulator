use crate::io::StubRegs;

/// CD-ROM drive emulation.
///
/// Parses ISO / BIN+CUE / PBP / CCD image formats and emulates the
/// CD-ROM controller (interrupts, DMA transfers, sector reads).
///
/// Mirrors DuckStation's `cdrom.cpp` / `cdrom_async_reader.cpp`.
pub struct CdRom {
    pub image: Option<Vec<u8>>,
    pub sector_size: u32,
    pub current_sector: u32,
    pub total_sectors: u32,
    /// The four registers at `0x1F801800`. Plain storage until the
    /// controller is emulated.
    registers: StubRegs,
}

/// Index/status register value with the parameter FIFO empty and not full.
const STATUS_IDLE: u32 = 0x18;

impl CdRom {
    pub fn new() -> Self {
        Self {
            image: None,
            sector_size: 2048,
            current_sector: 0,
            total_sectors: 0,
            registers: Self::idle_registers(),
        }
    }

    fn idle_registers() -> StubRegs {
        StubRegs::new(4).with_u32(0, STATUS_IDLE)
    }

    /// Register read; `off` is relative to `0x1F801800`.
    pub fn read(&mut self, off: u32, width: u32) -> u32 {
        // TODO: index/status, response FIFO, interrupt flags.
        self.registers.read(off, width)
    }

    /// Register write; `off` is relative to `0x1F801800`.
    pub fn write(&mut self, off: u32, width: u32, value: u32) {
        // TODO: command, parameter FIFO, interrupt enable/acknowledge.
        self.registers.write(off, width, value);
    }

    pub fn insert_image(&mut self, data: Vec<u8>) -> crate::types::Ps1Result<()> {
        // TODO: parse ISO/BIN+CUE/PBP headers, determine sector size and count.
        self.image = Some(data);
        self.total_sectors = 0;
        self.current_sector = 0;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.current_sector = 0;
        self.total_sectors = 0;
        self.registers = Self::idle_registers();
    }

    /// DMA: read one word of sector data.
    pub fn dma_read_word(&mut self) -> u32 {
        // TODO: pop the data FIFO once sector reads exist.
        0
    }

    pub fn step(&mut self) {
        // TODO: process CD-ROM command queue, DMA sector transfers.
    }

    pub fn read_sector(&mut self, _sector: u32) -> crate::types::Ps1Result<Vec<u8>> {
        match &self.image {
            Some(_img) => {
                // TODO: read sector from image
                Ok(vec![0u8; self.sector_size as usize])
            }
            None => Err(crate::types::Ps1Error::NoDiscInserted),
        }
    }
}

impl Default for CdRom {
    fn default() -> Self {
        Self::new()
    }
}
