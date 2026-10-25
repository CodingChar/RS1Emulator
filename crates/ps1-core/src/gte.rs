//! Geometry Transformation Engine (GTE), coprocessor 2.
//!
//! The GTE does the fixed-point vector math 3D games lean on: rotation and
//! translation, perspective projection, lighting and colour interpolation.
//! It has 32 data registers and 32 control registers, and executes commands
//! encoded in the COP2 instruction.
//!
//! Mirrors DuckStation's `gte.cpp`.
use serde::{Deserialize, Serialize};

// ----- Data register indices (cop2r0-31) -----
const VXY0: usize = 0;
const VZ0: usize = 1;
const RGBC: usize = 6;
const OTZ: usize = 7;
const IR0: usize = 8;
const IR1: usize = 9;
const IR2: usize = 10;
const IR3: usize = 11;
const SXY0: usize = 12;
const SXY1: usize = 13;
const SXY2: usize = 14;
const SXYP: usize = 15;
const SZ0: usize = 16;
const SZ1: usize = 17;
const SZ2: usize = 18;
const SZ3: usize = 19;
const RGB0: usize = 20;
const RGB1: usize = 21;
const RGB2: usize = 22;
const MAC0: usize = 24;
const MAC1: usize = 25;
const IRGB: usize = 28;
const ORGB: usize = 29;
const LZCS: usize = 30;
const LZCR: usize = 31;

// ----- Control register indices (cop2r32-63, minus 32) -----
/// Rotation matrix: 3x3 s16 packed two per register.
const MATRIX_ROTATION: usize = 0;
const TR: usize = 5;
const MATRIX_LIGHT: usize = 8;
const BK: usize = 13;
const MATRIX_COLOR: usize = 16;
const FC: usize = 21;
const OFX: usize = 24;
const OFY: usize = 25;
const H: usize = 26;
const DQA: usize = 27;
const DQB: usize = 28;
const ZSF3: usize = 29;
const ZSF4: usize = 30;
const FLAG: usize = 31;

// ----- FLAG bits -----
const FLAG_DIVIDE_OVERFLOW: u32 = 1 << 17;
const FLAG_SZ_OTZ_SATURATED: u32 = 1 << 18;
const FLAG_MAC0_POSITIVE: u32 = 1 << 16;
const FLAG_MAC0_NEGATIVE: u32 = 1 << 15;
const FLAG_SX_SATURATED: u32 = 1 << 14;
const FLAG_SY_SATURATED: u32 = 1 << 13;
const FLAG_IR0_SATURATED: u32 = 1 << 12;
/// Bits whose assertion also sets the summary bit 31.
const FLAG_ERROR_MASK: u32 = 0x7F87_E000;
/// Bits software can write.
const FLAG_WRITE_MASK: u32 = 0x7FFF_F000;

/// MAC1..3 are 44-bit accumulators.
const MAC_MAX: i64 = (1 << 43) - 1;
const MAC_MIN: i64 = -(1 << 43);

/// Reciprocal seed table for the perspective divide:
/// `max(0, (0x40000 / (i + 0x100) + 1) / 2 - 0x101)`.
const UNR_TABLE: [u8; 0x101] = {
    let mut table = [0u8; 0x101];
    let mut i = 0;
    while i <= 0x100 {
        let value = (0x40000 / (i as i32 + 0x100) + 1) / 2 - 0x101;
        table[i] = if value < 0 { 0 } else { value as u8 };
        i += 1;
    }
    table
};

type Matrix = [[i32; 3]; 3];
type Vector = [i32; 3];

/// The Geometry Transformation Engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gte {
    /// Data registers (`cop2r0`-`cop2r31`), stored as written.
    pub data: [u32; 32],
    /// Control registers (`cop2r32`-`cop2r63`), stored as written.
    pub control: [u32; 32],
}

impl Gte {
    pub fn new() -> Self {
        Self { data: [0; 32], control: [0; 32] }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    // ----- Register access (MFC2 / MTC2 / CFC2 / CTC2) -----

    /// Read a data register (`MFC2`, `SWC2`).
    pub fn read_data(&self, reg: usize) -> u32 {
        match reg & 31 {
            // 16-bit signed registers read back sign-extended.
            reg @ (1 | 3 | 5 | 8..=11) => self.data[reg] as i16 as i32 as u32,
            // 16-bit unsigned registers.
            reg @ (OTZ | 16..=19) => self.data[reg] & 0xFFFF,
            // Reading SXYP returns the newest screen coordinate.
            SXYP => self.data[SXY2],
            IRGB | ORGB => self.packed_ir_color(),
            LZCR => self.leading_count(),
            reg => self.data[reg],
        }
    }

    /// Write a data register (`MTC2`, `LWC2`).
    pub fn write_data(&mut self, reg: usize, value: u32) {
        match reg & 31 {
            // Writing SXYP pushes onto the screen coordinate FIFO.
            SXYP => {
                self.data[SXY0] = self.data[SXY1];
                self.data[SXY1] = self.data[SXY2];
                self.data[SXY2] = value;
            }
            // Writing IRGB expands 5-bit colour components into IR1..3.
            IRGB => {
                for i in 0..3 {
                    let component = (value >> (5 * i)) & 0x1F;
                    self.data[IR1 + i] = component * 0x80;
                }
            }
            reg @ (OTZ | 16..=19) => self.data[reg] = value & 0xFFFF,
            // Read-only registers ignore writes.
            ORGB | LZCR => {}
            reg => self.data[reg] = value,
        }
    }

    /// Read a control register (`CFC2`).
    pub fn read_control(&self, reg: usize) -> u32 {
        match reg & 31 {
            // Registers that hold a single signed 16-bit value.
            reg @ (4 | 12 | 20 | H | DQA | ZSF3 | ZSF4) => self.control[reg] as i16 as i32 as u32,
            FLAG => {
                let flag = self.control[FLAG];
                flag | if flag & FLAG_ERROR_MASK != 0 { 1 << 31 } else { 0 }
            }
            reg => self.control[reg],
        }
    }

    /// Write a control register (`CTC2`).
    pub fn write_control(&mut self, reg: usize, value: u32) {
        match reg & 31 {
            FLAG => self.control[FLAG] = value & FLAG_WRITE_MASK,
            reg => self.control[reg] = value,
        }
    }

    /// ORGB: IR1..3 divided by 0x80 and clamped to 5 bits each.
    fn packed_ir_color(&self) -> u32 {
        (0..3).fold(0u32, |acc, i| {
            let ir = self.data[IR1 + i] as i16 as i32;
            acc | (((ir >> 7).clamp(0, 0x1F) as u32) << (5 * i))
        })
    }

    /// LZCR: number of leading bits equal to the sign bit of LZCS.
    fn leading_count(&self) -> u32 {
        let value = self.data[LZCS];
        if value & 0x8000_0000 != 0 {
            (!value).leading_zeros()
        } else {
            value.leading_zeros()
        }
    }

    // ----- Typed views of the registers -----

    fn half(&self, reg: u32, high: bool) -> i32 {
        (if high { (reg >> 16) as i16 } else { reg as i16 }) as i32
    }

    fn matrix(&self, base: usize) -> Matrix {
        let mut m = [[0; 3]; 3];
        for k in 0..9 {
            m[k / 3][k % 3] = self.half(self.control[base + k / 2], k % 2 == 1);
        }
        m
    }

    /// Vector 0..2 are V0..V2; 3 is (IR1, IR2, IR3).
    fn vector(&self, index: usize) -> Vector {
        match index {
            0..=2 => [
                self.half(self.data[VXY0 + 2 * index], false),
                self.half(self.data[VXY0 + 2 * index], true),
                self.half(self.data[VZ0 + 2 * index], false),
            ],
            _ => self.ir(),
        }
    }

    fn ir(&self) -> Vector {
        [
            self.data[IR1] as i16 as i32,
            self.data[IR1 + 1] as i16 as i32,
            self.data[IR1 + 2] as i16 as i32,
        ]
    }

    fn control_vector(&self, base: usize) -> Vector {
        [
            self.control[base] as i32,
            self.control[base + 1] as i32,
            self.control[base + 2] as i32,
        ]
    }

    /// Red, green and blue bytes of RGBC.
    fn rgb(&self) -> [i64; 3] {
        let rgbc = self.data[RGBC];
        [(rgbc & 0xFF) as i64, ((rgbc >> 8) & 0xFF) as i64, ((rgbc >> 16) & 0xFF) as i64]
    }

    // ----- Saturation helpers (they raise FLAG bits) -----

    fn set_flag(&mut self, bits: u32) {
        self.control[FLAG] |= bits;
    }

    /// Check a 44-bit accumulator for overflow; `n` is 1..=3.
    fn check_mac(&mut self, n: usize, value: i64) -> i64 {
        if value > MAC_MAX {
            self.set_flag(1 << (31 - n));
        } else if value < MAC_MIN {
            self.set_flag(1 << (28 - n));
        }
        // The accumulator wraps to 44 bits.
        (value << 20) >> 20
    }

    fn set_mac0(&mut self, value: i64) {
        if value > i32::MAX as i64 {
            self.set_flag(FLAG_MAC0_POSITIVE);
        } else if value < i32::MIN as i64 {
            self.set_flag(FLAG_MAC0_NEGATIVE);
        }
        self.data[MAC0] = value as i32 as u32;
    }

    /// Clamp `mac` into IRn (`n` in 1..=3), raising the saturation flag.
    fn set_ir(&mut self, n: usize, mac: i32, lm: bool) {
        let min = if lm { 0 } else { -0x8000 };
        let ir = if mac > 0x7FFF {
            self.set_flag(1 << (25 - n));
            0x7FFF
        } else if mac < min {
            self.set_flag(1 << (25 - n));
            min
        } else {
            mac
        };
        self.data[IR1 - 1 + n] = ir as u32;
    }

    /// Store `accumulator >> shift` in MACn and clamp it into IRn.
    fn set_mac_ir(&mut self, n: usize, accumulator: i64, shift: u32, lm: bool) {
        let mac = (accumulator >> shift) as i32;
        self.data[MAC1 - 1 + n] = mac as u32;
        self.set_ir(n, mac, lm);
    }

    // ----- Building blocks -----

    /// `result = translation * 0x1000 + matrix * vector` for each row, with
    /// the 44-bit overflow check after every addition.
    fn multiply_matrix(&mut self, m: &Matrix, v: Vector, translation: Vector, shift: u32, lm: bool) {
        for i in 0..3 {
            let mut acc = (translation[i] as i64) << 12;
            for j in 0..3 {
                acc = self.check_mac(i + 1, acc + m[i][j] as i64 * v[j] as i64);
            }
            self.set_mac_ir(i + 1, acc, shift, lm);
        }
    }

    /// Same, with the far colour as translation. The hardware drops the
    /// first column's product and the translation from the result: they only
    /// contribute to the overflow and saturation flags.
    fn multiply_matrix_far_color(&mut self, m: &Matrix, v: Vector, shift: u32, lm: bool) {
        let far = self.control_vector(FC);
        for i in 0..3 {
            let partial = self.check_mac(i + 1, ((far[i] as i64) << 12) + m[i][0] as i64 * v[0] as i64);
            self.set_ir(i + 1, (partial >> shift) as i32, false);

            let mut acc = self.check_mac(i + 1, m[i][1] as i64 * v[1] as i64);
            acc = self.check_mac(i + 1, acc + m[i][2] as i64 * v[2] as i64);
            self.set_mac_ir(i + 1, acc, shift, lm);
        }
    }

    /// Push the colour in MAC1..3 (divided by 16, clamped to a byte) onto
    /// the colour FIFO, keeping the code byte of RGBC.
    fn push_color(&mut self) {
        let mut packed = self.data[RGBC] & 0xFF00_0000;
        for i in 0..3 {
            let mac = self.data[MAC1 + i] as i32;
            let component = mac >> 4;
            let clamped = if component < 0 || component > 0xFF {
                self.set_flag(1 << (21 - i));
                component.clamp(0, 0xFF)
            } else {
                component
            };
            packed |= (clamped as u32) << (8 * i);
        }
        self.data[RGB0] = self.data[RGB1];
        self.data[RGB1] = self.data[RGB2];
        self.data[RGB2] = packed;
    }

    /// Blend towards the far colour: `in + (FC - in) * IR0`.
    fn interpolate(&mut self, input: [i64; 3], shift: u32, lm: bool) {
        let far = self.control_vector(FC);
        let ir0 = self.data[IR0] as i16 as i64;
        for i in 0..3 {
            let difference = self.check_mac(i + 1, ((far[i] as i64) << 12) - input[i]);
            let mac = (difference >> shift) as i32;
            self.data[MAC1 + i] = mac as u32;
            self.set_ir(i + 1, mac, false);

            let ir = self.data[IR1 + i] as i16 as i64;
            let acc = self.check_mac(i + 1, ir * ir0 + input[i]);
            self.set_mac_ir(i + 1, acc, shift, lm);
        }
    }

    /// The reciprocal `H / SZ3` in 1.16 fixed point, clamped to 0x1FFFF
    /// (Newton-Raphson on a lookup table, as the hardware does).
    fn divide(&mut self, h: u32, sz3: u32) -> u32 {
        if h >= sz3 * 2 {
            self.set_flag(FLAG_DIVIDE_OVERFLOW);
            return 0x1FFFF;
        }
        let shift = (sz3 as u16).leading_zeros();
        let n = h << shift;
        let d = sz3 << shift;
        let u = UNR_TABLE[((d - 0x7FC0) >> 7) as usize] as u32 + 0x101;
        let d = (0x200_0080 - d * u) >> 8;
        let d = (0x80 + d * u) >> 8;
        (((n as u64 * d as u64) + 0x8000) >> 16).min(0x1FFFF) as u32
    }

    // ----- Commands -----

    /// Cycles a GTE command takes (psx-spx timing notes). The CPU goes on
    /// meanwhile and only waits if it touches the GTE before the command is done.
    pub fn latency(command: u32) -> u32 {
        match command & 0x3F {
            0x01 => 15, // RTPS
            0x06 => 8,  // NCLIP
            0x0C => 6,  // OP
            0x10 => 8,  // DPCS
            0x11 => 8,  // INTPL
            0x12 => 8,  // MVMVA
            0x13 => 19, // NCDS
            0x14 => 13, // CDP
            0x16 => 44, // NCDT
            0x1B => 17, // NCCS
            0x1C => 11, // CC
            0x1E => 14, // NCS
            0x20 => 30, // NCT
            0x28 => 5,  // SQR
            0x29 => 8,  // DCPL
            0x2A => 17, // DPCT
            0x2D => 5,  // AVSZ3
            0x2E => 6,  // AVSZ4
            0x30 => 23, // RTPT
            0x3D => 5,  // GPF
            0x3E => 5,  // GPL
            0x3F => 39, // NCCT
            _ => 1,
        }
    }

    /// Execute a GTE command (the low 25 bits of the COP2 instruction).
    pub fn execute(&mut self, command: u32) {
        self.control[FLAG] = 0;
        let shift = if command & (1 << 19) != 0 { 12 } else { 0 };
        let lm = command & (1 << 10) != 0;

        match command & 0x3F {
            0x01 => self.rtps(0, shift, lm),
            0x06 => self.nclip(),
            0x0C => self.op(shift, lm),
            0x10 => self.dpcs(shift, lm),
            0x11 => self.intpl(shift, lm),
            0x12 => self.mvmva(command, shift, lm),
            0x13 => self.ncds(0, shift, lm),
            0x14 => self.cdp(shift, lm),
            0x16 => (0..3).for_each(|v| self.ncds(v, shift, lm)),
            0x1B => self.nccs(0, shift, lm),
            0x1C => self.cc(shift, lm),
            0x1E => self.ncs(0, shift, lm),
            0x20 => (0..3).for_each(|v| self.ncs(v, shift, lm)),
            0x28 => self.sqr(shift, lm),
            0x29 => self.dcpl(shift, lm),
            0x2A => (0..3).for_each(|_| self.dpct(shift, lm)),
            0x2D => self.average_z(ZSF3, 3),
            0x2E => self.average_z(ZSF4, 4),
            0x30 => (0..3).for_each(|v| self.rtps(v, shift, lm)),
            0x3D => self.gpf(shift, lm),
            0x3E => self.gpl(shift, lm),
            0x3F => (0..3).for_each(|v| self.nccs(v, shift, lm)),
            // Undefined commands do nothing useful.
            _ => {}
        }
    }

    /// Perspective transformation of one vector (RTPS / each step of RTPT).
    fn rtps(&mut self, vector_index: usize, shift: u32, lm: bool) {
        let rotation = self.matrix(MATRIX_ROTATION);
        let v = self.vector(vector_index);
        let translation = self.control_vector(TR);

        let mut z = 0;
        for i in 0..3 {
            let mut acc = (translation[i] as i64) << 12;
            for j in 0..3 {
                acc = self.check_mac(i + 1, acc + rotation[i][j] as i64 * v[j] as i64);
            }
            if i < 2 {
                self.set_mac_ir(i + 1, acc, shift, lm);
            } else {
                // IR3's saturation flag looks at the value shifted by 12
                // regardless of the shift in the command; the stored IR3
                // is still clamped as usual.
                let mac = (acc >> shift) as i32;
                self.data[MAC1 + 2] = mac as u32;
                let flag_value = acc >> 12;
                if flag_value > 0x7FFF || flag_value < -0x8000 {
                    self.set_flag(1 << 22);
                }
                let min = if lm { 0 } else { -0x8000 };
                self.data[IR3] = mac.clamp(min, 0x7FFF) as u32;
                z = acc;
            }
        }

        // Depth FIFO.
        self.data[SZ0] = self.data[SZ1];
        self.data[SZ1] = self.data[SZ2];
        self.data[SZ2] = self.data[SZ3];
        let depth = z >> 12;
        let sz3 = if depth < 0 || depth > 0xFFFF {
            self.set_flag(FLAG_SZ_OTZ_SATURATED);
            depth.clamp(0, 0xFFFF)
        } else {
            depth
        };
        self.data[SZ3] = sz3 as u32;

        // Projection.
        let h = self.control[H] & 0xFFFF;
        let n = self.divide(h, sz3 as u32) as i64;
        let ir1 = self.data[IR1] as i16 as i64;
        let ir2 = self.data[IR2] as i16 as i64;

        let x = n * ir1 + self.control[OFX] as i32 as i64;
        self.set_mac0(x);
        let sx = self.clamp_screen((x >> 16) as i32, FLAG_SX_SATURATED);
        let y = n * ir2 + self.control[OFY] as i32 as i64;
        self.set_mac0(y);
        let sy = self.clamp_screen((y >> 16) as i32, FLAG_SY_SATURATED);

        self.data[SXY0] = self.data[SXY1];
        self.data[SXY1] = self.data[SXY2];
        self.data[SXY2] = (sx as u16 as u32) | ((sy as u16 as u32) << 16);

        // Depth cueing.
        let dqa = self.control[DQA] as i16 as i64;
        let dqb = self.control[DQB] as i32 as i64;
        let depth_cue = n * dqa + dqb;
        self.set_mac0(depth_cue);
        let ir0 = depth_cue >> 12;
        let ir0 = if ir0 < 0 || ir0 > 0x1000 {
            self.set_flag(FLAG_IR0_SATURATED);
            ir0.clamp(0, 0x1000)
        } else {
            ir0
        };
        self.data[IR0] = ir0 as u32;
    }

    /// Clamp a screen coordinate to the 11-bit range.
    fn clamp_screen(&mut self, value: i32, flag: u32) -> i32 {
        if !(-0x400..=0x3FF).contains(&value) {
            self.set_flag(flag);
            value.clamp(-0x400, 0x3FF)
        } else {
            value
        }
    }

    /// Normal clipping: twice the signed area of the screen-space triangle.
    fn nclip(&mut self) {
        let point = |reg: u32| (reg as i16 as i64, (reg >> 16) as i16 as i64);
        let (x0, y0) = point(self.data[SXY0]);
        let (x1, y1) = point(self.data[SXY1]);
        let (x2, y2) = point(self.data[SXY2]);
        self.set_mac0(x0 * y1 + x1 * y2 + x2 * y0 - x0 * y2 - x1 * y0 - x2 * y1);
    }

    /// Cross product of IR with the rotation matrix diagonal.
    fn op(&mut self, shift: u32, lm: bool) {
        let rotation = self.matrix(MATRIX_ROTATION);
        let d = [rotation[0][0] as i64, rotation[1][1] as i64, rotation[2][2] as i64];
        let ir = self.ir().map(|v| v as i64);

        let products = [
            ir[2] * d[1] - ir[1] * d[2],
            ir[0] * d[2] - ir[2] * d[0],
            ir[1] * d[0] - ir[0] * d[1],
        ];
        for (i, product) in products.into_iter().enumerate() {
            let acc = self.check_mac(i + 1, product);
            self.set_mac_ir(i + 1, acc, shift, lm);
        }
    }

    /// Multiply a vector by a matrix and add a translation.
    fn mvmva(&mut self, command: u32, shift: u32, lm: bool) {
        let matrix_select = (command >> 17) & 3;
        let vector_select = ((command >> 15) & 3) as usize;
        let translation_select = (command >> 13) & 3;

        let m = match matrix_select {
            0 => self.matrix(MATRIX_ROTATION),
            1 => self.matrix(MATRIX_LIGHT),
            2 => self.matrix(MATRIX_COLOR),
            _ => {
                // The "garbage" matrix real hardware exposes.
                let r = self.rgb()[0] as i32;
                let rotation = self.matrix(MATRIX_ROTATION);
                let ir0 = self.data[IR0] as i16 as i32;
                [
                    [-(r << 4), r << 4, ir0],
                    [rotation[0][2]; 3],
                    [rotation[1][1]; 3],
                ]
            }
        };
        let v = self.vector(vector_select);

        match translation_select {
            0 => {
                let t = self.control_vector(TR);
                self.multiply_matrix(&m, v, t, shift, lm);
            }
            1 => {
                let t = self.control_vector(BK);
                self.multiply_matrix(&m, v, t, shift, lm);
            }
            2 => self.multiply_matrix_far_color(&m, v, shift, lm),
            _ => self.multiply_matrix(&m, v, [0; 3], shift, lm),
        }
    }

    /// MACn = IRn², clamped into IRn.
    fn sqr(&mut self, shift: u32, lm: bool) {
        for (i, ir) in self.ir().into_iter().enumerate() {
            let acc = self.check_mac(i + 1, ir as i64 * ir as i64);
            self.set_mac_ir(i + 1, acc, shift, lm);
        }
    }

    /// OTZ from the average of the last three or four depths.
    fn average_z(&mut self, scale_register: usize, count: usize) {
        let scale = self.control[scale_register] as i16 as i64;
        let first = SZ3 + 1 - count;
        let sum: i64 = (first..=SZ3).map(|r| (self.data[r] & 0xFFFF) as i64).sum();
        let result = scale * sum;
        self.set_mac0(result);
        let otz = result >> 12;
        let otz = if otz < 0 || otz > 0xFFFF {
            self.set_flag(FLAG_SZ_OTZ_SATURATED);
            otz.clamp(0, 0xFFFF)
        } else {
            otz
        };
        self.data[OTZ] = otz as u32;
    }

    /// General purpose interpolation: MAC = IR * IR0.
    fn gpf(&mut self, shift: u32, lm: bool) {
        let ir0 = self.data[IR0] as i16 as i64;
        for (i, ir) in self.ir().into_iter().enumerate() {
            let acc = self.check_mac(i + 1, ir as i64 * ir0);
            self.set_mac_ir(i + 1, acc, shift, lm);
        }
        self.push_color();
    }

    /// General purpose interpolation with base: MAC = MAC << shift + IR * IR0.
    fn gpl(&mut self, shift: u32, lm: bool) {
        let ir0 = self.data[IR0] as i16 as i64;
        for (i, ir) in self.ir().into_iter().enumerate() {
            let mac = self.data[MAC1 + i] as i32 as i64;
            let acc = self.check_mac(i + 1, (mac << shift) + ir as i64 * ir0);
            self.set_mac_ir(i + 1, acc, shift, lm);
        }
        self.push_color();
    }

    // ----- Lighting and colour -----

    /// MAC/IR = light matrix * vector.
    fn light_vector(&mut self, vector_index: usize, shift: u32, lm: bool) {
        let m = self.matrix(MATRIX_LIGHT);
        let v = self.vector(vector_index);
        self.multiply_matrix(&m, v, [0; 3], shift, lm);
    }

    /// MAC/IR = background colour + light colour matrix * IR.
    fn light_color(&mut self, shift: u32, lm: bool) {
        let m = self.matrix(MATRIX_COLOR);
        let background = self.control_vector(BK);
        let ir = self.ir();
        self.multiply_matrix(&m, ir, background, shift, lm);
    }

    /// `RGBC * IR << 4` for each channel.
    fn modulate(&self) -> [i64; 3] {
        let rgb = self.rgb();
        let ir = self.ir();
        [
            (rgb[0] * ir[0] as i64) << 4,
            (rgb[1] * ir[1] as i64) << 4,
            (rgb[2] * ir[2] as i64) << 4,
        ]
    }

    /// MAC/IR = (RGBC * IR) << 4, then shifted down.
    fn apply_color(&mut self, shift: u32, lm: bool) {
        for (i, product) in self.modulate().into_iter().enumerate() {
            let acc = self.check_mac(i + 1, product);
            self.set_mac_ir(i + 1, acc, shift, lm);
        }
    }

    /// Normal colour, single.
    fn ncs(&mut self, vector_index: usize, shift: u32, lm: bool) {
        self.light_vector(vector_index, shift, lm);
        self.light_color(shift, lm);
        self.push_color();
    }

    /// Normal colour colour, single.
    fn nccs(&mut self, vector_index: usize, shift: u32, lm: bool) {
        self.light_vector(vector_index, shift, lm);
        self.light_color(shift, lm);
        self.apply_color(shift, lm);
        self.push_color();
    }

    /// Normal colour depth cue, single.
    fn ncds(&mut self, vector_index: usize, shift: u32, lm: bool) {
        self.light_vector(vector_index, shift, lm);
        self.light_color(shift, lm);
        let input = self.modulate();
        self.interpolate(input, shift, lm);
        self.push_color();
    }

    /// Colour colour: light colour matrix and colour modulation only.
    fn cc(&mut self, shift: u32, lm: bool) {
        self.light_color(shift, lm);
        self.apply_color(shift, lm);
        self.push_color();
    }

    /// Colour depth cue.
    fn cdp(&mut self, shift: u32, lm: bool) {
        self.light_color(shift, lm);
        let input = self.modulate();
        self.interpolate(input, shift, lm);
        self.push_color();
    }

    /// Depth cue colour light.
    fn dcpl(&mut self, shift: u32, lm: bool) {
        let input = self.modulate();
        self.interpolate(input, shift, lm);
        self.push_color();
    }

    /// Depth cue single: blend RGBC towards the far colour.
    fn dpcs(&mut self, shift: u32, lm: bool) {
        let rgb = self.rgb();
        self.interpolate([rgb[0] << 16, rgb[1] << 16, rgb[2] << 16], shift, lm);
        self.push_color();
    }

    /// Depth cue triple step: blend the oldest FIFO colour.
    fn dpct(&mut self, shift: u32, lm: bool) {
        let rgb0 = self.data[RGB0];
        let channel = |i: u32| ((rgb0 >> (8 * i)) & 0xFF) as i64;
        self.interpolate([channel(0) << 16, channel(1) << 16, channel(2) << 16], shift, lm);
        self.push_color();
    }

    /// Interpolate IR towards the far colour.
    fn intpl(&mut self, shift: u32, lm: bool) {
        let ir = self.ir();
        self.interpolate([(ir[0] as i64) << 12, (ir[1] as i64) << 12, (ir[2] as i64) << 12], shift, lm);
        self.push_color();
    }
}

impl Default for Gte {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
