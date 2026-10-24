use crate::io::extract;

pub mod sw;
pub mod backend;

/// GPU framebuffer dimensions (native PS1 resolution).
pub const FRAMEBUFFER_WIDTH: u32 = 320;
pub const FRAMEBUFFER_HEIGHT: u32 = 240;

/// `GPUSTAT` while the GPU is idle: ready to receive commands (bit 26), to
/// send VRAM to the CPU (bit 27) and to receive a DMA block (bit 28).
const GPUSTAT_IDLE: u32 = 0x1C00_0000;

/// Software-rendered GPU state.
#[derive(Clone, Debug)]
pub struct Gpu {
    /// Framebuffer in RGBA8888 format.
    pub framebuffer: Vec<u8>,
    /// GPU control registers (mirrored from hardware).
    ///
    /// For now: `[0]` = last GP0 word, `[1]` = last GP1 word, `[2]` = GPUREAD.
    pub regs: [u32; 16],
}

impl Gpu {
    pub fn new() -> Self {
        Self {
            framebuffer: vec![0u8; (FRAMEBUFFER_WIDTH * FRAMEBUFFER_HEIGHT * 4) as usize],
            regs: [0u32; 16],
        }
    }

    pub fn reset(&mut self) {
        self.framebuffer.fill(0);
        self.regs = [0u32; 16];
    }

    /// Advance GPU by one cycle (stub — real rasterizer goes here).
    pub fn step(&mut self) {
        // TODO: implement GPU command parser, texture blits, drawing.
    }

    /// Register read; `off` is relative to `0x1F801810`
    /// (0 = GPUREAD, 4 = GPUSTAT).
    pub fn read(&mut self, off: u32, width: u32) -> u32 {
        let word = match off & !3 {
            0 => self.regs[2],
            4 => GPUSTAT_IDLE, // TODO: real status bits
            _ => 0,
        };
        extract(word, off, width)
    }

    /// Register write; `off` is relative to `0x1F801810`
    /// (0 = GP0 render/VRAM commands, 4 = GP1 display control).
    pub fn write(&mut self, off: u32, _width: u32, value: u32) {
        // TODO: GP0 command parser and GP1 display commands.
        match off & !3 {
            0 => self.regs[0] = value,
            4 => self.regs[1] = value,
            _ => {}
        }
    }

    /// Return a copy of the current framebuffer (RGBA8888).
    pub fn framebuffer(&self) -> Vec<u8> {
        self.framebuffer.clone()
    }
}

impl Default for Gpu {
    fn default() -> Self {
        Self::new()
    }
}
