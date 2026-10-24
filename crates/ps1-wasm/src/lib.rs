use wasm_bindgen::prelude::*;
use console_error_panic_hook::set_once as set_panic_hook;

use ps1_core::disc::{Disc, MemorySource, SectorSource};
use ps1_core::system::System;

/// Size of every PS1 BIOS image.
const BIOS_SIZE: usize = 512 * 1024;

/// Runs when the module loads: better panic messages in the browser console.
#[wasm_bindgen(start)]
pub fn start() {
    set_panic_hook();
}

/// Disc images are matched by file name only, ignoring the folder and case.
fn base_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_ascii_lowercase()
}

/// The public WASM-facing emulator handle.
///
/// Instantiated from JavaScript and driven via `requestAnimationFrame`.
#[wasm_bindgen]
pub struct Ps1 {
    inner: System,
    /// Disc files received so far (`add_disc_file_*`), waiting for `insert_disc_files`.
    disc_files: Vec<(String, Vec<u8>)>,
    /// The file currently being received.
    incoming: Option<(String, Vec<u8>)>,
}

#[wasm_bindgen]
impl Ps1 {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self { inner: System::new(), disc_files: Vec::new(), incoming: None }
    }

    /// Load the PS1 BIOS ROM (required to boot). Called from JS with a Uint8Array.
    pub fn load_bios(&mut self, data: &[u8]) -> Result<(), JsValue> {
        if data.len() != BIOS_SIZE {
            return Err(JsValue::from_str(&format!(
                "a BIOS image is {BIOS_SIZE} bytes, this file has {}",
                data.len()
            )));
        }
        self.inner.load_bios(data).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Insert a game disc image. Called from JS with a Uint8Array.
    pub fn insert_disc(&mut self, data: &[u8]) -> Result<(), JsValue> {
        self.inner.insert_disc(data.to_vec()).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Start receiving a disc file (a `.bin`, `.iso` or `.img`) of `len` bytes.
    /// Send its contents with `add_disc_file_chunk` and close it with
    /// `end_disc_file`; a whole disc never has to exist as one JS array.
    pub fn begin_disc_file(&mut self, name: &str, len: usize) {
        self.end_disc_file();
        self.incoming = Some((name.to_string(), Vec::with_capacity(len)));
    }

    pub fn add_disc_file_chunk(&mut self, chunk: &[u8]) {
        if let Some((_, data)) = self.incoming.as_mut() {
            data.extend_from_slice(chunk);
        }
    }

    pub fn end_disc_file(&mut self) {
        if let Some(file) = self.incoming.take() {
            self.disc_files.push(file);
        }
    }

    /// Forget every disc file received so far (after a failed load).
    pub fn discard_disc_files(&mut self) {
        self.incoming = None;
        self.disc_files.clear();
    }

    /// Insert the disc made of the files received so far. With a CUE sheet the
    /// files it names are used; without one there must be exactly one image.
    pub fn insert_disc_files(&mut self, cue: Option<String>) -> Result<(), JsValue> {
        self.end_disc_file();
        let mut files = std::mem::take(&mut self.disc_files);
        let to_js = |e: ps1_core::Ps1Error| JsValue::from_str(&e.to_string());
        let disc = match cue {
            Some(cue) => Disc::from_cue(&cue, |name| {
                let wanted = base_name(name);
                let position = files
                    .iter()
                    .position(|(file, _)| base_name(file) == wanted)
                    .ok_or_else(|| format!("the file {name} was not provided"))?;
                let (_, data) = files.swap_remove(position);
                Ok(Box::new(MemorySource(data)) as Box<dyn SectorSource>)
            })
            .map_err(to_js)?,
            None => {
                if files.len() != 1 {
                    return Err(JsValue::from_str("select one disc image, or a .cue with its .bin files"));
                }
                let (_, data) = files.remove(0);
                Disc::from_image(data).map_err(to_js)?
            }
        };
        self.inner.insert_disc_image(disc);
        Ok(())
    }

    /// Frames per second of the current video mode (about 60 for NTSC, 50 for PAL).
    pub fn frames_per_second(&self) -> f64 {
        33_868_800.0 / self.inner.bus.gpu.cycles_per_frame() as f64
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

    /// Dimensions of the framebuffer returned by `framebuffer()`. They change
    /// whenever the game switches video mode, so read them every frame.
    pub fn framebuffer_width(&self) -> u32 {
        self.inner.bus.gpu.display_width() as u32
    }

    pub fn framebuffer_height(&self) -> u32 {
        self.inner.bus.gpu.display_height() as u32
    }

    /// Take the audio produced since the last call: interleaved stereo f32
    /// samples in -1.0..1.0 at `audio_sample_rate()`.
    pub fn drain_audio(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.inner.bus.spu.output_buffer)
    }

    pub fn audio_sample_rate(&self) -> u32 {
        self.inner.bus.spu.sample_rate
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
