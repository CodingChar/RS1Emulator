//! SPU reverb: a network of comb and all-pass filters whose delay lines live
//! in the top of sound RAM, run at 22.05 kHz.

use super::voice::SOUND_RAM_SIZE;

// Indices into the 32 reverb registers (`0x1F801DC0..0x1F801E00`).
const D_APF1: usize = 0;
const D_APF2: usize = 1;
const V_IIR: usize = 2;
const V_COMB1: usize = 3;
const V_COMB2: usize = 4;
const V_COMB3: usize = 5;
const V_COMB4: usize = 6;
const V_WALL: usize = 7;
const V_APF1: usize = 8;
const V_APF2: usize = 9;
const M_LSAME: usize = 10;
const M_RSAME: usize = 11;
const M_LCOMB1: usize = 12;
const M_RCOMB1: usize = 13;
const M_LCOMB2: usize = 14;
const M_RCOMB2: usize = 15;
const D_LSAME: usize = 16;
const D_RSAME: usize = 17;
const M_LDIFF: usize = 18;
const M_RDIFF: usize = 19;
const M_LCOMB3: usize = 20;
const M_RCOMB3: usize = 21;
const M_LCOMB4: usize = 22;
const M_RCOMB4: usize = 23;
const D_LDIFF: usize = 24;
const D_RDIFF: usize = 25;
const M_LAPF1: usize = 26;
const M_RAPF1: usize = 27;
const M_LAPF2: usize = 28;
const M_RAPF2: usize = 29;
const V_LIN: usize = 30;
const V_RIN: usize = 31;

pub const REGISTER_COUNT: usize = 32;

/// Signed 16-bit fixed-point multiply (0x8000 = -1.0).
fn mul(value: i32, volume: u16) -> i32 {
    (value * (volume as i16 as i32)) >> 15
}

pub struct Reverb {
    /// Offset of the "current" position from the start of the work area.
    pub current: usize,
}

/// The delay-line memory seen through the work-area window.
struct Area<'a> {
    ram: &'a mut [u8],
    base: usize,
    size: usize,
    current: usize,
    writable: bool,
}

impl Area<'_> {
    /// Byte address of a register value (units of 8 bytes) plus an offset.
    fn address(&self, units: u16, offset: isize) -> usize {
        let relative = (self.current as isize + units as isize * 8 + offset).rem_euclid(self.size as isize);
        self.base + relative as usize
    }

    fn read(&self, units: u16, offset: isize) -> i32 {
        let a = self.address(units, offset);
        i16::from_le_bytes([self.ram[a], self.ram[a + 1]]) as i32
    }

    fn write(&mut self, units: u16, value: i32) {
        if self.writable {
            let a = self.address(units, 0);
            let bytes = (value.clamp(-0x8000, 0x7FFF) as i16).to_le_bytes();
            self.ram[a] = bytes[0];
            self.ram[a + 1] = bytes[1];
        }
    }
}

impl Reverb {
    pub fn new() -> Self {
        Self { current: 0 }
    }

    /// Run one reverb step on the input pair; returns the output pair.
    /// `start` is the work-area start in bytes.
    pub fn process(
        &mut self,
        regs: &[u16],
        start: usize,
        ram: &mut [u8],
        input: (i32, i32),
        writable: bool,
    ) -> (i32, i32) {
        let base = start.min(SOUND_RAM_SIZE - 8);
        let size = SOUND_RAM_SIZE - base;
        let mut area = Area { ram, base, size, current: self.current % size, writable };
        let r = |i: usize| regs[i];

        let lin = mul(input.0, r(V_LIN));
        let rin = mul(input.1, r(V_RIN));

        // Same-side and cross-side reflections, low-passed by vIIR.
        let reflect = |area: &mut Area, source: usize, target: usize, value: i32| {
            let previous = area.read(r(target), -2);
            let fed = value + mul(area.read(r(source), 0), r(V_WALL)) - previous;
            let result = mul(fed, r(V_IIR)) + previous;
            area.write(r(target), result);
        };
        reflect(&mut area, D_LSAME, M_LSAME, lin);
        reflect(&mut area, D_RSAME, M_RSAME, rin);
        reflect(&mut area, D_RDIFF, M_LDIFF, lin);
        reflect(&mut area, D_LDIFF, M_RDIFF, rin);

        // Comb filter taps.
        let mut left = mul(area.read(r(M_LCOMB1), 0), r(V_COMB1))
            + mul(area.read(r(M_LCOMB2), 0), r(V_COMB2))
            + mul(area.read(r(M_LCOMB3), 0), r(V_COMB3))
            + mul(area.read(r(M_LCOMB4), 0), r(V_COMB4));
        let mut right = mul(area.read(r(M_RCOMB1), 0), r(V_COMB1))
            + mul(area.read(r(M_RCOMB2), 0), r(V_COMB2))
            + mul(area.read(r(M_RCOMB3), 0), r(V_COMB3))
            + mul(area.read(r(M_RCOMB4), 0), r(V_COMB4));

        // Two all-pass filters per side.
        let all_pass = |area: &mut Area, value: i32, target: usize, delay: usize, volume: usize| {
            let offset = -(r(delay) as isize) * 8;
            let delayed = area.read(r(target), offset);
            let fed = (value - mul(delayed, r(volume))).clamp(-0x8000, 0x7FFF);
            area.write(r(target), fed);
            mul(fed, r(volume)) + delayed
        };
        left = all_pass(&mut area, left, M_LAPF1, D_APF1, V_APF1);
        left = all_pass(&mut area, left, M_LAPF2, D_APF2, V_APF2);
        right = all_pass(&mut area, right, M_RAPF1, D_APF1, V_APF1);
        right = all_pass(&mut area, right, M_RAPF2, D_APF2, V_APF2);

        self.current = (self.current + 2) % size;
        (left.clamp(-0x8000, 0x7FFF), right.clamp(-0x8000, 0x7FFF))
    }
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}
