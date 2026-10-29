//! Rendering above the console's resolution.
//!
//! The VRAM of the console (1024x512 halfwords) stays the source of truth for
//! everything the game can see: textures, palettes, readbacks and uploads.
//! When an internal resolution above 1x, true colour, texture filtering or
//! PGXP is asked for, every drawing command is drawn a second time into a
//! *scaled VRAM* (`scale` times larger on both axes, 8 bits per channel) that
//! is only used for the picture shown on screen. That is the approach of a
//! software renderer with an "internal resolution" setting:
//!
//! - Polygons are rasterized on the finer grid with the same edge rules as the
//!   console (top-left fill rule, interpolation evaluated at the sample point),
//!   so at 1x with every option off the result equals the original rasterizer
//!   pixel for pixel (see the tests). Vertex positions keep a 1/16 pixel of the
//!   scaled grid, which is how the sub-pixel positions of PGXP are drawn.
//! - Textures are still read from the console's VRAM: with nearest sampling a
//!   texel covers `scale` x `scale` pixels, with bilinear filtering the 4
//!   nearest texels are blended (transparent texels are left out and the pixel
//!   is transparent when most of its weight is, "binary alpha"), clamped to the
//!   texture area of the primitive so neighbouring textures of the page do not
//!   bleed in.
//! - With PGXP depths, textures are perspective-correct: `u/w`, `v/w` and
//!   `1/w` are interpolated, as the hardware cannot.
//! - VRAM fills, uploads and copies are mirrored: a copy moves the detail that
//!   was drawn at high resolution, an upload repeats each pixel `scale` times.
//!
//! The scaled VRAM holds `0x00BBGGRR` with the mask bit in bit 24, so the
//! display is a plain copy. Without true colour every value is cut to 5 bits
//! like in the console (and expanded back), with it nothing is lost and
//! dithering is off.

use super::lane::Lane;
use super::sw::{
    self, blend, rgb888_to_15, Attributes, DrawSettings, Sampler, Vram, DITHER, MAX_PRIMITIVE_HEIGHT,
    MAX_PRIMITIVE_WIDTH, VRAM_HEIGHT, VRAM_WIDTH,
};

/// The largest internal resolution multiplier.
pub const MAX_SCALE: u32 = 4;
/// Bit 24 of a scaled VRAM pixel: bit 15 of the console's pixel.
pub const MASK: u32 = 1 << 24;
/// The colour bits of a scaled VRAM pixel (for the tests).
#[cfg(test)]
const COLOR: u32 = 0x00FF_FFFF;
const OPAQUE: u32 = 0xFF00_0000;

/// Sub-pixel precision of a vertex position: 1/8 of a pixel of the scaled VRAM.
const FRACTION: i64 = 8;
/// Perspective-correct texture coordinates are computed exactly this often
/// (in pixels) and interpolated linearly in between.
const PERSPECTIVE_SPAN: i64 = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextureFilter {
    /// The texel under the pixel, like the console.
    #[default]
    Nearest,
    /// A blend of the four nearest texels.
    Bilinear,
}

/// What the frontend can ask of the renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderSettings {
    /// Internal resolution as a multiple of the console's (1 to `MAX_SCALE`).
    pub scale: u32,
    /// Shade and blend with 8 bits per channel and no dithering.
    pub true_color: bool,
    /// How textures of polygons are sampled (sprites keep nearest sampling so
    /// text and menus stay sharp).
    pub texture_filter: TextureFilter,
    /// PGXP: draw polygons with the precise positions of the GTE.
    pub pgxp: bool,
    /// With PGXP: perspective-correct texture mapping.
    pub pgxp_textures: bool,
    /// With PGXP: squeeze 3D geometry horizontally by 3/4 around the centre of
    /// the drawing area, for a picture that is shown at 16:9 instead of 4:3.
    pub widescreen: bool,
    /// Smooth what the console only draws at its own resolution (pre-rendered
    /// backgrounds, menus, video) when the internal resolution is above 1x.
    pub smooth_2d: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            scale: 1,
            true_color: false,
            texture_filter: TextureFilter::Nearest,
            pgxp: false,
            pgxp_textures: false,
            widescreen: false,
            smooth_2d: false,
        }
    }
}

impl RenderSettings {
    /// Whether anything asked for needs the scaled VRAM; if not the console's
    /// own rasterizer and display are used as they are.
    pub fn needs_hires(&self) -> bool {
        self.scale > 1 || self.true_color || self.texture_filter != TextureFilter::Nearest || self.pgxp
    }

    /// The settings with values out of range or without effect corrected.
    pub fn sanitized(mut self) -> Self {
        self.scale = self.scale.clamp(1, MAX_SCALE);
        self.pgxp_textures &= self.pgxp;
        self.widescreen &= self.pgxp;
        self
    }
}

// ----- Colour -----

/// 5-bit channel to 8 bits, repeating the top bits into the low ones.
const EXPAND5: [u8; 32] = {
    let mut table = [0u8; 32];
    let mut c = 0;
    while c < 32 {
        table[c] = ((c << 3) | (c >> 2)) as u8;
        c += 1;
    }
    table
};

/// A colour channel (0..=255, plus the dither offset of -4..=3 and 4 to keep
/// the index positive) cut to 5 bits and expanded back to 8: what a pixel
/// shows after it has been written to the console's 15-bit VRAM.
const QUANTIZED: [u8; 512] = {
    let mut table = [0u8; 512];
    let mut i = 0;
    while i < 512 {
        let value = i as i32 - 4;
        let clamped = if value < 0 { 0 } else if value > 255 { 255 } else { value };
        table[i] = EXPAND5[(clamped >> 3) as usize];
        i += 1;
    }
    table
};

/// A console pixel as a scaled VRAM pixel.
#[inline(always)]
pub fn expand15(pixel: u16) -> u32 {
    EXPAND5[(pixel & 31) as usize] as u32
        | (EXPAND5[((pixel >> 5) & 31) as usize] as u32) << 8
        | (EXPAND5[((pixel >> 10) & 31) as usize] as u32) << 16
        | ((pixel as u32 >> 15) << 24)
}

/// The 15-bit colour (without the mask bit) of a scaled VRAM pixel.
#[inline(always)]
fn pack15(pixel: u32) -> u16 {
    (((pixel & 0xFF) >> 3) | (((pixel >> 8) & 0xFF) >> 3) << 5 | (((pixel >> 16) & 0xFF) >> 3) << 10) as u16
}

#[inline(always)]
fn pack8(rgb: [i32; 3]) -> u32 {
    rgb[0] as u32 | (rgb[1] as u32) << 8 | (rgb[2] as u32) << 16
}

#[inline(always)]
fn clamp8(value: i32) -> i32 {
    value.clamp(0, 255)
}

/// Semi-transparency with 8-bit channels (the same four modes as the console).
fn blend8(back: u32, front: [i32; 3], mode: u8) -> [i32; 3] {
    let mut out = [0; 3];
    for (c, out) in out.iter_mut().enumerate() {
        let b = ((back >> (8 * c)) & 0xFF) as i32;
        let f = front[c];
        *out = match mode {
            0 => (b + f) / 2,
            1 => (b + f).min(255),
            2 => (b - f).max(0),
            _ => (b + f / 4).min(255),
        };
    }
    out
}

// ----- Vertices and texels -----

/// A polygon vertex as the scaled renderer takes it.
#[derive(Clone, Copy, Debug, Default)]
pub struct HVertex {
    /// Position in console pixels before the drawing offset; fractional when
    /// it came from PGXP.
    pub x: f32,
    pub y: f32,
    /// The PGXP depth (SZ3), or 0 when the vertex has no precise position.
    pub w: f32,
    pub color: [u8; 3],
    pub u: u8,
    pub v: u8,
    /// The integer position the console itself uses, to drop what it drops.
    pub ix: i32,
    pub iy: i32,
}

/// A sampled texel: its three channels (5 bits each for texels taken straight
/// from VRAM, 8 bits for filtered ones and with true colour, see `Shading`)
/// and bit 15, which makes a semi-transparent primitive blend it.
#[derive(Clone, Copy)]
struct Texel {
    rgb: [i32; 3],
    stp: bool,
}

const NO_TEXEL: Texel = Texel { rgb: [0; 3], stp: false };

/// The texture area a primitive uses, to keep filtering inside it.
#[derive(Clone, Copy)]
struct UvBounds {
    min_u: i64,
    max_u: i64,
    min_v: i64,
    max_v: i64,
}

/// The texel at `(u, v)`; `None` when it is transparent (the colour 0).
#[inline(always)]
fn nearest_texel(sampler: &Sampler, vram: &Vram, u: u32, v: u32, exact: bool) -> Option<Texel> {
    let texel = sampler.sample(vram, u, v);
    if texel == 0 {
        return None;
    }
    let rgb = [(texel & 31) as usize, ((texel >> 5) & 31) as usize, ((texel >> 10) & 31) as usize];
    let rgb = if exact {
        [rgb[0] as i32, rgb[1] as i32, rgb[2] as i32]
    } else {
        [EXPAND5[rgb[0]] as i32, EXPAND5[rgb[1]] as i32, EXPAND5[rgb[2]] as i32]
    };
    Some(Texel { rgb, stp: texel & 0x8000 != 0 })
}

/// The four texels around the last bilinear sample, which the next pixel of a row
/// usually shares: they are only read again when the sample moves to another cell.
#[derive(Clone, Copy, Default)]
struct Taps {
    /// The cell (its top left texel) the texels belong to.
    u: i64,
    v: i64,
    texels: [u16; 4],
    valid: bool,
}

/// Bilinear sampling at the texture coordinate `(u, v)` in 8.8 fixed point.
/// The result has 8-bit channels.
#[inline(never)]
fn bilinear_texel(taps: &mut Taps, sampler: &Sampler, vram: &Vram, u: i64, v: i64, bounds: &UvBounds) -> Option<Texel> {
    // Texel centres sit half a texel in.
    let (us, vs) = (u - 128, v - 128);
    let (u0, v0) = (us >> 8, vs >> 8);
    let (fu, fv) = ((us & 255) as i32, (vs & 255) as i32);
    if !taps.valid || taps.u != u0 || taps.v != v0 {
        let fetch = |u: i64, v: i64| {
            sampler.sample(vram, u.clamp(bounds.min_u, bounds.max_u) as u32, v.clamp(bounds.min_v, bounds.max_v) as u32)
        };
        *taps = Taps { u: u0, v: v0, texels: [fetch(u0, v0), fetch(u0 + 1, v0), fetch(u0, v0 + 1), fetch(u0 + 1, v0 + 1)], valid: true };
    }
    let weights = [(256 - fu) * (256 - fv), fu * (256 - fv), (256 - fu) * fv, fu * fv];
    let (mut weight, mut sum) = (0i32, [0i32; 3]);
    for (texel, w) in taps.texels.into_iter().zip(weights) {
        if texel != 0 {
            weight += w;
            sum[0] += w * EXPAND5[(texel & 31) as usize] as i32;
            sum[1] += w * EXPAND5[((texel >> 5) & 31) as usize] as i32;
            sum[2] += w * EXPAND5[((texel >> 10) & 31) as usize] as i32;
        }
    }
    // Binary alpha: transparent when most of the weight is transparent.
    if weight < 32768 {
        return None;
    }
    // The texel the sample lies in is the nearest of the four.
    let nearest = taps.texels[((fv >= 128) as usize) * 2 + (fu >= 128) as usize];
    // With all four texels opaque the weights add up to 2^16.
    let rgb = if weight == 65536 {
        [sum[0] >> 16, sum[1] >> 16, sum[2] >> 16]
    } else {
        [sum[0] / weight, sum[1] / weight, sum[2] / weight]
    };
    Some(Texel { rgb, stp: nearest & 0x8000 != 0 })
}

type Point = (i64, i64);

#[inline(always)]
fn edge(a: Point, b: Point, px: i64, py: i64) -> i64 {
    (b.0 - a.0) * (py - a.1) - (b.1 - a.1) * (px - a.0)
}

/// Pixels exactly on a top or left edge belong to the triangle.
fn is_top_left(a: Point, b: Point) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    dy < 0 || (dy == 0 && dx > 0)
}

/// Smooth the parts of a scaled picture that carry no extra detail: every block of
/// `scale` x `scale` pixels that is one flat colour (a console pixel repeated, as
/// uploads, fills and video are) is replaced by a bilinear blend of the console
/// pixels around it. Blocks with detail (drawn polygons) are left as they are.
pub fn smooth_flat_blocks(pixels: &mut [u32], width: usize, height: usize, scale: usize) {
    if scale < 2 || width % scale != 0 || height % scale != 0 || pixels.len() != width * height {
        return;
    }
    let (bw, bh) = (width / scale, height / scale);
    // One colour per block (its centre) and whether the block is flat.
    let mut native = vec![0u32; bw * bh];
    let mut flat = vec![false; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let first = pixels[by * scale * width + bx * scale];
            let is_flat = (0..scale).all(|k| pixels[(by * scale + k) * width + bx * scale..][..scale].iter().all(|&p| p == first));
            native[by * bw + bx] = first;
            flat[by * bw + bx] = is_flat;
        }
    }
    // For the pixel `i` of a block: which neighbour it leans to (-1 or 1) and by how much (of 256).
    let lean: Vec<(isize, u32)> = (0..scale)
        .map(|i| {
            let off = 2 * i as isize + 1 - scale as isize;
            (if off < 0 { -1 } else { 1 }, (off.unsigned_abs() * 256 / (2 * scale)) as u32)
        })
        .collect();
    let at = |bx: isize, by: isize| native[by.clamp(0, bh as isize - 1) as usize * bw + bx.clamp(0, bw as isize - 1) as usize];
    for by in 0..bh {
        for bx in 0..bw {
            if !flat[by * bw + bx] {
                continue;
            }
            for j in 0..scale {
                let (dy, wy) = lean[j];
                for i in 0..scale {
                    let (dx, wx) = lean[i];
                    let (x, y) = (bx as isize, by as isize);
                    let corners = [(at(x, y), (256 - wx) * (256 - wy)), (at(x + dx, y), wx * (256 - wy)), (at(x, y + dy), (256 - wx) * wy), (at(x + dx, y + dy), wx * wy)];
                    let mut out = OPAQUE;
                    for shift in [0, 8, 16] {
                        let sum: u32 = corners.iter().map(|&(c, w)| ((c >> shift) & 0xFF) * w).sum();
                        out |= (sum >> 16) << shift;
                    }
                    pixels[(by * scale + j) * width + bx * scale + i] = out;
                }
            }
        }
    }
}

// ----- The scaled VRAM -----

pub struct HiRes {
    scale: usize,
    /// Pixels per row: `VRAM_WIDTH * scale`.
    stride: usize,
    rows: usize,
    pub pixels: Vec<u32>,
    true_color: bool,
    filter: TextureFilter,
    /// Reused between copies and sprites so they do not allocate.
    scratch: Vec<u32>,
    texels: Vec<Option<Texel>>,
}

impl HiRes {
    /// A scaled VRAM with the content of `vram`.
    pub fn from_vram(vram: &Vram, settings: &RenderSettings) -> Self {
        let scale = settings.scale.clamp(1, MAX_SCALE) as usize;
        let mut hires = Self {
            scale,
            stride: VRAM_WIDTH * scale,
            rows: VRAM_HEIGHT * scale,
            pixels: vec![0; VRAM_WIDTH * scale * VRAM_HEIGHT * scale],
            true_color: settings.true_color,
            filter: settings.texture_filter,
            scratch: Vec::new(),
            texels: Vec::new(),
        };
        hires.rebuild(vram);
        hires
    }

    pub fn scale(&self) -> usize {
        self.scale
    }

    /// Change what can change without a new buffer.
    pub fn configure(&mut self, settings: &RenderSettings) {
        self.true_color = settings.true_color;
        self.filter = settings.texture_filter;
    }

    /// Fill the buffer from the console's VRAM, every pixel repeated `scale` times.
    pub fn rebuild(&mut self, vram: &Vram) {
        let s = self.scale;
        for y in 0..VRAM_HEIGHT {
            let first = y * s * self.stride;
            for x in 0..VRAM_WIDTH {
                let pixel = expand15(vram.pixels[y * VRAM_WIDTH + x]);
                self.pixels[first + x * s..first + x * s + s].fill(pixel);
            }
            for k in 1..s {
                self.pixels.copy_within(first..first + self.stride, first + k * self.stride);
            }
        }
    }

    /// Set the block of pixels that covers console pixel `(x, y)`.
    #[inline]
    fn set_block(&mut self, x: usize, y: usize, value: u32) {
        let s = self.scale;
        for k in 0..s {
            let at = (y * s + k) * self.stride + x * s;
            // Uploads are made of these: the usual sizes are written without a loop.
            match self.pixels[at..at + s] {
                [ref mut a, ref mut b] => (*a, *b) = (value, value),
                [ref mut a, ref mut b, ref mut c] => (*a, *b, *c) = (value, value, value),
                ref mut block => block.fill(value),
            }
        }
    }

    /// Mirror an upload of one pixel (the caller has checked the mask bit).
    pub fn upload_pixel(&mut self, x: usize, y: usize, pixel: u16) {
        self.set_block(x % VRAM_WIDTH, y % VRAM_HEIGHT, expand15(pixel));
    }

    /// Mirror GP0(02h) with the area `sw::fill_area` returned.
    pub fn fill_rectangle(&mut self, x: usize, y: usize, width: usize, height: usize, color: [u8; 3]) {
        let value = if self.true_color {
            pack8([color[0] as i32, color[1] as i32, color[2] as i32])
        } else {
            expand15(rgb888_to_15(color))
        };
        let s = self.scale;
        for dy in 0..height {
            let row = (y + dy) % VRAM_HEIGHT;
            let end = x + width;
            for k in 0..s {
                let base = (row * s + k) * self.stride;
                if end <= VRAM_WIDTH {
                    self.pixels[base + x * s..base + end * s].fill(value);
                } else {
                    self.pixels[base + x * s..base + self.stride].fill(value);
                    self.pixels[base..base + (end - VRAM_WIDTH) * s].fill(value);
                }
            }
        }
    }

    /// Mirror GP0(80h): the high-resolution detail moves with the copy.
    pub fn copy_rectangle(&mut self, settings: &DrawSettings, src: (u32, u32), dst: (u32, u32), size: (u32, u32)) {
        let s = self.scale;
        let (width, height) = (sw::transfer_width(size.0), sw::transfer_height(size.1));
        let (sx, sy) = (src.0 as usize, src.1 as usize);
        let (dx0, dy0) = (dst.0 as usize, dst.1 as usize);

        let wraps = sx + width > VRAM_WIDTH || dx0 + width > VRAM_WIDTH || sy + height > VRAM_HEIGHT || dy0 + height > VRAM_HEIGHT;
        let disjoint = sx + width <= dx0 || dx0 + width <= sx || sy + height <= dy0 || dy0 + height <= sy;
        if !wraps && disjoint && !settings.check_mask {
            for row in 0..height * s {
                let from = (sy * s + row) * self.stride + sx * s;
                let to = (dy0 * s + row) * self.stride + dx0 * s;
                self.pixels.copy_within(from..from + width * s, to);
                if settings.set_mask {
                    for pixel in &mut self.pixels[to..to + width * s] {
                        *pixel |= MASK;
                    }
                }
            }
            return;
        }

        // Read everything first, then write (wrapping at the edges of VRAM).
        let (w, h) = (width * s, height * s);
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.clear();
        scratch.reserve(w * h);
        for row in 0..h {
            let y = (sy * s + row) % self.rows;
            for col in 0..w {
                scratch.push(self.pixels[y * self.stride + (sx * s + col) % self.stride]);
            }
        }
        for row in 0..h {
            let y = (dy0 * s + row) % self.rows;
            for col in 0..w {
                let at = y * self.stride + (dx0 * s + col) % self.stride;
                if settings.check_mask && self.pixels[at] & MASK != 0 {
                    continue;
                }
                self.pixels[at] = scratch[row * w + col] | if settings.set_mask { MASK } else { 0 };
            }
        }
        self.scratch = scratch;
    }

    // ----- Display -----

    /// The `width` x `height` pixels of the console's display that start at
    /// `start`, as `(width * scale) x (height * scale)` RGBA pixels in `out`.
    pub fn render_display(&self, out: &mut Vec<u32>, start: (usize, usize), width: usize, height: usize) {
        let s = self.scale;
        let (w, h) = (width * s, height * s);
        out.resize(w * h, 0);
        let first_x = (start.0 * s) % self.stride;
        for (y, line) in out.chunks_exact_mut(w).enumerate() {
            let base = ((start.1 * s + y) % self.rows) * self.stride;
            let row = &self.pixels[base..base + self.stride];
            let head = (self.stride - first_x).min(w);
            // The mask bit is the lowest bit of the alpha byte: forcing alpha opaque hides it.
            for (d, p) in line[..head].iter_mut().zip(&row[first_x..first_x + head]) {
                *d = p | OPAQUE;
            }
            for (d, p) in line[head..].iter_mut().zip(row) {
                *d = p | OPAQUE;
            }
        }
    }

    // ----- Triangles -----

    /// Draw a triangle. `perspective` asks for perspective-correct texture
    /// coordinates, which needs a depth (`w`) on every vertex.
    pub fn draw_triangle(
        &mut self,
        vram: &Vram,
        settings: &DrawSettings,
        attrs: &Attributes,
        vertices: [HVertex; 3],
        perspective: bool,
    ) {
        let [v0, mut v1, mut v2] = vertices;
        let too_big = |a: &HVertex, b: &HVertex| {
            (a.ix - b.ix).abs() > MAX_PRIMITIVE_WIDTH || (a.iy - b.iy).abs() > MAX_PRIMITIVE_HEIGHT
        };
        if too_big(&v0, &v1) || too_big(&v1, &v2) || too_big(&v2, &v0) {
            return;
        }

        let scale = self.scale as i64;
        let per_pixel = (scale * FRACTION) as f32;
        let place = |v: &HVertex| -> Point {
            (
                ((v.x + settings.offset_x as f32) * per_pixel).round() as i64,
                ((v.y + settings.offset_y as f32) * per_pixel).round() as i64,
            )
        };
        let (p0, mut p1, mut p2) = (place(&v0), place(&v1), place(&v2));

        let mut area = edge(p0, p1, p2.0, p2.1);
        if area == 0 {
            return;
        }
        if area < 0 {
            std::mem::swap(&mut p1, &mut p2);
            std::mem::swap(&mut v1, &mut v2);
            area = -area;
        }
        // Twice the area of what the console draws is at most 1023 * 511 pixels, which in
        // the sub-pixel grid still leaves the divisor of the exact interpolation below
        // 2^30. A polygon bigger than that (precise coordinates can be far off screen) is
        // interpolated with floating point instead.
        let huge = area >= 1 << 30;

        // The pixels whose sample point may be inside, inside the drawing area.
        let clip = &settings.area;
        let min_x = (p0.0.min(p1.0).min(p2.0) + FRACTION - 1).div_euclid(FRACTION).max(clip.left as i64 * scale);
        let max_x = p0.0.max(p1.0).max(p2.0).div_euclid(FRACTION).min((clip.right as i64 + 1) * scale - 1);
        let min_y = (p0.1.min(p1.1).min(p2.1) + FRACTION - 1).div_euclid(FRACTION).max(clip.top as i64 * scale);
        let max_y = p0.1.max(p1.1).max(p2.1).div_euclid(FRACTION).min((clip.bottom as i64 + 1) * scale - 1);
        if min_x > max_x || min_y > max_y {
            return;
        }

        // Edge i is (p1,p2), (p2,p0), (p0,p1): its function is the weight of the
        // opposite vertex. A pixel is inside when every weight is positive, or
        // zero on a top/left edge: `weight >= bias`.
        let edges = [(p1, p2), (p2, p0), (p0, p1)];
        let mut step_x = [0i64; 3];
        let mut step_y = [0i64; 3];
        let mut bias = [0i64; 3];
        let mut row_weight = [0i64; 3];
        let mut inverse_step = [0.0f64; 3];
        for (i, (a, b)) in edges.iter().enumerate() {
            step_x[i] = -(b.1 - a.1) * FRACTION;
            step_y[i] = (b.0 - a.0) * FRACTION;
            bias[i] = if is_top_left(*a, *b) { 0 } else { 1 };
            row_weight[i] = edge(*a, *b, min_x * FRACTION, min_y * FRACTION);
            if step_x[i] != 0 {
                inverse_step[i] = 1.0 / step_x[i].unsigned_abs() as f64;
            }
        }

        let divisor = area;
        let inverse = 1.0 / divisor as f64;
        let nearest = self.filter == TextureFilter::Nearest;
        let colors = [
            [v0.color[0], v1.color[0], v2.color[0]],
            [v0.color[1], v1.color[1], v2.color[1]],
            [v0.color[2], v1.color[2], v2.color[2]],
        ];
        let us = [v0.u, v1.u, v2.u];
        let vs = [v0.v, v1.v, v2.v];
        let context = SpanContext {
            sh: Shading::new(settings, attrs, self.true_color, nearest),
            flat: [v0.color[0] as i32, v0.color[1] as i32, v0.color[2] as i32],
            divisor: divisor as i32,
            sampler: Sampler::new(&settings.window, &attrs.texture),
            vram,
            filter: self.filter,
            bounds: UvBounds {
                min_u: *us.iter().min().unwrap() as i64,
                max_u: *us.iter().max().unwrap() as i64,
                min_v: *vs.iter().min().unwrap() as i64,
                max_v: *vs.iter().max().unwrap() as i64,
            },
        };
        let perspective = perspective && attrs.textured && v0.w > 0.0 && v1.w > 0.0 && v2.w > 0.0;
        // Perspective-correct texturing and the huge triangles are computed with floating
        // point (equal depths make the former plain affine mapping).
        let plane = (perspective || (huge && attrs.textured))
            .then(|| PerspectivePlane::new(if perspective { [v0.w, v1.w, v2.w] } else { [1.0; 3] }, us, vs, step_x));
        let float_colors = (huge && attrs.gouraud).then(|| FloatColors::new(colors, step_x, area));
        let floating = plane.is_some() || float_colors.is_some();

        // Exactly interpolated values: R, G, B and U, V in 8.8.
        let mut color_lanes = [Lane::default(); 3];
        let mut texture_lanes = [Lane::default(); 2];
        if attrs.gouraud && !huge {
            for (lane, values) in color_lanes.iter_mut().zip(&colors) {
                *lane = Lane::new((0..3).map(|i| step_x[i] * values[i] as i64).sum(), divisor, inverse);
            }
        }
        if attrs.textured && !floating {
            texture_lanes[0] = Lane::new((0..3).map(|i| step_x[i] * us[i] as i64 * 256).sum(), divisor, inverse);
            texture_lanes[1] = Lane::new((0..3).map(|i| step_x[i] * vs[i] as i64 * 256).sum(), divisor, inverse);
        }

        let width = max_x - min_x;
        for y in min_y..=max_y {
            let weights = row_weight;
            for i in 0..3 {
                row_weight[i] += step_y[i];
            }

            // The span of this row where every edge function passes.
            let (mut first, mut last) = (0i64, width);
            let mut empty = false;
            for i in 0..3 {
                let (w, step) = (weights[i], step_x[i]);
                if step == 0 {
                    empty |= w < bias[i];
                } else if step > 0 {
                    let need = bias[i] - w;
                    if need > 0 {
                        first = first.max(ceil_div(need, step, inverse_step[i]));
                    }
                } else {
                    let room = w - bias[i];
                    if room < 0 {
                        empty = true;
                    } else {
                        last = last.min(floor_div(room, -step, inverse_step[i]));
                    }
                }
            }
            if empty || first > last {
                continue;
            }

            let at_first = [weights[0] + first * step_x[0], weights[1] + first * step_x[1], weights[2] + first * step_x[2]];
            let mut colors_now = color_lanes;
            let mut texture_now = texture_lanes;
            if attrs.gouraud && !huge {
                for (lane, values) in colors_now.iter_mut().zip(&colors) {
                    lane.start((0..3).map(|i| at_first[i] * values[i] as i64).sum(), divisor, inverse);
                }
            }
            if attrs.textured && !floating {
                texture_now[0].start((0..3).map(|i| at_first[i] * us[i] as i64 * 256).sum(), divisor, inverse);
                texture_now[1].start((0..3).map(|i| at_first[i] * vs[i] as i64 * 256).sum(), divisor, inverse);
            }

            let x0 = (min_x + first) as usize;
            let start = y as usize * self.stride + x0;
            let row = &mut self.pixels[start..start + (last - first + 1) as usize];
            let y = y as usize;
            // The common cases have loops of their own (nearest sampling for textures, any
            // colour depth). They only read the pixel behind with semi-transparency or a mask
            // check (`blend`).
            let sh = &context.sh;
            let blend = sh.semi || sh.check_mask;
            // Perspective-correct texturing has the flattened loop too, unless the triangle is huge.
            let flat_perspective = plane.is_some() && !huge && nearest;
            macro_rules! shade_row {
                ($true_color:literal) => {
                    match (attrs.gouraud, attrs.textured, &plane) {
                        _ if floating && !flat_perspective => shade_float_span(
                            row, x0, y, &context, attrs.gouraud, colors_now, float_colors.as_ref(), plane.as_ref(), at_first,
                        ),
                        (false, false, _) if !blend => shade_span_untextured::<false, false, $true_color>(row, x0, y, &context, colors_now),
                        (false, false, _) => shade_span_untextured::<false, true, $true_color>(row, x0, y, &context, colors_now),
                        (true, false, _) if !blend => shade_span_untextured::<true, false, $true_color>(row, x0, y, &context, colors_now),
                        (true, false, _) => shade_span_untextured::<true, true, $true_color>(row, x0, y, &context, colors_now),
                        (false, true, Some(_)) if !blend => shade_span_nearest::<false, false, true, $true_color>(
                            row, x0, y, &context, colors_now, texture_now, plane.as_ref(), at_first,
                        ),
                        (false, true, Some(_)) => shade_span_nearest::<false, true, true, $true_color>(
                            row, x0, y, &context, colors_now, texture_now, plane.as_ref(), at_first,
                        ),
                        (true, true, Some(_)) if !blend => shade_span_nearest::<true, false, true, $true_color>(
                            row, x0, y, &context, colors_now, texture_now, plane.as_ref(), at_first,
                        ),
                        (true, true, Some(_)) => shade_span_nearest::<true, true, true, $true_color>(
                            row, x0, y, &context, colors_now, texture_now, plane.as_ref(), at_first,
                        ),
                        (false, true, None) if nearest && !blend => shade_span_nearest::<false, false, false, $true_color>(
                            row, x0, y, &context, colors_now, texture_now, None, at_first,
                        ),
                        (false, true, None) if nearest => shade_span_nearest::<false, true, false, $true_color>(
                            row, x0, y, &context, colors_now, texture_now, None, at_first,
                        ),
                        (true, true, None) if nearest && !blend => shade_span_nearest::<true, false, false, $true_color>(
                            row, x0, y, &context, colors_now, texture_now, None, at_first,
                        ),
                        (true, true, None) if nearest => shade_span_nearest::<true, true, false, $true_color>(
                            row, x0, y, &context, colors_now, texture_now, None, at_first,
                        ),
                        (false, true, None) => shade_span::<false, true>(row, x0, y, &context, colors_now, texture_now),
                        (true, true, None) => shade_span::<true, true>(row, x0, y, &context, colors_now, texture_now),
                    }
                };
            }
            if sh.true_color {
                shade_row!(true)
            } else {
                shade_row!(false)
            }
        }
    }

    // ----- Rectangles -----

    /// Draw a rectangle (a sprite). It covers the same console pixels as the
    /// original rasterizer, `scale` x `scale` pixels each, and always samples
    /// its texture with nearest filtering so 2D stays sharp.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_rectangle(
        &mut self,
        vram: &Vram,
        settings: &DrawSettings,
        attrs: &Attributes,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        color: [u8; 3],
        u: u8,
        v: u8,
        flip_x: bool,
        flip_y: bool,
    ) {
        let s = self.scale;
        let (x, y) = (x + settings.offset_x, y + settings.offset_y);
        let clip = &settings.area;
        let color = [color[0] as i32, color[1] as i32, color[2] as i32];
        let sampler = Sampler::new(&settings.window, &attrs.texture);
        let sh = Shading::new(settings, attrs, self.true_color, true);

        // The columns that survive the clip.
        let first = (clip.left - x).max(0);
        let last = (clip.right - x).min(width - 1);
        if first > last {
            return;
        }

        let solid = !attrs.textured && !attrs.semi_transparent && !attrs.gouraud && !settings.check_mask;
        let solid_pixel = flat_pixel(&sh, color);

        let mut texels = std::mem::take(&mut self.texels);
        for dy in 0..height {
            let py = y + dy;
            if py < clip.top || py > clip.bottom {
                continue;
            }
            let from = ((x + first) as usize) * s;
            let to = ((x + last + 1) as usize) * s;
            if solid {
                for k in 0..s {
                    let at = (py as usize * s + k) * self.stride;
                    self.pixels[at + from..at + to].fill(solid_pixel);
                }
                continue;
            }

            // One sample per console pixel of the row, shared by its scaled pixels.
            texels.clear();
            if attrs.textured {
                let tv = (if flip_y { v as i32 - dy } else { v as i32 + dy }) as u32 & 0xFF;
                for dx in first..=last {
                    let tu = (if flip_x { u as i32 - dx } else { u as i32 + dx }) as u32 & 0xFF;
                    texels.push(nearest_texel(&sampler, vram, tu, tv, sh.exact));
                }
            }
            // The common sprite (opaque, no dithering, no true colour) has one colour per
            // console pixel: work it out once and repeat it `scale` x `scale` times.
            if attrs.textured && !sh.true_color && !sh.semi && !sh.check_mask && !sh.dither {
                let mut values = std::mem::take(&mut self.scratch);
                values.clear();
                values.extend(texels.iter().map(|texel| match texel {
                    Some(texel) => sprite_pixel(&sh, color, texel),
                    None => TRANSPARENT,
                }));
                for k in 0..s {
                    let row = py as usize * s + k;
                    let line = &mut self.pixels[row * self.stride..(row + 1) * self.stride];
                    for (i, &value) in values.iter().enumerate() {
                        if value != TRANSPARENT {
                            let at = (x + first + i as i32) as usize * s;
                            line[at..at + s].fill(value);
                        }
                    }
                }
                self.scratch = values;
                continue;
            }
            for k in 0..s {
                let row = py as usize * s + k;
                let dither = &DITHER[row & 3];
                let line = &mut self.pixels[row * self.stride..(row + 1) * self.stride];
                for dx in first..=last {
                    let texel = if attrs.textured {
                        match texels[(dx - first) as usize] {
                            Some(texel) => texel,
                            None => continue,
                        }
                    } else {
                        NO_TEXEL
                    };
                    for sx in 0..s {
                        let column = (x + dx) as usize * s + sx;
                        put(&mut line[column], &sh, dither, column, color, &texel);
                    }
                }
            }
        }
        self.texels = texels;
    }

    // ----- Lines -----

    /// Draw a line the way the console does and repeat each of its pixels as a
    /// `scale` x `scale` block, so it is as thick as on the console's screen.
    pub fn draw_line(&mut self, settings: &DrawSettings, attrs: &Attributes, a: &HVertex, b: &HVertex) {
        let too_big = (a.ix - b.ix).abs() > MAX_PRIMITIVE_WIDTH || (a.iy - b.iy).abs() > MAX_PRIMITIVE_HEIGHT;
        if too_big {
            return;
        }
        let s = self.scale;
        let sh = Shading::new(settings, attrs, self.true_color, true);
        let (ax, ay) = (a.ix + settings.offset_x, a.iy + settings.offset_y);
        let (bx, by) = (b.ix + settings.offset_x, b.iy + settings.offset_y);
        let steps = (bx - ax).abs().max((by - ay).abs());
        let clip = &settings.area;

        for i in 0..=steps {
            let (x, y, color) = if steps == 0 {
                (ax, ay, a.color)
            } else {
                let t = i as i64;
                let n = steps as i64;
                let mix = |p: i32, q: i32| (p as i64 + (q as i64 - p as i64) * t / n) as i32;
                let color = if attrs.gouraud {
                    [mix(a.color[0] as i32, b.color[0] as i32) as u8, mix(a.color[1] as i32, b.color[1] as i32) as u8, mix(a.color[2] as i32, b.color[2] as i32) as u8]
                } else {
                    a.color
                };
                (mix(ax, bx), mix(ay, by), color)
            };
            if x < clip.left || x > clip.right || y < clip.top || y > clip.bottom {
                continue;
            }
            let color = [color[0] as i32, color[1] as i32, color[2] as i32];
            for sy in 0..s {
                let row = y as usize * s + sy;
                for sx in 0..s {
                    let column = x as usize * s + sx;
                    put(&mut self.pixels[row * self.stride + column], &sh, &DITHER[row & 3], column, color, &NO_TEXEL);
                }
            }
        }
    }
}

// ----- Shading -----

/// What decides how a pixel is shaded, the same for a whole primitive.
#[derive(Clone, Copy)]
struct Shading {
    check_mask: bool,
    /// `MASK` when the mask bit is forced on, else 0.
    mask_bit: u32,
    semi: bool,
    semi_mode: u8,
    textured: bool,
    raw: bool,
    /// The ordered dither pattern is added before the colour is cut to 5 bits.
    dither: bool,
    true_color: bool,
    /// Texels come straight from VRAM and the console's 5-bit formulas apply
    /// (otherwise their channels have 8 bits).
    exact: bool,
}

impl Shading {
    fn new(settings: &DrawSettings, attrs: &Attributes, true_color: bool, nearest: bool) -> Self {
        let modulated = attrs.textured && !attrs.raw_texture;
        Self {
            check_mask: settings.check_mask,
            mask_bit: if settings.set_mask { MASK } else { 0 },
            semi: attrs.semi_transparent,
            semi_mode: attrs.semi_mode,
            textured: attrs.textured,
            raw: attrs.raw_texture,
            dither: settings.dither && (attrs.gouraud || modulated) && !true_color,
            true_color,
            exact: !true_color && nearest,
        }
    }
}

/// The pixel of a flat opaque primitive of colour `color`.
fn flat_pixel(sh: &Shading, color: [i32; 3]) -> u32 {
    let color = [clamp8(color[0]), clamp8(color[1]), clamp8(color[2])];
    let pixel = if sh.true_color {
        pack8(color)
    } else {
        expand15(((color[0] >> 3) as u16) | (((color[1] >> 3) as u16) << 5) | (((color[2] >> 3) as u16) << 10))
    };
    pixel | sh.mask_bit
}

/// Stands for a transparent texel in a row of sprite pixels (no pixel has bits 25 to 31).
const TRANSPARENT: u32 = u32::MAX;

/// A textured pixel keeps the texel's bit 15 (the semi-transparency flag) in VRAM, where a later
/// pass can test it with the mask check, as the console does.
fn stp_bit(texel: &Texel) -> u32 {
    if texel.stp { MASK } else { 0 }
}

/// The pixel a textured sprite shows for a texel with 5-bit channels, without dithering,
/// semi-transparency or true colour.
fn sprite_pixel(sh: &Shading, color: [i32; 3], texel: &Texel) -> u32 {
    let t = texel.rgb;
    let rgb = if sh.raw {
        [t[0] << 3, t[1] << 3, t[2] << 3]
    } else {
        [((t[0] * color[0]) >> 4).min(255), ((t[1] * color[1]) >> 4).min(255), ((t[2] * color[2]) >> 4).min(255)]
    };
    let channel = |value: i32| QUANTIZED[(value + 4) as usize & 511] as u32;
    (channel(rgb[0]) | channel(rgb[1]) << 8 | channel(rgb[2]) << 16) | sh.mask_bit | stp_bit(texel)
}

/// Shade and write one pixel. `color` is the interpolated vertex colour
/// (0..=255 per channel), `texel` the sampled texture of a textured primitive,
/// `x` the column in the scaled VRAM (the dither pattern follows it) and
/// `dither` the row of the pattern.
#[inline(always)]
fn put(dst: &mut u32, sh: &Shading, dither: &[i32; 4], x: usize, color: [i32; 3], texel: &Texel) {
    let back = *dst;
    if sh.check_mask && back & MASK != 0 {
        return;
    }

    let mut rgb = color;
    let mut semi = sh.semi;
    if sh.textured {
        let t = texel.rgb;
        rgb = match (sh.exact, sh.raw) {
            (true, true) => [t[0] << 3, t[1] << 3, t[2] << 3],
            // texel * colour / 128, the texel scaled up to 8 bits.
            (true, false) => [
                ((t[0] * color[0]) >> 4).min(255),
                ((t[1] * color[1]) >> 4).min(255),
                ((t[2] * color[2]) >> 4).min(255),
            ],
            (false, true) => t,
            (false, false) => [
                ((t[0] * color[0]) >> 7).min(255),
                ((t[1] * color[1]) >> 7).min(255),
                ((t[2] * color[2]) >> 7).min(255),
            ],
        };
        // Only texels with bit 15 set are blended in a semi-transparent primitive.
        semi = sh.semi && texel.stp;
    }

    let pixel = if sh.true_color {
        let mut front = [clamp8(rgb[0]), clamp8(rgb[1]), clamp8(rgb[2])];
        if semi {
            front = blend8(back, front, sh.semi_mode);
        }
        pack8(front)
    } else {
        let offset = if sh.dither { dither[x & 3] } else { 0 } + 4;
        let channel = |value: i32| QUANTIZED[(value + offset) as usize & 511] as u32;
        let quantized = channel(rgb[0]) | channel(rgb[1]) << 8 | channel(rgb[2]) << 16;
        if semi {
            // Blend in the console's 5-bit channels.
            let front = (quantized & 0xFF) >> 3 | ((quantized >> 8) & 0xFF) >> 3 << 5 | ((quantized >> 16) & 0xFF) >> 3 << 10;
            expand15(blend(pack15(back), front as u16, sh.semi_mode))
        } else {
            quantized
        }
    };
    *dst = pixel | sh.mask_bit | stp_bit(texel);
}

// ----- Spans -----

/// What a row of a triangle needs besides its pixels.
struct SpanContext<'a> {
    sh: Shading,
    flat: [i32; 3],
    /// Twice the area of the triangle: the divisor of the interpolation.
    divisor: i32,
    sampler: Sampler,
    vram: &'a Vram,
    filter: TextureFilter,
    bounds: UvBounds,
}

/// `ceil(n / d)` for positive `n` and `d`, given `inverse = 1.0 / d`.
#[inline(always)]
fn ceil_div(n: i64, d: i64, inverse: f64) -> i64 {
    let mut q = (n as f64 * inverse) as i64;
    while q * d < n {
        q += 1;
    }
    while q > 0 && (q - 1) * d >= n {
        q -= 1;
    }
    q
}

/// `floor(n / d)` for non-negative `n` and positive `d`, given `inverse = 1.0 / d`.
#[inline(always)]
fn floor_div(n: i64, d: i64, inverse: f64) -> i64 {
    let mut q = (n as f64 * inverse) as i64;
    while q * d > n {
        q -= 1;
    }
    while (q + 1) * d <= n {
        q += 1;
    }
    q
}

/// Shade the pixels `row` of one scanline of a triangle, starting at column
/// `x0`; `colors` and `uv` hold the interpolated values at the first pixel.
#[inline(never)]
fn shade_span<const GOURAUD: bool, const TEXTURED: bool>(
    row: &mut [u32],
    x0: usize,
    y: usize,
    context: &SpanContext,
    mut colors: [Lane; 3],
    mut uv: [Lane; 2],
) {
    let sh = &context.sh;
    let mut taps = Taps::default();
    let dither = &DITHER[y & 3];
    let divisor = context.divisor;
    for (i, dst) in row.iter_mut().enumerate() {
        let color = if GOURAUD {
            [clamp8(colors[0].quotient), clamp8(colors[1].quotient), clamp8(colors[2].quotient)]
        } else {
            context.flat
        };
        if TEXTURED {
            let texel = if context.filter == TextureFilter::Nearest {
                nearest_texel(
                    &context.sampler,
                    context.vram,
                    uv[0].quotient.clamp(0, 0xFFFF) as u32 >> 8,
                    uv[1].quotient.clamp(0, 0xFFFF) as u32 >> 8,
                    sh.exact,
                )
            } else {
                bilinear_texel(&mut taps, &context.sampler, context.vram, uv[0].quotient as i64, uv[1].quotient as i64, &context.bounds)
            };
            if let Some(texel) = texel {
                put(dst, sh, dither, x0 + i, color, &texel);
            }
            uv[0].advance(divisor);
            uv[1].advance(divisor);
        } else {
            put(dst, sh, dither, x0 + i, color, &NO_TEXEL);
        }
        if GOURAUD {
            colors[0].advance(divisor);
            colors[1].advance(divisor);
            colors[2].advance(divisor);
        }
    }
}

/// Blend a quantised colour (8-bit channels holding 5-bit values) with the
/// pixel behind it, the way the console does it in its 15-bit VRAM.
#[inline(always)]
fn blend_quantized(back: u32, quantized: u32, mode: u8) -> u32 {
    let front = ((quantized & 0xFF) >> 3) | ((((quantized >> 8) & 0xFF) >> 3) << 5) | ((((quantized >> 16) & 0xFF) >> 3) << 10);
    expand15(blend(pack15(back), front as u16, mode))
}

/// `shade_span` for an untextured triangle, as a hand-flattened loop. With `BLEND`
/// it reads the pixel behind (mask check, semi-transparency); otherwise it only
/// writes, and a flat colour is a fill. With `TRUE_COLOR` the colours keep 8 bits.
#[inline(never)]
fn shade_span_untextured<const GOURAUD: bool, const BLEND: bool, const TRUE_COLOR: bool>(
    row: &mut [u32],
    x0: usize,
    y: usize,
    context: &SpanContext,
    mut colors: [Lane; 3],
) {
    let sh = &context.sh;
    if !GOURAUD && !BLEND {
        row.fill(flat_pixel(sh, context.flat));
        return;
    }
    let dither = &DITHER[y & 3];
    let divisor = context.divisor;
    let (use_dither, mask_bit, semi, check_mask, mode) = (sh.dither, sh.mask_bit, sh.semi, sh.check_mask, sh.semi_mode);
    let flat = context.flat;
    for (i, dst) in row.iter_mut().enumerate() {
        if !(BLEND && check_mask && *dst & MASK != 0) {
            let color = if GOURAUD {
                [clamp8(colors[0].quotient), clamp8(colors[1].quotient), clamp8(colors[2].quotient)]
            } else {
                flat
            };
            let pixel = if TRUE_COLOR {
                if BLEND && semi { pack8(blend8(*dst, color, mode)) } else { pack8(color) }
            } else {
                let offset = if use_dither { dither[(x0 + i) & 3] } else { 0 } + 4;
                let channel = |value: i32| QUANTIZED[(value + offset) as usize & 511] as u32;
                let quantized = channel(color[0]) | channel(color[1]) << 8 | channel(color[2]) << 16;
                if BLEND && semi { blend_quantized(*dst, quantized, mode) } else { quantized }
            };
            *dst = pixel | mask_bit;
        }
        if GOURAUD {
            colors[0].advance(divisor);
            colors[1].advance(divisor);
            colors[2].advance(divisor);
        }
    }
}

/// A texture coordinate (0..256 texels) as 16.16 fixed point.
fn fixed_16(texels: f64) -> i32 {
    (texels * 65536.0).clamp(0.0, 16_777_215.0) as i32
}

/// `shade_span` for the common textured case, nearest sampling, as a hand-flattened
/// loop. With `BLEND` it reads the pixel behind (mask check, semi-transparency);
/// otherwise it only writes. With `PERSPECTIVE` the texture coordinates come from
/// `plane` (exact every `PERSPECTIVE_SPAN` pixels and stepped linearly in between)
/// instead of `uv`. With `TRUE_COLOR` texels and colours keep 8 bits.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn shade_span_nearest<const GOURAUD: bool, const BLEND: bool, const PERSPECTIVE: bool, const TRUE_COLOR: bool>(
    row: &mut [u32],
    x0: usize,
    y: usize,
    context: &SpanContext,
    mut colors: [Lane; 3],
    mut uv: [Lane; 2],
    plane: Option<&PerspectivePlane>,
    at_first: [i64; 3],
) {
    let sh = &context.sh;
    let dither = &DITHER[y & 3];
    let divisor = context.divisor;
    let (sampler, vram) = (&context.sampler, context.vram);
    let (raw, use_dither, mask_bit, semi, check_mask, mode) =
        (sh.raw, sh.dither, sh.mask_bit, sh.semi, sh.check_mask, sh.semi_mode);
    let flat = context.flat;
    let last = row.len() as i64 - 1;
    let (mut u_fixed, mut v_fixed, mut u_step, mut v_step) = (0i32, 0i32, 0i32, 0i32);
    for (i, dst) in row.iter_mut().enumerate() {
        if PERSPECTIVE && i as i64 % PERSPECTIVE_SPAN == 0 {
            let plane = plane.expect("perspective spans have a plane");
            let k = i as i64;
            let end = (k + PERSPECTIVE_SPAN - 1).min(last);
            let (u, v) = plane.texture_at(at_first, k);
            (u_fixed, v_fixed) = (fixed_16(u), fixed_16(v));
            if end > k {
                let (u_end, v_end) = plane.texture_at(at_first, end);
                let span = (end - k) as f64;
                u_step = ((u_end - u) / span * 65536.0) as i32;
                v_step = ((v_end - v) / span * 65536.0) as i32;
            } else {
                (u_step, v_step) = (0, 0);
            }
        }
        if !(BLEND && check_mask && *dst & MASK != 0) {
            let (u, v) = if PERSPECTIVE {
                // Stepping can leave the 0..256 range by a rounding error at an edge of the texture.
                ((u_fixed >> 16).clamp(0, 255) as u32, (v_fixed >> 16).clamp(0, 255) as u32)
            } else {
                ((uv[0].quotient >> 8) as u32 & 0xFF, (uv[1].quotient >> 8) as u32 & 0xFF)
            };
            let texel = sampler.sample(vram, u, v);
            if texel != 0 {
                let mut c = [(texel & 31) as i32, ((texel >> 5) & 31) as i32, ((texel >> 10) & 31) as i32];
                if TRUE_COLOR {
                    c = [EXPAND5[c[0] as usize] as i32, EXPAND5[c[1] as usize] as i32, EXPAND5[c[2] as usize] as i32];
                }
                let color = if GOURAUD {
                    [clamp8(colors[0].quotient), clamp8(colors[1].quotient), clamp8(colors[2].quotient)]
                } else {
                    flat
                };
                // Only texels with bit 15 set are blended in a semi-transparent primitive.
                let blended = BLEND && semi && texel & 0x8000 != 0;
                let pixel = if TRUE_COLOR {
                    // texel * colour / 128 with 8-bit texels.
                    let rgb = if raw {
                        c
                    } else {
                        [((c[0] * color[0]) >> 7).min(255), ((c[1] * color[1]) >> 7).min(255), ((c[2] * color[2]) >> 7).min(255)]
                    };
                    if blended { pack8(blend8(*dst, rgb, mode)) } else { pack8(rgb) }
                } else {
                    let rgb = if raw {
                        [c[0] << 3, c[1] << 3, c[2] << 3]
                    } else {
                        [((c[0] * color[0]) >> 4).min(255), ((c[1] * color[1]) >> 4).min(255), ((c[2] * color[2]) >> 4).min(255)]
                    };
                    let offset = if use_dither { dither[(x0 + i) & 3] } else { 0 } + 4;
                    let channel = |value: i32| QUANTIZED[(value + offset) as usize & 511] as u32;
                    let quantized = channel(rgb[0]) | channel(rgb[1]) << 8 | channel(rgb[2]) << 16;
                    if blended { blend_quantized(*dst, quantized, mode) } else { quantized }
                };
                *dst = pixel | mask_bit | if texel & 0x8000 != 0 { MASK } else { 0 };
            }
        }
        if PERSPECTIVE {
            u_fixed += u_step;
            v_fixed += v_step;
        } else {
            uv[0].advance(divisor);
            uv[1].advance(divisor);
        }
        if GOURAUD {
            colors[0].advance(divisor);
            colors[1].advance(divisor);
            colors[2].advance(divisor);
        }
    }
}

/// The values perspective-correct texturing interpolates: `1/w`, `u/w` and
/// `v/w` at each vertex, and how the edge functions step along a row.
struct PerspectivePlane {
    inverse_w: [f64; 3],
    u_over_w: [f64; 3],
    v_over_w: [f64; 3],
    step_x: [f64; 3],
}

impl PerspectivePlane {
    fn new(w: [f32; 3], u: [u8; 3], v: [u8; 3], step_x: [i64; 3]) -> Self {
        let inverse_w = [1.0 / w[0] as f64, 1.0 / w[1] as f64, 1.0 / w[2] as f64];
        let over_w = |values: [u8; 3]| [values[0] as f64 * inverse_w[0], values[1] as f64 * inverse_w[1], values[2] as f64 * inverse_w[2]];
        Self { inverse_w, u_over_w: over_w(u), v_over_w: over_w(v), step_x: step_x.map(|step| step as f64) }
    }

    /// The exact texture coordinate `k` pixels after the first of a row,
    /// whose edge functions are `at_first` there.
    fn texture_at(&self, at_first: [i64; 3], k: i64) -> (f64, f64) {
        let e = [
            at_first[0] as f64 + k as f64 * self.step_x[0],
            at_first[1] as f64 + k as f64 * self.step_x[1],
            at_first[2] as f64 + k as f64 * self.step_x[2],
        ];
        let inverse = e[0] * self.inverse_w[0] + e[1] * self.inverse_w[1] + e[2] * self.inverse_w[2];
        let u = e[0] * self.u_over_w[0] + e[1] * self.u_over_w[1] + e[2] * self.u_over_w[2];
        let v = e[0] * self.v_over_w[0] + e[1] * self.v_over_w[1] + e[2] * self.v_over_w[2];
        if inverse > 0.0 { (u / inverse, v / inverse) } else { (0.0, 0.0) }
    }
}

/// The colours of a Gouraud triangle as floating point planes, for the triangles too
/// big for the exact interpolation.
struct FloatColors {
    values: [[f64; 3]; 3],
    step_x: [f64; 3],
    inverse_area: f64,
}

impl FloatColors {
    /// `colors[c][i]` is channel `c` of vertex `i`.
    fn new(colors: [[u8; 3]; 3], step_x: [i64; 3], area: i64) -> Self {
        let values = colors.map(|channel| channel.map(|value| value as f64));
        Self { values, step_x: step_x.map(|step| step as f64), inverse_area: 1.0 / area as f64 }
    }

    /// The colour `k` pixels after the first of a row, whose edge functions are `at_first` there.
    fn at(&self, at_first: [i64; 3], k: i64) -> [i32; 3] {
        let e = [
            at_first[0] as f64 + k as f64 * self.step_x[0],
            at_first[1] as f64 + k as f64 * self.step_x[1],
            at_first[2] as f64 + k as f64 * self.step_x[2],
        ];
        self.values.map(|channel| {
            let value = (e[0] * channel[0] + e[1] * channel[1] + e[2] * channel[2]) * self.inverse_area;
            (value + 1e-6).floor().clamp(0.0, 255.0) as i32
        })
    }
}

/// `shade_span` with floating point interpolation: for textured triangles with
/// perspective-correct texture coordinates (exact every `PERSPECTIVE_SPAN` pixels,
/// linear in between) and for triangles too big for the exact lanes. The colours
/// come from `float_colors` when there are some, else from the exact `colors`.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn shade_float_span(
    row: &mut [u32],
    x0: usize,
    y: usize,
    context: &SpanContext,
    gouraud: bool,
    mut colors: [Lane; 3],
    float_colors: Option<&FloatColors>,
    plane: Option<&PerspectivePlane>,
    at_first: [i64; 3],
) {
    let sh = &context.sh;
    let mut taps = Taps::default();
    let dither = &DITHER[y & 3];
    let divisor = context.divisor;
    let last = row.len() as i64 - 1;
    let (mut u, mut v, mut u_step, mut v_step) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (i, dst) in row.iter_mut().enumerate() {
        let k = i as i64;
        let color = match float_colors {
            Some(float_colors) => float_colors.at(at_first, k),
            None if gouraud => [clamp8(colors[0].quotient), clamp8(colors[1].quotient), clamp8(colors[2].quotient)],
            None => context.flat,
        };
        if let Some(plane) = plane {
            if k % PERSPECTIVE_SPAN == 0 {
                let end = (k + PERSPECTIVE_SPAN - 1).min(last);
                (u, v) = plane.texture_at(at_first, k);
                if end > k {
                    let (u_end, v_end) = plane.texture_at(at_first, end);
                    u_step = (u_end - u) / (end - k) as f64;
                    v_step = (v_end - v) / (end - k) as f64;
                } else {
                    (u_step, v_step) = (0.0, 0.0);
                }
            }
            let fixed = ((u * 256.0).floor().clamp(0.0, 65535.0) as i64, (v * 256.0).floor().clamp(0.0, 65535.0) as i64);
            u += u_step;
            v += v_step;
            let texel = if context.filter == TextureFilter::Nearest {
                nearest_texel(&context.sampler, context.vram, (fixed.0 >> 8) as u32, (fixed.1 >> 8) as u32, sh.exact)
            } else {
                bilinear_texel(&mut taps, &context.sampler, context.vram, fixed.0, fixed.1, &context.bounds)
            };
            if let Some(texel) = texel {
                put(dst, sh, dither, x0 + i, color, &texel);
            }
        } else {
            put(dst, sh, dither, x0 + i, color, &NO_TEXEL);
        }
        if float_colors.is_none() && gouraud {
            colors[0].advance(divisor);
            colors[1].advance(divisor);
            colors[2].advance(divisor);
        }
    }
}

#[cfg(test)]
mod tests;
