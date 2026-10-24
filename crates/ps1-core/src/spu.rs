/// Sound Processing Unit (SPU).
///
/// Generates PS1 audio (ADPCM-encoded voices, reverb, volume envelopes).
/// Outputs stereo PCM samples at 44.1 kHz.
///
/// Mirrors DuckStation's `spu.cpp`.
pub struct Spu {
    pub sound_ram: Vec<u8>,
    pub register_file: [u16; 64], // simplified
    pub output_buffer: Vec<f32>,   // stereo interleaved f32
    pub sample_rate: u32,
}

impl Spu {
    pub fn new() -> Self {
        Self {
            sound_ram: vec![0u8; 512 * 1024],
            register_file: [0u16; 64],
            output_buffer: Vec::new(),
            sample_rate: 44100,
        }
    }

    pub fn reset(&mut self) {
        self.sound_ram.fill(0);
        self.register_file = [0u16; 64];
        self.output_buffer.clear();
    }

    /// Advance SPU by one cycle and produce audio samples.
    pub fn step(&mut self, _bus: &mut crate::bus::Bus, _memory: &mut crate::memory::Memory) {
        // TODO: implement voice mixing, ADPCM decoding, reverb.
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
