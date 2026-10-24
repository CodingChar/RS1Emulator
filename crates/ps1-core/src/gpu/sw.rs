/// Software rasterizer for the PS1 GPU.
///
/// Mirrors DuckStation's `gpu_sw.cpp` / `gpu_sw_rasterizer.cpp`. Draws
/// polygons, sprites, and textured quads into the framebuffer.
///
/// This is a stub; fill in the rasterizer pipeline as the project matures.
pub struct SwRasterizer {
    pub width: u32,
    pub height: u32,
}

impl SwRasterizer {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn clear(&self, _fb: &mut [u8], _r: u8, _g: u8, _b: u8) {
        // TODO: implement framebuffer clear.
    }

    pub fn draw_triangle(&self, _fb: &mut [u8], _x1: i32, _y1: i32, _x2: i32, _y2: i32, _x3: i32, _y3: i32) {
        // TODO: implement triangle rasterization.
    }
}
