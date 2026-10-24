//! Motion decoder (MDEC): decompresses the macroblocks of FMV videos.
//!
//! The CPU (usually through DMA channel 0) feeds a command word followed by
//! run-length coded halfwords; the MDEC dequantizes them, runs an inverse DCT
//! and converts the result to 4/8/15/24-bit pixels that DMA channel 1 reads
//! back, one 16x16 macroblock after another.
//!
//! Decoding completes instantly when the last input word arrives, instead of
//! taking the time the real chip needs.

use crate::io::extract;

/// Command in the top three bits of the command word.
const CMD_DECODE: u32 = 1;
const CMD_SET_QUANT: u32 = 2;
const CMD_SET_SCALE: u32 = 3;

// Status register bits.
const STAT_OUTPUT_EMPTY: u32 = 1 << 31;
const STAT_BUSY: u32 = 1 << 29;
const STAT_INPUT_REQUEST: u32 = 1 << 28;
const STAT_OUTPUT_REQUEST: u32 = 1 << 27;

// Control register bits.
const CTRL_RESET: u32 = 1 << 31;
const CTRL_ENABLE_INPUT: u32 = 1 << 30;
const CTRL_ENABLE_OUTPUT: u32 = 1 << 29;

/// Marks the end of a block, and is also used as padding before one.
const END_OF_BLOCK: u16 = 0xFE00;

/// Stream position (zigzag order) → natural position inside an 8x8 block.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27,
    20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58,
    59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Pixel format of the decoded data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Depth {
    Bits4,
    Bits8,
    Bits24,
    Bits15,
}

/// What the words following a command word are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Nothing,
    Decode { depth: Depth, signed: bool, bit15: bool },
    QuantTables,
    ScaleTable,
}

pub struct Mdec {
    /// Last command word, for the format bits of the status register.
    command: u32,
    pending: Pending,
    /// Parameter words still expected for the current command.
    remaining: u32,
    /// Parameter words received so far.
    params: Vec<u32>,
    /// Luma and chroma quantization tables, in natural (row-major) order.
    luma_quant: [u8; 64],
    chroma_quant: [u8; 64],
    /// IDCT matrix as signed 16-bit fixed point, `scale[frequency * 8 + x]`.
    scale: [i16; 64],
    /// Decoded output not yet read, and the position of the next word.
    output: Vec<u32>,
    output_pos: usize,
    enable_input: bool,
    enable_output: bool,
}

impl Mdec {
    pub fn new() -> Self {
        let mut mdec = Self {
            command: 0,
            pending: Pending::Nothing,
            remaining: 0,
            params: Vec::new(),
            luma_quant: [0; 64],
            chroma_quant: [0; 64],
            scale: [0; 64],
            output: Vec::new(),
            output_pos: 0,
            enable_input: false,
            enable_output: false,
        };
        mdec.reset();
        mdec
    }

    pub fn reset(&mut self) {
        self.command = 0;
        self.pending = Pending::Nothing;
        self.remaining = 0;
        self.params.clear();
        self.output.clear();
        self.output_pos = 0;
        self.enable_input = false;
        self.enable_output = false;
    }

    /// Words of decoded data waiting to be read.
    pub fn output_words(&self) -> usize {
        self.output.len() - self.output_pos
    }

    /// Register read; `off` is relative to `0x1F801820`.
    pub fn read(&mut self, off: u32, width: u32) -> u32 {
        let word = if off & 4 == 0 { self.read_data() } else { self.status() };
        extract(word, off, width)
    }

    /// Register write; `off` is relative to `0x1F801820`.
    pub fn write(&mut self, off: u32, _width: u32, value: u32) {
        if off & 4 == 0 {
            self.write_command(value);
        } else {
            self.write_control(value);
        }
    }

    /// Next word of decoded data (what DMA channel 1 reads).
    pub fn read_data(&mut self) -> u32 {
        let word = self.output.get(self.output_pos).copied().unwrap_or(0);
        if self.output_pos < self.output.len() {
            self.output_pos += 1;
            if self.output_pos == self.output.len() {
                self.output.clear();
                self.output_pos = 0;
            }
        }
        word
    }

    pub fn status(&self) -> u32 {
        // Output depth, signedness and bit 15 (command bits 25-28) sit in
        // status bits 23-26; the "current block" field idles at 4 (Cr).
        let mut stat = ((self.command >> 2) & 0x0780_0000) | (4 << 16);
        if self.output_words() == 0 {
            stat |= STAT_OUTPUT_EMPTY;
        }
        if self.pending != Pending::Nothing {
            stat |= STAT_BUSY;
        }
        if self.enable_input {
            stat |= STAT_INPUT_REQUEST;
        }
        if self.enable_output && self.output_words() > 0 {
            stat |= STAT_OUTPUT_REQUEST;
        }
        if self.remaining > 0 {
            stat |= (self.remaining - 1) & 0xFFFF;
        }
        stat
    }

    fn write_control(&mut self, value: u32) {
        if value & CTRL_RESET != 0 {
            self.reset();
        }
        self.enable_input = value & CTRL_ENABLE_INPUT != 0;
        self.enable_output = value & CTRL_ENABLE_OUTPUT != 0;
    }

    /// A word written to the command/parameter port (DMA channel 0 writes here).
    pub fn write_command(&mut self, word: u32) {
        match self.pending {
            Pending::Nothing => self.start_command(word),
            _ => {
                self.params.push(word);
                self.remaining -= 1;
                if self.remaining == 0 {
                    self.finish_command();
                }
            }
        }
    }

    fn start_command(&mut self, word: u32) {
        self.command = word;
        self.params.clear();
        let (pending, words) = match word >> 29 {
            CMD_DECODE => {
                let depth = match (word >> 27) & 3 {
                    0 => Depth::Bits4,
                    1 => Depth::Bits8,
                    2 => Depth::Bits24,
                    _ => Depth::Bits15,
                };
                let pending = Pending::Decode {
                    depth,
                    signed: word & (1 << 26) != 0,
                    bit15: word & (1 << 25) != 0,
                };
                (pending, word & 0xFFFF)
            }
            CMD_SET_QUANT => (Pending::QuantTables, if word & 1 != 0 { 32 } else { 16 }),
            CMD_SET_SCALE => (Pending::ScaleTable, 32),
            _ => (Pending::Nothing, 0),
        };
        if words > 0 {
            self.pending = pending;
            self.remaining = words;
        }
    }

    fn finish_command(&mut self) {
        let params = std::mem::take(&mut self.params);
        match self.pending {
            Pending::QuantTables => {
                for (i, word) in params.iter().enumerate() {
                    for b in 0..4 {
                        let byte = (word >> (8 * b)) as u8;
                        let index = i * 4 + b;
                        if index < 64 {
                            self.luma_quant[index] = byte;
                        } else {
                            self.chroma_quant[index - 64] = byte;
                        }
                    }
                }
            }
            Pending::ScaleTable => {
                for (i, word) in params.iter().enumerate() {
                    self.scale[i * 2] = *word as i16;
                    self.scale[i * 2 + 1] = (*word >> 16) as i16;
                }
            }
            Pending::Decode { depth, signed, bit15 } => {
                self.decode(&params, depth, signed, bit15);
            }
            Pending::Nothing => {}
        }
        self.pending = Pending::Nothing;
        self.remaining = 0;
    }

    /// Decode a whole bitstream of macroblocks into `output`.
    fn decode(&mut self, words: &[u32], depth: Depth, signed: bool, bit15: bool) {
        let halfwords: Vec<u16> = words
            .iter()
            .flat_map(|w| [*w as u16, (*w >> 16) as u16])
            .collect();
        let mut pos = 0;
        let mut bytes: Vec<u8> = Vec::new();

        loop {
            match depth {
                Depth::Bits4 | Depth::Bits8 => {
                    let Some(block) = self.decode_block(&halfwords, &mut pos, false) else { break };
                    let mut pixels = [0u8; 64];
                    for (pixel, value) in pixels.iter_mut().zip(block.iter()) {
                        let y = (*value).clamp(-128, 127);
                        *pixel = if signed { y as u8 } else { (y + 128) as u8 };
                    }
                    if depth == Depth::Bits8 {
                        bytes.extend_from_slice(&pixels);
                    } else {
                        for pair in pixels.chunks(2) {
                            bytes.push((pair[0] >> 4) | (pair[1] & 0xF0));
                        }
                    }
                }
                Depth::Bits24 | Depth::Bits15 => {
                    let Some(cr) = self.decode_block(&halfwords, &mut pos, true) else { break };
                    let Some(cb) = self.decode_block(&halfwords, &mut pos, true) else { break };
                    let mut luma = [[0i32; 64]; 4];
                    let mut complete = true;
                    for block in luma.iter_mut() {
                        match self.decode_block(&halfwords, &mut pos, false) {
                            Some(b) => *block = b,
                            None => {
                                complete = false;
                                break;
                            }
                        }
                    }
                    if !complete {
                        break;
                    }
                    Self::emit_macroblock(&mut bytes, &cr, &cb, &luma, depth, signed, bit15);
                }
            }
        }

        for chunk in bytes.chunks(4) {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            self.output.push(u32::from_le_bytes(word));
        }
    }

    /// Decode one 8x8 block (dequantize + IDCT). Returns `None` if the input
    /// ran out before the block started.
    fn decode_block(&self, input: &[u16], pos: &mut usize, chroma: bool) -> Option<[i32; 64]> {
        let quant = if chroma { &self.chroma_quant } else { &self.luma_quant };
        let mut coefficients = [0i32; 64];

        // Skip padding, then read the DC word.
        let word = loop {
            let w = *input.get(*pos)?;
            *pos += 1;
            if w != END_OF_BLOCK {
                break w;
            }
        };
        let scale = ((word >> 10) & 0x3F) as i32;
        let dc = if scale == 0 {
            sign_extend_10(word) * 2
        } else {
            sign_extend_10(word) * quant[0] as i32
        };
        coefficients[0] = dc.clamp(-0x400, 0x3FF);

        // Each further word is a run of zeros followed by one coefficient.
        let mut k = 0usize;
        while let Some(&word) = input.get(*pos) {
            *pos += 1;
            k += ((word >> 10) & 0x3F) as usize + 1;
            if k > 63 {
                break;
            }
            let position = ZIGZAG[k];
            let value = if scale == 0 {
                sign_extend_10(word) * 2
            } else {
                (sign_extend_10(word) * quant[position] as i32 * scale + 4) / 8
            };
            coefficients[position] = value.clamp(-0x400, 0x3FF);
        }
        Some(self.idct(&coefficients))
    }

    /// Two-pass inverse DCT using the uploaded scale table.
    fn idct(&self, coefficients: &[i32; 64]) -> [i32; 64] {
        let mut temp = [0i32; 64];
        for x in 0..8 {
            for y in 0..8 {
                let mut sum = 0i32;
                for u in 0..8 {
                    sum += coefficients[u * 8 + x] * self.scale[u * 8 + y] as i32;
                }
                temp[y * 8 + x] = (sum + 0x7FFF) >> 16;
            }
        }
        let mut out = [0i32; 64];
        for y in 0..8 {
            for x in 0..8 {
                let mut sum = 0i32;
                for u in 0..8 {
                    sum += temp[y * 8 + u] * self.scale[u * 8 + x] as i32;
                }
                let v = (sum + 0x7FFF) >> 16;
                // The chip keeps 9 bits; saturate to the signed 8-bit pixel range.
                out[y * 8 + x] = sign_extend_9(v).clamp(-128, 127);
            }
        }
        out
    }

    /// Convert one macroblock to RGB and append it, 16 pixels per row.
    fn emit_macroblock(
        bytes: &mut Vec<u8>,
        cr: &[i32; 64],
        cb: &[i32; 64],
        luma: &[[i32; 64]; 4],
        depth: Depth,
        signed: bool,
        bit15: bool,
    ) {
        let offset = if signed { 0 } else { 128 };
        for y in 0..16usize {
            for x in 0..16usize {
                let block = &luma[(y / 8) * 2 + x / 8];
                let luma_value = block[(y % 8) * 8 + x % 8];
                let chroma = (y / 2) * 8 + x / 2;
                let (r_c, b_c) = (cr[chroma], cb[chroma]);
                let r = luma_value + ((r_c * 91_881) >> 16);
                let g = luma_value - ((b_c * 22_554 + r_c * 46_802) >> 16);
                let b = luma_value + ((b_c * 116_130) >> 16);
                let r = (r.clamp(-128, 127) + offset) as u8;
                let g = (g.clamp(-128, 127) + offset) as u8;
                let b = (b.clamp(-128, 127) + offset) as u8;
                match depth {
                    Depth::Bits15 => {
                        let pixel = (r as u16 >> 3)
                            | ((g as u16 >> 3) << 5)
                            | ((b as u16 >> 3) << 10)
                            | if bit15 { 0x8000 } else { 0 };
                        bytes.extend_from_slice(&pixel.to_le_bytes());
                    }
                    _ => bytes.extend_from_slice(&[r, g, b]),
                }
            }
        }
    }
}

impl Default for Mdec {
    fn default() -> Self {
        Self::new()
    }
}

fn sign_extend_10(word: u16) -> i32 {
    (((word & 0x3FF) as i32) << 22) >> 22
}

fn sign_extend_9(value: i32) -> i32 {
    (value << 23) >> 23
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The IDCT matrix real games upload: `32768 * c(u) * cos((2x+1)u*pi/16)`
    /// with `c(0) = 1/sqrt(2)`.
    fn standard_scale() -> [i16; 64] {
        let mut table = [0i16; 64];
        for u in 0..8 {
            for x in 0..8 {
                let c = if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                let angle = (2 * x + 1) as f64 * u as f64 * std::f64::consts::PI / 16.0;
                table[u * 8 + x] = (32768.0 * c * angle.cos()).round() as i16;
            }
        }
        table
    }

    fn ready_mdec() -> Mdec {
        let mut mdec = Mdec::new();
        // Luma and chroma tables filled with 2.
        mdec.write_command((CMD_SET_QUANT << 29) | 1);
        for _ in 0..32 {
            mdec.write_command(0x0202_0202);
        }
        mdec.write_command(CMD_SET_SCALE << 29);
        let scale = standard_scale();
        for pair in scale.chunks(2) {
            mdec.write_command((pair[0] as u16 as u32) | ((pair[1] as u16 as u32) << 16));
        }
        mdec
    }

    /// Start a decode command and feed it the halfwords, two per word.
    fn decode(mdec: &mut Mdec, mode: u32, halfwords: &[u16]) {
        let words = halfwords.len().div_ceil(2) as u32;
        mdec.write_command((CMD_DECODE << 29) | (mode << 25) | words);
        for pair in halfwords.chunks(2) {
            let high = pair.get(1).copied().unwrap_or(END_OF_BLOCK);
            mdec.write_command(pair[0] as u32 | ((high as u32) << 16));
        }
    }

    fn read_bytes(mdec: &mut Mdec, count: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        while bytes.len() < count {
            bytes.extend_from_slice(&mdec.read_data().to_le_bytes());
        }
        bytes.truncate(count);
        bytes
    }

    /// A block holding only a DC coefficient (quantizer scale 1, table value 2).
    fn dc_block(dc: i32) -> [u16; 2] {
        [(1 << 10) | (dc as u16 & 0x3FF), END_OF_BLOCK]
    }

    // Mode bits of the command word, shifted down by 25: bit 0 = bit 15 flag,
    // bit 1 = signed, bits 2-3 = depth.
    const MODE_8BIT: u32 = 1 << 2;
    const MODE_24BIT: u32 = 2 << 2;
    const MODE_15BIT: u32 = 3 << 2;

    #[test]
    fn status_after_reset_is_idle() {
        let mdec = Mdec::new();
        assert_eq!(mdec.status(), 0x8004_0000);
    }

    #[test]
    fn status_reports_busy_and_remaining_parameters() {
        let mut mdec = Mdec::new();
        mdec.write_command(CMD_SET_SCALE << 29);
        let status = mdec.status();
        assert_ne!(status & STAT_BUSY, 0);
        assert_eq!(status & 0xFFFF, 31, "32 words expected, minus one");
        for _ in 0..32 {
            mdec.write_command(0);
        }
        assert_eq!(mdec.status() & STAT_BUSY, 0);
    }

    #[test]
    fn control_register_enables_requests_and_resets() {
        let mut mdec = ready_mdec();
        mdec.write(4, 4, CTRL_ENABLE_INPUT | CTRL_ENABLE_OUTPUT);
        assert_ne!(mdec.status() & STAT_INPUT_REQUEST, 0);
        decode(&mut mdec, MODE_8BIT, &dc_block(16));
        assert_ne!(mdec.status() & STAT_OUTPUT_REQUEST, 0);
        assert_eq!(mdec.status() & STAT_OUTPUT_EMPTY, 0);
        mdec.write(4, 4, CTRL_RESET);
        assert_eq!(mdec.status(), 0x8004_0000);
        assert_eq!(mdec.output_words(), 0);
    }

    #[test]
    fn dc_only_block_decodes_to_a_flat_8bit_image() {
        let mut mdec = ready_mdec();
        // DC 32 * quant 2 = 64; the IDCT divides by 8: luma 8, shown as 136.
        decode(&mut mdec, MODE_8BIT, &dc_block(32));
        assert_eq!(mdec.output_words(), 16);
        assert!(read_bytes(&mut mdec, 64).iter().all(|&p| p == 136));
        assert_eq!(mdec.output_words(), 0);
    }

    #[test]
    fn zero_quantizer_scale_uses_the_raw_coefficient_times_two() {
        let mut mdec = ready_mdec();
        // qscale 0: value = 32 * 2 = 64, again luma 8.
        decode(&mut mdec, MODE_8BIT, &[32, END_OF_BLOCK]);
        assert!(read_bytes(&mut mdec, 64).iter().all(|&p| p == 136));
    }

    #[test]
    fn padding_words_before_a_block_are_skipped() {
        let mut mdec = ready_mdec();
        decode(&mut mdec, MODE_8BIT, &[END_OF_BLOCK, END_OF_BLOCK, 32, END_OF_BLOCK]);
        assert_eq!(read_bytes(&mut mdec, 1), vec![136]);
    }

    #[test]
    fn an_ac_coefficient_varies_along_x_only() {
        let mut mdec = ready_mdec();
        // Zigzag position 1 is the first horizontal frequency.
        // qscale 0 so the coefficient is 2 * 200 = 400 (run 0).
        decode(&mut mdec, MODE_8BIT, &[0, 200, END_OF_BLOCK]);
        let pixels = read_bytes(&mut mdec, 64);
        for y in 1..8 {
            assert_eq!(pixels[y * 8..y * 8 + 8], pixels[0..8], "row {y} equals row 0");
        }
        assert!(pixels[0] > 128 && pixels[7] < 128, "left bright, right dark: {pixels:?}");
    }

    #[test]
    fn a_run_skips_zigzag_positions() {
        let mut mdec = ready_mdec();
        // Run 1 lands on zigzag position 2, which is the first vertical frequency.
        decode(&mut mdec, MODE_8BIT, &[0, (1 << 10) | 200, END_OF_BLOCK]);
        let pixels = read_bytes(&mut mdec, 64);
        for x in 1..8 {
            assert_eq!(pixels[x], pixels[0], "column {x} equals column 0");
        }
        assert!(pixels[0] > 128 && pixels[56] < 128, "top bright, bottom dark");
    }

    #[test]
    fn grey_macroblock_in_24_bit_is_three_bytes_per_pixel() {
        let mut mdec = ready_mdec();
        let mut stream = Vec::new();
        stream.extend_from_slice(&dc_block(0)); // Cr
        stream.extend_from_slice(&dc_block(0)); // Cb
        for y in [16, 32, 48, 64] {
            stream.extend_from_slice(&dc_block(y)); // Y1..Y4: 2*y/8 above mid
        }
        decode(&mut mdec, MODE_24BIT, &stream);
        assert_eq!(mdec.output_words(), 192, "16x16x3 bytes");
        let bytes = read_bytes(&mut mdec, 768);
        let pixel = |x: usize, y: usize| bytes[(y * 16 + x) * 3..(y * 16 + x) * 3 + 3].to_vec();
        assert_eq!(pixel(0, 0), vec![132, 132, 132], "Y1 top-left");
        assert_eq!(pixel(15, 0), vec![136, 136, 136], "Y2 top-right");
        assert_eq!(pixel(0, 15), vec![140, 140, 140], "Y3 bottom-left");
        assert_eq!(pixel(15, 15), vec![144, 144, 144], "Y4 bottom-right");
    }

    #[test]
    fn chroma_tints_the_pixels() {
        let mut mdec = ready_mdec();
        let mut stream = Vec::new();
        stream.extend_from_slice(&dc_block(64)); // Cr = +16
        stream.extend_from_slice(&dc_block(-64)); // Cb = -16
        for _ in 0..4 {
            stream.extend_from_slice(&dc_block(0));
        }
        decode(&mut mdec, MODE_24BIT, &stream);
        let bytes = read_bytes(&mut mdec, 3);
        let (r, g, b) = (bytes[0] as i32, bytes[1] as i32, bytes[2] as i32);
        assert!(r > 128 && b < 128, "red up, blue down: {r} {g} {b}");
        assert!(g > b, "green above blue");
    }

    #[test]
    fn fifteen_bit_output_packs_rgb555_with_the_bit15_flag() {
        let mut mdec = ready_mdec();
        let mut stream = Vec::new();
        for _ in 0..6 {
            stream.extend_from_slice(&dc_block(0));
        }
        // 15-bit, unsigned, bit 15 set.
        decode(&mut mdec, MODE_15BIT | 1, &stream);
        assert_eq!(mdec.output_words(), 128, "16x16x2 bytes");
        let pixel = mdec.read_data() as u16;
        assert_eq!(pixel & 0x7FFF, (16 << 10) | (16 << 5) | 16);
        assert_ne!(pixel & 0x8000, 0);
    }

    #[test]
    fn signed_output_skips_the_128_offset() {
        let mut mdec = ready_mdec();
        decode(&mut mdec, MODE_8BIT | 2, &dc_block(32));
        assert_eq!(read_bytes(&mut mdec, 1), vec![8]);
    }

    #[test]
    fn truncated_macroblocks_are_discarded() {
        let mut mdec = ready_mdec();
        let mut stream = Vec::new();
        for _ in 0..3 {
            stream.extend_from_slice(&dc_block(0));
        }
        decode(&mut mdec, MODE_24BIT, &stream);
        assert_eq!(mdec.output_words(), 0);
    }

    #[test]
    fn several_macroblocks_come_out_back_to_back() {
        let mut mdec = ready_mdec();
        let mut stream = Vec::new();
        for _ in 0..12 {
            stream.extend_from_slice(&dc_block(0));
        }
        decode(&mut mdec, MODE_24BIT, &stream);
        assert_eq!(mdec.output_words(), 384);
    }
}
