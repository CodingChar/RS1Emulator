use super::*;

fn settings() -> DrawSettings {
    DrawSettings {
        area: DrawArea { left: 0, top: 0, right: 1023, bottom: 511 },
        ..DrawSettings::default()
    }
}

fn flat() -> Attributes {
    Attributes::default()
}

fn v(x: i32, y: i32, color: [u8; 3]) -> Vertex {
    Vertex { x, y, color, u: 0, v: 0 }
}

fn tv(x: i32, y: i32, u: u8, vv: u8) -> Vertex {
    Vertex { x, y, color: [128, 128, 128], u, v: vv }
}

const RED: [u8; 3] = [255, 0, 0];
const RED_15: u16 = 0x001F;

/// Number of non-zero pixels in `x0..x1` x `y0..y1`.
fn count(vram: &Vram, x0: usize, x1: usize, y0: usize, y1: usize) -> usize {
    (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|(x, y)| vram.get(*x, *y) != 0).count()
}

// ----- VRAM, colours -----

#[test]
fn vram_coordinates_wrap() {
    let mut vram = Vram::new();
    vram.set(1024 + 3, 512 + 2, 0x1234);
    assert_eq!(vram.get(3, 2), 0x1234);
    assert_eq!(vram.pixels.len(), 1024 * 512);
}

#[test]
fn colours_are_truncated_to_15_bits() {
    assert_eq!(rgb888_to_15([255, 255, 255]), 0x7FFF);
    assert_eq!(rgb888_to_15([8, 16, 24]), 1 | (2 << 5) | (3 << 10));
    assert_eq!(rgb888_to_15([7, 7, 7]), 0);
}

// ----- Fill, copy, upload -----

#[test]
fn fill_works_in_16_pixel_columns_and_ignores_the_draw_area() {
    let mut vram = Vram::new();
    fill_rectangle(&mut vram, 21, 4, 17, 2, RED);
    // x rounds down to 16, width rounds up to 32.
    assert_eq!(count(&vram, 0, 1024, 0, 512), 32 * 2);
    assert_eq!(vram.get(16, 4), RED_15);
    assert_eq!(vram.get(47, 5), RED_15);
    assert_eq!(vram.get(48, 5), 0);
    assert_eq!(vram.get(15, 4), 0);
}

#[test]
fn fill_wraps_around_vram() {
    let mut vram = Vram::new();
    fill_rectangle(&mut vram, 1008, 510, 32, 4, RED);
    assert_eq!(vram.get(1008, 510), RED_15);
    assert_eq!(vram.get(1023, 511), RED_15);
    assert_eq!(vram.get(0, 0), RED_15, "wraps horizontally and vertically");
    assert_eq!(vram.get(16, 0), 0, "but only as far as the fill is wide");
    assert_eq!(vram.get(1008, 1), RED_15);
    assert_eq!(vram.get(1008, 2), 0);
}

#[test]
fn fill_clears_the_mask_bit() {
    let mut vram = Vram::new();
    vram.set(16, 0, 0xFFFF);
    fill_rectangle(&mut vram, 16, 0, 16, 1, RED);
    assert_eq!(vram.get(16, 0), RED_15);
}

#[test]
fn transfer_sizes_treat_zero_as_the_maximum() {
    assert_eq!(transfer_width(0), 1024);
    assert_eq!(transfer_width(1), 1);
    assert_eq!(transfer_width(1025), 1);
    assert_eq!(transfer_height(0), 512);
    assert_eq!(transfer_height(512), 512);
}

#[test]
fn copy_moves_pixels_and_wraps() {
    let mut vram = Vram::new();
    for i in 0..4 {
        vram.set(10 + i, 20, 0x100 + i as u16);
    }
    copy_rectangle(&mut vram, &settings(), (10, 20), (1022, 30), (4, 1));
    assert_eq!(vram.get(1022, 30), 0x100);
    assert_eq!(vram.get(1023, 30), 0x101);
    assert_eq!(vram.get(0, 30), 0x102);
    assert_eq!(vram.get(1, 30), 0x103);
}

#[test]
fn copy_and_upload_honour_the_mask_bits() {
    let mut vram = Vram::new();
    vram.set(10, 10, 0x0001);
    vram.set(50, 10, 0x8000 | 0x0002);

    let mut s = settings();
    s.set_mask = true;
    copy_rectangle(&mut vram, &s, (10, 10), (100, 10), (1, 1));
    assert_eq!(vram.get(100, 10), 0x8001, "bit 15 forced on");

    s.set_mask = false;
    s.check_mask = true;
    copy_rectangle(&mut vram, &s, (10, 10), (50, 10), (1, 1));
    assert_eq!(vram.get(50, 10), 0x8002, "protected pixel untouched");

    upload_pixel(&mut vram, &s, 50, 10, 0x1234);
    assert_eq!(vram.get(50, 10), 0x8002);
    upload_pixel(&mut vram, &s, 51, 10, 0x1234);
    assert_eq!(vram.get(51, 10), 0x1234);
}

// ----- Triangles -----

#[test]
fn flat_triangle_covers_its_interior_only() {
    let mut vram = Vram::new();
    draw_triangle(&mut vram, &settings(), &flat(), [v(10, 10, RED), v(30, 10, RED), v(10, 30, RED)]);
    assert_eq!(vram.get(12, 12), RED_15);
    assert_eq!(vram.get(10, 10), RED_15, "the top-left vertex");
    assert_eq!(vram.get(25, 25), 0, "beyond the hypotenuse");
    assert_eq!(vram.get(9, 12), 0);
    assert_eq!(vram.get(12, 9), 0);
    // Pixel (x, y) is inside when x >= 10, y >= 10 and x + y < 40.
    let expected = (10..30).flat_map(|y| (10..30).map(move |x| (x, y))).filter(|(x, y)| x + y < 40).count();
    assert_eq!(count(&vram, 0, 100, 0, 100), expected);
}

#[test]
fn winding_order_does_not_matter() {
    let mut a = Vram::new();
    let mut b = Vram::new();
    draw_triangle(&mut a, &settings(), &flat(), [v(5, 5, RED), v(40, 12, RED), v(14, 33, RED)]);
    draw_triangle(&mut b, &settings(), &flat(), [v(5, 5, RED), v(14, 33, RED), v(40, 12, RED)]);
    assert_eq!(a.pixels, b.pixels);
    assert!(count(&a, 0, 100, 0, 100) > 100);
}

#[test]
fn adjacent_triangles_share_their_edge_without_gaps_or_overlaps() {
    // Additive blending of a constant: overlapping pixels would count twice.
    let attrs = Attributes { semi_transparent: true, semi_mode: 1, ..Attributes::default() };
    let one = [8, 0, 0]; // 5-bit value 1
    let mut vram = Vram::new();
    for (a, b, c) in [((0, 0), (16, 0), (0, 16)), ((16, 0), (16, 16), (0, 16))] {
        draw_triangle(&mut vram, &settings(), &attrs, [v(a.0, a.1, one), v(b.0, b.1, one), v(c.0, c.1, one)]);
    }
    for y in 0..16 {
        for x in 0..16 {
            assert_eq!(vram.get(x, y), 1, "pixel ({x}, {y}) covered exactly once");
        }
    }
    assert_eq!(count(&vram, 0, 1024, 0, 512), 256);
}

#[test]
fn degenerate_and_oversized_triangles_are_dropped() {
    let mut vram = Vram::new();
    draw_triangle(&mut vram, &settings(), &flat(), [v(0, 0, RED), v(10, 10, RED), v(20, 20, RED)]);
    assert_eq!(count(&vram, 0, 1024, 0, 512), 0, "collinear");

    draw_triangle(&mut vram, &settings(), &flat(), [v(0, 0, RED), v(1100, 0, RED), v(0, 20, RED)]);
    assert_eq!(count(&vram, 0, 1024, 0, 512), 0, "wider than 1023");

    draw_triangle(&mut vram, &settings(), &flat(), [v(0, 0, RED), v(20, 0, RED), v(0, 512, RED)]);
    assert_eq!(count(&vram, 0, 1024, 0, 512), 0, "taller than 511");
}

#[test]
fn drawing_is_clipped_to_the_draw_area() {
    let mut s = settings();
    s.area = DrawArea { left: 20, top: 20, right: 29, bottom: 29 };
    let mut vram = Vram::new();
    draw_triangle(&mut vram, &s, &flat(), [v(0, 0, RED), v(80, 0, RED), v(0, 80, RED)]);
    assert_eq!(count(&vram, 0, 1024, 0, 512), 100);
    assert_eq!(vram.get(20, 20), RED_15);
    assert_eq!(vram.get(29, 29), RED_15);
    assert_eq!(vram.get(30, 29), 0);
    assert_eq!(vram.get(19, 20), 0);
}

#[test]
fn the_drawing_offset_moves_everything() {
    let mut s = settings();
    s.offset_x = 100;
    s.offset_y = -5;
    let mut vram = Vram::new();
    draw_triangle(&mut vram, &s, &flat(), [v(10, 10, RED), v(30, 10, RED), v(10, 30, RED)]);
    assert_eq!(vram.get(112, 7), RED_15);
    assert_eq!(vram.get(12, 12), 0);
}

#[test]
fn gouraud_triangles_interpolate_colours() {
    let attrs = Attributes { gouraud: true, ..Attributes::default() };
    let mut vram = Vram::new();
    // A horizontal gradient from black (left edge) to full red (right vertex).
    draw_triangle(
        &mut vram,
        &settings(),
        &attrs,
        [v(0, 0, [0, 0, 0]), v(0, 64, [0, 0, 0]), v(64, 32, [255, 0, 0])],
    );
    let red = |x: usize, y: usize| vram.get(x, y) & 31;
    assert_eq!(red(1, 32), 0);
    assert!((14..=17).contains(&red(32, 32)), "midway is about half: {}", red(32, 32));
    assert!(red(60, 32) >= 28, "close to the red vertex: {}", red(60, 32));
    assert!((10..60).all(|x| red(x + 1, 32) >= red(x, 32)), "monotonic");
    assert_eq!(vram.get(32, 32) >> 5, 0, "no green or blue");
}

#[test]
fn flat_shading_uses_the_first_vertex_colour() {
    let mut vram = Vram::new();
    draw_triangle(
        &mut vram,
        &settings(),
        &flat(),
        [v(0, 0, RED), v(20, 0, [0, 255, 0]), v(0, 20, [0, 0, 255])],
    );
    assert_eq!(vram.get(3, 3), RED_15);
}

// ----- Rectangles -----

#[test]
fn flat_rectangle() {
    let mut vram = Vram::new();
    draw_rectangle(&mut vram, &settings(), &flat(), 10, 20, 8, 4, RED, 0, 0, false, false);
    assert_eq!(count(&vram, 0, 1024, 0, 512), 32);
    assert_eq!(vram.get(10, 20), RED_15);
    assert_eq!(vram.get(17, 23), RED_15);
    assert_eq!(vram.get(18, 23), 0);
}

#[test]
fn rectangles_are_clipped_to_the_draw_area() {
    let mut s = settings();
    s.area = DrawArea { left: 12, top: 21, right: 100, bottom: 100 };
    let mut vram = Vram::new();
    draw_rectangle(&mut vram, &s, &flat(), 10, 20, 8, 4, RED, 0, 0, false, false);
    assert_eq!(count(&vram, 0, 1024, 0, 512), 6 * 3);
}

/// A 15-bit texture page at (512, 0) with distinct texels along row 0.
fn texture_vram() -> Vram {
    let mut vram = Vram::new();
    vram.set(512, 0, 0x0000); // transparent
    vram.set(513, 0, 0x001F); // red
    vram.set(514, 0, 0x03E0); // green
    vram.set(515, 0, 0x7C00); // blue
    vram.set(512, 1, 0x7FFF);
    vram
}

fn texture_15() -> Texture {
    Texture { page_x: 512, page_y: 0, depth: TextureDepth::Bits15, clut_x: 0, clut_y: 0 }
}

fn textured(raw: bool) -> Attributes {
    Attributes { textured: true, raw_texture: raw, texture: texture_15(), ..Attributes::default() }
}

#[test]
fn textured_rectangle_copies_texels_and_skips_transparent_ones() {
    let mut vram = texture_vram();
    vram.set(100, 100, 0x1111);
    draw_rectangle(&mut vram, &settings(), &textured(true), 100, 100, 4, 1, [0; 3], 0, 0, false, false);
    assert_eq!(vram.get(100, 100), 0x1111, "texel 0 is transparent");
    assert_eq!(vram.get(101, 100), 0x001F);
    assert_eq!(vram.get(102, 100), 0x03E0);
    assert_eq!(vram.get(103, 100), 0x7C00);
}

#[test]
fn textured_rectangle_can_start_anywhere_and_wraps_at_256() {
    let mut vram = Vram::new();
    vram.set(512 + 255, 0, 0x001F);
    vram.set(512, 0, 0x03E0);
    draw_rectangle(&mut vram, &settings(), &textured(true), 0, 0, 2, 1, [0; 3], 255, 0, false, false);
    assert_eq!(vram.get(0, 0), 0x001F);
    assert_eq!(vram.get(1, 0), 0x03E0);
}

#[test]
fn rectangles_can_be_mirrored() {
    let mut vram = texture_vram();
    draw_rectangle(&mut vram, &settings(), &textured(true), 100, 100, 3, 1, [0; 3], 3, 0, true, false);
    assert_eq!(vram.get(100, 100), 0x7C00);
    assert_eq!(vram.get(101, 100), 0x03E0);
    assert_eq!(vram.get(102, 100), 0x001F);

    draw_rectangle(&mut vram, &settings(), &textured(true), 100, 200, 1, 2, [0; 3], 0, 1, false, true);
    assert_eq!(vram.get(100, 200), 0x7FFF, "row 1");
    assert_eq!(vram.get(100, 201), 0, "row 0, whose first texel is transparent");
}

#[test]
fn modulation_scales_texels_by_the_vertex_colour() {
    let mut vram = texture_vram();
    // 128 leaves the texel unchanged, 64 halves it, 255 nearly doubles it.
    for (color, expected_red) in [(128u8, 31u16), (64, 15), (255, 31)] {
        let mut copy = vram.clone();
        draw_rectangle(&mut copy, &settings(), &textured(false), 10, 10, 2, 1, [color; 3], 0, 1, false, false);
        assert_eq!(copy.get(10, 10) & 31, expected_red, "colour {color}");
    }
    // Raw textures ignore the colour.
    draw_rectangle(&mut vram, &settings(), &textured(true), 10, 10, 2, 1, [0; 3], 0, 1, false, false);
    assert_eq!(vram.get(10, 10), 0x7FFF);
}

#[test]
fn four_bit_textures_use_the_palette() {
    let mut vram = Vram::new();
    // Four texels in one halfword: indices 0, 1, 2, 3 (lowest nibble first).
    vram.set(512, 0, 0x3210);
    vram.set(64, 500, 0x0000);
    vram.set(65, 500, 0x001F);
    vram.set(66, 500, 0x03E0);
    vram.set(67, 500, 0x7C00);
    let texture = Texture { page_x: 512, page_y: 0, depth: TextureDepth::Bits4, clut_x: 64, clut_y: 500 };
    let attrs = Attributes { textured: true, raw_texture: true, texture, ..Attributes::default() };
    draw_rectangle(&mut vram, &settings(), &attrs, 200, 10, 4, 1, [0; 3], 0, 0, false, false);
    assert_eq!(vram.get(200, 10), 0, "index 0 maps to a transparent colour");
    assert_eq!(vram.get(201, 10), 0x001F);
    assert_eq!(vram.get(202, 10), 0x03E0);
    assert_eq!(vram.get(203, 10), 0x7C00);
}

#[test]
fn eight_bit_textures_use_the_palette() {
    let mut vram = Vram::new();
    vram.set(512, 0, 0x0201); // indices 1 then 2
    vram.set(0, 400, 0x0000);
    vram.set(1, 400, 0x001F);
    vram.set(2, 400, 0x03E0);
    let texture = Texture { page_x: 512, page_y: 0, depth: TextureDepth::Bits8, clut_x: 0, clut_y: 400 };
    let attrs = Attributes { textured: true, raw_texture: true, texture, ..Attributes::default() };
    draw_rectangle(&mut vram, &settings(), &attrs, 200, 10, 2, 1, [0; 3], 0, 0, false, false);
    assert_eq!(vram.get(200, 10), 0x001F);
    assert_eq!(vram.get(201, 10), 0x03E0);
}

#[test]
fn texture_window_repeats_a_region() {
    let mut vram = Vram::new();
    for u in 0..16 {
        vram.set(512 + u, 0, 0x0100 + u as u16);
    }
    let mut s = settings();
    // Keep only bit 3 of U from the offset: texels 0..7 read texels 8..15.
    s.window = TextureWindow { mask_x: 1, mask_y: 0, offset_x: 1, offset_y: 0 };
    draw_rectangle(&mut vram, &s, &textured(true), 300, 10, 4, 1, [0; 3], 0, 0, false, false);
    assert_eq!(vram.get(300, 10), 0x0108);
    assert_eq!(vram.get(303, 10), 0x010B);
}

#[test]
fn textured_triangles_interpolate_texture_coordinates() {
    let mut vram = Vram::new();
    for u in 0..64 {
        vram.set(512 + u, 0, 0x4000 | u as u16);
    }
    for y in 0..40 {
        for u in 0..64 {
            vram.set(512 + u, y, 0x4000 | u as u16);
        }
    }
    let attrs = textured(true);
    draw_triangle(
        &mut vram,
        &settings(),
        &attrs,
        [tv(0, 0, 0, 0), tv(63, 0, 63, 0), tv(0, 32, 0, 32)],
    );
    // U grows with X along a row.
    assert_eq!(vram.get(0, 4) & 0xFF, 0);
    assert_eq!(vram.get(20, 4) & 0xFF, 20);
    assert_eq!(vram.get(40, 4) & 0xFF, 40);
}

// ----- Semi-transparency, mask, dithering -----

fn blended(mode: u8) -> u16 {
    let mut vram = Vram::new();
    vram.set(5, 5, 10 | (20 << 5) | (30 << 10));
    let attrs = Attributes { semi_transparent: true, semi_mode: mode, ..Attributes::default() };
    // Front colour (4, 8, 12) in 5 bits.
    draw_rectangle(&mut vram, &settings(), &attrs, 5, 5, 1, 1, [32, 64, 96], 0, 0, false, false);
    vram.get(5, 5)
}

fn unpack(pixel: u16) -> (u16, u16, u16) {
    (pixel & 31, (pixel >> 5) & 31, (pixel >> 10) & 31)
}

#[test]
fn semi_transparency_modes() {
    assert_eq!(unpack(blended(0)), (7, 14, 21), "B/2 + F/2");
    assert_eq!(unpack(blended(1)), (14, 28, 31), "B + F, saturated");
    assert_eq!(unpack(blended(2)), (6, 12, 18), "B - F");
    assert_eq!(unpack(blended(3)), (11, 22, 31), "B + F/4");
}

#[test]
fn subtractive_blending_stops_at_zero() {
    let mut vram = Vram::new();
    vram.set(0, 0, 1);
    let attrs = Attributes { semi_transparent: true, semi_mode: 2, ..Attributes::default() };
    draw_rectangle(&mut vram, &settings(), &attrs, 0, 0, 1, 1, [255, 255, 255], 0, 0, false, false);
    assert_eq!(vram.get(0, 0), 0);
}

#[test]
fn only_texels_with_bit_15_blend_in_semi_transparent_textures() {
    let mut vram = Vram::new();
    vram.set(512, 0, 0x8000 | 0x001F); // blended red
    vram.set(513, 0, 0x001F); // opaque red
    vram.set(0, 0, 0x0010);
    vram.set(1, 0, 0x0010);
    let attrs = Attributes {
        textured: true,
        raw_texture: true,
        semi_transparent: true,
        semi_mode: 1,
        texture: texture_15(),
        ..Attributes::default()
    };
    draw_rectangle(&mut vram, &settings(), &attrs, 0, 0, 2, 1, [0; 3], 0, 0, false, false);
    assert_eq!(vram.get(0, 0) & 31, 31, "0x10 + 0x1F saturates");
    assert_eq!(vram.get(1, 0), 0x001F, "replaced, not added");
}

#[test]
fn set_mask_marks_pixels_and_check_mask_protects_them() {
    let mut s = settings();
    s.set_mask = true;
    let mut vram = Vram::new();
    draw_rectangle(&mut vram, &s, &flat(), 0, 0, 2, 1, RED, 0, 0, false, false);
    assert_eq!(vram.get(0, 0), 0x8000 | RED_15);

    s.set_mask = false;
    s.check_mask = true;
    draw_rectangle(&mut vram, &s, &flat(), 0, 0, 4, 1, [0, 255, 0], 0, 0, false, false);
    assert_eq!(vram.get(0, 0), 0x8000 | RED_15, "protected");
    assert_eq!(vram.get(2, 0), 0x03E0, "unprotected pixels are drawn");
}

#[test]
fn dithering_varies_gouraud_colours_by_position() {
    let attrs = Attributes { gouraud: true, ..Attributes::default() };
    let grey = [128, 128, 128];
    let mut dithered = settings();
    dithered.dither = true;

    let mut vram = Vram::new();
    draw_triangle(&mut vram, &dithered, &attrs, [v(0, 0, grey), v(32, 0, grey), v(0, 32, grey)]);
    let values: std::collections::BTreeSet<u16> =
        (0..16).flat_map(|y| (0..16).map(move |x| (x, y))).map(|(x, y)| vram.get(x, y) & 31).collect();
    assert_eq!(values, [15, 16].into_iter().collect(), "128 +/- up to 4 straddles 15.5 * 8");

    // Without the dither flag the same triangle is uniform.
    let mut plain = Vram::new();
    draw_triangle(&mut plain, &settings(), &attrs, [v(0, 0, grey), v(32, 0, grey), v(0, 32, grey)]);
    assert!((0..16).all(|x| plain.get(x, 0) & 31 == 16));

    // Flat-shaded untextured primitives are not dithered.
    let mut flat_vram = Vram::new();
    draw_triangle(&mut flat_vram, &dithered, &flat(), [v(0, 0, grey), v(32, 0, grey), v(0, 32, grey)]);
    assert!((0..16).all(|x| flat_vram.get(x, 0) & 31 == 16));
}

// ----- Lines -----

#[test]
fn horizontal_vertical_and_diagonal_lines() {
    let mut vram = Vram::new();
    draw_line(&mut vram, &settings(), &flat(), v(0, 0, RED), v(10, 0, RED));
    assert_eq!(count(&vram, 0, 1024, 0, 512), 11);

    let mut vram = Vram::new();
    draw_line(&mut vram, &settings(), &flat(), v(5, 5, RED), v(5, 0, RED));
    assert_eq!(count(&vram, 0, 1024, 0, 512), 6);
    assert_eq!(vram.get(5, 0), RED_15);

    let mut vram = Vram::new();
    draw_line(&mut vram, &settings(), &flat(), v(0, 0, RED), v(5, 5, RED));
    assert_eq!(count(&vram, 0, 1024, 0, 512), 6);
    for i in 0..=5 {
        assert_eq!(vram.get(i, i), RED_15);
    }
}

#[test]
fn a_zero_length_line_is_a_point() {
    let mut vram = Vram::new();
    draw_line(&mut vram, &settings(), &flat(), v(7, 9, RED), v(7, 9, RED));
    assert_eq!(count(&vram, 0, 1024, 0, 512), 1);
    assert_eq!(vram.get(7, 9), RED_15);
}

#[test]
fn gouraud_lines_blend_their_end_colours() {
    let attrs = Attributes { gouraud: true, ..Attributes::default() };
    let mut vram = Vram::new();
    draw_line(&mut vram, &settings(), &attrs, v(0, 0, [0, 0, 0]), v(64, 0, [255, 0, 0]));
    assert_eq!(vram.get(0, 0), 0);
    assert!((14..=17).contains(&(vram.get(32, 0) & 31)));
    assert_eq!(vram.get(64, 0) & 31, 31);
}

#[test]
fn lines_use_the_offset_and_the_clip_area() {
    let mut s = settings();
    s.offset_x = 10;
    s.area = DrawArea { left: 0, top: 0, right: 14, bottom: 511 };
    let mut vram = Vram::new();
    draw_line(&mut vram, &s, &flat(), v(0, 0, RED), v(9, 0, RED));
    assert_eq!(vram.get(10, 0), RED_15);
    assert_eq!(vram.get(14, 0), RED_15);
    assert_eq!(vram.get(15, 0), 0);
    assert_eq!(count(&vram, 0, 1024, 0, 512), 5);
}

#[test]
fn oversized_lines_are_dropped() {
    let mut vram = Vram::new();
    draw_line(&mut vram, &settings(), &flat(), v(0, 0, RED), v(1100, 0, RED));
    assert_eq!(count(&vram, 0, 1024, 0, 512), 0);
}
