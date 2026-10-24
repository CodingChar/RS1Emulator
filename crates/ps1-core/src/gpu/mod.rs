//! The graphics processor.
//!
//! The CPU talks to it through two registers at `0x1F801810`: `GP0` takes
//! drawing commands and VRAM transfers, `GP1` takes display control commands;
//! reading returns `GPUREAD` (VRAM data / info queries) and `GPUSTAT`.
//! Drawing happens immediately into a 1024x512 15-bit VRAM; the display is
//! produced from a window of that VRAM on demand.
//!
//! Mirrors DuckStation's `gpu.cpp`, `gpu_commands.cpp` and `gpu_sw.cpp`.

use crate::io::extract;

pub mod backend;
pub mod sw;

use sw::{
    Attributes, DrawSettings, Texture, TextureDepth, TextureWindow, Vertex, Vram, VRAM_HEIGHT,
    VRAM_WIDTH,
};

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
}

// ----- GPUSTAT bits that are always the same -----
const STAT_READY_FOR_COMMAND: u32 = 1 << 26;
const STAT_READY_FOR_VRAM_READ: u32 = 1 << 27;
const STAT_READY_FOR_DMA: u32 = 1 << 28;

/// What the GPU version query (GP1 10h, index 7) returns.
const GPU_VERSION: u32 = 2;

/// Where GP0 is in the middle of a multi-word command or transfer.
#[derive(Clone, Debug)]
enum Gp0State {
    /// Waiting for a command word.
    Command,
    /// Collecting the parameter words of a command.
    Parameters { words: Vec<u32>, needed: usize },
    /// Collecting vertices of a polyline until the terminator.
    Polyline { words: Vec<u32> },
    /// Receiving pixel data for a CPU-to-VRAM upload.
    Upload { x: usize, y: usize, width: usize, height: usize, index: usize },
}

/// A VRAM-to-CPU download in progress.
#[derive(Clone, Debug)]
struct Download {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    index: usize,
}

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

    // Video timing.
    /// Current scanline.
    pub scanline: u32,
    line_clock: u32,
    clock_remainder: u32,
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
            scanline: 0,
            line_clock: 0,
            clock_remainder: 0,
            dot_remainder: 0,
        };
        gpu.reset_state();
        gpu
    }

    /// Power-on reset: clears VRAM and the timing state.
    pub fn reset(&mut self) {
        *self = Self::new();
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

    // ----- Timing -----

    /// Advance the video timing by `cpu_cycles` CPU cycles.
    pub fn advance(&mut self, cpu_cycles: u32) -> VideoTick {
        let mut tick = VideoTick::default();

        self.clock_remainder += cpu_cycles * GPU_CLOCKS_PER_7_CPU_CYCLES;
        let gpu_clocks = self.clock_remainder / CPU_CYCLES_PER_11_GPU_CLOCKS;
        self.clock_remainder %= CPU_CYCLES_PER_11_GPU_CLOCKS;

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
        }
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
        stat |= STAT_READY_FOR_COMMAND | STAT_READY_FOR_VRAM_READ | STAT_READY_FOR_DMA;
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
        self.gp0_word_count += 1;
        match std::mem::replace(&mut self.gp0, Gp0State::Command) {
            Gp0State::Command => self.start_command(word),
            Gp0State::Parameters { mut words, needed } => {
                words.push(word);
                if words.len() == needed {
                    self.execute_command(&words);
                } else {
                    self.gp0 = Gp0State::Parameters { words, needed };
                }
            }
            Gp0State::Polyline { mut words } => {
                // The list ends with a word like 0x5xxx5xxx.
                if word & 0xF000_F000 == 0x5000_5000 && words.len() >= 3 {
                    self.draw_polyline(&words);
                } else {
                    words.push(word);
                    self.gp0 = Gp0State::Polyline { words };
                }
            }
            Gp0State::Upload { x, y, width, height, mut index } => {
                let total = width * height;
                for half in 0..2 {
                    if index < total {
                        let pixel = (word >> (16 * half)) as u16;
                        sw::upload_pixel(&mut self.vram, &self.settings, x + index % width, y + index / width, pixel);
                        index += 1;
                    }
                }
                if index < total {
                    self.gp0 = Gp0State::Upload { x, y, width, height, index };
                }
            }
        }
    }

    fn start_command(&mut self, word: u32) {
        match Self::command_length(word) {
            Some(1) => self.execute_command(&[word]),
            Some(needed) => self.gp0 = Gp0State::Parameters { words: vec![word], needed },
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
            }
            0x20..=0x3F => self.draw_polygon(words),
            0x40..=0x5F => self.draw_polyline(words),
            0x60..=0x7F => self.draw_rectangle(words),
            0x80..=0x9F => {
                let src = (words[1] & 0x3FF, (words[1] >> 16) & 0x1FF);
                let dst = (words[2] & 0x3FF, (words[2] >> 16) & 0x1FF);
                let size = (words[3] & 0xFFFF, words[3] >> 16);
                sw::copy_rectangle(&mut self.vram, &self.settings, src, dst, size);
            }
            0xA0..=0xBF => {
                let (x, y) = ((words[1] & 0x3FF) as usize, ((words[1] >> 16) & 0x1FF) as usize);
                let width = sw::transfer_width(words[2] & 0xFFFF);
                let height = sw::transfer_height(words[2] >> 16);
                self.gp0 = Gp0State::Upload { x, y, width, height, index: 0 };
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
        let command = words[0];
        let gouraud = command & (1 << 28) != 0;
        let quad = command & (1 << 27) != 0;
        let textured = command & (1 << 26) != 0;
        let semi = command & (1 << 25) != 0;
        let raw = command & (1 << 24) != 0;

        let base_color = rgb(command);
        let mut vertices = [Vertex::default(); 4];
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
        sw::draw_triangle(&mut self.vram, &self.settings, &attrs, [vertices[0], vertices[1], vertices[2]]);
        if quad {
            sw::draw_triangle(&mut self.vram, &self.settings, &attrs, [vertices[1], vertices[2], vertices[3]]);
        }
    }

    fn draw_rectangle(&mut self, words: &[u32]) {
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
    }

    /// Lines and polylines: `words` holds the command, then vertices (with a
    /// colour before each vertex but the first when Gouraud shaded).
    fn draw_polyline(&mut self, words: &[u32]) {
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
        }
    }

    // ----- Display output -----

    /// Render the displayed part of VRAM as RGBA8888, `display_width() *
    /// display_height() * 4` bytes.
    pub fn framebuffer(&self) -> Vec<u8> {
        let (width, height) = (self.display_width(), self.display_height());
        let mut out = vec![0u8; width * height * 4];
        for pixel in out.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        if !self.display_enabled {
            return out;
        }

        let (start_x, start_y) = (self.display_start.0 as usize, self.display_start.1 as usize);
        let true_color = self.display_mode & (1 << 4) != 0;
        for y in 0..height {
            let row = start_y + y;
            for x in 0..width {
                let (r, g, b) = if true_color {
                    // 24-bit mode: three bytes per pixel packed into the row.
                    let byte = |i: usize| {
                        let word = self.vram.get(start_x + i / 2, row);
                        (word >> ((i % 2) * 8)) as u8
                    };
                    (byte(x * 3), byte(x * 3 + 1), byte(x * 3 + 2))
                } else {
                    let pixel = self.vram.get(start_x + x, row);
                    let expand = |c: u16| ((c << 3) | (c >> 2)) as u8;
                    (expand(pixel & 31), expand((pixel >> 5) & 31), expand((pixel >> 10) & 31))
                };
                let at = (y * width + x) * 4;
                out[at] = r;
                out[at + 1] = g;
                out[at + 2] = b;
            }
        }
        out
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
        }
    }
}

impl Default for Gpu {
    fn default() -> Self {
        Self::new()
    }
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
