//! One of the SPU's 24 voices: ADPCM decoding, pitch counter, interpolation,
//! the ADSR envelope and the volume sweeps.
use serde::{Deserialize, Serialize};

use super::gauss::GAUSS;

pub const SOUND_RAM_SIZE: usize = 512 * 1024;
pub const SOUND_RAM_MASK: usize = SOUND_RAM_SIZE - 1;

/// ADPCM block: 16 bytes holding 28 samples.
const BLOCK_BYTES: usize = 16;
const BLOCK_SAMPLES: usize = 28;

const FILTER_POSITIVE: [i32; 5] = [0, 60, 115, 98, 122];
const FILTER_NEGATIVE: [i32; 5] = [0, 0, -52, -55, -60];

// Block flags (second byte of an ADPCM block).
const FLAG_LOOP_END: u8 = 1 << 0;
const FLAG_LOOP_REPEAT: u8 = 1 << 1;
const FLAG_LOOP_START: u8 = 1 << 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Off,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// How an envelope level moves: a shift/step pair as in the ADSR registers.
#[derive(Clone, Copy)]
pub struct Rate {
    pub shift: u32,
    pub step: i32,
    pub decreasing: bool,
    pub exponential: bool,
}

/// Advance an envelope level by one sample. `counter` accumulates until it
/// reaches 0x8000, at which point the level moves by one step.
pub fn tick_level(level: i32, counter: &mut u32, rate: Rate) -> i32 {
    let mut increment = 0x8000u32 >> rate.shift.saturating_sub(11);
    let mut step = rate.step << 11u32.saturating_sub(rate.shift);
    if rate.exponential && !rate.decreasing && level > 0x6000 {
        increment >>= 2;
    }
    if rate.exponential && rate.decreasing {
        step = (step * level) >> 15;
    }
    *counter += increment;
    if *counter & 0x8000 == 0 {
        return level;
    }
    *counter = 0;
    (level + step).clamp(0, 0x7FFF)
}

/// A volume register: either a fixed value or a sweep towards a target.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct Volume {
    level: i32,
    counter: u32,
}

impl Volume {
    /// Current volume (signed, 0x4000 = 1.0), after advancing one sample.
    pub fn tick(&mut self, reg: u16) -> i32 {
        if reg & 0x8000 == 0 {
            // Fixed volume: a signed 15-bit value stored doubled.
            let fixed = ((reg & 0x7FFF) << 1) as i16 as i32;
            self.level = fixed.abs();
            return fixed;
        }
        let decreasing = reg & 0x2000 != 0;
        let raw = (reg & 3) as i32;
        let rate = Rate {
            shift: ((reg >> 2) & 0x1F) as u32,
            step: if decreasing { -8 + raw } else { 7 - raw },
            decreasing,
            exponential: reg & 0x4000 != 0,
        };
        self.level = tick_level(self.level, &mut self.counter, rate);
        if reg & 0x1000 != 0 {
            -self.level
        } else {
            self.level
        }
    }
}

/// Registers of a voice that the mixer reads every sample.
pub struct VoiceRegs {
    pub volume_left: u16,
    pub volume_right: u16,
    pub pitch: u16,
    pub adsr_low: u16,
    pub adsr_high: u16,
}

#[derive(Serialize, Deserialize)]
pub struct Voice {
    pub phase: Phase,
    /// ADSR level, 0..=0x7FFF.
    pub level: i32,
    envelope_counter: u32,
    current_address: usize,
    pub repeat_address: usize,
    block: [i16; BLOCK_SAMPLES],
    /// Flags byte of the block in `block`.
    block_flags: u8,
    /// Index of the next sample of `block` to enter the window.
    next_index: usize,
    /// The newest four samples, oldest first.
    window: [i16; 4],
    /// Fractional position between `window[1]` and `window[2]`, in 1/4096.
    fraction: u32,
    old1: i32,
    old2: i32,
    volume_left: Volume,
    volume_right: Volume,
    /// Output of the voice before volume, for pitch modulation.
    pub last_output: i16,
}

/// What a voice reports back to the SPU for one sample.
#[derive(Default, Clone, Copy)]
pub struct VoiceOutput {
    pub left: i32,
    pub right: i32,
    /// The voice reached an end-of-loop flag this sample.
    pub loop_end: bool,
}

/// Raise the SPU interrupt when sound RAM is touched at the watched address.
pub struct IrqWatch {
    pub address: Option<usize>,
    pub hit: bool,
}

impl IrqWatch {
    pub fn check(&mut self, start: usize, len: usize) {
        if let Some(address) = self.address {
            if address >= start && address < start + len {
                self.hit = true;
            }
        }
    }
}

impl Voice {
    pub fn new() -> Self {
        Self {
            phase: Phase::Off,
            level: 0,
            envelope_counter: 0,
            current_address: 0,
            repeat_address: 0,
            block: [0; BLOCK_SAMPLES],
            block_flags: 0,
            next_index: 0,
            window: [0; 4],
            fraction: 0,
            old1: 0,
            old2: 0,
            volume_left: Volume::default(),
            volume_right: Volume::default(),
            last_output: 0,
        }
    }

    pub fn key_on(&mut self, start_address: usize, ram: &[u8], irq: &mut IrqWatch) {
        self.current_address = start_address & SOUND_RAM_MASK & !(BLOCK_BYTES - 1);
        self.repeat_address = self.current_address;
        self.phase = Phase::Attack;
        self.level = 0;
        self.envelope_counter = 0;
        self.old1 = 0;
        self.old2 = 0;
        self.window = [0; 4];
        self.fraction = 0;
        self.volume_left = Volume::default();
        self.volume_right = Volume::default();
        self.load_block(ram, irq);
    }

    pub fn key_off(&mut self) {
        if self.phase != Phase::Off {
            self.phase = Phase::Release;
            self.envelope_counter = 0;
        }
    }

    /// Decode the block at `current_address` into `block`.
    fn load_block(&mut self, ram: &[u8], irq: &mut IrqWatch) {
        let address = self.current_address;
        irq.check(address, BLOCK_BYTES);
        let header = ram[address];
        let flags = ram[address + 1];
        let shift = {
            let s = (header & 0xF) as u32;
            if s > 12 { 9 } else { s }
        };
        let filter = (((header >> 4) & 7) as usize).min(4);
        if flags & FLAG_LOOP_START != 0 {
            self.repeat_address = address;
        }
        self.block_flags = flags;
        for i in 0..BLOCK_SAMPLES {
            let byte = ram[address + 2 + i / 2];
            let nibble = (byte >> ((i & 1) * 4)) & 0xF;
            let mut sample = (((nibble as u16) << 12) as i16 as i32) >> shift;
            sample += (self.old1 * FILTER_POSITIVE[filter] + self.old2 * FILTER_NEGATIVE[filter] + 32) >> 6;
            let sample = sample.clamp(-0x8000, 0x7FFF);
            self.old2 = self.old1;
            self.old1 = sample;
            self.block[i] = sample as i16;
        }
        self.next_index = 0;
    }

    /// Move to the block after the one that just ended, honouring its flags.
    /// Returns whether it carried the loop-end flag.
    fn next_block(&mut self, ram: &[u8], irq: &mut IrqWatch) -> bool {
        let flags = self.block_flags;
        let loop_end = flags & FLAG_LOOP_END != 0;
        if loop_end {
            self.current_address = self.repeat_address;
            if flags & FLAG_LOOP_REPEAT == 0 {
                // One-shot sample: the voice is silenced.
                self.phase = Phase::Off;
                self.level = 0;
            }
        } else {
            self.current_address = (self.current_address + BLOCK_BYTES) & SOUND_RAM_MASK;
        }
        self.load_block(ram, irq);
        loop_end
    }

    /// The SPU's four-point Gaussian interpolation of the last four samples,
    /// at the phase given by bits 4-11 of the pitch counter.
    #[inline(always)]
    fn interpolate(&self) -> i32 {
        let phase = ((self.fraction >> 4) & 0xFF) as usize;
        let [oldest, older, old, new] = self.window.map(|s| s as i32);
        let mut out = (GAUSS[0xFF - phase] as i32 * oldest) >> 15;
        out += (GAUSS[0x1FF - phase] as i32 * older) >> 15;
        out += (GAUSS[0x100 + phase] as i32 * old) >> 15;
        out += (GAUSS[phase] as i32 * new) >> 15;
        out.clamp(-0x8000, 0x7FFF)
    }

    fn tick_envelope(&mut self, regs: &VoiceRegs) {
        let lo = regs.adsr_low;
        let hi = regs.adsr_high;
        match self.phase {
            Phase::Off => {}
            Phase::Attack => {
                let rate = Rate {
                    shift: ((lo >> 10) & 0x1F) as u32,
                    step: 7 - ((lo >> 8) & 3) as i32,
                    decreasing: false,
                    exponential: lo & 0x8000 != 0,
                };
                self.level = tick_level(self.level, &mut self.envelope_counter, rate);
                if self.level >= 0x7FFF {
                    self.phase = Phase::Decay;
                    self.envelope_counter = 0;
                }
            }
            Phase::Decay => {
                let rate = Rate {
                    shift: ((lo >> 4) & 0xF) as u32,
                    step: -8,
                    decreasing: true,
                    exponential: true,
                };
                self.level = tick_level(self.level, &mut self.envelope_counter, rate);
                let sustain_level = ((lo & 0xF) as i32 + 1) * 0x800;
                if self.level <= sustain_level {
                    self.phase = Phase::Sustain;
                    self.envelope_counter = 0;
                }
            }
            Phase::Sustain => {
                let decreasing = hi & 0x4000 != 0;
                let raw = ((hi >> 6) & 3) as i32;
                let rate = Rate {
                    shift: ((hi >> 8) & 0x1F) as u32,
                    step: if decreasing { -8 + raw } else { 7 - raw },
                    decreasing,
                    exponential: hi & 0x8000 != 0,
                };
                self.level = tick_level(self.level, &mut self.envelope_counter, rate);
            }
            Phase::Release => {
                let rate = Rate {
                    shift: (hi & 0x1F) as u32,
                    step: -8,
                    decreasing: true,
                    exponential: hi & 0x20 != 0,
                };
                self.level = tick_level(self.level, &mut self.envelope_counter, rate);
                if self.level == 0 {
                    self.phase = Phase::Off;
                }
            }
        }
    }

    /// Produce one output sample. `noise` replaces the ADPCM data when the
    /// voice is set to noise; `modulation` is the previous voice's output when
    /// pitch modulation is on.
    pub fn tick(
        &mut self,
        regs: &VoiceRegs,
        noise: Option<i16>,
        modulation: Option<i16>,
        ram: &[u8],
        irq: &mut IrqWatch,
    ) -> VoiceOutput {
        let mut output = VoiceOutput::default();
        if self.phase == Phase::Off {
            self.last_output = 0;
            // Sweeping volumes keep running while the voice is silent.
            self.volume_left.tick(regs.volume_left);
            self.volume_right.tick(regs.volume_right);
            return output;
        }

        let raw = match noise {
            Some(n) => n as i32,
            None => self.interpolate(),
        };
        let enveloped = (raw * self.level) >> 15;
        self.last_output = enveloped as i16;
        let left = self.volume_left.tick(regs.volume_left);
        let right = self.volume_right.tick(regs.volume_right);
        output.left = (enveloped * left) >> 15;
        output.right = (enveloped * right) >> 15;

        self.tick_envelope(regs);

        // Advance the pitch counter.
        let mut step = (regs.pitch as i32).min(0x3FFF);
        if let Some(previous) = modulation {
            let factor = previous as i32 + 0x8000;
            step = ((step * factor) >> 15).clamp(0, 0x3FFF);
        }
        self.fraction += step as u32;
        while self.fraction >= 0x1000 {
            self.fraction -= 0x1000;
            let sample = self.block[self.next_index];
            self.window = [self.window[1], self.window[2], self.window[3], sample];
            self.next_index += 1;
            if self.next_index == BLOCK_SAMPLES && self.next_block(ram, irq) {
                output.loop_end = true;
            }
            if self.phase == Phase::Off {
                break;
            }
        }
        output
    }
}

impl Default for Voice {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ram_with_block(header: u8, flags: u8, nibbles: &[u8]) -> Vec<u8> {
        let mut ram = vec![0u8; SOUND_RAM_SIZE];
        ram[0x100] = header;
        ram[0x101] = flags;
        for (i, n) in nibbles.iter().enumerate() {
            ram[0x102 + i / 2] |= (n & 0xF) << ((i & 1) * 4);
        }
        ram
    }

    fn watch() -> IrqWatch {
        IrqWatch { address: None, hit: false }
    }

    fn regs(pitch: u16) -> VoiceRegs {
        VoiceRegs { volume_left: 0x3FFF, volume_right: 0x3FFF, pitch, adsr_low: 0x000F, adsr_high: 0 }
    }

    #[test]
    fn adpcm_nibbles_are_sign_extended_and_shifted() {
        let ram = ram_with_block(0x0C, 0, &[1, 7, 0xF, 8]); // shift 12, filter 0
        let mut voice = Voice::new();
        voice.key_on(0x100, &ram, &mut watch());
        assert_eq!(&voice.block[..4], &[1, 7, -1, -8]);
        let ram = ram_with_block(0x08, 0, &[1, 0xF]); // shift 8
        voice.key_on(0x100, &ram, &mut watch());
        assert_eq!(&voice.block[..2], &[16, -16]);
    }

    #[test]
    fn adpcm_filter_adds_the_previous_samples() {
        // Shift 0 gives 4096 per nibble step; filter 1 adds 60/64 of the last sample.
        let ram = ram_with_block(0x10, 0, &[1, 0, 0]);
        let mut voice = Voice::new();
        voice.key_on(0x100, &ram, &mut watch());
        assert_eq!(voice.block[0], 4096);
        let second = ((4096i32 * 60 + 32) >> 6) as i16;
        let third = ((second as i32 * 60 + 32) >> 6) as i16;
        assert_eq!(voice.block[1], second);
        assert_eq!(voice.block[2], third);
    }

    #[test]
    fn out_of_range_shift_is_treated_as_nine() {
        let ram = ram_with_block(0x0F, 0, &[1]);
        let mut voice = Voice::new();
        voice.key_on(0x100, &ram, &mut watch());
        assert_eq!(voice.block[0], 4096 >> 9);
    }

    #[test]
    fn linear_envelope_rises_by_the_step_every_sample() {
        let mut counter = 0;
        let rate = Rate { shift: 0, step: 7, decreasing: false, exponential: false };
        let a = tick_level(0, &mut counter, rate);
        assert_eq!(a, 7 << 11);
        let b = tick_level(a, &mut counter, rate);
        assert_eq!(b, 14 << 11);
        assert_eq!(tick_level(b, &mut counter, rate), 0x7FFF, "clamped at the top");
    }

    #[test]
    fn slow_envelopes_wait_between_steps() {
        let mut counter = 0;
        let rate = Rate { shift: 13, step: 7, decreasing: false, exponential: false };
        // 0x8000 >> 2 per sample: one step every four samples.
        let levels: Vec<i32> = (0..8)
            .scan(0, |level, _| {
                *level = tick_level(*level, &mut counter, rate);
                Some(*level)
            })
            .collect();
        assert_eq!(levels, vec![0, 0, 0, 7, 7, 7, 7, 14]);
    }

    #[test]
    fn exponential_decrease_scales_with_the_level() {
        let mut counter = 0;
        let rate = Rate { shift: 0, step: -8, decreasing: true, exponential: true };
        assert_eq!(tick_level(0x4000, &mut counter, rate), 0x4000 - 0x2000);
        assert_eq!(tick_level(0x7FFF, &mut counter, rate), 0x3FFF);
    }

    #[test]
    fn the_highest_shift_never_moves() {
        let mut counter = 0;
        let rate = Rate { shift: 31, step: 3, decreasing: false, exponential: false };
        for _ in 0..1000 {
            assert_eq!(tick_level(100, &mut counter, rate), 100);
        }
    }

    #[test]
    fn fixed_volumes_are_doubled_signed_15_bit_values() {
        let mut v = Volume::default();
        assert_eq!(v.tick(0x3FFF), 0x7FFE);
        assert_eq!(v.tick(0x1000), 0x2000);
        assert_eq!(v.tick(0x4000), -0x8000, "bit 14 is the sign");
    }

    #[test]
    fn sweeping_volumes_move_towards_the_target() {
        let mut v = Volume::default();
        // Sweep mode, linear, increasing, shift 0, step 0: +7 << 11 each sample.
        let reg = 0x8000;
        assert_eq!(v.tick(reg), 7 << 11);
        assert_eq!(v.tick(reg), 14 << 11);
        // Phase invert flips the sign.
        assert!(v.tick(reg | 0x1000) < 0);
    }

    #[test]
    fn loop_end_without_repeat_silences_the_voice() {
        let ram = ram_with_block(0x00, FLAG_LOOP_END, &[7; 28]);
        let mut voice = Voice::new();
        voice.key_on(0x100, &ram, &mut watch());
        let mut ended = false;
        for _ in 0..40 {
            ended |= voice.tick(&regs(0x1000), None, None, &ram, &mut watch()).loop_end;
        }
        assert!(ended);
        assert_eq!(voice.phase, Phase::Off);
    }

    #[test]
    fn loop_start_moves_the_repeat_address() {
        let mut ram = ram_with_block(0x00, FLAG_LOOP_START, &[7; 28]);
        // Next block: loop end + repeat, back to the loop start.
        ram[0x110] = 0;
        ram[0x111] = FLAG_LOOP_END | FLAG_LOOP_REPEAT;
        let mut voice = Voice::new();
        voice.key_on(0x100, &ram, &mut watch());
        assert_eq!(voice.repeat_address, 0x100);
        for _ in 0..70 {
            voice.tick(&regs(0x1000), None, None, &ram, &mut watch());
        }
        assert_ne!(voice.phase, Phase::Off, "the loop keeps playing");
        assert_eq!(voice.repeat_address, 0x100);
    }

    #[test]
    fn decay_stops_at_the_sustain_level() {
        let ram = ram_with_block(0x00, FLAG_LOOP_END | FLAG_LOOP_REPEAT, &[7; 28]);
        let mut voice = Voice::new();
        voice.key_on(0x100, &ram, &mut watch());
        // Fast attack, decay shift 0, sustain level 3 -> (3 + 1) * 0x800.
        let regs = VoiceRegs { volume_left: 0, volume_right: 0, pitch: 0x1000, adsr_low: 0x0003, adsr_high: 0xC000 };
        for _ in 0..50 {
            voice.tick(&regs, None, None, &ram, &mut watch());
        }
        assert_eq!(voice.phase, Phase::Sustain);
        assert!(voice.level <= 0x2000, "{:#X}", voice.level);
    }

    #[test]
    fn the_watched_address_flags_an_irq() {
        let ram = ram_with_block(0x00, 0, &[0; 28]);
        let mut voice = Voice::new();
        let mut irq = IrqWatch { address: Some(0x108), hit: false };
        voice.key_on(0x100, &ram, &mut irq);
        assert!(irq.hit);
        let mut other = IrqWatch { address: Some(0x110), hit: false };
        voice.key_on(0x100, &ram, &mut other);
        assert!(!other.hit);
    }
}
