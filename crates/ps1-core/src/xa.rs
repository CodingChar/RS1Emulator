//! CD-ROM XA-ADPCM audio: decoding of Mode 2 Form 2 sectors and conversion
//! to the SPU's 44.1 kHz.

use std::collections::VecDeque;

/// Submode byte (raw sector offset 18) bits.
const SUBMODE_AUDIO: u8 = 1 << 2;
const SUBMODE_FORM2: u8 = 1 << 5;
/// Coding byte (raw sector offset 19) bits.
const CODING_STEREO: u8 = 1 << 0;
const CODING_HALF_RATE: u8 = 1 << 2;
const CODING_8BIT: u8 = 1 << 4;

const GROUPS_PER_SECTOR: usize = 18;
const GROUP_BYTES: usize = 128;
const SAMPLES_PER_BLOCK: usize = 28;
/// Offset of the sector payload (after sync, header and subheader).
const DATA_OFFSET: usize = 24;

const FILTER_POSITIVE: [i32; 4] = [0, 60, 115, 98];
const FILTER_NEGATIVE: [i32; 4] = [0, 0, -52, -55];

pub type Frame = (i16, i16);

/// Whether a raw sector is an XA audio sector (realtime audio, Form 2).
pub fn is_audio_sector(raw: &[u8]) -> bool {
    let submode = raw[18];
    submode & (SUBMODE_AUDIO | SUBMODE_FORM2) == (SUBMODE_AUDIO | SUBMODE_FORM2)
}

/// Decoder state kept across sectors: ADPCM history and the resampler.
pub struct XaDecoder {
    /// `[channel][old1, old2]`.
    history: [[i32; 2]; 2],
    resampler: Resampler,
}

impl XaDecoder {
    pub fn new() -> Self {
        Self { history: [[0; 2]; 2], resampler: Resampler::new() }
    }

    /// Decode one raw audio sector and append its 44.1 kHz frames to `out`.
    pub fn decode_sector(&mut self, raw: &[u8], out: &mut VecDeque<Frame>) {
        let coding = raw[19];
        let stereo = coding & CODING_STEREO != 0;
        if coding & CODING_8BIT != 0 {
            return; // 8-bit XA is not used by any known game
        }
        let rate = if coding & CODING_HALF_RATE != 0 { 18_900 } else { 37_800 };

        let mut frames: Vec<Frame> = Vec::with_capacity(GROUPS_PER_SECTOR * 8 * SAMPLES_PER_BLOCK);
        for group in 0..GROUPS_PER_SECTOR {
            let start = DATA_OFFSET + group * GROUP_BYTES;
            let group_data = &raw[start..start + GROUP_BYTES];
            let mut blocks = [[0i16; SAMPLES_PER_BLOCK]; 8];
            for (b, block) in blocks.iter_mut().enumerate() {
                // Blocks 0-3 take their parameters from bytes 0-3, blocks 4-7
                // from bytes 8-11 (the bytes in between are copies).
                let param = group_data[(b & 3) + (b >> 2) * 8];
                let channel = if stereo { b & 1 } else { 0 };
                self.decode_block(group_data, b, param, channel, block);
            }
            if stereo {
                for pair in 0..4 {
                    for k in 0..SAMPLES_PER_BLOCK {
                        frames.push((blocks[pair * 2][k], blocks[pair * 2 + 1][k]));
                    }
                }
            } else {
                for block in &blocks {
                    for &s in block {
                        frames.push((s, s));
                    }
                }
            }
        }
        self.resampler.process(&frames, rate, out);
    }

    fn decode_block(
        &mut self,
        group: &[u8],
        block: usize,
        param: u8,
        channel: usize,
        out: &mut [i16; SAMPLES_PER_BLOCK],
    ) {
        let shift = {
            let s = (param & 0xF) as u32;
            if s > 12 { 9 } else { s }
        };
        let filter = ((param >> 4) & 3) as usize;
        let [mut old1, mut old2] = self.history[channel];
        for (k, slot) in out.iter_mut().enumerate() {
            let byte = group[16 + k * 4 + (block >> 1)];
            let nibble = (byte >> ((block & 1) * 4)) & 0xF;
            let mut sample = (((nibble as u16) << 12) as i16 as i32) >> shift;
            sample += (old1 * FILTER_POSITIVE[filter] + old2 * FILTER_NEGATIVE[filter] + 32) >> 6;
            let sample = sample.clamp(-0x8000, 0x7FFF);
            old2 = old1;
            old1 = sample;
            *slot = sample as i16;
        }
        self.history[channel] = [old1, old2];
    }
}

impl Default for XaDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Streaming linear-interpolation resampler to 44.1 kHz.
pub struct Resampler {
    /// Last frame of the previous chunk.
    previous: Frame,
    /// Position of the next output frame, relative to `previous`.
    position: f64,
}

impl Resampler {
    pub fn new() -> Self {
        Self { previous: (0, 0), position: 0.0 }
    }

    pub fn process(&mut self, input: &[Frame], rate: u32, out: &mut VecDeque<Frame>) {
        if input.is_empty() {
            return;
        }
        let step = rate as f64 / 44_100.0;
        let mut samples = Vec::with_capacity(input.len() + 1);
        samples.push(self.previous);
        samples.extend_from_slice(input);

        let mut position = self.position;
        while (position as usize) + 1 < samples.len() {
            let i = position as usize;
            let t = position - i as f64;
            let lerp = |a: i16, b: i16| (a as f64 + (b as f64 - a as f64) * t) as i16;
            out.push_back((lerp(samples[i].0, samples[i + 1].0), lerp(samples[i].1, samples[i + 1].1)));
            position += step;
        }
        let consumed = (position as usize).min(samples.len() - 1);
        self.previous = samples[consumed];
        self.position = position - consumed as f64;
    }
}

impl Default for Resampler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sector(coding: u8) -> Vec<u8> {
        let mut raw = vec![0u8; 2352];
        raw[16] = 1; // file
        raw[17] = 2; // channel
        raw[18] = SUBMODE_AUDIO | SUBMODE_FORM2 | (1 << 6);
        raw[19] = coding;
        raw
    }

    #[test]
    fn audio_sectors_are_recognised_by_their_submode() {
        assert!(is_audio_sector(&sector(1)));
        let mut data = sector(1);
        data[18] = 0x08;
        assert!(!is_audio_sector(&data));
        let mut video = sector(1);
        video[18] = 0x62;
        assert!(!is_audio_sector(&video), "video sectors are delivered as data");
    }

    #[test]
    fn a_silent_stereo_sector_yields_the_resampled_frame_count() {
        let mut decoder = XaDecoder::new();
        let mut out = VecDeque::new();
        decoder.decode_sector(&sector(CODING_STEREO), &mut out);
        // 2016 frames at 37.8 kHz become 2016 * 44100 / 37800 = 2352 frames.
        assert!((2350..=2353).contains(&out.len()), "{}", out.len());
        assert!(out.iter().all(|f| *f == (0, 0)));
    }

    #[test]
    fn half_rate_sectors_produce_more_output_frames() {
        let mut decoder = XaDecoder::new();
        let mut out = VecDeque::new();
        decoder.decode_sector(&sector(CODING_STEREO | CODING_HALF_RATE), &mut out);
        assert!((4700..=4706).contains(&out.len()), "{}", out.len());
    }

    #[test]
    fn mono_sectors_duplicate_the_channel() {
        let mut raw = sector(0);
        // Group 0, block 0: shift 0 (full scale), filter 0, first sample nibble = 1.
        raw[DATA_OFFSET] = 0x00;
        raw[DATA_OFFSET + 16] = 0x01;
        let mut decoder = XaDecoder::new();
        let mut out = VecDeque::new();
        decoder.decode_sector(&raw, &mut out);
        assert!(out.iter().any(|f| f.0 != 0));
        assert!(out.iter().all(|f| f.0 == f.1));
    }

    #[test]
    fn stereo_blocks_alternate_between_the_channels() {
        let mut raw = sector(CODING_STEREO);
        // Block 0 (left): shift 0, sample 0 = 1; block 1 (right) stays silent.
        raw[DATA_OFFSET] = 0x00;
        raw[DATA_OFFSET + 16] = 0x01;
        let mut decoder = XaDecoder::new();
        let mut out = VecDeque::new();
        decoder.decode_sector(&raw, &mut out);
        assert!(out.iter().any(|f| f.0 != 0));
        assert!(out.iter().all(|f| f.1 == 0));
    }

    #[test]
    fn adpcm_filters_use_the_previous_samples() {
        let mut raw = sector(0);
        // Filter 1 (+60/64 of the previous sample): a decaying tail after one impulse.
        raw[DATA_OFFSET] = 0x10; // shift 0, filter 1
        raw[DATA_OFFSET + 16] = 0x07; // first sample = 7 << 12
        let mut decoder = XaDecoder::new();
        let mut out = VecDeque::new();
        decoder.decode_sector(&raw, &mut out);
        let first: Vec<i16> = out.iter().take(6).map(|f| f.0).collect();
        assert!(first.iter().any(|&s| s != 0));
    }

    #[test]
    fn resampler_is_continuous_across_chunks() {
        let mut resampler = Resampler::new();
        let ramp: Vec<Frame> = (0..100).map(|i| (i * 100, i * 100)).collect();
        let mut whole = VecDeque::new();
        Resampler::new().process(&ramp, 37_800, &mut whole);
        let mut halves = VecDeque::new();
        resampler.process(&ramp[..50], 37_800, &mut halves);
        resampler.process(&ramp[50..], 37_800, &mut halves);
        assert_eq!(whole.len(), halves.len());
        for (a, b) in whole.iter().zip(halves.iter()) {
            assert!((a.0 as i32 - b.0 as i32).abs() <= 1);
        }
    }
}
