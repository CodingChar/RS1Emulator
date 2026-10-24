/// Trait for presenting the emulated GPU framebuffer to the host/window.
///
/// Implementations:
/// - `ps1-wasm`: WebGL2 canvas texture upload.
/// - (future) `ps1-desktop`: OpenGL / Vulkan / Metal.
pub trait GpuBackend {
    fn present(&mut self, rgba: &[u8], width: u32, height: u32);
}
