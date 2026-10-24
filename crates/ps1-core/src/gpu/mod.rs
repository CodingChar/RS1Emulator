pub mod sw;
pub mod backend;

/// GPU framebuffer dimensions (native PS1 resolution).
pub const FRAMEBUFFER_WIDTH: u32 = 320;
pub const FRAMEBUFFER_HEIGHT: u32 = 240;

/// Software-rendered GPU state.
#[derive(Clone, Debug)]
pub struct Gpu {
    /// Framebuffer in RGBA8888 format.
    pub framebuffer: Vec<u8>,
    /// GPU control registers (mirrored from hardware).
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
