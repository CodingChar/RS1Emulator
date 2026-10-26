use super::*;

fn gp0(gpu: &mut Gpu, words: &[u32]) {
    for word in words {
        gpu.write(0, 4, *word);
    }
}

fn gp1(gpu: &mut Gpu, word: u32) {
    gpu.write(4, 4, word);
}

/// A GPU whose draw area is the whole of VRAM.
fn gpu() -> Gpu {
    let mut gpu = Gpu::new();
    gp0(&mut gpu, &[0xE300_0000, 0xE400_0000 | (511 << 10) | 1023]);
    gpu
}

/// Pack a screen position.
fn xy(x: i32, y: i32) -> u32 {
    ((y as u32 & 0x7FF) << 16) | (x as u32 & 0x7FF)
}

const RED: u32 = 0x0000FF;
const GREEN: u32 = 0x00FF00;
const BLUE: u32 = 0xFF0000;
const RED_15: u16 = 0x001F;

fn pixels_set(gpu: &Gpu) -> usize {
    gpu.vram.pixels.iter().filter(|p| **p != 0).count()
}

// ----- Command lengths -----

#[test]
fn command_lengths() {
    let len = |opcode: u32| Gpu::command_length(opcode << 24);
    assert_eq!(len(0x00), Some(1));
    assert_eq!(len(0x02), Some(3));
    assert_eq!(len(0x20), Some(4), "flat triangle");
    assert_eq!(len(0x28), Some(5), "flat quad");
    assert_eq!(len(0x24), Some(7), "textured triangle");
    assert_eq!(len(0x2C), Some(9), "textured quad");
    assert_eq!(len(0x30), Some(6), "Gouraud triangle");
    assert_eq!(len(0x38), Some(8), "Gouraud quad");
    assert_eq!(len(0x34), Some(9), "textured Gouraud triangle");
    assert_eq!(len(0x3C), Some(12), "textured Gouraud quad");
    assert_eq!(len(0x40), Some(3), "line");
    assert_eq!(len(0x50), Some(4), "Gouraud line");
    assert_eq!(len(0x48), None, "polyline");
    assert_eq!(len(0x58), None, "Gouraud polyline");
    assert_eq!(len(0x60), Some(3), "variable rectangle");
    assert_eq!(len(0x68), Some(2), "1x1 dot");
    assert_eq!(len(0x70), Some(2), "8x8");
    assert_eq!(len(0x78), Some(2), "16x16");
    assert_eq!(len(0x64), Some(4), "textured variable rectangle");
    assert_eq!(len(0x7C), Some(3), "textured 16x16");
    assert_eq!(len(0x80), Some(4), "VRAM copy");
    assert_eq!(len(0xA0), Some(3));
    assert_eq!(len(0xC0), Some(3));
    assert_eq!(len(0xE1), Some(1));
}

// ----- Fill, upload, download, copy -----

#[test]
fn fill_through_gp0() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0x0200_00FF, xy(32, 8), (2 << 16) | 16]);
    assert_eq!(pixels_set(&gpu), 32);
    assert_eq!(gpu.vram.get(32, 8), RED_15);
    assert_eq!(gpu.vram.get(47, 9), RED_15);
}

#[test]
fn commands_may_arrive_one_word_at_a_time() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0x0200_00FF]);
    assert_eq!(pixels_set(&gpu), 0, "not complete yet");
    gp0(&mut gpu, &[xy(0, 0)]);
    assert_eq!(pixels_set(&gpu), 0);
    gp0(&mut gpu, &[(1 << 16) | 16]);
    assert_eq!(pixels_set(&gpu), 16);
}

#[test]
fn upload_then_download_round_trips() {
    let mut gpu = gpu();
    // 3x2 pixels = 6 halfwords = 3 words.
    gp0(&mut gpu, &[0xA000_0000, xy(100, 50), (2 << 16) | 3]);
    gp0(&mut gpu, &[0x0002_0001, 0x0004_0003, 0x0006_0005]);
    for (i, expected) in (1..=6).enumerate() {
        assert_eq!(gpu.vram.get(100 + i % 3, 50 + i / 3), expected as u16);
    }

    gp0(&mut gpu, &[0xC000_0000, xy(100, 50), (2 << 16) | 3]);
    assert_eq!(gpu.read(0, 4), 0x0002_0001);
    assert_eq!(gpu.read(0, 4), 0x0004_0003);
    assert_eq!(gpu.read(0, 4), 0x0006_0005);
    // The transfer is over: GPUREAD keeps returning the last word.
    assert_eq!(gpu.read(0, 4), 0x0006_0005);
}

#[test]
fn an_upload_with_an_odd_pixel_count_ignores_the_spare_halfword() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0xA000_0000, xy(0, 0), (1 << 16) | 3]);
    gp0(&mut gpu, &[0x0002_0001, 0x7777_0003]);
    assert_eq!(gpu.vram.get(2, 0), 3);
    assert_eq!(gpu.vram.get(3, 0), 0, "the upper halfword of the last word is dropped");
    // The next word starts a fresh command.
    gp0(&mut gpu, &[0x0200_00FF, xy(0, 8), (1 << 16) | 16]);
    assert_eq!(gpu.vram.get(0, 8), RED_15);
}

#[test]
fn uploads_wrap_around_the_right_edge_of_vram() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0xA000_0000, xy(1023, 0), (1 << 16) | 2]);
    gp0(&mut gpu, &[0x0002_0001]);
    assert_eq!(gpu.vram.get(1023, 0), 1);
    assert_eq!(gpu.vram.get(0, 0), 2, "wrapped to the next column... of the same row");
}

#[test]
fn vram_copy_through_gp0() {
    let mut gpu = gpu();
    gpu.vram.set(10, 10, 0x1234);
    gpu.vram.set(11, 10, 0x2345);
    gp0(&mut gpu, &[0x8000_0000, xy(10, 10), xy(200, 100), (1 << 16) | 2]);
    assert_eq!(gpu.vram.get(200, 100), 0x1234);
    assert_eq!(gpu.vram.get(201, 100), 0x2345);
}

#[test]
fn mask_settings_apply_to_uploads() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0xE600_0001]); // set mask bit
    gp0(&mut gpu, &[0xA000_0000, xy(0, 0), (1 << 16) | 2, 0x0002_0001]);
    assert_eq!(gpu.vram.get(0, 0), 0x8001);

    gpu.vram.set(1, 0, 0); // leave only the first pixel protected
    gp0(&mut gpu, &[0xE600_0002]); // check mask bit
    gp0(&mut gpu, &[0xA000_0000, xy(0, 0), (1 << 16) | 2, 0x0004_0003]);
    assert_eq!(gpu.vram.get(0, 0), 0x8001, "protected");
    assert_eq!(gpu.vram.get(1, 0), 4, "unprotected pixel written, no mask bit");
}

// ----- Polygons -----

#[test]
fn flat_triangle() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0x2000_0000 | RED, xy(10, 10), xy(30, 10), xy(10, 30)]);
    assert_eq!(gpu.vram.get(12, 12), RED_15);
    assert_eq!(gpu.vram.get(40, 40), 0);
}

#[test]
fn flat_quad_covers_the_whole_square() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0x2800_0000 | GREEN, xy(0, 0), xy(16, 0), xy(0, 16), xy(16, 16)]);
    assert_eq!(pixels_set(&gpu), 256);
    assert_eq!(gpu.vram.get(0, 0), 0x03E0);
    assert_eq!(gpu.vram.get(15, 15), 0x03E0);
}

#[test]
fn gouraud_triangle_reads_a_colour_before_each_later_vertex() {
    let mut gpu = gpu();
    gp0(
        &mut gpu,
        &[
            0x3000_0000 | RED,
            xy(0, 0),
            GREEN,
            xy(64, 0),
            BLUE,
            xy(0, 64),
        ],
    );
    let pixel = |x, y| gpu.vram.get(x, y);
    let (red, green, blue) = (|p: u16| p & 31, |p: u16| (p >> 5) & 31, |p: u16| (p >> 10) & 31);
    assert!(red(pixel(1, 1)) >= 29, "near the red vertex");
    assert!(green(pixel(60, 1)) >= 28, "near the green vertex");
    assert!(blue(pixel(1, 60)) >= 28, "near the blue vertex");
    assert!(green(pixel(1, 1)) <= 1 && blue(pixel(1, 1)) <= 1);
}

#[test]
fn the_drawing_offset_applies_to_polygons() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0xE500_0000 | (20 << 11) | 100]); // offset (100, 20)
    gp0(&mut gpu, &[0x2000_0000 | RED, xy(0, 0), xy(20, 0), xy(0, 20)]);
    assert_eq!(gpu.vram.get(102, 22), RED_15);
    assert_eq!(gpu.vram.get(2, 2), 0);
}

#[test]
fn negative_offsets_and_coordinates_are_sign_extended() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0xE500_0000 | (0x7FF) | (0x7F0 << 11)]); // offset (-1, -16)
    assert_eq!(gpu.offset_raw, 0x7FF | (0x7F0 << 11));
    assert_eq!(gpu.settings.offset_x, -1);
    assert_eq!(gpu.settings.offset_y, -16);
    assert_eq!(sign_extend_11(0x400), -1024);
    assert_eq!(sign_extend_11(0x3FF), 1023);
}

#[test]
fn the_draw_area_clips_polygons() {
    let mut gpu = Gpu::new();
    gp0(&mut gpu, &[0xE300_0000 | (10 << 10) | 10, 0xE400_0000 | (19 << 10) | 19]);
    gp0(&mut gpu, &[0x2800_0000 | RED, xy(0, 0), xy(100, 0), xy(0, 100), xy(100, 100)]);
    assert_eq!(pixels_set(&gpu), 100);
    assert_eq!(gpu.vram.get(10, 10), RED_15);
    assert_eq!(gpu.vram.get(19, 19), RED_15);
    assert_eq!(gpu.vram.get(20, 19), 0);
}

#[test]
fn semi_transparent_polygons_blend_with_the_current_mode() {
    let mut gpu = gpu();
    gpu.vram.set(5, 5, 10);
    // Mode 1 (additive) through GP0(E1h) bits 5-6.
    gp0(&mut gpu, &[0xE100_0000 | (1 << 5)]);
    gp0(&mut gpu, &[0x2200_0000 | 0x000008, xy(0, 0), xy(20, 0), xy(0, 20)]); // semi-transparent, red = 8 -> 1
    assert_eq!(gpu.vram.get(5, 5), 11);
}

#[test]
fn textured_polygons_take_their_page_from_the_second_vertex() {
    let mut gpu = gpu();
    // A 15-bit texture of a single red texel run at page (x=8*64=512, y=0).
    for u in 0..32 {
        for v in 0..32 {
            gpu.vram.set(512 + u, v, 0x001F);
        }
    }
    let page = 8 | (2 << 7); // x = 8, 15-bit
    gp0(
        &mut gpu,
        &[
            0x2500_0000, // textured raw triangle
            xy(100, 100),
            0x0000_0000, // u=0 v=0, clut 0
            xy(120, 100),
            (page << 16) | 0x0000_0014, // u=20
            xy(100, 120),
            0x0000_1400, // v=20
        ],
    );
    assert_eq!(gpu.vram.get(105, 105), RED_15);
    assert_eq!(gpu.status() & 0xF, 8, "GPUSTAT shows the new texture page");
    assert_eq!((gpu.status() >> 7) & 3, 2);
}

#[test]
fn textured_rectangle_with_a_palette() {
    let mut gpu = gpu();
    // 4-bit texture at page x = 0 (VRAM x 0), palette at (0, 500) = 16 * 0, row 500.
    gpu.vram.set(0, 0, 0x2211); // texels 1, 1, 2, 2
    gpu.vram.set(1, 500, 0x001F); // colour 1: red
    gpu.vram.set(2, 500, 0x03E0); // colour 2: green
    let clut = (500 << 6) | 0;
    gp0(&mut gpu, &[0xE100_0000]); // 4-bit, page (0, 0)
    gp0(&mut gpu, &[0x6500_0000, xy(200, 100), (clut << 16) | 0x0000, (1 << 16) | 4]);
    assert_eq!(gpu.vram.get(200, 100), RED_15);
    assert_eq!(gpu.vram.get(201, 100), RED_15);
    assert_eq!(gpu.vram.get(202, 100), 0x03E0);
    assert_eq!(gpu.vram.get(203, 100), 0x03E0);
}

#[test]
fn fixed_size_rectangles() {
    for (opcode, size) in [(0x68u32, 1usize), (0x70, 8), (0x78, 16)] {
        let mut gpu = gpu();
        gp0(&mut gpu, &[(opcode << 24) | RED, xy(40, 40)]);
        assert_eq!(pixels_set(&gpu), size * size, "opcode {opcode:#x}");
        assert_eq!(gpu.vram.get(40, 40), RED_15);
    }
}

#[test]
fn variable_rectangle() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0x6000_0000 | BLUE, xy(5, 6), (3 << 16) | 7]);
    assert_eq!(pixels_set(&gpu), 21);
    assert_eq!(gpu.vram.get(5, 6), 0x7C00);
    assert_eq!(gpu.vram.get(11, 8), 0x7C00);
    assert_eq!(gpu.vram.get(12, 8), 0);
}

#[test]
fn mirrored_rectangles_follow_gp0_e1() {
    let mut gpu = gpu();
    for u in 0..4 {
        gpu.vram.set(u, 0, 0x0001 + u as u16);
    }
    gp0(&mut gpu, &[0xE100_0000 | (2 << 7) | (1 << 12)]); // 15-bit, x-flip
    gp0(&mut gpu, &[0x7500_0000, xy(100, 100), 0x0000_0003]); // 1x1... textured 8x8 at u=3
    assert_eq!(gpu.vram.get(100, 100), 4, "starts at u=3 and walks backwards");
    assert_eq!(gpu.vram.get(103, 100), 1);
}

// ----- Lines -----

#[test]
fn flat_line() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0x4000_0000 | RED, xy(0, 0), xy(10, 0)]);
    assert_eq!(pixels_set(&gpu), 11);
}

#[test]
fn gouraud_line_has_a_colour_per_vertex() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0x5000_0000, xy(0, 0), RED, xy(10, 0)]);
    assert_eq!(gpu.vram.get(0, 0), 0);
    assert_eq!(gpu.vram.get(10, 0), RED_15);
}

#[test]
fn polylines_run_until_the_terminator() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0x4800_0000 | RED, xy(0, 0), xy(10, 0), xy(10, 10)]);
    assert_eq!(pixels_set(&gpu), 0, "still waiting for the terminator");
    gp0(&mut gpu, &[0x5555_5555]);
    assert_eq!(pixels_set(&gpu), 11 + 10, "two segments sharing a corner");
    // The next word is a normal command again.
    gp0(&mut gpu, &[0x0200_00FF, xy(0, 100), (1 << 16) | 16]);
    assert_eq!(gpu.vram.get(0, 100), RED_15);
}

#[test]
fn gouraud_polylines_alternate_colours_and_vertices() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0x5800_0000, xy(0, 0), RED, xy(10, 0), BLUE, xy(10, 10), 0x5555_5555]);
    assert_eq!(gpu.vram.get(10, 0), RED_15);
    assert_eq!(gpu.vram.get(10, 10), 0x7C00);
}

// ----- State registers -----

#[test]
fn draw_mode_updates_gpustat() {
    let mut gpu = Gpu::new();
    // Page (5, 1), semi mode 2, 8-bit textures, dither on, draw to display on.
    gp0(&mut gpu, &[0xE100_0000 | 5 | (1 << 4) | (2 << 5) | (1 << 7) | (1 << 9) | (1 << 10)]);
    let stat = gpu.status();
    assert_eq!(stat & 0xF, 5);
    assert_eq!((stat >> 4) & 1, 1);
    assert_eq!((stat >> 5) & 3, 2);
    assert_eq!((stat >> 7) & 3, 1);
    assert_eq!((stat >> 9) & 1, 1);
    assert_eq!((stat >> 10) & 1, 1);
}

#[test]
fn mask_bits_show_in_gpustat() {
    let mut gpu = Gpu::new();
    gp0(&mut gpu, &[0xE600_0003]);
    assert_eq!((gpu.status() >> 11) & 3, 3);
    gp0(&mut gpu, &[0xE600_0000]);
    assert_eq!((gpu.status() >> 11) & 3, 0);
}

#[test]
fn info_queries_return_the_settings() {
    let mut gpu = Gpu::new();
    gp0(&mut gpu, &[0xE200_0000 | 0x12345, 0xE300_0000 | 0x00ABC, 0xE400_0000 | 0x00DEF, 0xE500_0000 | 0x00321]);
    for (index, expected) in [(2u32, 0x12345u32), (3, 0xABC), (4, 0xDEF), (5, 0x321), (7, 2)] {
        gp1(&mut gpu, 0x1000_0000 | index);
        assert_eq!(gpu.read(0, 4), expected, "query {index}");
    }
}

#[test]
fn gp1_reset_restores_the_defaults_but_keeps_vram() {
    let mut gpu = gpu();
    gpu.vram.set(1, 1, 0x1234);
    gp0(&mut gpu, &[0xE100_000F, 0xE600_0003]);
    gp1(&mut gpu, 0x0300_0000); // display on
    gp1(&mut gpu, 0x0000_0000);
    let stat = gpu.status();
    assert_eq!(stat & 0xF, 0);
    assert_eq!((stat >> 11) & 3, 0);
    assert_ne!(stat & (1 << 23), 0, "display disabled again");
    assert_eq!(gpu.vram.get(1, 1), 0x1234);
}

#[test]
fn gp1_resets_the_command_buffer() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0x2000_0000 | RED, xy(0, 0)]); // half a triangle
    gp1(&mut gpu, 0x0100_0000);
    gp0(&mut gpu, &[0x0200_00FF, xy(0, 0), (1 << 16) | 16]);
    assert_eq!(gpu.vram.get(0, 0), RED_15);
}

// ----- Interrupt, DMA direction, status flags -----

#[test]
fn gp0_1f_raises_the_gpu_interrupt_until_acknowledged() {
    let mut gpu = Gpu::new();
    assert!(!gpu.take_irq());
    gp0(&mut gpu, &[0x1F00_0000]);
    assert!(gpu.take_irq());
    assert!(!gpu.take_irq(), "an edge, not a level");
    assert_ne!(gpu.status() & (1 << 24), 0);

    gp0(&mut gpu, &[0x1F00_0000]);
    assert!(!gpu.take_irq(), "already pending");
    gp1(&mut gpu, 0x0200_0000);
    assert_eq!(gpu.status() & (1 << 24), 0);
    gp0(&mut gpu, &[0x1F00_0000]);
    assert!(gpu.take_irq());
}

#[test]
fn dma_direction_shows_in_gpustat() {
    let mut gpu = Gpu::new();
    assert_eq!(gpu.status() & (1 << 25), 0);
    for direction in 1..=3u32 {
        gp1(&mut gpu, 0x0400_0000 | direction);
        assert_eq!((gpu.status() >> 29) & 3, direction);
        assert_ne!(gpu.status() & (1 << 25), 0, "data request");
    }
    gp1(&mut gpu, 0x0400_0000);
    assert_eq!(gpu.status() & (1 << 25), 0);
}

#[test]
fn gpu_is_always_ready_for_commands_and_dma() {
    let gpu = Gpu::new();
    assert_eq!(gpu.status() & 0x1C00_0000, 0x1C00_0000);
}

#[test]
fn odd_even_bit_follows_the_scanline_outside_vblank() {
    let mut gpu = Gpu::new();
    assert_eq!(gpu.status() >> 31, 0, "line 0");
    gpu.scanline = 7;
    assert_eq!(gpu.status() >> 31, 1);
    gpu.scanline = 8;
    assert_eq!(gpu.status() >> 31, 0);
    gpu.scanline = 241;
    assert_eq!(gpu.status() >> 31, 0, "stuck at 0 in vblank");
    assert_ne!(gpu.status() & (1 << 13), 0, "field bit reads 1 when not interlaced");
}

#[test]
fn interlaced_mode_exposes_the_field_instead() {
    let mut gpu = Gpu::new();
    gp1(&mut gpu, 0x0800_0000 | (1 << 5) | (1 << 2)); // 480i
    let before = gpu.status() >> 31;
    // A vblank flips the field.
    while !gpu.advance(1000).vblank_started {}
    let after = gpu.status() >> 31;
    assert_ne!(before, after);
    assert_eq!((gpu.status() >> 13) & 1, 1 - after);
}

// ----- Display -----

#[test]
fn display_mode_sets_the_resolution() {
    let mut gpu = Gpu::new();
    for (mode, width, height) in [
        (0u32, 256usize, 240usize),
        (1, 320, 240),
        (2, 512, 240),
        (3, 640, 240),
        (1 | (1 << 2), 320, 480),
        (1 << 6, 368, 240),
    ] {
        gp1(&mut gpu, 0x0800_0000 | mode);
        assert_eq!((gpu.display_width(), gpu.display_height()), (width, height), "mode {mode:#x}");
    }
}

#[test]
fn display_mode_shows_in_gpustat() {
    let mut gpu = Gpu::new();
    gp1(&mut gpu, 0x0800_0000 | 0b0101_1111 | (1 << 7));
    let stat = gpu.status();
    assert_eq!((stat >> 17) & 3, 3, "horizontal resolution 1");
    assert_eq!((stat >> 19) & 1, 1);
    assert_eq!((stat >> 20) & 1, 1, "PAL");
    assert_eq!((stat >> 21) & 1, 1, "24-bit colour");
    assert_eq!((stat >> 16) & 1, 1, "368 wide");
    assert_eq!((stat >> 14) & 1, 1, "reverse flag");
}

#[test]
fn framebuffer_shows_the_display_area_in_15_bit_colour() {
    let mut gpu = Gpu::new();
    gp1(&mut gpu, 0x0800_0001); // 320x240
    gp1(&mut gpu, 0x0300_0000);
    gpu.vram.set(100, 40, 0x001F); // red
    gpu.vram.set(101, 40, 0x03E0); // green
    gpu.vram.set(102, 40, 0x7C00); // blue
    gpu.vram.set(103, 40, 0x7FFF); // white
    gp1(&mut gpu, 0x0500_0000 | (40 << 10) | 100); // display starts at (100, 40)

    let fb = gpu.framebuffer();
    assert_eq!(fb.len(), 320 * 240 * 4);
    assert_eq!(&fb[0..4], &[255, 0, 0, 255]);
    assert_eq!(&fb[4..8], &[0, 255, 0, 255]);
    assert_eq!(&fb[8..12], &[0, 0, 255, 255]);
    assert_eq!(&fb[12..16], &[255, 255, 255, 255]);
    assert_eq!(&fb[16..20], &[0, 0, 0, 255], "empty VRAM is black");
}

#[test]
fn a_disabled_display_is_black() {
    let mut gpu = Gpu::new();
    gp1(&mut gpu, 0x0800_0001);
    gpu.vram.set(0, 0, 0x7FFF);
    let fb = gpu.framebuffer();
    assert_eq!(&fb[0..4], &[0, 0, 0, 255]);
    gp1(&mut gpu, 0x0300_0000);
    assert_eq!(&gpu.framebuffer()[0..4], &[255, 255, 255, 255]);
    gp1(&mut gpu, 0x0300_0001);
    assert_eq!(&gpu.framebuffer()[0..4], &[0, 0, 0, 255]);
}

#[test]
fn true_colour_mode_reads_packed_24_bit_pixels() {
    let mut gpu = Gpu::new();
    gp1(&mut gpu, 0x0800_0000 | 1 | (1 << 4)); // 320 wide, 24-bit
    gp1(&mut gpu, 0x0300_0000);
    // Two pixels = six bytes = three halfwords: (10, 20, 30) and (40, 50, 60).
    gpu.vram.set(0, 0, 10 | (20 << 8));
    gpu.vram.set(1, 0, 30 | (40 << 8));
    gpu.vram.set(2, 0, 50 | (60 << 8));
    let fb = gpu.framebuffer();
    assert_eq!(&fb[0..4], &[10, 20, 30, 255]);
    assert_eq!(&fb[4..8], &[40, 50, 60, 255]);
}

#[test]
fn drawing_is_visible_on_the_display() {
    let mut gpu = gpu();
    gp1(&mut gpu, 0x0800_0001);
    gp1(&mut gpu, 0x0300_0000);
    gp0(&mut gpu, &[0x2000_0000 | RED, xy(0, 0), xy(100, 0), xy(0, 100)]);
    let fb = gpu.framebuffer();
    let at = (10 * 320 + 10) * 4;
    assert_eq!(&fb[at..at + 4], &[255, 0, 0, 255]);
}

// ----- Video timing (see also the CPU-visible effects in the bus tests) -----

/// Advance one CPU cycle at a time, summing the video events.
fn run(gpu: &mut Gpu, cycles: u64) -> (u64, u64, u64) {
    let (mut dots, mut hblanks, mut vblanks) = (0u64, 0u64, 0u64);
    for _ in 0..cycles {
        let t = gpu.advance(1);
        dots += t.dot_ticks as u64;
        hblanks += t.hblanks as u64;
        vblanks += t.vblank_started as u64;
    }
    (dots, hblanks, vblanks)
}

#[test]
fn a_frame_is_263_lines_and_one_vblank() {
    let mut gpu = Gpu::new();
    let (_, hblanks, vblanks) = run(&mut gpu, CPU_CYCLES_PER_FRAME);
    assert_eq!(vblanks, 1);
    assert!((262..=263).contains(&hblanks), "got {hblanks} scanlines");
}

#[test]
fn vblank_starts_at_line_240() {
    let mut gpu = Gpu::new();
    let mut cycles = 0u64;
    while !gpu.advance(1).vblank_started {
        cycles += 1;
    }
    cycles += 1;
    // 240 lines * 3413 GPU clocks * 7/11 CPU cycles per GPU clock.
    assert!((521_258..=521_260).contains(&cycles), "got {cycles}");
    assert!(gpu.in_vblank());
}

#[test]
fn video_timing_is_periodic() {
    let mut gpu = Gpu::new();
    // Three whole frames of GPU clocks, rounded up to whole CPU cycles
    // (a frame is not an integer number of CPU cycles).
    let gpu_clocks = 3 * NTSC_LINES_PER_FRAME as u64 * NTSC_CLOCKS_PER_LINE as u64;
    let cycles = (gpu_clocks * 7).div_ceil(11);
    let (_, hblanks, vblanks) = run(&mut gpu, cycles);
    assert_eq!(vblanks, 3);
    assert_eq!(hblanks, 3 * NTSC_LINES_PER_FRAME as u64);
    assert_eq!(gpu.scanline, 0);
}

#[test]
fn pal_frames_are_longer() {
    let mut gpu = Gpu::new();
    let ntsc = gpu.cycles_per_frame();
    gp1(&mut gpu, 0x0800_0000 | (1 << 3));
    assert!(gpu.is_pal());
    let pal = gpu.cycles_per_frame();
    assert!(pal > ntsc, "{pal} vs {ntsc}");
    let (_, hblanks, vblanks) = run(&mut gpu, pal + 10);
    assert_eq!(vblanks, 1);
    assert!((313..=314).contains(&hblanks), "got {hblanks} scanlines");
}

#[test]
fn dot_clock_follows_the_horizontal_resolution() {
    for (mode, divider) in [(0u32, 10u64), (1, 8), (2, 5), (3, 4), (1 << 6, 7)] {
        let mut gpu = Gpu::new();
        gp1(&mut gpu, 0x0800_0000 | mode);
        let (dots, _, _) = run(&mut gpu, 7 * 1000);
        // 7000 CPU cycles = 11000 GPU clocks.
        assert_eq!(dots, 11_000 / divider, "mode {mode:#x}");
    }
}

#[test]
fn advancing_in_big_steps_matches_single_steps() {
    let mut a = Gpu::new();
    let mut b = Gpu::new();
    let mut sum = VideoTick::default();
    for _ in 0..1000 {
        let t = a.advance(1000);
        sum.dot_ticks += t.dot_ticks;
        sum.hblanks += t.hblanks;
        sum.vblank_started |= t.vblank_started;
    }
    let (dots, hblanks, _) = run(&mut b, 1_000_000);
    assert_eq!(sum.dot_ticks as u64, dots);
    assert_eq!(sum.hblanks as u64, hblanks);
    assert!(sum.vblank_started);
}

// ----- Save states -----

#[test]
fn registers_and_vram_survive_a_save_and_load() {
    let mut a = gpu();
    gp0(&mut a, &[0xE100_0000 | 3 | (1 << 4) | (2 << 5) | (2 << 7), 0xE200_0000 | 0x12345, 0xE500_0000 | 0x321, 0xE600_0003]);
    gp1(&mut a, 0x0800_0000 | 0b1011);
    gp1(&mut a, 0x0300_0000);
    gp1(&mut a, 0x0500_0000 | (7 << 10) | 9);
    a.vram.set(77, 33, 0xBEEF);
    a.scanline = 123;

    let mut b = Gpu::new();
    b.load_registers(&a.save_registers());
    b.load_vram_bytes(&a.vram_bytes());

    assert_eq!(b.status() & !(1 << 31), a.status() & !(1 << 31));
    assert_eq!(b.save_registers(), a.save_registers());
    assert_eq!(b.vram.get(77, 33), 0xBEEF);
    assert_eq!(b.framebuffer(), a.framebuffer());
    assert_eq!(b.scanline, 123);
}

#[test]
fn vram_bytes_round_trip_and_reject_wrong_sizes() {
    let mut gpu = Gpu::new();
    gpu.vram.set(1023, 511, 0x1234);
    let bytes = gpu.vram_bytes();
    assert_eq!(bytes.len(), 1024 * 512 * 2);
    assert_eq!(&bytes[bytes.len() - 2..], &[0x34, 0x12]);

    let mut other = Gpu::new();
    other.load_vram_bytes(&bytes[..100]);
    assert_eq!(other.vram.get(1023, 511), 0, "wrong size ignored");
    other.load_vram_bytes(&bytes);
    assert_eq!(other.vram.get(1023, 511), 0x1234);
}

#[test]
fn register_access_is_32_bit_with_partial_reads() {
    let mut gpu = Gpu::new();
    let stat = gpu.status();
    assert_eq!(gpu.read(4, 4), stat);
    assert_eq!(gpu.read(4, 2), stat & 0xFFFF);
    assert_eq!(gpu.read(6, 2), stat >> 16);
    assert_eq!(gpu.read(0xC, 4), 0, "no register there");
}

// ----- Blanking intervals -----

#[test]
fn hblank_is_the_part_of_the_line_outside_the_display_range() {
    let mut gpu = Gpu::new();
    // Default range: 0x200 .. 0x200 + 2560 clocks out of 3413.
    gpu.line_clock = 100;
    assert!(gpu.in_hblank());
    gpu.line_clock = 0x200;
    assert!(!gpu.in_hblank());
    gpu.line_clock = 0x200 + 2559;
    assert!(!gpu.in_hblank());
    gpu.line_clock = 0x200 + 2560;
    assert!(gpu.in_hblank());
}

#[test]
fn the_next_blank_edge_is_never_later_than_the_real_one() {
    // Advancing to just before the reported edge never changes the blanking state.
    let mut gpu = Gpu::new();
    for step in 0..4000u32 {
        let before = (gpu.in_hblank(), gpu.in_vblank());
        let edge = gpu.video_timing().next_blank_edge;
        assert!(edge >= 1);
        if edge > 1 {
            gpu.advance(edge - 1);
            assert_eq!((gpu.in_hblank(), gpu.in_vblank()), before, "step {step}");
        }
        gpu.advance(1 + step % 3);
    }
}

#[test]
fn advancing_reports_when_hblank_and_vblank_begin() {
    let mut gpu = Gpu::new();
    gpu.line_clock = 0x200 + 2559;
    let tick = gpu.advance(4);
    assert!(tick.in_hblank && tick.hblank_started);
    let tick = gpu.advance(1);
    assert!(tick.in_hblank && !tick.hblank_started, "still the same interval");
    gpu.scanline = VBLANK_START_LINE - 1;
    gpu.line_clock = 3400;
    let tick = gpu.advance(10);
    assert!(tick.in_vblank && tick.vblank_started);
}

// ----- Drawing time -----

const READY_BITS: u32 = (1 << 26) | (1 << 28);

#[test]
fn a_fresh_gpu_is_ready() {
    assert_eq!(gpu().status() & READY_BITS, READY_BITS);
}

#[test]
fn drawing_keeps_the_gpu_busy_until_enough_time_passes() {
    let mut gpu = gpu();
    // A 256x256 flat rectangle.
    gp0(&mut gpu, &[0x6000_00FF, xy(0, 0), xy(256, 256)]);
    assert_eq!(gpu.status() & READY_BITS, 0, "busy right after the command");
    gpu.advance(10);
    assert_eq!(gpu.status() & READY_BITS, 0, "still busy a few cycles later");
    gpu.advance(200_000);
    assert_eq!(gpu.status() & READY_BITS, READY_BITS, "done");
}

#[test]
fn bigger_work_takes_longer_and_clears_do_not_leave_the_gpu_busy_for_ever() {
    let busy_for = |words: &[u32]| {
        let mut gpu = gpu();
        gp0(&mut gpu, words);
        let mut cycles = 0;
        while gpu.is_busy() {
            gpu.advance(100);
            cycles += 100;
            assert!(cycles < 5_000_000, "never finishes");
        }
        cycles
    };
    let small = busy_for(&[0x6000_00FF, xy(0, 0), xy(8, 8)]);
    let large = busy_for(&[0x6000_00FF, xy(0, 0), xy(256, 256)]);
    assert!(large > small * 10, "{large} vs {small}");
    let textured = busy_for(&[0x6500_0000, xy(0, 0), 0, xy(64, 64)]);
    let flat = busy_for(&[0x6000_00FF, xy(0, 0), xy(64, 64)]);
    assert!(textured > flat);
    assert!(busy_for(&[0x0200_00FF, xy(0, 0), xy(1024, 512)]) < 1_000_000);
}

#[test]
fn a_gpu_reset_drops_the_queued_work() {
    let mut gpu = gpu();
    gp0(&mut gpu, &[0x6000_00FF, xy(0, 0), xy(256, 256)]);
    gp1(&mut gpu, 0);
    assert_eq!(gpu.status() & READY_BITS, READY_BITS);
}

// ----- Scaled rendering and PGXP -----

fn scaled(settings: RenderSettings) -> Gpu {
    let mut gpu = gpu();
    gpu.set_render_settings(settings);
    gpu
}

fn tag(x: f32, y: f32, w: f32, word: u32) -> PreciseVertex {
    PreciseVertex { x, y, w, value: word }
}

/// How many scaled pixels of row `y` are drawn.
fn covered(gpu: &Gpu, y: usize) -> usize {
    let (pixels, stride) = gpu.scaled_vram().unwrap();
    pixels[y * stride..(y + 1) * stride].iter().filter(|p| **p != 0).count()
}

/// A flat red triangle (0,0) (8,0) (0,8) whose vertices carry the given tags.
fn tagged_triangle(gpu: &mut Gpu, tags: [PreciseVertex; 3]) {
    let words = [xy(0, 0), xy(8, 0), xy(0, 8)];
    gpu.write(0, 4, 0x2000_00FF);
    for (word, tag) in words.iter().zip(tags) {
        gpu.write_gp0_precise(*word, tag);
    }
}

#[test]
fn polygons_are_drawn_into_both_the_console_and_the_scaled_vram() {
    let mut gpu = scaled(RenderSettings { scale: 2, ..RenderSettings::default() });
    gp0(&mut gpu, &[0x2000_00FF, xy(10, 10), xy(30, 10), xy(10, 30)]);
    assert_eq!(gpu.vram.get(12, 12), RED_15);
    let (pixels, stride) = gpu.scaled_vram().unwrap();
    assert_eq!(pixels[24 * stride + 24] & 0xFFFFFF, 0x0000FF);
    assert_eq!(pixels[25 * stride + 25] & 0xFFFFFF, 0x0000FF);
    assert_eq!(pixels[10 * stride + 10], 0, "outside the triangle");
}

#[test]
fn precise_vertices_change_the_scaled_picture_but_not_the_console_one() {
    let triangle = |tags: [PreciseVertex; 3]| {
        let mut gpu = scaled(RenderSettings { scale: 4, pgxp: true, ..RenderSettings::default() });
        tagged_triangle(&mut gpu, tags);
        gpu
    };
    let words = [xy(0, 0), xy(8, 0), xy(0, 8)];
    let plain = triangle([PreciseVertex::NONE; 3]);
    let moved = triangle([
        tag(0.0, 0.0, 100.0, words[0]),
        tag(8.5, 0.0, 100.0, words[1]),
        tag(0.0, 8.0, 100.0, words[2]),
    ]);
    assert!(covered(&moved, 4) > covered(&plain, 4), "the edge moved outwards");
    assert_eq!(moved.vram.pixels, plain.vram.pixels, "the console VRAM only sees integers");
}

#[test]
fn a_precise_vertex_far_from_the_integer_one_is_not_trusted() {
    let words = [xy(0, 0), xy(8, 0), xy(0, 8)];
    let triangle = |x: f32| {
        let mut gpu = scaled(RenderSettings { scale: 4, pgxp: true, ..RenderSettings::default() });
        tagged_triangle(
            &mut gpu,
            [tag(0.0, 0.0, 100.0, words[0]), tag(x, 0.0, 100.0, words[1]), tag(0.0, 8.0, 100.0, words[2])],
        );
        gpu
    };
    let plain = triangle(8.0);
    // The GTE clamped this one: the integer vertex says 8, the "precise" one 30.
    assert_eq!(covered(&triangle(30.0), 4), covered(&plain, 4));
    assert!(covered(&triangle(8.75), 4) > covered(&plain, 4), "within a pixel: trusted");
}

#[test]
fn without_pgxp_the_tags_are_ignored() {
    let words = [xy(0, 0), xy(8, 0), xy(0, 8)];
    let mut gpu = scaled(RenderSettings { scale: 4, ..RenderSettings::default() });
    tagged_triangle(
        &mut gpu,
        [tag(0.0, 0.0, 100.0, words[0]), tag(8.75, 0.0, 100.0, words[1]), tag(0.0, 8.0, 100.0, words[2])],
    );
    let mut reference = scaled(RenderSettings { scale: 4, ..RenderSettings::default() });
    tagged_triangle(&mut reference, [PreciseVertex::NONE; 3]);
    assert_eq!(covered(&gpu, 4), covered(&reference, 4));
}

#[test]
fn widescreen_squeezes_3d_geometry_towards_the_middle_of_the_drawing_area() {
    // A draw area 320 wide: the middle is x = 160.
    let draw = |widescreen: bool, precise: bool| {
        let mut gpu = Gpu::new();
        gp0(&mut gpu, &[0xE300_0000, 0xE400_0000 | (239 << 10) | 319]);
        gpu.set_render_settings(RenderSettings { scale: 1, pgxp: true, widescreen, ..RenderSettings::default() });
        let words = [xy(60, 10), xy(260, 10), xy(60, 30)];
        gpu.write(0, 4, 0x2000_00FF);
        for word in words {
            let vertex = if precise {
                let (x, y) = ((word & 0x7FF) as f32, (word >> 16) as f32);
                tag(x, y, 100.0, word)
            } else {
                PreciseVertex::NONE
            };
            gpu.write_gp0_precise(word, vertex);
        }
        let (pixels, stride) = gpu.scaled_vram().unwrap();
        let row = &pixels[12 * stride..13 * stride];
        row.iter().position(|p| *p != 0).unwrap()
    };
    // 60 -> 160 - 100 * 0.75 = 85.
    assert_eq!(draw(true, true), 85);
    assert_eq!(draw(false, true), 60);
    assert_eq!(draw(true, false), 60, "2D elements (no precise vertices) are left alone");
}

#[test]
fn fills_uploads_and_copies_reach_the_scaled_vram() {
    let mut gpu = scaled(RenderSettings { scale: 2, ..RenderSettings::default() });
    gp0(&mut gpu, &[0x0200_00FF, xy(32, 8), (2 << 16) | 16]);
    gp0(&mut gpu, &[0xA000_0000, xy(100, 50), (1 << 16) | 2, 0x7FFF_001F]);
    gp0(&mut gpu, &[0x8000_0000, xy(100, 50), xy(200, 60), (1 << 16) | 2]);
    let (pixels, stride) = gpu.scaled_vram().unwrap();
    assert_eq!(pixels[16 * stride + 64] & 0xFFFFFF, 0x0000FF, "fill");
    assert_eq!(pixels[100 * stride + 200] & 0xFFFFFF, 0x0000FF, "upload: red, repeated 2x2");
    assert_eq!(pixels[101 * stride + 201] & 0xFFFFFF, 0x0000FF);
    assert_eq!(pixels[100 * stride + 202] & 0xFFFFFF, 0xFFFFFF, "upload: white next to it");
    assert_eq!(pixels[120 * stride + 400] & 0xFFFFFF, 0x0000FF, "copy");
    assert_eq!(pixels[120 * stride + 402] & 0xFFFFFF, 0xFFFFFF);
}

#[test]
fn the_frame_is_as_large_as_the_display_times_the_internal_resolution() {
    let mut gpu = gpu();
    gp1(&mut gpu, 0x0300_0000); // display on
    gp1(&mut gpu, 0x0800_0001); // 320x240
    let mut frame = Vec::new();
    assert_eq!(gpu.render_frame(&mut frame), (320, 240));
    assert_eq!(frame.len(), 320 * 240);
    gpu.set_render_settings(RenderSettings { scale: 3, ..RenderSettings::default() });
    assert_eq!(gpu.render_frame(&mut frame), (960, 720));
    assert_eq!(frame.len(), 960 * 720);
    gp1(&mut gpu, 0x0800_0007); // 640x480
    assert_eq!(gpu.render_frame(&mut frame), (1920, 1440));
    gpu.set_render_settings(RenderSettings::default());
    assert_eq!(gpu.render_frame(&mut frame), (640, 480));
}

#[test]
fn a_24_bit_picture_is_shown_as_the_console_has_it_repeated() {
    let mut gpu = scaled(RenderSettings { scale: 2, ..RenderSettings::default() });
    gp1(&mut gpu, 0x0300_0000);
    gp1(&mut gpu, 0x0800_0010); // 256x240, 24-bit
    // Two pixels of 24-bit data: the bytes 01 02 03 | 04 05 06 are the words 0x0201, 0x0403, 0x0605.
    gpu.vram.set(0, 0, 0x0201);
    gpu.vram.set(1, 0, 0x0403);
    gpu.vram.set(2, 0, 0x0605);
    let mut frame = Vec::new();
    assert_eq!(gpu.render_frame(&mut frame), (512, 480));
    assert_eq!(frame[0], 0xFF03_0201);
    assert_eq!(frame[1], 0xFF03_0201, "repeated");
    assert_eq!(frame[2], 0xFF06_0504);
    assert_eq!(frame[512], 0xFF03_0201, "and the row below");
}

#[test]
fn a_blank_screen_is_black_at_any_scale() {
    let mut gpu = scaled(RenderSettings { scale: 2, ..RenderSettings::default() });
    gpu.vram.set(0, 0, 0x7FFF);
    let mut frame = Vec::new();
    let (w, h) = gpu.render_frame(&mut frame);
    assert_eq!((w, h), (512, 480));
    assert!(frame.iter().all(|p| *p == 0xFF00_0000));
}

#[test]
fn the_scaled_vram_follows_the_console_across_a_reset_and_new_settings() {
    let mut gpu = scaled(RenderSettings { scale: 2, ..RenderSettings::default() });
    gp0(&mut gpu, &[0x0200_00FF, xy(0, 0), (1 << 16) | 16]);
    // Switching the resolution rebuilds the scaled picture from the console VRAM.
    gpu.set_render_settings(RenderSettings { scale: 3, ..RenderSettings::default() });
    let (pixels, stride) = gpu.scaled_vram().unwrap();
    assert_eq!(stride, 3072);
    assert_eq!(pixels[2 * stride + 2] & 0xFFFFFF, 0x0000FF);
    // A reset clears VRAM but keeps the way of rendering.
    gpu.reset();
    assert_eq!(gpu.render_settings().scale, 3);
    assert_eq!(gpu.scaled_vram().unwrap().0[2 * 3072 + 2], 0);
    // Turning everything off drops the scaled VRAM.
    gpu.set_render_settings(RenderSettings::default());
    assert!(gpu.scaled_vram().is_none());
}

#[test]
fn a_saturated_vertex_keeps_its_precise_position_but_a_stale_tag_is_not_trusted() {
    // The GTE clamps to 11 bits: a vertex 3000 pixels to the right shows as 1023.
    assert!(agrees(3000.0, 1023));
    assert!(agrees(-5000.5, -1024));
    assert!(!agrees(3000.0, 500), "a tag from another value");
    assert!(!agrees(100.0, 1023));
    // Inside the range the integer is the precise value rounded down; one pixel of slack.
    assert!(agrees(10.75, 10) && agrees(10.75, 11) && agrees(-0.25, -1));
    assert!(!agrees(10.75, 12));
    assert!(!agrees(f32::NAN, 0));
}

#[test]
fn a_polygon_crossing_the_edge_of_the_screen_keeps_its_shape_with_precise_vertices() {
    // The right corner is really 3000 pixels away; the console's integer vertex says 1023.
    let words = [xy(0, 0), xy(1023, 0), xy(0, 8)];
    let triangle = |right: f32| {
        let mut gpu = scaled(RenderSettings { scale: 2, pgxp: true, ..RenderSettings::default() });
        let tags = [tag(0.0, 0.0, 100.0, words[0]), tag(right, 0.0, 100.0, words[1]), tag(0.0, 8.0, 100.0, words[2])];
        gpu.write(0, 4, 0x2000_00FF);
        for (word, tag) in words.iter().zip(tags) {
            gpu.write_gp0_precise(*word, tag);
        }
        gpu
    };
    // With the saturated integer the triangle is 1023 wide, with the precise one 3000 wide: its
    // slanted edge is far flatter, so a row near the bottom is much wider.
    let (clamped, precise) = (triangle(1023.0), triangle(3000.0));
    let width = |gpu: &Gpu| covered(gpu, 12);
    assert!(width(&precise) > width(&clamped) + 200, "{} <= {}", width(&precise), width(&clamped));
}
