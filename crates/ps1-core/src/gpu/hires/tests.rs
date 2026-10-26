//! The scaled renderer must agree with the console's rasterizer where they
//! overlap: at 1x with every option off it draws the same pixels, and at a
//! higher scale the pixels that coincide with the console's sample points are
//! the same too. Random primitives over random VRAM, as in `sw/equivalence.rs`.

use super::*;
use crate::gpu::sw::{self, DrawArea, Texture, TextureDepth, TextureWindow, Vertex};

/// xorshift64*: a small deterministic generator, so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn range(&mut self, low: i32, high: i32) -> i32 {
        low + self.below((high - low + 1) as u64) as i32
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// VRAM with some zero (transparent) texels and some with the mask bit set.
fn random_vram(rng: &mut Rng) -> Vram {
    let mut vram = Vram::new();
    for pixel in vram.pixels.iter_mut() {
        *pixel = if rng.chance(15) { 0 } else { rng.next() as u16 };
    }
    vram
}

/// Drawing state that draws into the top half of VRAM only, so textures (which
/// live in the bottom half) are never overwritten while they are read: the
/// scaled renderer reads them before the console's one has drawn.
fn random_settings(rng: &mut Rng, dither: bool) -> DrawSettings {
    let left = rng.range(0, 700);
    let top = rng.range(0, 200);
    DrawSettings {
        area: DrawArea {
            left,
            top,
            right: (left + rng.range(0, 300)).min(1023),
            bottom: (top + rng.range(0, 55)).min(255),
        },
        offset_x: rng.range(-200, 200),
        offset_y: rng.range(-100, 100),
        window: TextureWindow {
            mask_x: if rng.chance(30) { rng.below(32) as u32 } else { 0 },
            mask_y: if rng.chance(30) { rng.below(32) as u32 } else { 0 },
            offset_x: rng.below(32) as u32,
            offset_y: rng.below(32) as u32,
        },
        dither: dither && rng.chance(50),
        set_mask: rng.chance(25),
        check_mask: rng.chance(25),
    }
}

fn random_attributes(rng: &mut Rng) -> Attributes {
    Attributes {
        textured: rng.chance(50),
        raw_texture: rng.chance(30),
        semi_transparent: rng.chance(40),
        semi_mode: rng.below(4) as u8,
        gouraud: rng.chance(50),
        texture: Texture {
            page_x: rng.below(16) as u32 * 64,
            page_y: 256,
            depth: match rng.below(3) {
                0 => TextureDepth::Bits4,
                1 => TextureDepth::Bits8,
                _ => TextureDepth::Bits15,
            },
            clut_x: rng.below(64) as u32 * 16,
            clut_y: 256 + rng.below(256) as u32,
        },
    }
}

fn random_vertex(rng: &mut Rng, around: (i32, i32), spread: i32) -> Vertex {
    Vertex {
        x: around.0 + rng.range(-spread, spread),
        y: around.1 + rng.range(-spread, spread),
        color: [rng.next() as u8, rng.next() as u8, rng.next() as u8],
        u: rng.next() as u8,
        v: rng.next() as u8,
    }
}

fn scaled_vertex(v: &Vertex) -> HVertex {
    HVertex { x: v.x as f32, y: v.y as f32, w: 0.0, color: v.color, u: v.u, v: v.v, ix: v.x, iy: v.y }
}

fn settings_for(scale: u32) -> RenderSettings {
    RenderSettings { scale, ..RenderSettings::default() }
}

fn assert_matches_console(hires: &HiRes, vram: &Vram, what: &str, case: usize) {
    for (i, pixel) in vram.pixels.iter().enumerate() {
        assert_eq!(
            hires.pixels[i],
            expand15(*pixel),
            "{what} #{case}: pixel ({}, {}) differs from the console's {:#06X}",
            i % VRAM_WIDTH,
            i / VRAM_WIDTH,
            pixel
        );
    }
}

/// Every scaled pixel that falls on a sample point of the console equals the
/// console's pixel (the same picture drawn at 1x).
fn assert_refines(fine: &HiRes, coarse: &HiRes, what: &str, case: usize) {
    let s = fine.scale;
    for y in 0..VRAM_HEIGHT {
        for x in 0..VRAM_WIDTH {
            assert_eq!(
                fine.pixels[y * s * fine.stride + x * s],
                coarse.pixels[y * coarse.stride + x],
                "{what} #{case}: pixel ({x}, {y})"
            );
        }
    }
}

#[test]
fn at_1x_triangles_match_the_console() {
    let mut rng = Rng(0x0DDB_1A5E_5BAD_5EED);
    let mut vram = random_vram(&mut rng);
    let mut hires = HiRes::from_vram(&vram, &settings_for(1));
    for case in 0..300 {
        let settings = random_settings(&mut rng, true);
        let attrs = random_attributes(&mut rng);
        let spread = match rng.below(10) {
            0 => 700,
            1..=2 => 150,
            _ => 40,
        };
        let around = (rng.range(0, 700), rng.range(0, 250));
        let vertices = [
            random_vertex(&mut rng, around, spread),
            random_vertex(&mut rng, around, spread),
            random_vertex(&mut rng, around, spread),
        ];
        hires.draw_triangle(&vram, &settings, &attrs, vertices.map(|v| scaled_vertex(&v)), false);
        sw::draw_triangle(&mut vram, &settings, &attrs, vertices);
        assert_matches_console(&hires, &vram, "triangle", case);
    }
}

#[test]
fn at_1x_rectangles_lines_fills_and_copies_match_the_console() {
    let mut rng = Rng(0x1357_9BDF_0246_8ACE);
    let mut vram = random_vram(&mut rng);
    let mut hires = HiRes::from_vram(&vram, &settings_for(1));
    for case in 0..200 {
        let settings = random_settings(&mut rng, true);
        let attrs = random_attributes(&mut rng);
        match rng.below(4) {
            0 => {
                let (x, y) = (rng.range(0, 800), rng.range(0, 240));
                let (width, height) = (rng.range(1, 90), rng.range(1, 40));
                let color = [rng.next() as u8, rng.next() as u8, rng.next() as u8];
                let (u, v) = (rng.next() as u8, rng.next() as u8);
                let (flip_x, flip_y) = (rng.chance(30), rng.chance(30));
                hires.draw_rectangle(&vram, &settings, &attrs, x, y, width, height, color, u, v, flip_x, flip_y);
                sw::draw_rectangle(&mut vram, &settings, &attrs, x, y, width, height, color, u, v, flip_x, flip_y);
            }
            1 => {
                let attrs = Attributes { textured: false, ..attrs };
                let a = random_vertex(&mut rng, (400, 120), 200);
                let b = random_vertex(&mut rng, (400, 120), 200);
                hires.draw_line(&settings, &attrs, &scaled_vertex(&a), &scaled_vertex(&b));
                sw::draw_line(&mut vram, &settings, &attrs, a, b);
            }
            2 => {
                let (x, y) = (rng.below(1024) as u32, rng.below(512) as u32);
                let (w, h) = (rng.below(600) as u32, rng.below(300) as u32);
                let color = [rng.next() as u8, rng.next() as u8, rng.next() as u8];
                sw::fill_rectangle(&mut vram, x, y, w, h, color);
                let (fx, fy, fw, fh) = sw::fill_area(x, y, w, h);
                hires.fill_rectangle(fx, fy, fw, fh, color);
            }
            _ => {
                // Copies between areas that do not overlap, so reading order is not an issue.
                let src = (rng.below(400) as u32, rng.below(200) as u32);
                let dst = (500 + rng.below(400) as u32, 250 + rng.below(200) as u32);
                let size = (rng.range(1, 100) as u32, rng.range(1, 50) as u32);
                sw::copy_rectangle(&mut vram, &settings, src, dst, size);
                hires.copy_rectangle(&settings, src, dst, size);
            }
        }
        assert_matches_console(&hires, &vram, "primitive", case);
    }
}

#[test]
fn uploads_through_the_mask_rules_match_the_console() {
    let mut rng = Rng(0xABCD_EF01_2345_6789);
    let mut vram = random_vram(&mut rng);
    let mut hires = HiRes::from_vram(&vram, &settings_for(1));
    for case in 0..20 {
        let settings = random_settings(&mut rng, false);
        for _ in 0..500 {
            let (x, y) = (rng.below(1024) as usize, rng.below(512) as usize);
            let pixel = rng.next() as u16;
            // What `Gpu` does: decide with the console's pixel before it changes.
            if !(settings.check_mask && vram.get(x, y) & 0x8000 != 0) {
                hires.upload_pixel(x, y, pixel | if settings.set_mask { 0x8000 } else { 0 });
            }
            sw::upload_pixel(&mut vram, &settings, x, y, pixel);
        }
        assert_matches_console(&hires, &vram, "upload", case);
    }
}

#[test]
fn a_higher_scale_draws_the_same_picture_on_the_console_sample_points() {
    let mut rng = Rng(0x0F0F_1234_ABCD_9876);
    let vram = random_vram(&mut rng);
    for scale in [2u32, 3] {
        let mut coarse = HiRes::from_vram(&vram, &settings_for(1));
        let mut fine = HiRes::from_vram(&vram, &settings_for(scale));
        for case in 0..60 {
            // Dithering depends on the pixel position, which is finer at a higher scale.
            let settings = random_settings(&mut rng, false);
            let attrs = random_attributes(&mut rng);
            match rng.below(3) {
                0 => {
                    let around = (rng.range(0, 700), rng.range(0, 250));
                    let spread = if rng.chance(20) { 150 } else { 40 };
                    let vertices = [
                        random_vertex(&mut rng, around, spread),
                        random_vertex(&mut rng, around, spread),
                        random_vertex(&mut rng, around, spread),
                    ];
                    for hires in [&mut coarse, &mut fine] {
                        hires.draw_triangle(&vram, &settings, &attrs, vertices.map(|v| scaled_vertex(&v)), false);
                    }
                }
                1 => {
                    let (x, y) = (rng.range(0, 800), rng.range(0, 240));
                    let (width, height) = (rng.range(1, 60), rng.range(1, 30));
                    let color = [rng.next() as u8, rng.next() as u8, rng.next() as u8];
                    let (u, v) = (rng.next() as u8, rng.next() as u8);
                    for hires in [&mut coarse, &mut fine] {
                        hires.draw_rectangle(&vram, &settings, &attrs, x, y, width, height, color, u, v, false, false);
                    }
                }
                _ => {
                    let attrs = Attributes { textured: false, ..attrs };
                    let a = random_vertex(&mut rng, (400, 120), 100);
                    let b = random_vertex(&mut rng, (400, 120), 100);
                    for hires in [&mut coarse, &mut fine] {
                        hires.draw_line(&settings, &attrs, &scaled_vertex(&a), &scaled_vertex(&b));
                    }
                }
            }
            assert_refines(&fine, &coarse, &format!("{scale}x"), case);
        }
    }
}

fn open_area() -> DrawSettings {
    DrawSettings { area: DrawArea { left: 0, top: 0, right: 1023, bottom: 511 }, ..DrawSettings::default() }
}

#[test]
fn sprites_and_fills_cover_whole_blocks() {
    let vram = Vram::new();
    let mut hires = HiRes::from_vram(&vram, &settings_for(3));
    hires.draw_rectangle(&vram, &open_area(), &Attributes::default(), 10, 20, 4, 2, [255, 0, 0], 0, 0, false, false);
    let red = expand15(0x001F);
    let at = |hires: &HiRes, x: usize, y: usize| hires.pixels[y * hires.stride + x];
    for y in 60..66 {
        for x in 30..42 {
            assert_eq!(at(&hires, x, y), red, "inside ({x}, {y})");
        }
    }
    assert_eq!(at(&hires, 29, 60), 0);
    assert_eq!(at(&hires, 42, 60), 0);
    assert_eq!(at(&hires, 30, 59), 0);
    assert_eq!(at(&hires, 30, 66), 0);
}

#[test]
fn copies_move_the_high_resolution_detail() {
    let vram = Vram::new();
    let mut hires = HiRes::from_vram(&vram, &settings_for(2));
    // A scaled pixel with no console equivalent (as a fractional vertex edge leaves):
    // the odd column 21 of row 10 is console pixel (10, 5), second half.
    hires.pixels[10 * hires.stride + 21] = expand15(0x7FFF);
    hires.copy_rectangle(&DrawSettings::default(), (10, 5), (100, 100), (4, 4));
    // Console pixel (100, 100) starts at scaled (200, 200): the same offset inside the block.
    assert_eq!(hires.pixels[200 * hires.stride + 201], expand15(0x7FFF));
    assert_eq!(hires.pixels[10 * hires.stride + 21], expand15(0x7FFF), "the source is untouched");
}

#[test]
fn the_display_shows_the_scaled_picture() {
    let mut vram = Vram::new();
    vram.pixels[3 * VRAM_WIDTH + 5] = 0x001F; // a red pixel at (5, 3)
    let hires = HiRes::from_vram(&vram, &settings_for(2));
    let mut out = Vec::new();
    // A window of the VRAM starting at (4, 2), 8x4 pixels.
    hires.render_display(&mut out, (4, 2), 8, 4);
    assert_eq!(out.len(), 16 * 8);
    // The red pixel is at (1, 1) in the window: scaled pixels (2..4, 2..4).
    for y in 0..8 {
        for x in 0..16 {
            let red = (2..4).contains(&x) && (2..4).contains(&y);
            assert_eq!(out[y * 16 + x], if red { 0xFF00_00FF } else { 0xFF00_0000 }, "({x}, {y})");
        }
    }
}

#[test]
fn the_display_wraps_around_the_edges_of_vram() {
    let mut vram = Vram::new();
    vram.pixels[VRAM_WIDTH - 1] = 0x7C00; // blue, last pixel of row 0
    vram.pixels[0] = 0x001F; // red, first pixel of row 0
    let hires = HiRes::from_vram(&vram, &settings_for(1));
    let mut out = Vec::new();
    hires.render_display(&mut out, (VRAM_WIDTH - 1, 0), 3, 1);
    assert_eq!(out, vec![0xFFFF_0000, 0xFF00_00FF, 0xFF00_0000]);
}

// ----- Colour -----

fn flat_triangle(color: [u8; 3]) -> [HVertex; 3] {
    let v = |x: f32, y: f32| HVertex { x, y, w: 0.0, color, u: 0, v: 0, ix: x as i32, iy: y as i32 };
    [v(0.0, 0.0), v(40.0, 0.0), v(0.0, 40.0)]
}

#[test]
fn true_color_keeps_all_eight_bits() {
    let vram = Vram::new();
    let colored = |true_color: bool| {
        let settings = RenderSettings { true_color, ..settings_for(1) };
        let mut hires = HiRes::from_vram(&vram, &settings);
        hires.draw_triangle(&vram, &open_area(), &Attributes::default(), flat_triangle([100, 101, 7]), false);
        hires.pixels[5 * hires.stride + 5]
    };
    // 15-bit: 100 >> 3 = 12, expanded to 99; 101 -> 99 too; 7 -> 0.
    assert_eq!(colored(false), 99 | 99 << 8);
    assert_eq!(colored(true), 100 | 101 << 8 | 7 << 16);
}

#[test]
fn true_color_blends_with_eight_bits() {
    let mut vram = Vram::new();
    vram.pixels.fill(0x001F); // red backdrop
    let settings = RenderSettings { true_color: true, ..settings_for(1) };
    let mut hires = HiRes::from_vram(&vram, &settings);
    let attrs = Attributes { semi_transparent: true, semi_mode: 0, ..Attributes::default() };
    hires.draw_triangle(&vram, &open_area(), &attrs, flat_triangle([0, 0, 200]), false);
    // (255 + 0) / 2 red, (0 + 200) / 2 blue.
    assert_eq!(hires.pixels[5 * hires.stride + 5], 127 | 100 << 16);
}

// ----- Texture filtering -----

/// A 15-bit texture page at (0, 256) whose texels are `texel(u, v)`.
fn texture_vram(texel: impl Fn(usize, usize) -> u16) -> Vram {
    let mut vram = Vram::new();
    for v in 0..256 {
        for u in 0..256 {
            vram.pixels[(256 + v) * VRAM_WIDTH + u] = texel(u, v);
        }
    }
    vram
}

fn textured() -> Attributes {
    Attributes {
        textured: true,
        raw_texture: true,
        texture: Texture { page_x: 0, page_y: 256, depth: TextureDepth::Bits15, clut_x: 0, clut_y: 0 },
        ..Attributes::default()
    }
}

/// A square of `size` pixels at the origin sampling `u0..=u1` from left to right.
fn sampled_quad(u0: u8, u1: u8, v: u8, size: f32) -> [[HVertex; 3]; 2] {
    let vertex = |x: f32, y: f32, u: u8| HVertex { x, y, w: 0.0, color: [128; 3], u, v, ix: x as i32, iy: y as i32 };
    [
        [vertex(0.0, 0.0, u0), vertex(size, 0.0, u1), vertex(0.0, size, u0)],
        [vertex(size, 0.0, u1), vertex(0.0, size, u0), vertex(size, size, u1)],
    ]
}

fn draw_quad(hires: &mut HiRes, vram: &Vram, attrs: &Attributes, quad: [[HVertex; 3]; 2]) {
    for triangle in quad {
        hires.draw_triangle(vram, &open_area(), attrs, triangle, false);
    }
}

#[test]
fn nearest_sampling_repeats_each_texel() {
    // Texels alternate black (non-transparent) and white along u.
    let vram = texture_vram(|u, _| if u % 2 == 0 { 0x0001 } else { 0x7FFF });
    let mut hires = HiRes::from_vram(&vram, &settings_for(2));
    draw_quad(&mut hires, &vram, &textured(), sampled_quad(0, 16, 0, 16.0));
    // Console pixel 1 shows texel 1 (white) on both of its scaled pixels.
    assert_eq!(hires.pixels[3 * hires.stride + 2] & COLOR, 0xFFFFFF);
    assert_eq!(hires.pixels[3 * hires.stride + 3] & COLOR, 0xFFFFFF);
    assert_ne!(hires.pixels[3 * hires.stride + 4] & COLOR, 0xFFFFFF);
}

#[test]
fn bilinear_sampling_blends_neighbouring_texels() {
    // Left half dark red, right half bright red, one texel boundary at u = 4.
    let vram = texture_vram(|u, _| if u < 4 { 0x0004 } else { 0x001C });
    let settings = RenderSettings { texture_filter: TextureFilter::Bilinear, ..settings_for(4) };
    let mut hires = HiRes::from_vram(&vram, &settings);
    draw_quad(&mut hires, &vram, &textured(), sampled_quad(0, 8, 0, 8.0));
    let red = |x: usize| (hires.pixels[8 * hires.stride + x] & 0xFF) as i32;
    // Far from the boundary the texels are as they are; around it the colours blend.
    assert_eq!(red(4), (expand15(0x0004) & 0xFF) as i32);
    assert_eq!(red(28), (expand15(0x001C) & 0xFF) as i32);
    let (left, middle, right) = (red(12), red(16), red(20));
    assert!(left < middle && middle < right, "a ramp across the boundary: {left} {middle} {right}");
}

#[test]
fn bilinear_filtering_does_not_bleed_in_texels_outside_the_polygon_area() {
    // Texels 0..8 are red; everything beyond is white. A quad that samples only u in 0..=7
    // must never show white, even at its edge.
    let vram = texture_vram(|u, _| if u < 8 { 0x001F } else { 0x7FFF });
    let settings = RenderSettings { texture_filter: TextureFilter::Bilinear, ..settings_for(2) };
    let mut hires = HiRes::from_vram(&vram, &settings);
    draw_quad(&mut hires, &vram, &textured(), sampled_quad(0, 7, 0, 16.0));
    let mut drawn = 0;
    for x in 0..30 {
        let pixel = hires.pixels[6 * hires.stride + x] & COLOR;
        if pixel != 0 {
            drawn += 1;
            assert_eq!(pixel & 0xFFFF00, 0, "pixel {x} is not pure red: {pixel:#X}");
        }
    }
    assert!(drawn > 20, "the quad was drawn");
}

#[test]
fn bilinear_filtering_keeps_transparent_texels_transparent() {
    // Opaque red on the left, transparent (0) on the right.
    let vram = texture_vram(|u, _| if u < 4 { 0x001F } else { 0 });
    let settings = RenderSettings { texture_filter: TextureFilter::Bilinear, ..settings_for(4) };
    let mut hires = HiRes::from_vram(&vram, &settings);
    draw_quad(&mut hires, &vram, &textured(), sampled_quad(0, 7, 0, 8.0));
    let pixel = |x: usize| hires.pixels[8 * hires.stride + x] & COLOR;
    assert_eq!(pixel(2), expand15(0x001F), "opaque side");
    assert_eq!(pixel(30), 0, "transparent side stays untouched, not a dark fringe");
    // No pixel is a darkened red: transparent texels are left out of the average.
    for x in 0..32 {
        assert!(pixel(x) == 0 || pixel(x) == expand15(0x001F), "pixel {x}: {:#X}", pixel(x));
    }
}

// ----- PGXP -----

#[test]
fn precise_vertices_move_the_edges_by_a_fraction_of_a_pixel() {
    let vram = Vram::new();
    // The right-angle triangle with its second vertex moved right by `dx` console pixels.
    let triangle = |dx: f32| {
        let v = |x: f32, y: f32, w: f32| HVertex { x, y, w, color: [255, 0, 0], u: 0, v: 0, ix: x as i32, iy: y as i32 };
        [v(0.0, 0.0, 1.0), v(8.0 + dx, 0.0, 1.0), v(0.0, 8.0, 1.0)]
    };
    let covered = |dx: f32| {
        let mut hires = HiRes::from_vram(&vram, &settings_for(4));
        hires.draw_triangle(&vram, &open_area(), &Attributes::default(), triangle(dx), false);
        // The row one console pixel down, where the long edge crosses.
        (0..hires.stride).filter(|x| hires.pixels[4 * hires.stride + x] != 0).count()
    };
    let (plain, shifted) = (covered(0.0), covered(0.5));
    // The edge moves by 0.4375 console pixels there: more than one pixel of the 4x buffer.
    assert!(shifted > plain, "{shifted} <= {plain}");
    assert!(shifted <= plain + 3, "{shifted} is too many");
}

#[test]
fn perspective_correct_textures_follow_the_depth() {
    // The colour of a texel is its u coordinate, so the output tells which u was sampled.
    let vram = texture_vram(|u, _| (u as u16 & 31) | 0x0400);
    let attrs = textured();
    let quad = |w: [f32; 4]| {
        let vertex = |x: f32, y: f32, u: u8, w: f32| HVertex { x, y, w, color: [128; 3], u, v: 0, ix: x as i32, iy: y as i32 };
        [
            [vertex(0.0, 0.0, 0, w[0]), vertex(32.0, 0.0, 31, w[1]), vertex(0.0, 32.0, 0, w[2])],
            [vertex(32.0, 0.0, 31, w[1]), vertex(0.0, 32.0, 0, w[2]), vertex(32.0, 32.0, 31, w[3])],
        ]
    };
    let settings = RenderSettings { pgxp: true, pgxp_textures: true, ..settings_for(1) };
    let sample_at_middle = |w: [f32; 4], perspective: bool| {
        let mut hires = HiRes::from_vram(&vram, &settings);
        for triangle in quad(w) {
            hires.draw_triangle(&vram, &open_area(), &attrs, triangle, perspective);
        }
        // The red channel of the 5-bit texel is u (0..31).
        (hires.pixels[8 * hires.stride + 16] & 0xFF) >> 3
    };
    // The right side is four times as far away, so its half of the texture is squeezed:
    // the middle of the quad is much nearer the start of the texture than a linear ramp says.
    let affine = sample_at_middle([1.0, 4.0, 1.0, 4.0], false);
    let corrected = sample_at_middle([1.0, 4.0, 1.0, 4.0], true);
    assert!((affine as i32 - 15).abs() <= 1, "affine mapping is linear in screen space: {affine}");
    assert!(corrected < 10, "perspective mapping lags behind towards the far side: {corrected}");
    // Equal depths make it the same as affine.
    assert_eq!(sample_at_middle([2.0; 4], true), sample_at_middle([2.0; 4], false));
}

#[test]
fn settings_are_cleaned_up_and_say_when_the_scaled_vram_is_needed() {
    let off = RenderSettings::default();
    assert!(!off.needs_hires());
    assert!(RenderSettings { scale: 2, ..off }.needs_hires());
    assert!(RenderSettings { true_color: true, ..off }.needs_hires());
    assert!(RenderSettings { texture_filter: TextureFilter::Bilinear, ..off }.needs_hires());
    assert!(RenderSettings { pgxp: true, ..off }.needs_hires());
    let clean = RenderSettings { scale: 9, pgxp_textures: true, widescreen: true, ..off }.sanitized();
    assert_eq!(clean.scale, MAX_SCALE);
    assert!(!clean.pgxp_textures && !clean.widescreen, "they need PGXP");
    assert_eq!(RenderSettings { scale: 0, ..off }.sanitized().scale, 1);
}

// ----- Polygons far bigger than the screen -----

#[test]
fn a_gouraud_triangle_far_bigger_than_the_screen_is_interpolated_correctly() {
    let vram = Vram::new();
    let settings = RenderSettings { true_color: true, ..settings_for(1) };
    let mut hires = HiRes::from_vram(&vram, &settings);
    // Corners 6000 pixels apart: too big for the exact lanes, drawn with floating point.
    let vertex = |x: f32, y: f32, color: [u8; 3]| HVertex { x, y, w: 0.0, color, u: 0, v: 0, ix: 0, iy: 0 };
    let triangle = [
        vertex(-2000.0, -2000.0, [255, 0, 0]),
        vertex(4000.0, -2000.0, [0, 255, 0]),
        vertex(-2000.0, 4000.0, [0, 0, 255]),
    ];
    let attrs = Attributes { gouraud: true, ..Attributes::default() };
    hires.draw_triangle(&vram, &open_area(), &attrs, triangle, false);
    // At (500, 300) the weights are 0.2, 0.4167 and 0.3833.
    let pixel = hires.pixels[300 * hires.stride + 500];
    let channels = [(pixel & 0xFF) as i32, ((pixel >> 8) & 0xFF) as i32, ((pixel >> 16) & 0xFF) as i32];
    assert!((channels[0] - 51).abs() <= 1 && (channels[1] - 106).abs() <= 1 && (channels[2] - 97).abs() <= 1, "{channels:?}");
    assert_ne!(hires.pixels[500 * hires.stride + 1000], 0, "the far corner of VRAM is inside");
}

#[test]
fn a_textured_triangle_far_bigger_than_the_screen_samples_the_right_texel() {
    // The red channel of a texel is its u coordinate divided by 8.
    let vram = texture_vram(|u, _| ((u as u16) >> 3) & 31 | 0x0400);
    let mut hires = HiRes::from_vram(&vram, &settings_for(1));
    let vertex = |x: f32, y: f32, u: u8| HVertex { x, y, w: 0.0, color: [128; 3], u, v: 0, ix: 0, iy: 0 };
    let triangle = [vertex(-2000.0, -2000.0, 0), vertex(4000.0, -2000.0, 255), vertex(-2000.0, 4000.0, 0)];
    hires.draw_triangle(&vram, &open_area(), &textured(), triangle, false);
    // u = 255 * (500 + 2000) / 6000 = 106.25, texel 13.
    let red = (hires.pixels[300 * hires.stride + 500] & 0xFF) >> 3;
    assert!((red as i32 - 13).abs() <= 1, "{red}");
}

// ----- The flattened loops against the general path -----

/// A rectangle (textured or flat) drawn as a sprite (the general `put` path, or the sprite fast
/// path) and as two triangles with the same texture mapping (the flattened triangle
/// loops) must give the same pixels, in 15-bit and in true colour, at any scale.
#[test]
fn triangles_and_sprites_agree_in_every_colour_mode() {
    let mut rng = Rng(0x7357_0000_C0FF_EE11);
    let vram = random_vram(&mut rng);
    for true_color in [false, true] {
        for scale in [1u32, 2] {
            let settings_for_mode = RenderSettings { scale, true_color, ..RenderSettings::default() };
            let mut sprites = HiRes::from_vram(&vram, &settings_for_mode);
            let mut triangles = HiRes::from_vram(&vram, &settings_for_mode);
            for case in 0..120 {
                let settings = random_settings(&mut rng, true);
                let attrs = Attributes { textured: rng.chance(70), gouraud: false, ..random_attributes(&mut rng) };
                let (width, height) = (rng.range(1, 40), rng.range(1, 30));
                let (x, y) = (rng.range(0, 600), rng.range(0, 200));
                let (u, v) = (rng.range(0, 255 - width), rng.range(0, 255 - height));
                let color = [rng.next() as u8, rng.next() as u8, rng.next() as u8];
                sprites.draw_rectangle(&vram, &settings, &attrs, x, y, width, height, color, u as u8, v as u8, false, false);

                let corner = |dx: i32, dy: i32| HVertex {
                    x: (x + dx * width) as f32,
                    y: (y + dy * height) as f32,
                    w: 0.0,
                    color,
                    u: (u + dx * width) as u8,
                    v: (v + dy * height) as u8,
                    ix: x + dx * width,
                    iy: y + dy * height,
                };
                let (a, b, c, d) = (corner(0, 0), corner(1, 0), corner(0, 1), corner(1, 1));
                triangles.draw_triangle(&vram, &settings, &attrs, [a, b, c], false);
                triangles.draw_triangle(&vram, &settings, &attrs, [b, c, d], false);
                if let Some(i) = (0..sprites.pixels.len()).find(|i| sprites.pixels[*i] != triangles.pixels[*i]) {
                    panic!(
                        "true colour {true_color}, {scale}x, case {case}: pixel ({}, {}) is {:#X} as triangles and {:#X} as a sprite",
                        i % sprites.stride,
                        i / sprites.stride,
                        triangles.pixels[i],
                        sprites.pixels[i]
                    );
                }
            }
        }
    }
}
