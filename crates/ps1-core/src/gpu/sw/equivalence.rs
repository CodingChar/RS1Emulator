//! Differential tests: the optimised rasterizer must draw exactly what the
//! straightforward one in `reference.rs` draws, for random primitives with
//! random drawing state over random VRAM contents.

use super::*;

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

fn random_vram(rng: &mut Rng) -> Vram {
    let mut vram = Vram::new();
    for pixel in vram.pixels.iter_mut() {
        // Some zero (transparent) texels, some with the mask bit set.
        *pixel = if rng.chance(15) { 0 } else { rng.next() as u16 };
    }
    vram
}

fn random_settings(rng: &mut Rng) -> DrawSettings {
    let left = rng.range(0, 700);
    let top = rng.range(0, 300);
    DrawSettings {
        area: DrawArea {
            left,
            top,
            right: (left + rng.range(0, 400)).min(1023),
            bottom: (top + rng.range(0, 250)).min(511),
        },
        offset_x: rng.range(-200, 200),
        offset_y: rng.range(-100, 100),
        window: TextureWindow {
            mask_x: if rng.chance(30) { rng.below(32) as u32 } else { 0 },
            mask_y: if rng.chance(30) { rng.below(32) as u32 } else { 0 },
            offset_x: rng.below(32) as u32,
            offset_y: rng.below(32) as u32,
        },
        dither: rng.chance(50),
        set_mask: rng.chance(25),
        check_mask: rng.chance(25),
    }
}

fn random_attributes(rng: &mut Rng) -> Attributes {
    let textured = rng.chance(50);
    Attributes {
        textured,
        raw_texture: rng.chance(30),
        semi_transparent: rng.chance(40),
        semi_mode: rng.below(4) as u8,
        gouraud: rng.chance(50),
        texture: Texture {
            page_x: rng.below(16) as u32 * 64,
            page_y: rng.below(2) as u32 * 256,
            depth: match rng.below(3) {
                0 => TextureDepth::Bits4,
                1 => TextureDepth::Bits8,
                _ => TextureDepth::Bits15,
            },
            clut_x: rng.below(64) as u32 * 16,
            clut_y: rng.below(512) as u32,
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

fn assert_same(a: &Vram, b: &Vram, what: &str, case: usize) {
    if let Some(i) = (0..a.pixels.len()).find(|i| a.pixels[*i] != b.pixels[*i]) {
        panic!(
            "{what} #{case}: pixel ({}, {}) is {:#06X}, the reference draws {:#06X}",
            i % VRAM_WIDTH,
            i / VRAM_WIDTH,
            a.pixels[i],
            b.pixels[i]
        );
    }
}

#[test]
fn triangles_match_the_reference() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let base = random_vram(&mut rng);
    for case in 0..400 {
        let settings = random_settings(&mut rng);
        let attrs = random_attributes(&mut rng);
        // Mostly small and medium triangles, a few huge or thin ones.
        let spread = match rng.below(10) {
            0 => 700,
            1..=2 => 150,
            _ => 40,
        };
        let centre = (rng.range(0, 600), rng.range(0, 300));
        let vertices = [
            random_vertex(&mut rng, centre, spread),
            random_vertex(&mut rng, centre, spread),
            random_vertex(&mut rng, centre, spread),
        ];
        let mut fast = base.clone();
        let mut slow = base.clone();
        draw_triangle(&mut fast, &settings, &attrs, vertices);
        reference::draw_triangle(&mut slow, &settings, &attrs, vertices);
        assert_same(&fast, &slow, "triangle", case);
    }
}

#[test]
fn thin_and_axis_aligned_triangles_match_the_reference() {
    let mut rng = Rng(0xDEAD_BEEF_0BAD_F00D);
    let base = random_vram(&mut rng);
    for case in 0..300 {
        let settings = random_settings(&mut rng);
        let attrs = random_attributes(&mut rng);
        let centre = (rng.range(50, 500), rng.range(50, 250));
        let mut vertices = [
            random_vertex(&mut rng, centre, 60),
            random_vertex(&mut rng, centre, 60),
            random_vertex(&mut rng, centre, 60),
        ];
        // Share coordinates so edges are horizontal, vertical or very thin.
        match case % 4 {
            0 => vertices[1].y = vertices[0].y,
            1 => vertices[2].x = vertices[0].x,
            2 => {
                vertices[1].x = vertices[0].x + 1;
                vertices[1].y = vertices[0].y + 60;
            }
            _ => vertices[2].y = vertices[1].y,
        }
        let mut fast = base.clone();
        let mut slow = base.clone();
        draw_triangle(&mut fast, &settings, &attrs, vertices);
        reference::draw_triangle(&mut slow, &settings, &attrs, vertices);
        assert_same(&fast, &slow, "thin triangle", case);
    }
}

#[test]
fn rectangles_match_the_reference() {
    let mut rng = Rng(0x0F0F_1234_ABCD_9876);
    let base = random_vram(&mut rng);
    for case in 0..400 {
        let settings = random_settings(&mut rng);
        let mut attrs = random_attributes(&mut rng);
        // Rectangles are never Gouraud shaded, but sometimes flat and opaque
        // (the row-fill fast path).
        attrs.gouraud = false;
        if rng.chance(40) {
            attrs.textured = false;
            attrs.semi_transparent = false;
        }
        let (x, y) = (rng.range(-100, 900), rng.range(-50, 450));
        let (width, height) = (rng.range(0, 300), rng.range(0, 200));
        let color = [rng.next() as u8, rng.next() as u8, rng.next() as u8];
        let (u, v) = (rng.next() as u8, rng.next() as u8);
        let (flip_x, flip_y) = (rng.chance(30), rng.chance(30));
        let mut fast = base.clone();
        let mut slow = base.clone();
        draw_rectangle(&mut fast, &settings, &attrs, x, y, width, height, color, u, v, flip_x, flip_y);
        reference::draw_rectangle(&mut slow, &settings, &attrs, x, y, width, height, color, u, v, flip_x, flip_y);
        assert_same(&fast, &slow, "rectangle", case);
    }
}

#[test]
fn fills_match_the_reference() {
    let mut rng = Rng(0x7777_8888_9999_AAAA);
    let base = random_vram(&mut rng);
    for case in 0..200 {
        let (x, y) = (rng.below(0x800) as u32, rng.below(0x400) as u32);
        let (width, height) = (rng.below(0x800) as u32, rng.below(0x400) as u32);
        let color = [rng.next() as u8, rng.next() as u8, rng.next() as u8];
        let mut fast = base.clone();
        let mut slow = base.clone();
        fill_rectangle(&mut fast, x, y, width, height, color);
        reference::fill_rectangle(&mut slow, x, y, width, height, color);
        assert_same(&fast, &slow, "fill", case);
    }
}

#[test]
fn copies_match_the_reference() {
    let mut rng = Rng(0x1357_9BDF_2468_ACE0);
    let base = random_vram(&mut rng);
    for case in 0..250 {
        let settings = random_settings(&mut rng);
        // Mix of far-apart, overlapping and wrapping rectangles.
        let src = (rng.below(1100) as u32, rng.below(560) as u32);
        let dst = if rng.chance(30) {
            (src.0 + rng.below(40) as u32, src.1 + rng.below(40) as u32)
        } else {
            (rng.below(1100) as u32, rng.below(560) as u32)
        };
        let size = (rng.below(300) as u32, rng.below(200) as u32);
        let mut fast = base.clone();
        let mut slow = base.clone();
        copy_rectangle(&mut fast, &settings, src, dst, size);
        reference::copy_rectangle(&mut slow, &settings, src, dst, size);
        assert_same(&fast, &slow, "copy", case);
    }
}
