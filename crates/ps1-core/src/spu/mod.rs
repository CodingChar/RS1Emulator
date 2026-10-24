//! Sound Processing Unit: 24 ADPCM voices with ADSR envelopes, noise, pitch
//! modulation, reverb, and a mixer that adds the CD audio input. Produces
//! 44.1 kHz stereo.

mod gauss;
mod reverb;
mod voice;

use serde::{Deserialize, Serialize};
use crate::cdrom::CdRom;
use reverb::Reverb;
use voice::{IrqWatch, Phase, Voice, VoiceRegs, SOUND_RAM_MASK, SOUND_RAM_SIZE};

/// Number of 16-bit registers in the SPU window `0x1F801C00..0x1F802000`.
const REGISTER_COUNT: usize = 0x200;
const VOICE_COUNT: usize = 24;

// Byte offsets (from `0x1F801C00`) of the global registers.
const MAIN_VOLUME_LEFT: u32 = 0x180;
const MAIN_VOLUME_RIGHT: u32 = 0x182;
const REVERB_VOLUME_LEFT: u32 = 0x184;
const REVERB_VOLUME_RIGHT: u32 = 0x186;
const KEY_ON_LOW: u32 = 0x188;
const KEY_ON_HIGH: u32 = 0x18A;
const KEY_OFF_LOW: u32 = 0x18C;
const KEY_OFF_HIGH: u32 = 0x18E;
const PITCH_MOD_LOW: u32 = 0x190;
const PITCH_MOD_HIGH: u32 = 0x192;
const NOISE_LOW: u32 = 0x194;
const NOISE_HIGH: u32 = 0x196;
const REVERB_ON_LOW: u32 = 0x198;
const REVERB_ON_HIGH: u32 = 0x19A;
const ENDX_LOW: u32 = 0x19C;
const ENDX_HIGH: u32 = 0x19E;
const REVERB_START: u32 = 0x1A2;
const IRQ_ADDRESS: u32 = 0x1A4;
const TRANSFER_ADDRESS_OFFSET: u32 = 0x1A6;
const TRANSFER_FIFO_OFFSET: u32 = 0x1A8;
const SPUCNT_OFFSET: u32 = 0x1AA;
const SPUSTAT_OFFSET: u32 = 0x1AE;
const CD_VOLUME_LEFT: u32 = 0x1B0;
const CD_VOLUME_RIGHT: u32 = 0x1B2;
const REVERB_REGS: u32 = 0x1C0;

// SPUCNT bits.
const CNT_ENABLE: u16 = 1 << 15;
const CNT_UNMUTE: u16 = 1 << 14;
const CNT_IRQ_ENABLE: u16 = 1 << 6;
const CNT_REVERB_ENABLE: u16 = 1 << 7;
const CNT_CD_ENABLE: u16 = 1 << 0;
const CNT_CD_REVERB: u16 = 1 << 2;

/// SPUSTAT mirrors the low 6 bits of SPUCNT.
const SPUSTAT_MIRROR_MASK: u16 = 0x3F;
const SPUSTAT_IRQ: u16 = 1 << 6;

/// CPU cycles per output sample (33.8688 MHz / 44.1 kHz).
const CYCLES_PER_SAMPLE: u32 = 768;
/// The host is expected to drain the output; if it does not, keep only the
/// most recent two seconds.
const OUTPUT_LIMIT: usize = 44_100 * 2 * 2;

/// Sound Processing Unit (SPU).
#[derive(Serialize, Deserialize)]
pub struct Spu {
    pub sound_ram: Vec<u8>,
    /// Register window, one entry per halfword from `0x1F801C00`.
    #[serde(with = "crate::serde_arrays")]
    pub register_file: [u16; REGISTER_COUNT],
    /// Stereo interleaved samples ready for the host, in -1.0..1.0.
    #[serde(skip)]
    pub output_buffer: Vec<f32>,
    pub sample_rate: u32,
    /// Current sound RAM address (bytes) for CPU/DMA transfers.
    transfer_address: usize,

    voices: Vec<Voice>,
    reverb: Reverb,
    reverb_output: (i32, i32),
    reverb_toggle: bool,
    main_left: voice::Volume,
    main_right: voice::Volume,
    key_on: u32,
    key_off: u32,
    /// Voices that reached an end-of-loop flag (ENDX).
    end_flags: u32,
    noise_level: u16,
    noise_counter: u32,
    cycles: u32,
    /// The SPU interrupt fired and has not been acknowledged.
    irq_flag: bool,
    irq: bool,
}

impl Spu {
    pub fn new() -> Self {
        Self {
            sound_ram: vec![0u8; SOUND_RAM_SIZE],
            register_file: [0u16; REGISTER_COUNT],
            output_buffer: Vec::new(),
            sample_rate: 44100,
            transfer_address: 0,
            voices: (0..VOICE_COUNT).map(|_| Voice::new()).collect(),
            reverb: Reverb::new(),
            reverb_output: (0, 0),
            reverb_toggle: false,
            main_left: voice::Volume::default(),
            main_right: voice::Volume::default(),
            key_on: 0,
            key_off: 0,
            end_flags: 0,
            noise_level: 1,
            noise_counter: 0,
            cycles: 0,
            irq_flag: false,
            irq: false,
        }
    }

    pub fn reset(&mut self) {
        let sample_rate = self.sample_rate;
        *self = Self::new();
        self.sample_rate = sample_rate;
    }

    fn reg(&self, off: u32) -> u16 {
        self.register_file[(off >> 1) as usize]
    }

    /// 32-bit voice mask stored in a pair of registers.
    fn mask(&self, low: u32, high: u32) -> u32 {
        self.reg(low) as u32 | ((self.reg(high) as u32 & 0xFF) << 16)
    }

    /// Whether the SPU interrupt fired since the last call.
    pub fn take_irq(&mut self) -> bool {
        std::mem::take(&mut self.irq)
    }

    fn raise_irq(&mut self) {
        if !self.irq_flag {
            self.irq_flag = true;
            self.irq = true;
        }
    }

    fn irq_watch(&self) -> IrqWatch {
        let cnt = self.reg(SPUCNT_OFFSET);
        let armed = cnt & (CNT_ENABLE | CNT_IRQ_ENABLE) == (CNT_ENABLE | CNT_IRQ_ENABLE);
        IrqWatch {
            address: armed.then(|| (self.reg(IRQ_ADDRESS) as usize * 8) & SOUND_RAM_MASK),
            hit: false,
        }
    }

    // ----- Sound RAM transfers -----

    /// Store a halfword at the transfer address in sound RAM and advance it.
    fn push_transfer_half(&mut self, value: u16) {
        let [lo, hi] = value.to_le_bytes();
        let mut watch = self.irq_watch();
        watch.check(self.transfer_address, 2);
        self.sound_ram[self.transfer_address & SOUND_RAM_MASK] = lo;
        self.sound_ram[(self.transfer_address + 1) & SOUND_RAM_MASK] = hi;
        self.transfer_address = (self.transfer_address + 2) & SOUND_RAM_MASK;
        if watch.hit {
            self.raise_irq();
        }
    }

    fn pop_transfer_half(&mut self) -> u16 {
        let lo = self.sound_ram[self.transfer_address & SOUND_RAM_MASK];
        let hi = self.sound_ram[(self.transfer_address + 1) & SOUND_RAM_MASK];
        self.transfer_address = (self.transfer_address + 2) & SOUND_RAM_MASK;
        u16::from_le_bytes([lo, hi])
    }

    /// DMA: write one word into sound RAM.
    pub fn dma_write_word(&mut self, word: u32) {
        self.push_transfer_half(word as u16);
        self.push_transfer_half((word >> 16) as u16);
    }

    /// DMA: read one word from sound RAM.
    pub fn dma_read_word(&mut self) -> u32 {
        let lo = self.pop_transfer_half() as u32;
        lo | ((self.pop_transfer_half() as u32) << 16)
    }

    // ----- Time -----

    /// CPU cycles until the next sample is mixed.
    pub fn cycles_until_event(&self) -> u32 {
        CYCLES_PER_SAMPLE - self.cycles
    }

    /// Advance by `cycles` cycles that end before the next sample is due.
    pub fn skip_cycles(&mut self, cycles: u32) {
        self.cycles += cycles;
    }

    /// Advance the SPU by one CPU cycle; every 768 cycles it mixes one
    /// stereo sample, taking the CD audio input from the drive.
    pub fn step(&mut self, cdrom: &mut CdRom) {
        self.cycles += 1;
        if self.cycles >= CYCLES_PER_SAMPLE {
            self.cycles -= CYCLES_PER_SAMPLE;
            let cd = cdrom.pop_audio_frame();
            self.tick_sample(cd);
        }
    }

    fn voice_regs(&self, v: usize) -> VoiceRegs {
        let base = (v * 8) as usize;
        VoiceRegs {
            volume_left: self.register_file[base],
            volume_right: self.register_file[base + 1],
            pitch: self.register_file[base + 2],
            adsr_low: self.register_file[base + 4],
            adsr_high: self.register_file[base + 5],
        }
    }

    fn update_noise(&mut self, cnt: u16) {
        let shift = (cnt >> 10) & 0xF;
        let step = ((cnt >> 8) & 3) as u32;
        self.noise_counter += 4 + step;
        let threshold = 0x2_0000u32 >> shift;
        while self.noise_counter >= threshold {
            self.noise_counter -= threshold;
            let l = self.noise_level;
            let bit = ((l >> 15) ^ (l >> 12) ^ (l >> 11) ^ (l >> 10) ^ 1) & 1;
            self.noise_level = (l << 1) | bit;
        }
    }

    /// Produce one stereo output sample.
    fn tick_sample(&mut self, cd: (i16, i16)) {
        let cnt = self.reg(SPUCNT_OFFSET);
        let mut watch = self.irq_watch();

        // Key on/off requests take effect at the next sample.
        let (key_on, key_off) = (self.key_on, self.key_off);
        self.key_on = 0;
        self.key_off = 0;
        for v in 0..VOICE_COUNT {
            if key_off & (1 << v) != 0 {
                self.voices[v].key_off();
            }
            if key_on & (1 << v) != 0 {
                let start = self.register_file[v * 8 + 3] as usize * 8;
                self.voices[v].key_on(start, &self.sound_ram, &mut watch);
                self.end_flags &= !(1 << v);
            }
        }

        self.update_noise(cnt);
        let pitch_mod = self.mask(PITCH_MOD_LOW, PITCH_MOD_HIGH);
        let noise_on = self.mask(NOISE_LOW, NOISE_HIGH);
        let reverb_on = self.mask(REVERB_ON_LOW, REVERB_ON_HIGH);

        let (mut left, mut right) = (0i32, 0i32);
        let (mut reverb_in_left, mut reverb_in_right) = (0i32, 0i32);
        for v in 0..VOICE_COUNT {
            let regs = self.voice_regs(v);
            let noise = (noise_on & (1 << v) != 0).then_some(self.noise_level as i16);
            let modulation = (v > 0 && pitch_mod & (1 << v) != 0).then(|| self.voices[v - 1].last_output);
            let out = self.voices[v].tick(&regs, noise, modulation, &self.sound_ram, &mut watch);
            if out.loop_end {
                self.end_flags |= 1 << v;
            }
            left += out.left;
            right += out.right;
            if reverb_on & (1 << v) != 0 {
                reverb_in_left += out.left;
                reverb_in_right += out.right;
            }
        }

        // CD audio input.
        if cnt & CNT_CD_ENABLE != 0 {
            let cd_left = (cd.0 as i32 * self.reg(CD_VOLUME_LEFT) as i16 as i32) >> 15;
            let cd_right = (cd.1 as i32 * self.reg(CD_VOLUME_RIGHT) as i16 as i32) >> 15;
            left += cd_left;
            right += cd_right;
            if cnt & CNT_CD_REVERB != 0 {
                reverb_in_left += cd_left;
                reverb_in_right += cd_right;
            }
        }

        // The reverb network runs at half the sample rate.
        self.reverb_toggle = !self.reverb_toggle;
        if self.reverb_toggle {
            let writable = cnt & CNT_REVERB_ENABLE != 0;
            let start = self.reg(REVERB_START) as usize * 8;
            let regs = &self.register_file[(REVERB_REGS >> 1) as usize..][..reverb::REGISTER_COUNT];
            self.reverb_output = self.reverb.process(
                regs,
                start,
                &mut self.sound_ram,
                (reverb_in_left.clamp(-0x8000, 0x7FFF), reverb_in_right.clamp(-0x8000, 0x7FFF)),
                writable,
            );
        }
        if cnt & CNT_REVERB_ENABLE != 0 {
            left += (self.reverb_output.0 * self.reg(REVERB_VOLUME_LEFT) as i16 as i32) >> 15;
            right += (self.reverb_output.1 * self.reg(REVERB_VOLUME_RIGHT) as i16 as i32) >> 15;
        }

        let main_left = self.main_left.tick(self.reg(MAIN_VOLUME_LEFT));
        let main_right = self.main_right.tick(self.reg(MAIN_VOLUME_RIGHT));
        let left = ((left.clamp(-0x8000, 0x7FFF) * main_left) >> 15).clamp(-0x8000, 0x7FFF);
        let right = ((right.clamp(-0x8000, 0x7FFF) * main_right) >> 15).clamp(-0x8000, 0x7FFF);

        if watch.hit {
            self.raise_irq();
        }

        let audible = cnt & (CNT_ENABLE | CNT_UNMUTE) == (CNT_ENABLE | CNT_UNMUTE);
        let (left, right) = if audible { (left, right) } else { (0, 0) };
        if self.output_buffer.len() >= OUTPUT_LIMIT {
            self.output_buffer.drain(..OUTPUT_LIMIT / 2);
        }
        self.output_buffer.push(left as f32 / 32768.0);
        self.output_buffer.push(right as f32 / 32768.0);
    }

    // ----- Registers -----

    fn read_half(&self, off: u32) -> u16 {
        let index = (off >> 1) as usize;
        if index >= REGISTER_COUNT {
            return 0;
        }
        match off {
            SPUSTAT_OFFSET => {
                let cnt = self.reg(SPUCNT_OFFSET);
                let dma_mode = (cnt >> 4) & 3;
                let mut status = cnt & SPUSTAT_MIRROR_MASK;
                if self.irq_flag {
                    status |= SPUSTAT_IRQ;
                }
                status |= (dma_mode >> 1) << 7;
                status |= ((dma_mode == 2) as u16) << 8;
                status |= ((dma_mode == 3) as u16) << 9;
                status
            }
            ENDX_LOW => self.end_flags as u16,
            ENDX_HIGH => ((self.end_flags >> 16) & 0xFF) as u16,
            // Per-voice current envelope level and repeat address.
            0..=0x17F if off & 0xF == 0xC => (self.voices[(off >> 4) as usize].level) as u16,
            0..=0x17F if off & 0xF == 0xE => {
                (self.voices[(off >> 4) as usize].repeat_address >> 3) as u16
            }
            _ => self.register_file[index],
        }
    }

    fn write_half(&mut self, off: u32, value: u16) {
        let index = (off >> 1) as usize;
        // SPUSTAT and the live voice registers are read-only.
        if index < REGISTER_COUNT && off != SPUSTAT_OFFSET && off != ENDX_LOW && off != ENDX_HIGH {
            self.register_file[index] = value;
        }
        match off {
            0..=0x17F if off & 0xF == 0xE => {
                self.voices[(off >> 4) as usize].repeat_address = (value as usize * 8) & SOUND_RAM_MASK;
            }
            KEY_ON_LOW => self.key_on |= value as u32,
            KEY_ON_HIGH => self.key_on |= (value as u32 & 0xFF) << 16,
            KEY_OFF_LOW => self.key_off |= value as u32,
            KEY_OFF_HIGH => self.key_off |= (value as u32 & 0xFF) << 16,
            REVERB_START => self.reverb.current = 0,
            TRANSFER_ADDRESS_OFFSET => self.transfer_address = (value as usize * 8) & SOUND_RAM_MASK,
            TRANSFER_FIFO_OFFSET => self.push_transfer_half(value),
            SPUCNT_OFFSET => {
                // Clearing the interrupt-enable bit acknowledges the interrupt.
                if value & CNT_IRQ_ENABLE == 0 {
                    self.irq_flag = false;
                }
            }
            _ => {}
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
                let half = self.register_file.get((off >> 1) as usize).copied().unwrap_or(0) as u32;
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

    /// Whether voice `v` is currently producing sound.
    pub fn voice_active(&self, v: usize) -> bool {
        self.voices[v].phase != Phase::Off
    }
}

impl Default for Spu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
