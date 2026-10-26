//! Rasterizer micro-benchmark: how long the console's rasterizer and the scaled
//! one (`gpu::hires`) take per pixel for the usual kinds of triangle.
//!
//! ```text
//! cargo run --release -p ps1-core --example raster_bench --target <host triple>
//! ```

use std::time::Instant;

use ps1_core::gpu::hires::{HVertex, HiRes};
use ps1_core::gpu::sw::{self, Attributes, DrawArea, DrawSettings, Texture, TextureDepth, Vertex, Vram};
use ps1_core::gpu::{RenderSettings, TextureFilter};

/// xorshift64*: a small deterministic generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn range(&mut self, low: i32, high: i32) -> i32 {
        low + (self.next() % (high - low + 1) as u64) as i32
    }
}

const SPREAD: i32 = 20;

/// Half the width of the triangles in console pixels (`--size N`) and how many are drawn (`--count N`).
fn triangles(rng: &mut Rng, count: usize, size: i32) -> Vec<[Vertex; 3]> {
    (0..count)
        .map(|_| {
            let (cx, cy) = (rng.range(size, 320 - size).min(size + SPREAD), rng.range(size, 220 - size).min(size + SPREAD));
            let mut vertex = || Vertex {
                x: cx + rng.range(-size, size),
                y: cy + rng.range(-size, size),
                color: [rng.next() as u8, rng.next() as u8, rng.next() as u8],
                u: rng.next() as u8,
                v: rng.next() as u8,
            };
            [vertex(), vertex(), vertex()]
        })
        .collect()
}

fn pixels_drawn(list: &[[Vertex; 3]]) -> u64 {
    list.iter()
        .map(|t| {
            let cross = (t[1].x - t[0].x) as i64 * (t[2].y - t[0].y) as i64
                - (t[2].x - t[0].x) as i64 * (t[1].y - t[0].y) as i64;
            cross.unsigned_abs() / 2
        })
        .sum()
}

fn main() {
    let mut rng = Rng(0x5EED_1234_ABCD_0001);
    let mut vram = Vram::new();
    for pixel in vram.pixels.iter_mut() {
        *pixel = rng.next() as u16 | 1;
    }
    let settings = DrawSettings {
        area: DrawArea { left: 0, top: 0, right: 511, bottom: 255 },
        dither: true,
        ..DrawSettings::default()
    };
    let texture = Texture { page_x: 512, page_y: 256, depth: TextureDepth::Bits15, clut_x: 0, clut_y: 0 };
    let (mut count, mut size) = (3000usize, 40i32);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--count" => count = args.next().and_then(|v| v.parse().ok()).expect("--count N"),
            "--size" => size = args.next().and_then(|v| v.parse().ok()).expect("--size N"),
            other => panic!("unknown argument {other}"),
        }
    }
    let list = triangles(&mut rng, count, size);
    let area = pixels_drawn(&list);
    println!("{count} triangles of about {} pixels, {area} pixels at 1x\n", 2 * size * size / 3);

    let modes: [(&str, Attributes); 5] = [
        ("flat", Attributes::default()),
        ("gouraud", Attributes { gouraud: true, ..Attributes::default() }),
        ("textured", Attributes { textured: true, raw_texture: true, texture, ..Attributes::default() }),
        ("textured+gouraud", Attributes { textured: true, gouraud: true, texture, ..Attributes::default() }),
        (
            "textured+gouraud+semi",
            Attributes { textured: true, gouraud: true, semi_transparent: true, semi_mode: 0, texture, ..Attributes::default() },
        ),
    ];

    println!("{:24} {:>12} {:>12} {:>12} {:>12}", "ns per 1x pixel", "console", "scaled 1x", "scaled 2x", "scaled 3x");
    for (name, attrs) in &modes {
        let mut row = Vec::new();

        let mut copy = vram.clone();
        row.push(best_of(area, || {
            for t in &list {
                sw::draw_triangle(&mut copy, &settings, attrs, *t);
            }
        }));

        for scale in [1u32, 2, 3] {
            let mut hires = HiRes::from_vram(&vram, &RenderSettings { scale, ..RenderSettings::default() });
            row.push(best_of(area, || {
                for t in &list {
                    let v = t.map(|v| HVertex { x: v.x as f32, y: v.y as f32, w: 0.0, color: v.color, u: v.u, v: v.v, ix: v.x, iy: v.y });
                    hires.draw_triangle(&vram, &settings, attrs, v, false);
                }
            }));
        }
        println!("{name:24} {:>12.1} {:>12.1} {:>12.1} {:>12.1}", row[0], row[1], row[2], row[3]);
    }

    println!();
    for (label, filter, perspective) in [("bilinear", TextureFilter::Bilinear, false), ("perspective", TextureFilter::Nearest, true), ("bilinear+perspective", TextureFilter::Bilinear, true)] {
        let attrs = Attributes { textured: true, gouraud: true, texture, ..Attributes::default() };
        let mut hires = HiRes::from_vram(&vram, &RenderSettings { scale: 2, texture_filter: filter, ..RenderSettings::default() });
        let ns = best_of(area, || {
            for t in &list {
                let v = t.map(|v| HVertex { x: v.x as f32, y: v.y as f32, w: 100.0 + (v.x & 63) as f32, color: v.color, u: v.u, v: v.v, ix: v.x, iy: v.y });
                hires.draw_triangle(&vram, &settings, &attrs, v, perspective);
            }
        });
        println!("{label:24} scaled 2x: {ns:.1} ns per 1x pixel");
    }
}

/// The fastest of several runs of `work`, in nanoseconds per pixel of `area`.
fn best_of(area: u64, mut work: impl FnMut()) -> f64 {
    (0..5)
        .map(|_| {
            let start = Instant::now();
            work();
            start.elapsed().as_secs_f64() * 1e9 / area as f64
        })
        .fold(f64::MAX, f64::min)
}
