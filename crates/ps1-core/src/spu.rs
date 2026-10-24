/// Number of 16-bit registers in the SPU window `0x1F801C00..0x1F802000`.
const REGISTER_COUNT: usize = 0x200;
/// Byte offsets (from `0x1F801C00`) of the control and status registers.
const SPUCNT_OFFSET: u32 = 0x1AA;
const SPUSTAT_OFFSET: u32 = 0x1AE;
/// SPUSTAT mirrors the low 6 bits of SPUCNT.
const SPUSTAT_MIRROR_MASK: u16 = 0x3F;

/// Sound Processing Unit (SPU).
///
/// Generates PS1 audio (ADPCM-encoded voices, reverb, volume envelopes).
/// Outputs stereo PCM samples at 44.1 kHz.
///
/// Mirrors DuckStation's `spu.cpp`.
pub struct Spu {
    pub sound_ram: Vec<u8>,
    /// Register window, one entry per halfword from `0x1F801C00`.
    pub register_file: [u16; REGISTER_COUNT],
    pub output_buffer: Vec<f32>,   // stereo interleaved f32
    pub sample_rate: u32,
}

impl Spu {
    pub fn new() -> Self {
        Self {
            sound_ram: vec![0u8; 512 * 1024],
            register_file: [0u16; REGISTER_COUNT],
            output_buffer: Vec::new(),
            sample_rate: 44100,
        }
    }

    pub fn reset(&mut self) {
        self.sound_ram.fill(0);
        self.register_file = [0u16; REGISTER_COUNT];
        self.output_buffer.clear();
    }

    /// Advance SPU by one cycle and produce audio samples.
    pub fn step(&mut self) {
        // TODO: implement voice mixing, ADPCM decoding, reverb.
    }

    fn read_half(&self, off: u32) -> u16 {
        let index = (off >> 1) as usize;
        if index >= REGISTER_COUNT {
            return 0;
        }
        if off == SPUSTAT_OFFSET {
            return self.register_file[(SPUCNT_OFFSET >> 1) as usize] & SPUSTAT_MIRROR_MASK;
        }
        self.register_file[index]
    }

    fn write_half(&mut self, off: u32, value: u16) {
        let index = (off >> 1) as usize;
        // SPUSTAT is read-only.
        if index < REGISTER_COUNT && off != SPUSTAT_OFFSET {
            self.register_file[index] = value;
        }
    }

    /// Register read; `off` is relative to `0x1F801C00`.
    pub fn read(&mut self, off: u32, width: u32) -> u32 {
        match width {
            1 => (self.read_half(off & !1) as u32 >> ((off & 1) * 8)) & 0xFF,
            2 => self.read_half(off & !1) as u32,
            _ => self.read_half(off) as u32 | ((self.read_half(off + 2) as u32) << 16),
        }
    }

    /// Register write; `off` is relative to `0x1F801C00`.
    pub fn write(&mut self, off: u32, width: u32, value: u32) {
        match width {
            1 => {
                let shift = (off & 1) * 8;
                let half = self.read_half(off & !1) as u32;
                let merged = (half & !(0xFF << shift)) | ((value & 0xFF) << shift);
                self.write_half(off & !1, merged as u16);
            }
            2 => self.write_half(off & !1, value as u16),
            _ => {
                self.write_half(off, value as u16);
                self.write_half(off + 2, (value >> 16) as u16);
            }
        }
    }

    /// Drain the output buffer into the provided slice (stereo f32 interleaved).
    pub fn drain_samples(&mut self, dst: &mut [f32]) {
        let n = dst.len().min(self.output_buffer.len());
        dst[..n].copy_from_slice(&self.output_buffer[..n]);
        self.output_buffer.drain(..n);
    }
}

impl Default for Spu {
    fn default() -> Self {
        Self::new()
    }
}
