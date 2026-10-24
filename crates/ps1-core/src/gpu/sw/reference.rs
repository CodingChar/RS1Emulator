//! The straightforward rasterizer the optimised one in `sw.rs` replaced,
//! kept to check it against: both must produce exactly the same VRAM.

#![allow(clippy::too_many_arguments)]

use super::*;

// ----- Pixel pipeline -----

fn blend_component(back: u16, front: u16, mode: u8) -> u16 {
    match mode {
        0 => (back + front) / 2,
        1 => (back + front).min(31),
        2 => back.saturating_sub(front),
        _ => (back + front / 4).min(31),
    }
}

fn blend(back: u16, front: u16, mode: u8) -> u16 {
    let channel = |shift: u32| {
        blend_component((back >> shift) & 31, (front >> shift) & 31, mode) << shift
    };
    channel(0) | channel(5) | channel(10)
}

/// Fetch one texel (a 15-bit colour, 0 meaning transparent).
fn sample_texture(vram: &Vram, window: &TextureWindow, texture: &Texture, u: u32, v: u32) -> u16 {
    let u = (u & !(window.mask_x * 8)) | ((window.offset_x & window.mask_x) * 8);
    let v = (v & !(window.mask_y * 8)) | ((window.offset_y & window.mask_y) * 8);
    let (px, py) = (texture.page_x as usize, texture.page_y as usize + v as usize);
    let (clut_x, clut_y) = (texture.clut_x as usize, texture.clut_y as usize);

    match texture.depth {
        TextureDepth::Bits4 => {
            let word = vram.get(px + (u / 4) as usize, py);
            let index = (word >> ((u % 4) * 4)) & 0xF;
            vram.get(clut_x + index as usize, clut_y)
        }
        TextureDepth::Bits8 => {
            let word = vram.get(px + (u / 2) as usize, py);
            let index = (word >> ((u % 2) * 8)) & 0xFF;
            vram.get(clut_x + index as usize, clut_y)
        }
        TextureDepth::Bits15 => vram.get(px + u as usize, py),
    }
}

/// Shade and write one pixel. `color` is the interpolated vertex colour
/// (0..=255); `uv` is the texel coordinate for textured primitives.
fn put_pixel(
    vram: &mut Vram,
    settings: &DrawSettings,
    attrs: &Attributes,
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
    let back = vram.get(xu, yu);
    if settings.check_mask && back & 0x8000 != 0 {
        return;
    }

    let mut rgb = color;
    let mut semi = attrs.semi_transparent;
    let mut modulated = false;
    if attrs.textured {
        let texel = sample_texture(vram, &settings.window, &attrs.texture, uv.0, uv.1);
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
        let offset = DITHER[yu & 3][xu & 3];
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
    vram.set(xu, yu, pixel);
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

    let top_left = [is_top_left(&v1, &v2), is_top_left(&v2, &v0), is_top_left(&v0, &v1)];

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let w = [edge(&v1, &v2, x, y), edge(&v2, &v0, x, y), edge(&v0, &v1, x, y)];
            let inside = (0..3).all(|i| w[i] > 0 || (w[i] == 0 && top_left[i]));
            if !inside {
                continue;
            }
            let lerp = |a: i32, b: i32, c: i32| -> i32 {
                ((w[0] * a as i64 + w[1] * b as i64 + w[2] * c as i64) / area) as i32
            };
            let color = if attrs.gouraud {
                [
                    lerp(v0.color[0] as i32, v1.color[0] as i32, v2.color[0] as i32),
                    lerp(v0.color[1] as i32, v1.color[1] as i32, v2.color[1] as i32),
                    lerp(v0.color[2] as i32, v1.color[2] as i32, v2.color[2] as i32),
                ]
            } else {
                [v0.color[0] as i32, v0.color[1] as i32, v0.color[2] as i32]
            };
            let uv = if attrs.textured {
                (
                    lerp(v0.u as i32, v1.u as i32, v2.u as i32).clamp(0, 255) as u32,
                    lerp(v0.v as i32, v1.v as i32, v2.v as i32).clamp(0, 255) as u32,
                )
            } else {
                (0, 0)
            };
            put_pixel(vram, settings, attrs, x, y, color, uv);
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

    for dy in 0..height {
        let py = y + dy;
        if py < clip.top || py > clip.bottom {
            continue;
        }
        let tv = (if flip_y { v as i32 - dy } else { v as i32 + dy }) as u32 & 0xFF;
        for dx in 0..width {
            let px = x + dx;
            if px < clip.left || px > clip.right {
                continue;
            }
            let tu = (if flip_x { u as i32 - dx } else { u as i32 + dx }) as u32 & 0xFF;
            put_pixel(vram, settings, attrs, px, py, color, (tu, tv));
        }
    }
}


/// GP0(02h): fill a rectangle with a solid colour. Ignores the mask bit,
/// the drawing area and the drawing offset; X and width work in 16-pixel
/// units.
pub fn fill_rectangle(vram: &mut Vram, x: u32, y: u32, width: u32, height: u32, color: [u8; 3]) {
    let x = (x & 0x3F0) as usize;
    let y = (y & 0x1FF) as usize;
    let width = (((width & 0x3FF) + 0xF) & !0xF) as usize;
    let height = (height & 0x1FF) as usize;
    let pixel = rgb888_to_15(color);
    for dy in 0..height {
        for dx in 0..width {
            vram.set(x + dx, y + dy, pixel);
        }
    }
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
    for dy in 0..height {
        for dx in 0..width {
            let pixel = vram.get(src.0 as usize + dx, src.1 as usize + dy);
            let (x, y) = (dst.0 as usize + dx, dst.1 as usize + dy);
            if settings.check_mask && vram.get(x, y) & 0x8000 != 0 {
                continue;
            }
            vram.set(x, y, pixel | if settings.set_mask { 0x8000 } else { 0 });
        }
    }
}

