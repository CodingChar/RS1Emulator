//! The graphics processor.
//!
//! The CPU talks to it through two registers at `0x1F801810`: `GP0` takes
//! drawing commands and VRAM transfers, `GP1` takes display control commands;
//! reading returns `GPUREAD` (VRAM data / info queries) and `GPUSTAT`.
//! Drawing happens immediately into a 1024x512 15-bit VRAM; the display is
//! produced from a window of that VRAM on demand.
//!
//! Mirrors DuckStation's `gpu.cpp`, `gpu_commands.cpp` and `gpu_sw.cpp`.

use serde::{Deserialize, Serialize};
use crate::io::extract;

pub mod backend;
pub mod hires;
mod lane;
pub mod sw;

use crate::pgxp::PreciseVertex;
use hires::{HVertex, HiRes};
pub use hires::{RenderSettings, TextureFilter};
use sw::{
    Attributes, DrawSettings, Texture, TextureDepth, TextureWindow, Vertex, Vram, VRAM_HEIGHT,
    VRAM_WIDTH,
};

/// The range of a screen coordinate: the GTE clamps what it computes to 11 bits.
const SCREEN_MIN: f32 = -1024.0;
const SCREEN_MAX: f32 = 1023.0;
/// Horizontal scale of the widescreen option: 4:3 content in a 16:9 picture.
const WIDESCREEN_SCALE: f32 = 0.75;

// ----- Video timing -----

/// GPU clocks per scanline and scanlines per frame.
const NTSC_CLOCKS_PER_LINE: u32 = 3413;
const NTSC_LINES_PER_FRAME: u32 = 263;
const PAL_CLOCKS_PER_LINE: u32 = 3406;
const PAL_LINES_PER_FRAME: u32 = 314;
/// First scanline of the vertical blanking interval.
pub const VBLANK_START_LINE: u32 = 240;
/// The GPU runs at 53.693175 MHz, the CPU at 33.8688 MHz: 11 GPU clocks per
/// 7 CPU cycles.
const GPU_CLOCKS_PER_7_CPU_CYCLES: u32 = 11;
const CPU_CYCLES_PER_11_GPU_CLOCKS: u32 = 7;

/// CPU cycles in one NTSC frame.
pub const CPU_CYCLES_PER_FRAME: u64 = (NTSC_LINES_PER_FRAME as u64
    * NTSC_CLOCKS_PER_LINE as u64
    * CPU_CYCLES_PER_11_GPU_CLOCKS as u64)
    / GPU_CLOCKS_PER_7_CPU_CYCLES as u64;

/// When the video clocks that drive timers 0 and 1 will pulse next, as lower
/// bounds in CPU cycles.
#[derive(Clone, Copy, Debug)]
pub struct VideoTiming {
    /// Until the next dot clock pulse.
    pub next_dot: u32,
    /// Between two dot clock pulses.
    pub dot_period: u32,
    /// Until the next horizontal blank.
    pub next_hblank: u32,
    /// Between two horizontal blanks.
    pub hblank_period: u32,
    /// Until the horizontal or vertical blanking interval next starts or ends.
    pub next_blank_edge: u32,
}

/// What happened on screen while the GPU advanced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoTick {
    /// Dot-clock ticks elapsed (timer 0 clock source).
    pub dot_ticks: u32,
    /// Scanlines completed (timer 1 clock source).
    pub hblanks: u32,
    /// The vertical blanking interval started (IRQ0).
    pub vblank_started: bool,
    /// Whether the beam is now in the horizontal / vertical blanking interval
    /// (the gate of the timers' sync modes).
    pub in_hblank: bool,
    pub in_vblank: bool,
    /// A blanking interval started during this advance.
    pub hblank_started: bool,
}

// ----- GPUSTAT bits that are always the same -----
const STAT_READY_FOR_COMMAND: u32 = 1 << 26;
const STAT_READY_FOR_VRAM_READ: u32 = 1 << 27;
const STAT_READY_FOR_DMA: u32 = 1 << 28;

/// The most GPU clocks of drawing work that can be queued (about a frame).
const MAX_BUSY_CLOCKS: u32 = 1 << 20;

/// GPU clocks a drawing command keeps the GPU busy, an estimate: the real
/// pipeline draws about a pixel per clock, textured and blended pixels cost more,
/// and every command has a setup cost.
mod cost {
    const SETUP: u32 = 16;

    pub fn fill(width: u32, height: u32) -> u32 {
        // Fills work in 8-pixel units per line.
        height.min(512) * (width.min(1024) / 8 + 9)
    }

    pub fn triangle(area2: u64, textured: bool, semi: bool) -> u32 {
        let pixels = (area2 / 2).min(1024 * 512) as u32;
        let per_pixel = 1 + textured as u32;
        SETUP + 8 + pixels * per_pixel + if semi { pixels / 2 } else { 0 }
    }

    pub fn rectangle(width: u32, height: u32, textured: bool, semi: bool) -> u32 {
        let pixels = width.min(1024) * height.min(512);
        SETUP + pixels * (1 + textured as u32) + if semi { pixels / 2 } else { 0 }
    }

    pub fn line(dx: u32, dy: u32, semi: bool) -> u32 {
        let length = dx.max(dy).min(2048);
        SETUP + length * 2 + if semi { length / 2 } else { 0 }
    }

    pub fn copy(width: u32, height: u32) -> u32 {
        SETUP + width.min(1024) * height.min(512) * 2
    }
}

/// What the GPU version query (GP1 10h, index 7) returns.
const GPU_VERSION: u32 = 2;

/// Where GP0 is in the middle of a multi-word command or transfer.
#[derive(Clone, Debug, Serialize, Deserialize)]
enum Gp0State {
    /// Waiting for a command word.
    Command,
    /// Collecting the parameter words of a command (the longest takes 12).
    Parameters { words: [u32; 12], len: usize, needed: usize },
    /// Collecting vertices of a polyline until the terminator.
    Polyline { words: Vec<u32> },
    /// Receiving pixel data for a CPU-to-VRAM upload; `col` and `row` are the
    /// position of the next pixel inside the rectangle.
    Upload { x: usize, y: usize, width: usize, height: usize, col: usize, row: usize },
}

/// A VRAM-to-CPU download in progress.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Download {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    index: usize,
}

#[derive(Serialize, Deserialize)]
pub struct Gpu {
    pub vram: Vram,

    // Drawing state (GP0 E1h-E6h).
    settings: DrawSettings,
    page_x: u32,
    page_y: u32,
    semi_mode: u8,
    depth_bits: u32,
    draw_to_display: bool,
    texture_disable: bool,
    rect_flip_x: bool,
    rect_flip_y: bool,
    /// Raw values, for the GP1(10h) info queries.
    window_raw: u32,
    area_top_left_raw: u32,
    area_bottom_right_raw: u32,
    offset_raw: u32,

    // Display state (GP1).
    display_enabled: bool,
    dma_direction: u8,
    display_start: (u32, u32),
    horizontal_range: (u32, u32),
    vertical_range: (u32, u32),
    /// GP1(08h) display mode bits.
    display_mode: u32,
    irq1: bool,
    irq_edge: bool,
    /// Which field of an interlaced frame is being displayed.
    field: bool,

    // Command processing.
    gp0: Gp0State,
    download: Option<Download>,
    gpuread: u32,
    /// Number of words received through GP0, for debugging.
    pub gp0_word_count: u64,

    // Rendering beyond the console's resolution (not part of the machine, so
    // not saved: the frontend sets it again after a state is loaded).
    #[serde(skip)]
    render: RenderSettings,
    #[serde(skip)]
    hires: Option<Box<HiRes>>,
    /// PGXP: the precise vertex of each word of the command being collected.
    #[serde(skip)]
    precise: [PreciseVertex; 12],
    /// The precise vertex that comes with the word being written.
    #[serde(skip)]
    incoming: PreciseVertex,

    // Video timing.
    /// Current scanline.
    pub scanline: u32,
    line_clock: u32,
    clock_remainder: u32,
    /// GPU clocks of queued drawing work still to go (see `cost`).
    busy_clocks: u32,
    dot_remainder: u32,
}

impl Gpu {
    pub fn new() -> Self {
        let mut gpu = Self {
            vram: Vram::new(),
            settings: DrawSettings::default(),
            page_x: 0,
            page_y: 0,
            semi_mode: 0,
            depth_bits: 0,
            draw_to_display: false,
            texture_disable: false,
            rect_flip_x: false,
            rect_flip_y: false,
            window_raw: 0,
            area_top_left_raw: 0,
            area_bottom_right_raw: 0,
            offset_raw: 0,
            display_enabled: false,
            dma_direction: 0,
            display_start: (0, 0),
            horizontal_range: (0x200, 0x200 + 256 * 10),
            vertical_range: (0x10, 0x10 + 240),
            display_mode: 0,
            irq1: false,
            irq_edge: false,
            field: false,
            gp0: Gp0State::Command,
            download: None,
            gpuread: 0,
            gp0_word_count: 0,
            render: RenderSettings::default(),
            hires: None,
            precise: [PreciseVertex::NONE; 12],
            incoming: PreciseVertex::NONE,
            scanline: 0,
            line_clock: 0,
            clock_remainder: 0,
            busy_clocks: 0,
            dot_remainder: 0,
        };
        gpu.reset_state();
        gpu
    }

    /// Power-on reset: clears VRAM and the timing state.
    pub fn reset(&mut self) {
        let render = self.render;
        *self = Self::new();
        self.set_render_settings(render);
    }

    // ----- Rendering options -----

    pub fn render_settings(&self) -> RenderSettings {
        self.render
    }

    /// Choose how the picture is drawn (see [`RenderSettings`]). Changing the
    /// internal resolution rebuilds the scaled VRAM from the console's VRAM.
    pub fn set_render_settings(&mut self, settings: RenderSettings) {
        let settings = settings.sanitized();
        self.render = settings;
        if !settings.needs_hires() {
            self.hires = None;
            return;
        }
        match self.hires.as_deref_mut() {
            Some(hires) if hires.scale() == settings.scale as usize => hires.configure(&settings),
            _ => self.hires = Some(Box::new(HiRes::from_vram(&self.vram, &settings))),
        }
    }

    /// The scaled VRAM (`0x00BBGGRR`, the mask bit in bit 24) and its width in
    /// pixels, while scaled rendering is on.
    pub fn scaled_vram(&self) -> Option<(&[u32], usize)> {
        self.hires.as_deref().map(|hires| (hires.pixels.as_slice(), VRAM_WIDTH * hires.scale()))
    }

    /// Write a GP0 word that comes from RAM together with the PGXP vertex the
    /// GTE made for it (`NONE` when there is none).
    pub fn write_gp0_precise(&mut self, word: u32, vertex: PreciseVertex) {
        self.incoming = vertex;
        self.write_gp0(word);
        self.incoming = PreciseVertex::NONE;
    }

    /// GP1(00h): reset the GPU's registers (VRAM and timing are kept).
    fn reset_state(&mut self) {
        self.settings = DrawSettings::default();
        self.page_x = 0;
        self.page_y = 0;
        self.semi_mode = 0;
        self.depth_bits = 0;
        self.draw_to_display = false;
        self.texture_disable = false;
        self.rect_flip_x = false;
        self.rect_flip_y = false;
        self.window_raw = 0;
        self.area_top_left_raw = 0;
        self.area_bottom_right_raw = 0;
        self.offset_raw = 0;
        self.display_enabled = false;
        self.dma_direction = 0;
        self.display_start = (0, 0);
        self.horizontal_range = (0x200, 0x200 + 256 * 10);
        self.vertical_range = (0x10, 0x10 + 240);
        self.display_mode = 0;
        self.irq1 = false;
        self.busy_clocks = 0;
        self.gp0 = Gp0State::Command;
        self.download = None;
    }

    // ----- Display geometry -----

    pub fn is_pal(&self) -> bool {
        self.display_mode & (1 << 3) != 0
    }

    fn is_interlaced(&self) -> bool {
        self.display_mode & (1 << 5) != 0
    }

    /// Width in pixels of the displayed area.
    pub fn display_width(&self) -> usize {
        if self.display_mode & (1 << 6) != 0 {
            368
        } else {
            [256, 320, 512, 640][(self.display_mode & 3) as usize]
        }
    }

    /// Height in lines of the displayed area.
    pub fn display_height(&self) -> usize {
        if self.display_mode & (1 << 2) != 0 { 480 } else { 240 }
    }

    /// GPU clocks per dot at the current horizontal resolution.
    fn dot_clock_divider(&self) -> u32 {
        match self.display_width() {
            256 => 10,
            320 => 8,
            368 => 7,
            512 => 5,
            _ => 4,
        }
    }

    fn clocks_per_line(&self) -> u32 {
        if self.is_pal() { PAL_CLOCKS_PER_LINE } else { NTSC_CLOCKS_PER_LINE }
    }

    fn lines_per_frame(&self) -> u32 {
        if self.is_pal() { PAL_LINES_PER_FRAME } else { NTSC_LINES_PER_FRAME }
    }

    /// CPU cycles in one frame at the current video mode.
    pub fn cycles_per_frame(&self) -> u64 {
        self.lines_per_frame() as u64 * self.clocks_per_line() as u64
            * CPU_CYCLES_PER_11_GPU_CLOCKS as u64
            / GPU_CLOCKS_PER_7_CPU_CYCLES as u64
    }

    /// Whether the beam is in the vertical blanking interval.
    pub fn in_vblank(&self) -> bool {
        self.scanline >= VBLANK_START_LINE
    }

    /// The part of each scanline that is displayed, in GPU clocks since the
    /// start of the line (from the horizontal display range).
    fn visible_clocks(&self) -> (u32, u32) {
        let per_line = self.clocks_per_line();
        let start = self.horizontal_range.0.min(per_line - 1);
        let end = self.horizontal_range.1.clamp(start + 1, per_line);
        (start, end)
    }

    /// Whether the beam is in the horizontal blanking interval: the part of the
    /// line outside the displayed range.
    pub fn in_hblank(&self) -> bool {
        let (start, end) = self.visible_clocks();
        self.line_clock < start || self.line_clock >= end
    }

    /// Whether the GPU is still drawing what it was last given (GPUSTAT bits 26
    /// and 28 read as not ready until it finishes).
    pub fn is_busy(&self) -> bool {
        self.busy_clocks > 0
    }

    fn add_busy(&mut self, clocks: u32) {
        self.busy_clocks = (self.busy_clocks + clocks).min(MAX_BUSY_CLOCKS);
    }

    // ----- Timing -----

    /// Advance the video timing by `cpu_cycles` CPU cycles.
    pub fn advance(&mut self, cpu_cycles: u32) -> VideoTick {
        let mut tick = VideoTick::default();
        let was_in_hblank = self.in_hblank();

        self.clock_remainder += cpu_cycles * GPU_CLOCKS_PER_7_CPU_CYCLES;
        let gpu_clocks = self.clock_remainder / CPU_CYCLES_PER_11_GPU_CLOCKS;
        self.clock_remainder %= CPU_CYCLES_PER_11_GPU_CLOCKS;
        self.busy_clocks = self.busy_clocks.saturating_sub(gpu_clocks);

        let divider = self.dot_clock_divider();
        self.dot_remainder += gpu_clocks;
        tick.dot_ticks = self.dot_remainder / divider;
        self.dot_remainder %= divider;

        self.line_clock += gpu_clocks;
        let (clocks_per_line, lines_per_frame) = (self.clocks_per_line(), self.lines_per_frame());
        while self.line_clock >= clocks_per_line {
            self.line_clock -= clocks_per_line;
            self.scanline += 1;
            tick.hblanks += 1;
            if self.scanline == VBLANK_START_LINE {
                tick.vblank_started = true;
                self.field = !self.field;
            }
            if self.scanline >= lines_per_frame {
                self.scanline = 0;
            }
        }
        tick.in_hblank = self.in_hblank();
        tick.in_vblank = self.in_vblank();
        tick.hblank_started = tick.in_hblank && !was_in_hblank;
        tick
    }

    /// Lower bounds on when the dot clock and hblank next pulse.
    pub fn video_timing(&self) -> VideoTiming {
        // gpu_clocks(c) = (remainder + 11 c) / 7 must reach `clocks`.
        let cycles_for = |clocks: u32| {
            (clocks * CPU_CYCLES_PER_11_GPU_CLOCKS).saturating_sub(self.clock_remainder)
                / GPU_CLOCKS_PER_7_CPU_CYCLES
        };
        let divider = self.dot_clock_divider();
        let per_line = self.clocks_per_line();
        VideoTiming {
            next_dot: cycles_for(divider - self.dot_remainder.min(divider - 1)),
            dot_period: divider * CPU_CYCLES_PER_11_GPU_CLOCKS / GPU_CLOCKS_PER_7_CPU_CYCLES,
            next_hblank: cycles_for(per_line - self.line_clock.min(per_line - 1)),
            hblank_period: per_line * CPU_CYCLES_PER_11_GPU_CLOCKS / GPU_CLOCKS_PER_7_CPU_CYCLES,
            next_blank_edge: cycles_for(self.clocks_to_blank_edge()).min(self.cycles_until_frame_edge()).max(1),
        }
    }

    /// GPU clocks until the horizontal blanking interval starts or ends.
    fn clocks_to_blank_edge(&self) -> u32 {
        let (start, end) = self.visible_clocks();
        let line = self.line_clock;
        if line < start {
            start - line
        } else if line < end {
            end - line
        } else {
            self.clocks_per_line() - line + start
        }
    }

    /// A lower bound on the CPU cycles until the vertical blanking interval
    /// starts or ends.
    fn cycles_until_frame_edge(&self) -> u32 {
        let lines = if self.scanline < VBLANK_START_LINE {
            VBLANK_START_LINE - self.scanline
        } else {
            self.lines_per_frame() - self.scanline
        };
        let clocks = (lines * self.clocks_per_line()).saturating_sub(self.line_clock);
        (clocks * CPU_CYCLES_PER_11_GPU_CLOCKS).saturating_sub(self.clock_remainder) / GPU_CLOCKS_PER_7_CPU_CYCLES
    }

    /// A lower bound on the CPU cycles until the next vertical blank starts.
    pub fn cycles_until_vblank(&self) -> u32 {
        let per_line = self.clocks_per_line();
        let lines = if self.scanline < VBLANK_START_LINE {
            VBLANK_START_LINE - self.scanline
        } else {
            (self.lines_per_frame() + VBLANK_START_LINE).saturating_sub(self.scanline)
        };
        let clocks = (lines * per_line).saturating_sub(self.line_clock);
        // gpu_clocks(c) = (remainder + 11 c) / 7 must reach `clocks`.
        (clocks * CPU_CYCLES_PER_11_GPU_CLOCKS).saturating_sub(self.clock_remainder)
            / GPU_CLOCKS_PER_7_CPU_CYCLES
    }

    /// Advance GPU by one cycle. Drawing is immediate, so there is no work.
    pub fn step(&mut self) {}

    /// Whether the GPU raised its interrupt (GP0 1Fh) since the last call.
    pub fn take_irq(&mut self) -> bool {
        std::mem::take(&mut self.irq_edge)
    }

    // ----- Registers -----

    /// `GPUSTAT`.
    pub fn status(&self) -> u32 {
        let mut stat = self.page_x
            | (self.page_y << 4)
            | ((self.semi_mode as u32) << 5)
            | (self.depth_bits << 7)
            | ((self.settings.dither as u32) << 9)
            | ((self.draw_to_display as u32) << 10)
            | ((self.settings.set_mask as u32) << 11)
            | ((self.settings.check_mask as u32) << 12)
            | ((self.texture_disable as u32) << 15);

        // Display mode: hres2 (bit 6) -> 16, hres1 -> 17-18, vres -> 19,
        // video mode -> 20, colour depth -> 21, interlace -> 22, reverse -> 14.
        let mode = self.display_mode;
        stat |= ((mode >> 6) & 1) << 16;
        stat |= (mode & 3) << 17;
        stat |= ((mode >> 2) & 1) << 19;
        stat |= ((mode >> 3) & 1) << 20;
        stat |= ((mode >> 4) & 1) << 21;
        stat |= ((mode >> 5) & 1) << 22;
        stat |= ((mode >> 7) & 1) << 14;

        stat |= (!self.display_enabled as u32) << 23;
        stat |= (self.irq1 as u32) << 24;
        // Data request: follows the DMA direction.
        stat |= ((self.dma_direction != 0) as u32) << 25;
        stat |= STAT_READY_FOR_VRAM_READ;
        if !self.is_busy() {
            stat |= STAT_READY_FOR_COMMAND | STAT_READY_FOR_DMA;
        }
        stat |= (self.dma_direction as u32) << 29;

        // Interlace field / odd-even line.
        if self.is_interlaced() {
            stat |= (!self.field as u32) << 13;
            stat |= (self.field as u32) << 31;
        } else {
            stat |= 1 << 13;
            if !self.in_vblank() {
                stat |= (self.scanline & 1) << 31;
            }
        }
        stat
    }

    /// Register read; `off` is relative to `0x1F801810`
    /// (0 = GPUREAD, 4 = GPUSTAT).
    pub fn read(&mut self, off: u32, width: u32) -> u32 {
        let word = match off & !3 {
            0 => self.read_gpuread(),
            4 => self.status(),
            _ => 0,
        };
        extract(word, off, width)
    }

    /// Register write; `off` is relative to `0x1F801810`
    /// (0 = GP0 render/VRAM commands, 4 = GP1 display control).
    pub fn write(&mut self, off: u32, _width: u32, value: u32) {
        match off & !3 {
            0 => self.write_gp0(value),
            4 => self.write_gp1(value),
            _ => {}
        }
    }

    fn read_gpuread(&mut self) -> u32 {
        let Some(download) = &mut self.download else { return self.gpuread };

        // Two pixels per word, the first in the low half.
        let mut word = 0;
        for half in 0..2 {
            if download.index < download.width * download.height {
                let x = download.x + download.index % download.width;
                let y = download.y + download.index / download.width;
                word |= (self.vram.get(x, y) as u32) << (16 * half);
                download.index += 1;
            }
        }
        if download.index >= download.width * download.height {
            self.download = None;
        }
        self.gpuread = word;
        word
    }

    // ----- GP1: display control -----

    fn write_gp1(&mut self, value: u32) {
        let argument = value & 0x00FF_FFFF;
        match value >> 24 {
            0x00 => self.reset_state(),
            0x01 => {
                // Reset the command buffer.
                self.gp0 = Gp0State::Command;
                self.download = None;
            }
            0x02 => self.irq1 = false,
            0x03 => self.display_enabled = argument & 1 == 0,
            0x04 => self.dma_direction = (argument & 3) as u8,
            0x05 => self.display_start = (argument & 0x3FF, (argument >> 10) & 0x1FF),
            0x06 => self.horizontal_range = (argument & 0xFFF, (argument >> 12) & 0xFFF),
            0x07 => self.vertical_range = (argument & 0x3FF, (argument >> 10) & 0x3FF),
            0x08 => self.display_mode = argument & 0xFF,
            0x09 => {} // allow texture disable: not needed
            0x10..=0x1F => self.gpu_info(argument & 0xF),
            _ => {}
        }
    }

    /// GP1(10h): put the requested register into GPUREAD.
    fn gpu_info(&mut self, index: u32) {
        match index {
            2 => self.gpuread = self.window_raw,
            3 => self.gpuread = self.area_top_left_raw,
            4 => self.gpuread = self.area_bottom_right_raw,
            5 => self.gpuread = self.offset_raw,
            7 => self.gpuread = GPU_VERSION,
            _ => {}
        }
    }

    // ----- GP0: rendering commands -----

    /// Number of words (including the command word) a GP0 command takes, or
    /// `None` for a polyline whose length is unknown.
    fn command_length(command: u32) -> Option<usize> {
        let opcode = (command >> 24) as u8;
        Some(match opcode {
            0x02 => 3,
            0x20..=0x3F => {
                let vertices = if opcode & 0x08 != 0 { 4 } else { 3 };
                let textured = opcode & 0x04 != 0;
                let gouraud = opcode & 0x10 != 0;
                1 + vertices * (1 + textured as usize) + if gouraud { vertices - 1 } else { 0 }
            }
            0x40..=0x5F => {
                if opcode & 0x08 != 0 {
                    return None; // polyline
                }
                if opcode & 0x10 != 0 { 4 } else { 3 }
            }
            0x60..=0x7F => {
                let textured = opcode & 0x04 != 0;
                let variable = opcode & 0x18 == 0;
                2 + textured as usize + variable as usize
            }
            0x80..=0x9F => 4,
            0xA0..=0xBF | 0xC0..=0xDF => 3,
            _ => 1,
        })
    }

    fn write_gp0(&mut self, word: u32) {
        let _area = crate::profile::enter(crate::profile::Area::GpuCommand);
        self.gp0_word_count += 1;
        match &mut self.gp0 {
            Gp0State::Command => self.start_command(word),
            Gp0State::Parameters { words, len, needed } => {
                words[*len] = word;
                self.precise[*len] = self.incoming;
                *len += 1;
                if *len == *needed {
                    let (words, needed) = (*words, *needed);
                    self.gp0 = Gp0State::Command;
                    self.execute_command(&words[..needed]);
                }
            }
            Gp0State::Polyline { words } => {
                // The list ends with a word like 0x5xxx5xxx.
                if word & 0xF000_F000 == 0x5000_5000 && words.len() >= 3 {
                    let words = std::mem::take(words);
                    self.gp0 = Gp0State::Command;
                    self.draw_polyline(&words);
                } else {
                    words.push(word);
                }
            }
            Gp0State::Upload { x, y, width, height, col, row } => {
                for half in 0..2 {
                    if *row < *height {
                        let pixel = (word >> (16 * half)) as u16;
                        let (px, py) = (*x + *col, *y + *row);
                        if let Some(hires) = self.hires.as_deref_mut() {
                            let blocked = self.settings.check_mask && self.vram.get(px, py) & 0x8000 != 0;
                            if !blocked {
                                hires.upload_pixel(px, py, pixel | if self.settings.set_mask { 0x8000 } else { 0 });
                            }
                        }
                        sw::upload_pixel(&mut self.vram, &self.settings, px, py, pixel);
                        *col += 1;
                        if *col == *width {
                            *col = 0;
                            *row += 1;
                        }
                    }
                }
                if *row >= *height {
                    self.gp0 = Gp0State::Command;
                }
            }
        }
    }

    fn start_command(&mut self, word: u32) {
        match Self::command_length(word) {
            Some(1) => self.execute_command(&[word]),
            Some(needed) => {
                let mut words = [0; 12];
                words[0] = word;
                self.gp0 = Gp0State::Parameters { words, len: 1, needed };
            }
            None => self.gp0 = Gp0State::Polyline { words: vec![word] },
        }
    }

    fn execute_command(&mut self, words: &[u32]) {
        let command = words[0];
        match (command >> 24) as u8 {
            0x00 | 0x01 | 0x03..=0x1E => {} // NOP / cache clear / unknown
            0x1F => {
                if !self.irq1 {
                    self.irq1 = true;
                    self.irq_edge = true;
                }
            }
            0x02 => {
                let (x, y) = (words[1] & 0xFFFF, words[1] >> 16);
                let (w, h) = (words[2] & 0xFFFF, words[2] >> 16);
                sw::fill_rectangle(&mut self.vram, x, y, w, h, rgb(command));
                if let Some(hires) = self.hires.as_deref_mut() {
                    let (fx, fy, fw, fh) = sw::fill_area(x, y, w, h);
                    hires.fill_rectangle(fx, fy, fw, fh, rgb(command));
                }
                self.add_busy(cost::fill(w, h));
            }
            0x20..=0x3F => self.draw_polygon(words),
            0x40..=0x5F => self.draw_polyline(words),
            0x60..=0x7F => self.draw_rectangle(words),
            0x80..=0x9F => {
                let src = (words[1] & 0x3FF, (words[1] >> 16) & 0x1FF);
                let dst = (words[2] & 0x3FF, (words[2] >> 16) & 0x1FF);
                let size = (words[3] & 0xFFFF, words[3] >> 16);
                sw::copy_rectangle(&mut self.vram, &self.settings, src, dst, size);
                if let Some(hires) = self.hires.as_deref_mut() {
                    hires.copy_rectangle(&self.settings, src, dst, size);
                }
                self.add_busy(cost::copy(size.0, size.1));
            }
            0xA0..=0xBF => {
                let (x, y) = ((words[1] & 0x3FF) as usize, ((words[1] >> 16) & 0x1FF) as usize);
                let width = sw::transfer_width(words[2] & 0xFFFF);
                let height = sw::transfer_height(words[2] >> 16);
                self.gp0 = Gp0State::Upload { x, y, width, height, col: 0, row: 0 };
            }
            0xC0..=0xDF => {
                let (x, y) = ((words[1] & 0x3FF) as usize, ((words[1] >> 16) & 0x1FF) as usize);
                let width = sw::transfer_width(words[2] & 0xFFFF);
                let height = sw::transfer_height(words[2] >> 16);
                self.download = Some(Download { x, y, width, height, index: 0 });
            }
            0xE1 => self.set_draw_mode(command),
            0xE2 => {
                self.window_raw = command & 0xFFFFF;
                self.settings.window = TextureWindow {
                    mask_x: command & 0x1F,
                    mask_y: (command >> 5) & 0x1F,
                    offset_x: (command >> 10) & 0x1F,
                    offset_y: (command >> 15) & 0x1F,
                };
            }
            0xE3 => {
                self.area_top_left_raw = command & 0xFFFFF;
                self.settings.area.left = (command & 0x3FF) as i32;
                self.settings.area.top = ((command >> 10) & 0x1FF) as i32;
            }
            0xE4 => {
                self.area_bottom_right_raw = command & 0xFFFFF;
                self.settings.area.right = (command & 0x3FF) as i32;
                self.settings.area.bottom = ((command >> 10) & 0x1FF) as i32;
            }
            0xE5 => {
                self.offset_raw = command & 0x3FFFFF;
                self.settings.offset_x = sign_extend_11(command & 0x7FF);
                self.settings.offset_y = sign_extend_11((command >> 11) & 0x7FF);
            }
            0xE6 => {
                self.settings.set_mask = command & 1 != 0;
                self.settings.check_mask = command & 2 != 0;
            }
            _ => {}
        }
    }

    /// GP0(E1h): texture page and drawing mode.
    fn set_draw_mode(&mut self, command: u32) {
        self.set_texture_page(command);
        self.settings.dither = command & (1 << 9) != 0;
        self.draw_to_display = command & (1 << 10) != 0;
        self.texture_disable = command & (1 << 11) != 0;
        self.rect_flip_x = command & (1 << 12) != 0;
        self.rect_flip_y = command & (1 << 13) != 0;
    }

    /// Texture page attributes (bits 0-8), shared by GP0(E1h) and the second
    /// vertex of a textured polygon.
    fn set_texture_page(&mut self, page: u32) {
        self.page_x = page & 0xF;
        self.page_y = (page >> 4) & 1;
        self.semi_mode = ((page >> 5) & 3) as u8;
        self.depth_bits = (page >> 7) & 3;
    }

    /// The texture settings in force, with a palette position.
    fn current_texture(&self, clut: u32) -> Texture {
        Texture {
            page_x: self.page_x * 64,
            page_y: self.page_y * 256,
            depth: match self.depth_bits {
                0 => TextureDepth::Bits4,
                1 => TextureDepth::Bits8,
                _ => TextureDepth::Bits15,
            },
            clut_x: (clut & 0x3F) * 16,
            clut_y: (clut >> 6) & 0x1FF,
        }
    }

    fn draw_polygon(&mut self, words: &[u32]) {
        let _area = crate::profile::enter(crate::profile::Area::GpuTriangle);
        let command = words[0];
        let gouraud = command & (1 << 28) != 0;
        let quad = command & (1 << 27) != 0;
        let textured = command & (1 << 26) != 0;
        let semi = command & (1 << 25) != 0;
        let raw = command & (1 << 24) != 0;

        let base_color = rgb(command);
        let mut vertices = [Vertex::default(); 4];
        let mut precise = [PreciseVertex::NONE; 4];
        let mut index = 1;
        let mut clut = 0;
        for (i, vertex) in vertices.iter_mut().enumerate().take(if quad { 4 } else { 3 }) {
            let color = if i > 0 && gouraud {
                index += 1;
                rgb(words[index - 1])
            } else {
                base_color
            };
            let position = words[index];
            precise[i] = self.precise[index];
            index += 1;
            *vertex = Vertex {
                x: sign_extend_11(position & 0x7FF),
                y: sign_extend_11((position >> 16) & 0x7FF),
                color,
                u: 0,
                v: 0,
            };
            if textured {
                let texcoord = words[index];
                index += 1;
                vertex.u = texcoord as u8;
                vertex.v = (texcoord >> 8) as u8;
                match i {
                    0 => clut = texcoord >> 16,
                    1 => self.set_texture_page(texcoord >> 16),
                    _ => {}
                }
            }
        }

        let attrs = Attributes {
            textured,
            raw_texture: raw,
            semi_transparent: semi,
            semi_mode: self.semi_mode,
            gouraud,
            texture: self.current_texture(clut),
        };
        // The scaled picture goes first: it reads textures from the console's
        // VRAM, which the console's own drawing below may overwrite.
        if self.hires.is_some() {
            self.draw_polygon_scaled(&vertices, &precise, quad, &attrs);
        }
        sw::draw_triangle(&mut self.vram, &self.settings, &attrs, [vertices[0], vertices[1], vertices[2]]);
        self.add_busy(cost::triangle(doubled_area(&vertices[0..3]), textured, semi));
        if quad {
            sw::draw_triangle(&mut self.vram, &self.settings, &attrs, [vertices[1], vertices[2], vertices[3]]);
            self.add_busy(cost::triangle(doubled_area(&vertices[1..4]), textured, semi));
        }
    }

    /// The same polygon in the scaled VRAM, with PGXP positions where the GTE
    /// made them: a vertex uses its precise position only if its tag is valid
    /// and agrees with the integer position the console computed.
    fn draw_polygon_scaled(&mut self, vertices: &[Vertex; 4], precise: &[PreciseVertex; 4], quad: bool, attrs: &Attributes) {
        let render = self.render;
        let count = if quad { 4 } else { 3 };
        // The polygon is drawn at its precise positions only if every vertex has them: a
        // vertex the CPU computed has none, and mixing precise and integer vertices would
        // open seams between neighbouring polygons.
        let trusted = render.pgxp
            && (0..count).all(|i| precise[i].is_valid() && agrees(precise[i].x, vertices[i].x) && agrees(precise[i].y, vertices[i].y));
        let mut scaled = [HVertex::default(); 4];
        for i in 0..count {
            let (v, tag) = (&vertices[i], &precise[i]);
            scaled[i] = HVertex {
                x: if trusted { tag.x } else { v.x as f32 },
                y: if trusted { tag.y } else { v.y as f32 },
                w: if trusted { tag.w } else { 0.0 },
                color: v.color,
                u: v.u,
                v: v.v,
                ix: v.x,
                iy: v.y,
            };
        }

        // Widescreen: 3D geometry (every vertex precise) is squeezed towards the
        // middle of the drawing area; 2D elements are left to be stretched.
        if render.widescreen && trusted {
            let area = &self.settings.area;
            let centre = (area.left + area.right + 1) as f32 / 2.0;
            let offset = self.settings.offset_x as f32;
            for v in &mut scaled[..count] {
                v.x = centre + (v.x + offset - centre) * WIDESCREEN_SCALE - offset;
            }
        }

        let Some(hires) = self.hires.as_deref_mut() else { return };
        let perspective = trusted && render.pgxp_textures;
        let first = [scaled[0], scaled[1], scaled[2]];
        hires.draw_triangle(&self.vram, &self.settings, attrs, first, perspective);
        if quad {
            let second = [scaled[1], scaled[2], scaled[3]];
            hires.draw_triangle(&self.vram, &self.settings, attrs, second, perspective);
        }
    }

    fn draw_rectangle(&mut self, words: &[u32]) {
        let _area = crate::profile::enter(crate::profile::Area::GpuRectangle);
        let command = words[0];
        let textured = command & (1 << 26) != 0;
        let raw = command & (1 << 24) != 0;
        let semi = command & (1 << 25) != 0;

        let position = words[1];
        let (x, y) = (sign_extend_11(position & 0x7FF), sign_extend_11((position >> 16) & 0x7FF));
        let mut index = 2;
        let (mut u, mut v, mut clut) = (0u8, 0u8, 0);
        if textured {
            let texcoord = words[index];
            index += 1;
            u = texcoord as u8;
            v = (texcoord >> 8) as u8;
            clut = texcoord >> 16;
        }
        let (width, height) = match (command >> 27) & 3 {
            0 => {
                let size = words[index];
                ((size & 0x3FF) as i32, ((size >> 16) & 0x1FF) as i32)
            }
            1 => (1, 1),
            2 => (8, 8),
            _ => (16, 16),
        };

        let attrs = Attributes {
            textured,
            raw_texture: raw,
            semi_transparent: semi,
            semi_mode: self.semi_mode,
            gouraud: false,
            texture: self.current_texture(clut),
        };
        if let Some(hires) = self.hires.as_deref_mut() {
            hires.draw_rectangle(
                &self.vram,
                &self.settings,
                &attrs,
                x,
                y,
                width,
                height,
                rgb(command),
                u,
                v,
                self.rect_flip_x,
                self.rect_flip_y,
            );
        }
        sw::draw_rectangle(
            &mut self.vram,
            &self.settings,
            &attrs,
            x,
            y,
            width,
            height,
            rgb(command),
            u,
            v,
            self.rect_flip_x,
            self.rect_flip_y,
        );
        self.add_busy(cost::rectangle(width as u32, height as u32, textured, semi));
    }

    /// Lines and polylines: `words` holds the command, then vertices (with a
    /// colour before each vertex but the first when Gouraud shaded).
    fn draw_polyline(&mut self, words: &[u32]) {
        let _area = crate::profile::enter(crate::profile::Area::GpuLine);
        let command = words[0];
        let gouraud = command & (1 << 28) != 0;
        let semi = command & (1 << 25) != 0;
        let attrs = Attributes {
            semi_transparent: semi,
            semi_mode: self.semi_mode,
            gouraud,
            ..Attributes::default()
        };

        let mut points: Vec<Vertex> = Vec::new();
        let mut index = 1;
        let mut color = rgb(command);
        while index < words.len() {
            if !points.is_empty() && gouraud {
                color = rgb(words[index]);
                index += 1;
                if index >= words.len() {
                    break;
                }
            }
            let position = words[index];
            index += 1;
            points.push(Vertex {
                x: sign_extend_11(position & 0x7FF),
                y: sign_extend_11((position >> 16) & 0x7FF),
                color,
                u: 0,
                v: 0,
            });
        }
        for pair in points.windows(2) {
            sw::draw_line(&mut self.vram, &self.settings, &attrs, pair[0], pair[1]);
            if let Some(hires) = self.hires.as_deref_mut() {
                let scaled = |v: &Vertex| HVertex { x: v.x as f32, y: v.y as f32, w: 0.0, color: v.color, u: 0, v: 0, ix: v.x, iy: v.y };
                hires.draw_line(&self.settings, &attrs, &scaled(&pair[0]), &scaled(&pair[1]));
            }
            self.add_busy(cost::line(pair[0].x.abs_diff(pair[1].x), pair[0].y.abs_diff(pair[1].y), semi));
        }
    }

    // ----- Display output -----

    /// Render the displayed part of VRAM as RGBA8888, `display_width() *
    /// display_height() * 4` bytes.
    pub fn framebuffer(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.render_into(&mut out);
        out
    }

    /// Convert the displayed part of VRAM to RGBA8888 in `out`, which is
    /// resized to `display_width() * display_height() * 4` bytes. This is the
    /// console's own picture; `render_frame` is what the frontend shows.
    pub fn render_into(&self, out: &mut Vec<u8>) {
        let mut pixels = Vec::new();
        self.render_native(&mut pixels);
        out.clear();
        out.extend(pixels.iter().flat_map(|pixel| pixel.to_le_bytes()));
    }

    /// The console's picture as `0xAABBGGRR` pixels: `display_width() *
    /// display_height()` of them, converted straight from VRAM.
    fn render_native(&self, out: &mut Vec<u32>) {
        let _area = crate::profile::enter(crate::profile::Area::Display);
        let (width, height) = (self.display_width(), self.display_height());
        out.resize(width * height, 0);
        if !self.display_enabled {
            out.fill(0xFF00_0000);
            return;
        }

        let (start_x, start_y) = (self.display_start.0 as usize, self.display_start.1 as usize);
        let true_color = self.display_mode & (1 << 4) != 0;
        let pixels = &self.vram.pixels;
        for (y, line) in out.chunks_exact_mut(width).enumerate() {
            let row = ((start_y + y) % sw::VRAM_HEIGHT) * sw::VRAM_WIDTH;
            if true_color {
                // 24-bit mode: three bytes per pixel packed into the row.
                let byte = |i: usize| (pixels[row + (start_x + i / 2) % sw::VRAM_WIDTH] >> ((i % 2) * 8)) as u32;
                for (x, pixel) in line.iter_mut().enumerate() {
                    *pixel = 0xFF00_0000 | byte(x * 3) | byte(x * 3 + 1) << 8 | byte(x * 3 + 2) << 16;
                }
            } else {
                for (x, pixel) in line.iter_mut().enumerate() {
                    *pixel = hires::expand15(pixels[row + (start_x + x) % sw::VRAM_WIDTH]) & 0x00FF_FFFF | 0xFF00_0000;
                }
            }
        }
    }

    /// What is on screen, as `0xAABBGGRR` pixels (the bytes of RGBA8888 on a
    /// little-endian machine) in `out`, and its size in pixels: the console's
    /// display size times the internal resolution. Reusing `out` every frame
    /// avoids allocating and zeroing a buffer each time.
    pub fn render_frame(&self, out: &mut Vec<u32>) -> (usize, usize) {
        let (width, height) = (self.display_width(), self.display_height());
        let Some(hires) = self.hires.as_deref() else {
            self.render_native(out);
            return (width, height);
        };
        let _area = crate::profile::enter(crate::profile::Area::Display);
        let scale = hires.scale();
        let true_color_display = self.display_mode & (1 << 4) != 0;
        if self.display_enabled && !true_color_display {
            let start = (self.display_start.0 as usize, self.display_start.1 as usize);
            hires.render_display(out, start, width, height);
        } else {
            // A 24-bit picture (video) or a blank screen has no scaled version:
            // the console's own picture, each pixel repeated.
            let mut native = Vec::new();
            self.render_native(&mut native);
            out.resize(width * scale * height * scale, 0);
            for (y, line) in out.chunks_exact_mut(width * scale).enumerate() {
                let source = &native[(y / scale) * width..][..width];
                for (x, pixel) in line.iter_mut().enumerate() {
                    *pixel = source[x / scale];
                }
            }
        }
        (width * scale, height * scale)
    }

    // ----- Save states -----

    /// The registers worth saving, packed into words.
    pub fn save_registers(&self) -> [u32; 16] {
        let mut regs = [0u32; 16];
        regs[0] = self.page_x | (self.page_y << 4) | ((self.semi_mode as u32) << 5) | (self.depth_bits << 7);
        regs[1] = self.window_raw;
        regs[2] = self.area_top_left_raw;
        regs[3] = self.area_bottom_right_raw;
        regs[4] = self.offset_raw;
        regs[5] = (self.settings.dither as u32)
            | ((self.draw_to_display as u32) << 1)
            | ((self.texture_disable as u32) << 2)
            | ((self.rect_flip_x as u32) << 3)
            | ((self.rect_flip_y as u32) << 4)
            | ((self.settings.set_mask as u32) << 5)
            | ((self.settings.check_mask as u32) << 6)
            | ((self.display_enabled as u32) << 7)
            | ((self.irq1 as u32) << 8)
            | ((self.field as u32) << 9);
        regs[6] = self.display_start.0 | (self.display_start.1 << 16);
        regs[7] = self.horizontal_range.0 | (self.horizontal_range.1 << 16);
        regs[8] = self.vertical_range.0 | (self.vertical_range.1 << 16);
        regs[9] = self.display_mode | ((self.dma_direction as u32) << 8);
        regs[10] = self.scanline;
        regs[11] = self.gpuread;
        regs
    }

    pub fn load_registers(&mut self, regs: &[u32; 16]) {
        self.set_texture_page(regs[0]);
        for (opcode, value) in [(0xE2u32, regs[1]), (0xE3, regs[2]), (0xE4, regs[3]), (0xE5, regs[4])] {
            self.execute_command(&[(opcode << 24) | value]);
        }
        let flags = regs[5];
        self.settings.dither = flags & 1 != 0;
        self.draw_to_display = flags & 2 != 0;
        self.texture_disable = flags & 4 != 0;
        self.rect_flip_x = flags & 8 != 0;
        self.rect_flip_y = flags & 16 != 0;
        self.settings.set_mask = flags & 32 != 0;
        self.settings.check_mask = flags & 64 != 0;
        self.display_enabled = flags & 128 != 0;
        self.irq1 = flags & 256 != 0;
        self.field = flags & 512 != 0;
        self.display_start = (regs[6] & 0xFFFF, regs[6] >> 16);
        self.horizontal_range = (regs[7] & 0xFFFF, regs[7] >> 16);
        self.vertical_range = (regs[8] & 0xFFFF, regs[8] >> 16);
        self.display_mode = regs[9] & 0xFF;
        self.dma_direction = ((regs[9] >> 8) & 3) as u8;
        self.scanline = regs[10];
        self.gpuread = regs[11];
        self.gp0 = Gp0State::Command;
        self.download = None;
    }

    /// VRAM as bytes (little-endian halfwords), for save states.
    pub fn vram_bytes(&self) -> Vec<u8> {
        self.vram.pixels.iter().flat_map(|p| p.to_le_bytes()).collect()
    }

    pub fn load_vram_bytes(&mut self, bytes: &[u8]) {
        if bytes.len() == VRAM_WIDTH * VRAM_HEIGHT * 2 {
            for (pixel, chunk) in self.vram.pixels.iter_mut().zip(bytes.chunks_exact(2)) {
                *pixel = u16::from_le_bytes([chunk[0], chunk[1]]);
            }
            if let Some(hires) = self.hires.as_deref_mut() {
                hires.rebuild(&self.vram);
            }
        }
    }
}

impl Default for Gpu {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a precise coordinate is the one the console's integer coordinate came
/// from. The GTE rounds down and clamps to 11 bits (so a vertex far off screen has a
/// saturated integer but keeps a useful precise position); its divider is
/// approximate, so one pixel more or less is allowed. Anything else is a tag that
/// belongs to some other value.
fn agrees(precise: f32, integer: i32) -> bool {
    (precise.floor().clamp(SCREEN_MIN, SCREEN_MAX) - integer as f32).abs() <= 1.0
}

/// Twice the area of the triangle the first three vertices make.
fn doubled_area(vertices: &[Vertex]) -> u64 {
    let (a, b, c) = (vertices[0], vertices[1], vertices[2]);
    let cross = (b.x as i64 - a.x as i64) * (c.y as i64 - a.y as i64)
        - (c.x as i64 - a.x as i64) * (b.y as i64 - a.y as i64);
    cross.unsigned_abs()
}

/// The 24-bit colour in the low bits of a command word.
fn rgb(word: u32) -> [u8; 3] {
    [word as u8, (word >> 8) as u8, (word >> 16) as u8]
}

/// Sign-extend an 11-bit coordinate.
fn sign_extend_11(value: u32) -> i32 {
    ((value << 21) as i32) >> 21
}

#[cfg(test)]
mod tests;
