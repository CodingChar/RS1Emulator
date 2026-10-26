//! Exact interpolation without dividing.
//!
//! The rasterizers interpolate colours and texture coordinates across a
//! triangle as `numerator / divisor` where the numerator is a linear function
//! of the pixel (the edge functions weighted with the vertex values) and the
//! divisor is twice the triangle's area. Dividing for every pixel is slow
//! (a 128-bit multiplication on WebAssembly), so a `Lane` keeps the quotient and
//! remainder and updates both with additions: exact, as the tests that compare
//! the rasterizers with simple reference implementations show.

/// `numerator / divisor` of a value that grows by a fixed step per pixel,
/// kept as quotient and remainder so every pixel is exact without dividing.
/// The divisor (twice the area of the triangle, in units of the sub-pixel
/// grid squared) is below 2^30 for any triangle the console can draw, so
/// the remainders fit 32 bits.
#[derive(Clone, Copy, Default)]
pub(super) struct Lane {
    pub(super) quotient: i32,
    pub(super) remainder: i32,
    pub(super) step_quotient: i32,
    pub(super) step_remainder: i32,
}

impl Lane {
    pub(super) fn new(step: i64, divisor: i64, inverse: f64) -> Self {
        let mut lane = Self::default();
        lane.start(step, divisor, inverse);
        (lane.step_quotient, lane.step_remainder) = (lane.quotient, lane.remainder);
        (lane.quotient, lane.remainder) = (0, 0);
        lane
    }

    /// Start at `numerator`, which may be negative; `inverse` is `1.0 / divisor`,
    /// which gets the quotient to within one (a division of 64-bit integers is
    /// much slower).
    pub(super) fn start(&mut self, numerator: i64, divisor: i64, inverse: f64) {
        let mut quotient = (numerator as f64 * inverse).floor() as i64;
        let mut remainder = numerator - quotient * divisor;
        if remainder < 0 {
            quotient -= 1;
            remainder += divisor;
        } else if remainder >= divisor {
            quotient += 1;
            remainder -= divisor;
        }
        self.quotient = quotient as i32;
        self.remainder = remainder as i32;
    }

    /// One pixel further. The carry is computed without a branch: whether the
    /// remainder overflows is as unpredictable as the pixels themselves.
    #[inline(always)]
    pub(super) fn advance(&mut self, divisor: i32) {
        self.remainder += self.step_remainder;
        let carry = (self.remainder >= divisor) as i32;
        self.remainder -= carry * divisor;
        self.quotient += self.step_quotient + carry;
    }
}
