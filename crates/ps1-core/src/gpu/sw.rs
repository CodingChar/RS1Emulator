//! Software rasterizer for the PS1 GPU.
//!
//! Draws into the 1024x512 halfword VRAM: Gouraud-shaded and textured
//! triangles, rectangles ("sprites"), lines and VRAM fills/copies, with
//! semi-transparency, dithering, texture windows, the mask bit and the
//! drawing area clip.
//!
//! Mirrors DuckStation's `gpu_sw_rasterizer.cpp`.
use super::lane::Lane;
use serde::{Deserialize, Serialize};

pub const VRAM_WIDTH: usize = 1024;
pub const VRAM_HEIGHT: usize = 512;

/// The largest span the hardware draws: bigger primitives are dropped.
pub(super) const MAX_PRIMITIVE_WIDTH: i32 = 1023;
pub(super) const MAX_PRIMITIVE_HEIGHT: i32 = 511;

/// 4x4 ordered-dither offsets added to the 8-bit colour before it is cut to
/// 5 bits.
pub(super) const DITHER: [[i32; 4]; 4] = [
    [-4, 0, -3, 1],
    [2, -2, 3, -1],
    [-3, 1, -4, 0],
    [3, -1, 2, -2],
];

/// 1 MB of video memory as 1024x512 15-bit pixels.
#[derive(Clone, Serialize, Deserialize)]
pub struct Vram {
    pub pixels: Vec<u16>,
}

impl Vram {
    pub fn new() -> Self {
        Self { pixels: vec![0; VRAM_WIDTH * VRAM_HEIGHT] }
    }

    /// Read a pixel; coordinates wrap around the edges.
    pub fn get(&self, x: usize, y: usize) -> u16 {
        self.pixels[(y % VRAM_HEIGHT) * VRAM_WIDTH + (x % VRAM_WIDTH)]
    }

    pub fn set(&mut self, x: usize, y: usize, value: u16) {
        self.pixels[(y % VRAM_HEIGHT) * VRAM_WIDTH + (x % VRAM_WIDTH)] = value;
    }
}

impl Default for Vram {
    fn default() -> Self {
        Self::new()
    }
}

/// Inclusive clip rectangle set with GP0(E3h)/(E4h).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawArea {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Texture window (GP0 E2h): in units of 8 texels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureWindow {
    pub mask_x: u32,
    pub mask_y: u32,
    pub offset_x: u32,
    pub offset_y: u32,
}

/// Global drawing state shared by every primitive.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrawSettings {
    pub area: DrawArea,
    pub offset_x: i32,
    pub offset_y: i32,
    pub window: TextureWindow,
    pub dither: bool,
    /// GP0(E6h) bit 0: force bit 15 of every pixel written.
    pub set_mask: bool,
    /// GP0(E6h) bit 1: do not overwrite pixels whose bit 15 is set.
    pub check_mask: bool,
}

impl Default for DrawSettings {
    fn default() -> Self {
        Self {
            area: DrawArea { left: 0, top: 0, right: 0, bottom: 0 },
            offset_x: 0,
            offset_y: 0,
            window: TextureWindow::default(),
            dither: false,
            set_mask: false,
            check_mask: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureDepth {
    /// 4 bits per texel, looked up in a 16-entry palette.
    Bits4,
    /// 8 bits per texel, 256-entry palette.
    Bits8,
    /// 15-bit direct colour.
    Bits15,
}

/// Where a textured primitive reads its texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Texture {
    /// Texture page origin in VRAM pixels.
    pub page_x: u32,
    pub page_y: u32,
    pub depth: TextureDepth,
    /// Palette origin in VRAM pixels.
    pub clut_x: u32,
    pub clut_y: u32,
}

impl Default for Texture {
    fn default() -> Self {
        Self { page_x: 0, page_y: 0, depth: TextureDepth::Bits4, clut_x: 0, clut_y: 0 }
    }
}

/// How one primitive is shaded.
#[derive(Clone, Copy, Debug, Default)]
pub struct Attributes {
    pub textured: bool,
    /// Textured but not modulated by the vertex colour.
    pub raw_texture: bool,
    pub semi_transparent: bool,
    /// 0: B/2+F/2, 1: B+F, 2: B-F, 3: B+F/4.
    pub semi_mode: u8,
    pub gouraud: bool,
    pub texture: Texture,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vertex {
    pub x: i32,
    pub y: i32,
    pub color: [u8; 3],
    pub u: u8,
    pub v: u8,
}

// ----- Pixel pipeline -----

pub(super) fn blend_component(back: u16, front: u16, mode: u8) -> u16 {
    match mode {
        0 => (back + front) / 2,
        1 => (back + front).min(31),
        2 => back.saturating_sub(front),
        _ => (back + front / 4).min(31),
    }
}

pub(super) fn blend(back: u16, front: u16, mode: u8) -> u16 {
    let channel = |shift: u32| {
        blend_component((back >> shift) & 31, (front >> shift) & 31, mode) << shift
    };
    channel(0) | channel(5) | channel(10)
}

/// Texel lookup with the texture window and the palette position worked out
/// once per primitive instead of once per pixel.
#[derive(Clone, Copy)]
pub(super) struct Sampler {
    keep_u: u32,
    force_u: u32,
    keep_v: u32,
    force_v: u32,
    page_x: usize,
    page_y: usize,
    clut_x: usize,
    clut_y: usize,
    depth: TextureDepth,
}

impl Sampler {
    pub(super) fn new(window: &TextureWindow, texture: &Texture) -> Self {
        Self {
            keep_u: !(window.mask_x * 8),
            force_u: (window.offset_x & window.mask_x) * 8,
            keep_v: !(window.mask_y * 8),
            force_v: (window.offset_y & window.mask_y) * 8,
            page_x: texture.page_x as usize,
            page_y: texture.page_y as usize,
            clut_x: texture.clut_x as usize,
            clut_y: texture.clut_y as usize,
            depth: texture.depth,
        }
    }

    /// Fetch one texel (a 15-bit colour, 0 meaning transparent).
    #[inline(always)]
    pub(super) fn sample(&self, vram: &Vram, u: u32, v: u32) -> u16 {
        let u = (u & self.keep_u) | self.force_u;
        let v = (v & self.keep_v) | self.force_v;
        let py = self.page_y + v as usize;
        match self.depth {
            TextureDepth::Bits4 => {
                let word = vram.get(self.page_x + (u / 4) as usize, py);
                let index = (word >> ((u % 4) * 4)) & 0xF;
                vram.get(self.clut_x + index as usize, self.clut_y)
            }
            TextureDepth::Bits8 => {
                let word = vram.get(self.page_x + (u / 2) as usize, py);
                let index = (word >> ((u % 2) * 8)) & 0xFF;
                vram.get(self.clut_x + index as usize, self.clut_y)
            }
            TextureDepth::Bits15 => vram.get(self.page_x + u as usize, py),
        }
    }
}

/// Shade and write the pixel at `(x, y)`, which must be inside VRAM and the
/// drawing area. `color` is the interpolated vertex colour (0..=255); `u` and
/// `v` are the texel coordinate for textured primitives.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn shade_pixel(
    vram: &mut Vram,
    settings: &DrawSettings,
    attrs: &Attributes,
    sampler: &Sampler,
    x: usize,
    y: usize,
    color: [i32; 3],
    u: u32,
    v: u32,
) {
    let back = vram.get(x, y);
    if settings.check_mask && back & 0x8000 != 0 {
        return;
    }

    let mut rgb = color;
    let mut semi = attrs.semi_transparent;
    let mut modulated = false;
    // A textured pixel keeps the texel's bit 15 in VRAM (a later pass can test it with the mask check).
    let mut stp = 0;
    if attrs.textured {
        let texel = sampler.sample(vram, u, v);
        stp = texel & 0x8000;
        if texel == 0 {
            return; // fully transparent
        }
        let channels = [
            (texel & 31) as i32,
            ((texel >> 5) & 31) as i32,
            ((texel >> 10) & 31) as i32,
        ];
        if attrs.raw_texture {
            rgb = [channels[0] << 3, channels[1] << 3, channels[2] << 3];
        } else {
            // texel * colour / 128, with the texel scaled up to 8 bits.
            rgb = [
                ((channels[0] * color[0]) >> 4).min(255),
                ((channels[1] * color[1]) >> 4).min(255),
                ((channels[2] * color[2]) >> 4).min(255),
            ];
            modulated = true;
        }
        // Only texels with bit 15 set are blended in a semi-transparent primitive.
        semi = attrs.semi_transparent && texel & 0x8000 != 0;
    }

    if settings.dither && (attrs.gouraud || modulated) {
        let offset = DITHER[y & 3][x & 3];
        for c in rgb.iter_mut() {
            *c = (*c + offset).clamp(0, 255);
        }
    }

    let mut pixel = ((rgb[0] >> 3) as u16) | (((rgb[1] >> 3) as u16) << 5) | (((rgb[2] >> 3) as u16) << 10);
    if semi {
        pixel = blend(back, pixel, attrs.semi_mode);
    }
    if settings.set_mask {
        pixel |= 0x8000;
    }
    vram.set(x, y, pixel | stp);
}

/// Shade and write one pixel, if it is inside the drawing area.
#[allow(clippy::too_many_arguments)]
fn put_pixel(
    vram: &mut Vram,
    settings: &DrawSettings,
    attrs: &Attributes,
    sampler: &Sampler,
    x: i32,
    y: i32,
    color: [i32; 3],
    uv: (u32, u32),
) {
    let area = &settings.area;
    if x < area.left || x > area.right || y < area.top || y > area.bottom {
        return;
    }
    let (xu, yu) = (x as usize & (VRAM_WIDTH - 1), y as usize & (VRAM_HEIGHT - 1));
    shade_pixel(vram, settings, attrs, sampler, xu, yu, color, uv.0, uv.1);
}

// ----- Triangles -----

fn edge(a: &Vertex, b: &Vertex, px: i32, py: i32) -> i64 {
    (b.x - a.x) as i64 * (py - a.y) as i64 - (b.y - a.y) as i64 * (px - a.x) as i64
}

/// Pixels exactly on a top or left edge belong to the triangle; pixels on
/// the other edges belong to the neighbour.
fn is_top_left(a: &Vertex, b: &Vertex) -> bool {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    dy < 0 || (dy == 0 && dx > 0)
}

fn too_big(a: &Vertex, b: &Vertex) -> bool {
    (a.x - b.x).abs() > MAX_PRIMITIVE_WIDTH || (a.y - b.y).abs() > MAX_PRIMITIVE_HEIGHT
}

/// Draw a triangle. Vertex coordinates are before the drawing offset.
///
/// The edge functions are stepped incrementally, each scanline's covered span
/// is found directly from them, and the interpolated values are stepped the
/// same way (see `Lane`), so a pixel costs a couple of additions instead of a
/// dozen multiplications and divisions.
pub fn draw_triangle(vram: &mut Vram, settings: &DrawSettings, attrs: &Attributes, vertices: [Vertex; 3]) {
    let [mut v0, mut v1, mut v2] = vertices;
    if too_big(&v0, &v1) || too_big(&v1, &v2) || too_big(&v2, &v0) {
        return;
    }
    for v in [&mut v0, &mut v1, &mut v2] {
        v.x += settings.offset_x;
        v.y += settings.offset_y;
    }

    let mut area = edge(&v0, &v1, v2.x, v2.y);
    if area == 0 {
        return;
    }
    if area < 0 {
        std::mem::swap(&mut v1, &mut v2);
        area = -area;
    }

    let clip = &settings.area;
    let min_x = v0.x.min(v1.x).min(v2.x).max(clip.left);
    let max_x = v0.x.max(v1.x).max(v2.x).min(clip.right);
    let min_y = v0.y.min(v1.y).min(v2.y).max(clip.top);
    let max_y = v0.y.max(v1.y).max(v2.y).min(clip.bottom);
    if min_x > max_x || min_y > max_y {
        return;
    }

    // Edge i is (v1,v2), (v2,v0), (v0,v1): its function is the weight of the
    // opposite vertex. A pixel is inside when every weight is positive, or
    // zero on a top/left edge: `weight >= bias`.
    let edges = [(&v1, &v2), (&v2, &v0), (&v0, &v1)];
    let mut step_x = [0i64; 3];
    let mut step_y = [0i64; 3];
    let mut bias = [0i64; 3];
    let mut row_weight = [0i64; 3];
    for (i, (a, b)) in edges.iter().enumerate() {
        step_x[i] = -((b.y - a.y) as i64);
        step_y[i] = (b.x - a.x) as i64;
        bias[i] = if is_top_left(a, b) { 0 } else { 1 };
        row_weight[i] = edge(a, b, min_x, min_y);
    }

    let divisor = area;
    let inverse = 1.0 / divisor as f64;
    let sampler = Sampler::new(&settings.window, &attrs.texture);
    let shading = Shading::new(settings, attrs);
    // Whether a pixel reads what is behind it: semi-transparency and the mask check do.
    let blend = settings.check_mask || attrs.semi_transparent;
    let flat = [v0.color[0] as i32, v0.color[1] as i32, v0.color[2] as i32];
    // Values interpolated across the triangle: R, G, B, U, V.
    let channels: [[i64; 3]; 5] = [
        [v0.color[0] as i64, v1.color[0] as i64, v2.color[0] as i64],
        [v0.color[1] as i64, v1.color[1] as i64, v2.color[1] as i64],
        [v0.color[2] as i64, v1.color[2] as i64, v2.color[2] as i64],
        [v0.u as i64, v1.u as i64, v2.u as i64],
        [v0.v as i64, v1.v as i64, v2.v as i64],
    ];
    // How each value changes from one pixel to the next, as quotient and remainder.
    let mut lanes = [Lane::default(); 5];
    // The lanes in use are `first_used..used`: R, G, B (Gouraud), then U and V (textured).
    let (first_used, used) = match (attrs.gouraud, attrs.textured) {
        (false, false) => (0, 0),
        (true, false) => (0, 3),
        (false, true) => (3, 5),
        (true, true) => (0, 5),
    };
    for c in first_used..used {
        lanes[c] = Lane::new((0..3).map(|i| step_x[i] * channels[c][i]).sum(), divisor, inverse);
    }

    let width = (max_x - min_x) as i64;
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
                    first = first.max((need + step - 1) / step);
                }
            } else {
                let room = w - bias[i];
                if room < 0 {
                    empty = true;
                } else {
                    last = last.min(room / -step);
                }
            }
        }
        if empty || first > last {
            continue;
        }

        let yu = y as usize & (VRAM_HEIGHT - 1);
        let mut now = lanes;
        for c in first_used..used {
            let numerator = (0..3).map(|i| (weights[i] + first * step_x[i]) * channels[c][i]).sum();
            now[c].start(numerator, divisor, inverse);
        }
        // The clip rectangle is inside VRAM, so the span does not wrap.
        let x0 = (min_x + first as i32) as usize;
        let count = (last - first + 1) as usize;
        let base = yu * VRAM_WIDTH + x0;
        let divisor = divisor as i32;
        match (attrs.gouraud, attrs.textured, blend) {
            (false, false, false) => span_untextured::<false, false>(&mut vram.pixels[base..base + count], x0, yu, &shading, flat, now, divisor),
            (false, false, true) => span_untextured::<false, true>(&mut vram.pixels[base..base + count], x0, yu, &shading, flat, now, divisor),
            (true, false, false) => span_untextured::<true, false>(&mut vram.pixels[base..base + count], x0, yu, &shading, flat, now, divisor),
            (true, false, true) => span_untextured::<true, true>(&mut vram.pixels[base..base + count], x0, yu, &shading, flat, now, divisor),
            (false, true, false) => span_textured::<false, false>(vram, base, count, x0, yu, &shading, &sampler, flat, now, divisor),
            (false, true, true) => span_textured::<false, true>(vram, base, count, x0, yu, &shading, &sampler, flat, now, divisor),
            (true, true, false) => span_textured::<true, false>(vram, base, count, x0, yu, &shading, &sampler, flat, now, divisor),
            (true, true, true) => span_textured::<true, true>(vram, base, count, x0, yu, &shading, &sampler, flat, now, divisor),
        }
    }
}

// ----- Spans of a triangle -----
//
// The rows of a triangle are shaded by loops specialised for what the triangle
// needs (Gouraud or flat, textured or not, with or without a read of the pixel
// behind), which is much faster than one general function per pixel. They do
// exactly what `shade_pixel` does; the tests compare them with it.

/// A colour channel (0..=255 plus a dither offset of -4..=3, shifted by 4 to keep
/// the index positive) cut to the 5 bits of the console's VRAM.
const QUANTIZED_5: [u8; 512] = {
    let mut table = [0u8; 512];
    let mut i = 0;
    while i < 512 {
        let value = i as i32 - 4;
        let clamped = if value < 0 { 0 } else if value > 255 { 255 } else { value };
        table[i] = (clamped >> 3) as u8;
        i += 1;
    }
    table
};

/// What a row of a triangle needs to know about how it is shaded.
struct Shading {
    check_mask: bool,
    mask_bit: u16,
    semi: bool,
    semi_mode: u8,
    raw: bool,
    dither: bool,
}

impl Shading {
    fn new(settings: &DrawSettings, attrs: &Attributes) -> Self {
        let modulated = attrs.textured && !attrs.raw_texture;
        Self {
            check_mask: settings.check_mask,
            mask_bit: if settings.set_mask { 0x8000 } else { 0 },
            semi: attrs.semi_transparent,
            semi_mode: attrs.semi_mode,
            raw: attrs.raw_texture,
            dither: settings.dither && (attrs.gouraud || modulated),
        }
    }
}

#[inline(always)]
fn pack_quantized(rgb: [i32; 3], offset: i32) -> u16 {
    let channel = |value: i32| QUANTIZED_5[(value + offset) as usize & 511] as u16;
    channel(rgb[0]) | channel(rgb[1]) << 5 | channel(rgb[2]) << 10
}

/// Shade an untextured row. With `BLEND` it reads the pixel behind (mask check,
/// semi-transparency); without it only writes, and a flat colour is a fill.
#[inline(never)]
fn span_untextured<const GOURAUD: bool, const BLEND: bool>(
    row: &mut [u16],
    x0: usize,
    y: usize,
    sh: &Shading,
    flat: [i32; 3],
    mut colors: [Lane; 5],
    divisor: i32,
) {
    if !GOURAUD && !BLEND {
        row.fill(pack_quantized(flat, 4) | sh.mask_bit);
        return;
    }
    let dither = &DITHER[y & 3];
    for (i, dst) in row.iter_mut().enumerate() {
        let back = *dst;
        if !(BLEND && sh.check_mask && back & 0x8000 != 0) {
            let color = if GOURAUD {
                [colors[0].quotient, colors[1].quotient, colors[2].quotient]
            } else {
                flat
            };
            let offset = if sh.dither { dither[(x0 + i) & 3] } else { 0 } + 4;
            let mut pixel = pack_quantized(color, offset);
            if BLEND && sh.semi {
                pixel = blend(back, pixel, sh.semi_mode);
            }
            *dst = pixel | sh.mask_bit;
        }
        if GOURAUD {
            colors[0].advance(divisor);
            colors[1].advance(divisor);
            colors[2].advance(divisor);
        }
    }
}

/// Shade a textured row (`base` is the index in VRAM of its first pixel). The texture
/// is read from the same VRAM the row is written to, as the console does.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn span_textured<const GOURAUD: bool, const BLEND: bool>(
    vram: &mut Vram,
    base: usize,
    count: usize,
    x0: usize,
    y: usize,
    sh: &Shading,
    sampler: &Sampler,
    flat: [i32; 3],
    mut lanes: [Lane; 5],
    divisor: i32,
) {
    let dither = &DITHER[y & 3];
    for i in 0..count {
        let index = base + i;
        let back = if BLEND { vram.pixels[index] } else { 0 };
        if !(BLEND && sh.check_mask && back & 0x8000 != 0) {
            let u = (lanes[3].quotient as u32).min(255);
            let v = (lanes[4].quotient as u32).min(255);
            let texel = sampler.sample(vram, u, v);
            if texel != 0 {
                let channels = [(texel & 31) as i32, ((texel >> 5) & 31) as i32, ((texel >> 10) & 31) as i32];
                let rgb = if sh.raw {
                    [channels[0] << 3, channels[1] << 3, channels[2] << 3]
                } else {
                    // texel * colour / 128, with the texel scaled up to 8 bits.
                    let color = if GOURAUD {
                        [lanes[0].quotient, lanes[1].quotient, lanes[2].quotient]
                    } else {
                        flat
                    };
                    [
                        ((channels[0] * color[0]) >> 4).min(255),
                        ((channels[1] * color[1]) >> 4).min(255),
                        ((channels[2] * color[2]) >> 4).min(255),
                    ]
                };
                let offset = if sh.dither { dither[(x0 + i) & 3] } else { 0 } + 4;
                let mut pixel = pack_quantized(rgb, offset);
                // Only texels with bit 15 set are blended in a semi-transparent primitive.
                if BLEND && sh.semi && texel & 0x8000 != 0 {
                    pixel = blend(back, pixel, sh.semi_mode);
                }
                // The texel's bit 15 stays in VRAM, where a later pass can test it with the mask check.
                vram.pixels[index] = pixel | sh.mask_bit | (texel & 0x8000);
            }
        }
        lanes[3].advance(divisor);
        lanes[4].advance(divisor);
        if GOURAUD {
            lanes[0].advance(divisor);
            lanes[1].advance(divisor);
            lanes[2].advance(divisor);
        }
    }
}

// ----- Rectangles -----

/// Draw a `width` x `height` rectangle with its top-left corner at `(x, y)`
/// (before the drawing offset). Textured rectangles start at `(u, v)` and may
/// be mirrored.
#[allow(clippy::too_many_arguments)]
pub fn draw_rectangle(
    vram: &mut Vram,
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
    let (x, y) = (x + settings.offset_x, y + settings.offset_y);
    let clip = &settings.area;
    let color = [color[0] as i32, color[1] as i32, color[2] as i32];
    let sampler = Sampler::new(&settings.window, &attrs.texture);

    // The columns that survive the clip, found once for every row.
    let first = (clip.left - x).max(0);
    let last = (clip.right - x).min(width - 1);
    if first > last {
        return;
    }

    // Opaque flat rectangles (clears, backgrounds) are a plain row fill.
    let solid = !attrs.textured
        && !attrs.semi_transparent
        && !attrs.gouraud
        && !settings.check_mask
        && clip.left >= 0
        && clip.right < VRAM_WIDTH as i32;
    let solid_pixel = || {
        ((color[0] >> 3) as u16)
            | (((color[1] >> 3) as u16) << 5)
            | (((color[2] >> 3) as u16) << 10)
            | if settings.set_mask { 0x8000 } else { 0 }
    };

    for dy in 0..height {
        let py = y + dy;
        if py < clip.top || py > clip.bottom {
            continue;
        }
        let yu = py as usize & (VRAM_HEIGHT - 1);
        if solid {
            let start = yu * VRAM_WIDTH + ((x + first) as usize & (VRAM_WIDTH - 1));
            let count = (last - first + 1) as usize;
            // The clip rectangle is inside VRAM, so the row cannot wrap.
            vram.pixels[start..start + count].fill(solid_pixel());
            continue;
        }
        let tv = (if flip_y { v as i32 - dy } else { v as i32 + dy }) as u32 & 0xFF;
        for dx in first..=last {
            let xu = (x + dx) as usize & (VRAM_WIDTH - 1);
            let tu = (if flip_x { u as i32 - dx } else { u as i32 + dx }) as u32 & 0xFF;
            shade_pixel(vram, settings, attrs, &sampler, xu, yu, color, tu, tv);
        }
    }
}

// ----- Lines -----

/// Draw a one-pixel-wide line, interpolating the colour when Gouraud shaded.
pub fn draw_line(vram: &mut Vram, settings: &DrawSettings, attrs: &Attributes, a: Vertex, b: Vertex) {
    if too_big(&a, &b) {
        return;
    }
    let (ax, ay) = (a.x + settings.offset_x, a.y + settings.offset_y);
    let (bx, by) = (b.x + settings.offset_x, b.y + settings.offset_y);
    let steps = (bx - ax).abs().max((by - ay).abs());
    let sampler = Sampler::new(&settings.window, &attrs.texture);

    for i in 0..=steps {
        let (x, y, color) = if steps == 0 {
            (ax, ay, a.color)
        } else {
            let t = i as i64;
            let n = steps as i64;
            let mix = |p: i32, q: i32| (p as i64 + (q as i64 - p as i64) * t / n) as i32;
            let color = if attrs.gouraud {
                [
                    mix(a.color[0] as i32, b.color[0] as i32) as u8,
                    mix(a.color[1] as i32, b.color[1] as i32) as u8,
                    mix(a.color[2] as i32, b.color[2] as i32) as u8,
                ]
            } else {
                a.color
            };
            (mix(ax, bx), mix(ay, by), color)
        };
        let color = [color[0] as i32, color[1] as i32, color[2] as i32];
        put_pixel(vram, settings, attrs, &sampler, x, y, color, (0, 0));
    }
}

// ----- VRAM transfers -----

/// The area GP0(02h) really fills for the given parameters: `(x, y, width,
/// height)`, with X and the width rounded to 16-pixel units.
pub fn fill_area(x: u32, y: u32, width: u32, height: u32) -> (usize, usize, usize, usize) {
    (
        (x & 0x3F0) as usize,
        (y & 0x1FF) as usize,
        (((width & 0x3FF) + 0xF) & !0xF) as usize,
        (height & 0x1FF) as usize,
    )
}

/// GP0(02h): fill a rectangle with a solid colour. Ignores the mask bit,
/// the drawing area and the drawing offset; X and width work in 16-pixel
/// units.
pub fn fill_rectangle(vram: &mut Vram, x: u32, y: u32, width: u32, height: u32, color: [u8; 3]) {
    let (x, y, width, height) = fill_area(x, y, width, height);
    let pixel = rgb888_to_15(color);
    for dy in 0..height {
        let row = ((y + dy) % VRAM_HEIGHT) * VRAM_WIDTH;
        let end = x + width;
        if end <= VRAM_WIDTH {
            vram.pixels[row + x..row + end].fill(pixel);
        } else {
            // The row wraps around to its start.
            vram.pixels[row + x..row + VRAM_WIDTH].fill(pixel);
            vram.pixels[row..row + end - VRAM_WIDTH].fill(pixel);
        }
    }
}

/// Sizes in VRAM transfers: a zero size means the maximum.
pub fn transfer_width(raw: u32) -> usize {
    (((raw.wrapping_sub(1)) & 0x3FF) + 1) as usize
}

pub fn transfer_height(raw: u32) -> usize {
    (((raw.wrapping_sub(1)) & 0x1FF) + 1) as usize
}

/// GP0(80h): copy a rectangle inside VRAM, honouring the mask bit settings.
pub fn copy_rectangle(
    vram: &mut Vram,
    settings: &DrawSettings,
    src: (u32, u32),
    dst: (u32, u32),
    size: (u32, u32),
) {
    let (width, height) = (transfer_width(size.0), transfer_height(size.1));
    let (sx, sy) = (src.0 as usize, src.1 as usize);
    let (dx0, dy0) = (dst.0 as usize, dst.1 as usize);

    // Whole rows can be moved at once when nothing wraps, the destination
    // does not overlap the source (which would change what later pixels
    // read) and no pixel needs the mask check.
    let wraps = sx + width > VRAM_WIDTH
        || dx0 + width > VRAM_WIDTH
        || sy + height > VRAM_HEIGHT
        || dy0 + height > VRAM_HEIGHT;
    let disjoint = sx + width <= dx0 || dx0 + width <= sx || sy + height <= dy0 || dy0 + height <= sy;
    if !wraps && disjoint && !settings.check_mask {
        for dy in 0..height {
            let from = (sy + dy) * VRAM_WIDTH + sx;
            let to = (dy0 + dy) * VRAM_WIDTH + dx0;
            vram.pixels.copy_within(from..from + width, to);
            if settings.set_mask {
                for pixel in &mut vram.pixels[to..to + width] {
                    *pixel |= 0x8000;
                }
            }
        }
        return;
    }

    for dy in 0..height {
        for dx in 0..width {
            let pixel = vram.get(sx + dx, sy + dy);
            let (x, y) = (dx0 + dx, dy0 + dy);
            if settings.check_mask && vram.get(x, y) & 0x8000 != 0 {
                continue;
            }
            vram.set(x, y, pixel | if settings.set_mask { 0x8000 } else { 0 });
        }
    }
}

/// Write one pixel of a CPU-to-VRAM upload, honouring the mask bit settings.
pub fn upload_pixel(vram: &mut Vram, settings: &DrawSettings, x: usize, y: usize, pixel: u16) {
    if settings.check_mask && vram.get(x, y) & 0x8000 != 0 {
        return;
    }
    vram.set(x, y, pixel | if settings.set_mask { 0x8000 } else { 0 });
}

/// 24-bit colour to the 15-bit VRAM format.
pub fn rgb888_to_15(color: [u8; 3]) -> u16 {
    ((color[0] >> 3) as u16) | (((color[1] >> 3) as u16) << 5) | (((color[2] >> 3) as u16) << 10)
}

#[cfg(test)]
mod equivalence;
#[cfg(test)]
mod reference;
#[cfg(test)]
mod tests;
