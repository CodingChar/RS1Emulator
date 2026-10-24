use wasm_bindgen::prelude::*;
use console_error_panic_hook::set_once as set_panic_hook;

use ps1_core::system::System;

/// Set up better panic messages when Rust panics inside WASM.
/// Call this once from the host page (or from ps1-web's #[start]).
pub fn init_panic_hook() {
    set_panic_hook();
}

/// The public WASM-facing emulator handle.
///
/// Instantiated from JavaScript and driven via `requestAnimationFrame`.
#[wasm_bindgen]
pub struct Ps1 {
    inner: System,
}

#[wasm_bindgen]
impl Ps1 {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self { inner: System::new() }
    }

    /// Load the PS1 BIOS ROM (required to boot). Called from JS with a Uint8Array.
    pub fn load_bios(&mut self, data: &[u8]) -> Result<(), JsValue> {
        self.inner.load_bios(data).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Insert a game disc image. Called from JS with a Uint8Array.
    pub fn insert_disc(&mut self, data: &[u8]) -> Result<(), JsValue> {
        self.inner.insert_disc(data.to_vec()).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Power on / reset the system.
    pub fn reset(&mut self) -> Result<(), JsValue> {
        self.inner.reset().map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Advance emulation by one video frame.
    pub fn run_frame(&mut self) -> Result<(), JsValue> {
        self.inner.run_frame().map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Copy the GPU framebuffer (RGBA8888) as a Vec<u8>.
    pub fn framebuffer(&self) -> Vec<u8> {
        self.inner.framebuffer()
    }

    /// Framebuffer dimensions.
    pub fn framebuffer_width(&self) -> u32 {
        ps1_core::gpu::FRAMEBUFFER_WIDTH
    }
    pub fn framebuffer_height(&self) -> u32 {
        ps1_core::gpu::FRAMEBUFFER_HEIGHT
    }

    /// Set controller input for a port.
    pub fn set_input(
        &mut self,
        port: u32,
        buttons: u16,
        left_stick_x: i8,
        left_stick_y: i8,
        right_stick_x: i8,
        right_stick_y: i8,
    ) {
        self.inner.set_input(
            port as usize,
            buttons,
            left_stick_x,
            left_stick_y,
            right_stick_x,
            right_stick_y,
        );
    }

    /// Save state as bytes (Uint8Array from JS).
    pub fn save_state(&self) -> Vec<u8> {
        self.inner.save_state()
    }

    /// Load state from bytes (Uint8Array from JS).
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), JsValue> {
        self.inner.load_state(data).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Plug a 128 KB memory card image into slot 0 or 1.
    pub fn insert_memory_card(&mut self, port: u32, data: &[u8]) -> Result<(), JsValue> {
        self.inner
            .insert_memory_card(port as usize, data.to_vec())
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Plug a freshly formatted, empty memory card into slot 0 or 1.
    pub fn insert_formatted_memory_card(&mut self, port: u32) {
        self.inner.insert_formatted_memory_card(port as usize);
    }

    /// The card's contents (Uint8Array), or undefined if the slot is empty.
    pub fn memory_card_data(&self, port: u32) -> Option<Vec<u8>> {
        self.inner.memory_card_data(port as usize)
    }

    /// Whether the game wrote to the card since the last `clear_memory_card_dirty`.
    pub fn memory_card_dirty(&self, port: u32) -> bool {
        self.inner.memory_card_dirty(port as usize)
    }

    pub fn clear_memory_card_dirty(&mut self, port: u32) {
        self.inner.clear_memory_card_dirty(port as usize);
    }

    /// Whether the system is running.
    pub fn is_running(&self) -> bool {
        self.inner.running
    }
}
