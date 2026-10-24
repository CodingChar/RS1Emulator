use crate::io::extract;

pub mod sw;
pub mod backend;

/// GPU framebuffer dimensions (native PS1 resolution).
pub const FRAMEBUFFER_WIDTH: u32 = 320;
pub const FRAMEBUFFER_HEIGHT: u32 = 240;

/// GPU clock cycles per scanline (NTSC).
pub const GPU_CLOCKS_PER_LINE: u32 = 3413;
/// Scanlines per frame (NTSC, non-interlaced).
pub const LINES_PER_FRAME: u32 = 263;
/// First scanline of the vertical blanking interval.
pub const VBLANK_START_LINE: u32 = 240;
/// The GPU runs at 53.693175 MHz, the CPU at 33.8688 MHz: 11 GPU clocks per
/// 7 CPU cycles.
const GPU_CLOCKS_PER_7_CPU_CYCLES: u32 = 11;
const CPU_CYCLES_PER_11_GPU_CLOCKS: u32 = 7;
/// GPU clocks per dot-clock tick at 320 pixels wide.
/// TODO: derive from the horizontal resolution set with GP1(08h).
const DOT_CLOCK_DIVIDER: u32 = 8;

/// CPU cycles in one NTSC frame.
pub const CPU_CYCLES_PER_FRAME: u64 = (LINES_PER_FRAME as u64
    * GPU_CLOCKS_PER_LINE as u64
    * CPU_CYCLES_PER_11_GPU_CLOCKS as u64)
    / GPU_CLOCKS_PER_7_CPU_CYCLES as u64;

/// `GPUSTAT` while the GPU is idle: ready to receive commands (bit 26), to
/// send VRAM to the CPU (bit 27) and to receive a DMA block (bit 28).
const GPUSTAT_IDLE: u32 = 0x1C00_0000;

/// What happened on screen while the GPU advanced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoTick {
    /// Dot-clock ticks elapsed (timer 0 clock source).
    pub dot_ticks: u32,
    /// Scanlines completed (timer 1 clock source).
    pub hblanks: u32,
    /// The vertical blanking interval started (IRQ0).
    pub vblank_started: bool,
}

/// Software-rendered GPU state.
#[derive(Clone, Debug)]
pub struct Gpu {
    /// Framebuffer in RGBA8888 format.
    pub framebuffer: Vec<u8>,
    /// GPU control registers (mirrored from hardware).
    ///
    /// For now: `[0]` = last GP0 word, `[1]` = last GP1 word, `[2]` = GPUREAD.
    pub regs: [u32; 16],
    /// Number of words received through GP0 (until commands are parsed).
    pub gp0_word_count: u64,

    /// Current scanline, `0..LINES_PER_FRAME`.
    pub scanline: u32,
    /// GPU clocks elapsed in the current scanline.
    line_clock: u32,
    /// Remainder of the CPU-to-GPU clock conversion (in 1/7 GPU clocks).
    clock_remainder: u32,
    /// Remainder of the GPU-to-dot clock division.
    dot_remainder: u32,
}

impl Gpu {
    pub fn new() -> Self {
        Self {
            framebuffer: vec![0u8; (FRAMEBUFFER_WIDTH * FRAMEBUFFER_HEIGHT * 4) as usize],
            regs: [0u32; 16],
            gp0_word_count: 0,
            scanline: 0,
            line_clock: 0,
            clock_remainder: 0,
            dot_remainder: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Whether the beam is in the vertical blanking interval.
    pub fn in_vblank(&self) -> bool {
        self.scanline >= VBLANK_START_LINE
    }

    /// Advance the video timing by `cpu_cycles` CPU cycles.
    pub fn advance(&mut self, cpu_cycles: u32) -> VideoTick {
        let mut tick = VideoTick::default();

        self.clock_remainder += cpu_cycles * GPU_CLOCKS_PER_7_CPU_CYCLES;
        let gpu_clocks = self.clock_remainder / CPU_CYCLES_PER_11_GPU_CLOCKS;
        self.clock_remainder %= CPU_CYCLES_PER_11_GPU_CLOCKS;

        self.dot_remainder += gpu_clocks;
        tick.dot_ticks = self.dot_remainder / DOT_CLOCK_DIVIDER;
        self.dot_remainder %= DOT_CLOCK_DIVIDER;

        self.line_clock += gpu_clocks;
        while self.line_clock >= GPU_CLOCKS_PER_LINE {
            self.line_clock -= GPU_CLOCKS_PER_LINE;
            self.scanline += 1;
            tick.hblanks += 1;
            if self.scanline == VBLANK_START_LINE {
                tick.vblank_started = true;
            }
            if self.scanline >= LINES_PER_FRAME {
                self.scanline = 0;
            }
        }
        tick
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
            0 => {
                self.regs[0] = value;
                self.gp0_word_count += 1;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Advance one CPU cycle at a time, summing the video events.
    fn run(gpu: &mut Gpu, cycles: u64) -> (u64, u64, u64) {
        let (mut dots, mut hblanks, mut vblanks) = (0u64, 0u64, 0u64);
        for _ in 0..cycles {
            let t = gpu.advance(1);
            dots += t.dot_ticks as u64;
            hblanks += t.hblanks as u64;
            vblanks += t.vblank_started as u64;
        }
        (dots, hblanks, vblanks)
    }

    #[test]
    fn a_frame_is_263_lines_and_one_vblank() {
        let mut gpu = Gpu::new();
        let (_, hblanks, vblanks) = run(&mut gpu, CPU_CYCLES_PER_FRAME);
        assert_eq!(vblanks, 1);
        assert!((262..=263).contains(&hblanks), "got {hblanks} scanlines");
    }

    #[test]
    fn vblank_starts_at_line_240() {
        let mut gpu = Gpu::new();
        let mut cycles = 0u64;
        while !gpu.advance(1).vblank_started {
            cycles += 1;
        }
        cycles += 1;
        // 240 lines * 3413 GPU clocks * 7/11 CPU cycles per GPU clock.
        assert!((521_258..=521_260).contains(&cycles), "got {cycles}");
        assert!(gpu.in_vblank());
    }

    #[test]
    fn video_timing_is_periodic() {
        let mut gpu = Gpu::new();
        // Three whole frames of GPU clocks, rounded up to whole CPU cycles
        // (a frame is not an integer number of CPU cycles).
        let gpu_clocks = 3 * LINES_PER_FRAME as u64 * GPU_CLOCKS_PER_LINE as u64;
        let cycles = (gpu_clocks * 7).div_ceil(11);
        let (_, hblanks, vblanks) = run(&mut gpu, cycles);
        assert_eq!(vblanks, 3);
        assert_eq!(hblanks, 3 * LINES_PER_FRAME as u64);
        assert_eq!(gpu.scanline, 0);
    }

    #[test]
    fn dot_clock_is_one_eighth_of_the_gpu_clock_at_320_pixels() {
        let mut gpu = Gpu::new();
        let (dots, _, _) = run(&mut gpu, 7 * 8 * 1000);
        // 7 CPU cycles = 11 GPU clocks; 8 GPU clocks = 1 dot.
        assert_eq!(dots, 11 * 1000);
    }

    #[test]
    fn advancing_in_big_steps_matches_single_steps() {
        let mut a = Gpu::new();
        let mut b = Gpu::new();
        let mut sum = VideoTick::default();
        for _ in 0..1000 {
            let t = a.advance(1000);
            sum.dot_ticks += t.dot_ticks;
            sum.hblanks += t.hblanks;
            sum.vblank_started |= t.vblank_started;
        }
        let (dots, hblanks, _) = run(&mut b, 1_000_000);
        assert_eq!(sum.dot_ticks as u64, dots);
        assert_eq!(sum.hblanks as u64, hblanks);
        assert!(sum.vblank_started);
    }
}
