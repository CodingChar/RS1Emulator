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

/// A disc image file that is read on demand through a JavaScript function
/// `read(offset, length) -> Uint8Array`, so it never has to fit in memory.
struct JsSource {
    read: js_sys::Function,
    len: u64,
}

impl SectorSource for JsSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        let result = self.read.call2(
            &JsValue::NULL,
            &JsValue::from_f64(offset as f64),
            &JsValue::from_f64(buf.len() as f64),
        );
        match result {
            Ok(value) => {
                let bytes = js_sys::Uint8Array::new(&value);
                if bytes.length() as usize != buf.len() {
                    return false;
                }
                bytes.copy_to(buf);
                true
            }
            Err(_) => false,
        }
    }
}

/// A disc file waiting to become (part of) a disc.
enum DiscFile {
    Memory(Vec<u8>),
    Lazy(JsSource),
}

impl DiscFile {
    fn into_source(self) -> Box<dyn SectorSource> {
        match self {
            DiscFile::Memory(data) => Box::new(MemorySource(data)),
            DiscFile::Lazy(source) => Box::new(source),
        }
    }
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
    disc_files: Vec<(String, DiscFile)>,
    /// The file currently being received.
    incoming: Option<(String, Vec<u8>)>,
    /// A CloneCD control file (`.ccd`) that describes the image received, if any.
    ccd: Option<String>,
    /// Replacement subchannel data (`.sbi`) for the next disc inserted, if any.
    sbi: Option<Vec<u8>>,
    /// The last rendered frame (RGBA8888), see `render_frame`.
    frame: Vec<u8>,
}

#[wasm_bindgen]
impl Ps1 {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self { inner: System::new(), disc_files: Vec::new(), incoming: None, ccd: None, sbi: None, frame: Vec::new() }
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
        if let Some((name, data)) = self.incoming.take() {
            self.disc_files.push((name, DiscFile::Memory(data)));
        }
    }

    /// Add a disc file of `len` bytes that is read on demand by calling
    /// `read(offset, length)`, which must return a `Uint8Array` of exactly
    /// that length. Used from a worker with `FileReaderSync`, so a disc of
    /// hundreds of megabytes costs no memory.
    pub fn add_lazy_disc_file(&mut self, name: &str, len: f64, read: js_sys::Function) {
        self.end_disc_file();
        self.disc_files.push((name.to_string(), DiscFile::Lazy(JsSource { read, len: len as u64 })));
    }

    /// The text of a CloneCD `.ccd` file: the disc is then the one image received.
    pub fn set_disc_ccd(&mut self, text: String) {
        self.ccd = Some(text);
    }

    /// The contents of an `.sbi` file, applied to the disc inserted next.
    pub fn set_disc_sbi(&mut self, data: &[u8]) {
        self.sbi = Some(data.to_vec());
    }

    /// Forget every disc file received so far (after a failed load).
    pub fn discard_disc_files(&mut self) {
        self.incoming = None;
        self.disc_files.clear();
        self.ccd = None;
        self.sbi = None;
    }

    /// Insert the disc made of the files received so far. With a CUE sheet the
    /// files it names are used; without one there must be exactly one image.
    pub fn insert_disc_files(&mut self, cue: Option<String>) -> Result<(), JsValue> {
        let disc = self.take_disc(cue)?;
        self.inner.insert_disc_image(disc);
        Ok(())
    }

    /// Like `insert_disc_files`, for a console that is running: the drive
    /// reports the lid as opened and the game decides what to do.
    pub fn swap_disc_files(&mut self, cue: Option<String>) -> Result<(), JsValue> {
        let disc = self.take_disc(cue)?;
        self.inner.swap_disc_image(disc);
        Ok(())
    }

    fn take_disc(&mut self, cue: Option<String>) -> Result<Disc, JsValue> {
        self.end_disc_file();
        let mut files = std::mem::take(&mut self.disc_files);
        let to_js = |e: ps1_core::Ps1Error| JsValue::from_str(&e.to_string());
        let ccd = self.ccd.take();
        let sbi = self.sbi.take();
        let mut disc = match (ccd, cue) {
            (Some(ccd), _) => {
                if files.len() != 1 {
                    return Err(JsValue::from_str("a .ccd goes with exactly one .img file"));
                }
                let (_, file) = files.remove(0);
                Disc::from_ccd(&ccd, file.into_source()).map_err(to_js)?
            }
            (None, cue) => match cue {
            Some(cue) => Disc::from_cue(&cue, |name| {
                let wanted = base_name(name);
                let position = files
                    .iter()
                    .position(|(file, _)| base_name(file) == wanted)
                    .ok_or_else(|| format!("the file {name} was not provided"))?;
                let (_, file) = files.swap_remove(position);
                Ok(file.into_source())
            })
            .map_err(to_js)?,
            None => {
                if files.len() != 1 {
                    return Err(JsValue::from_str("select one disc image, or a .cue with its .bin files"));
                }
                let (_, file) = files.remove(0);
                Disc::from_image_source(file.into_source()).map_err(to_js)?
            }
            },
        };
        if let Some(sbi) = sbi {
            disc.load_sbi(&sbi).map_err(to_js)?;
        }
        Ok(disc)
    }

    /// The bytes the console sent through its serial port (SIO1) since the last call.
    pub fn take_serial_output(&mut self) -> Vec<u8> {
        self.inner.take_serial_output()
    }

    /// A byte arriving at the serial port from the other end of the cable.
    pub fn serial_input(&mut self, byte: u8) {
        self.inner.serial_input(byte);
    }

    /// A line describing the loaded BIOS (model or version, region, checksum).
    pub fn bios_description(&self) -> String {
        self.inner.bios_info().describe()
    }

    /// Queue a PS-X EXE program to run instead of the BIOS shell (the BIOS
    /// boots first; call `reset` afterwards to start).
    pub fn load_exe(&mut self, data: &[u8]) -> Result<(), JsValue> {
        self.inner.load_exe(data).map_err(|e| JsValue::from_str(&e.to_string()))
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

    /// Render the display into a buffer inside the module, without copying it
    /// out: read it with `frame_ptr()` / `frame_len()` as a view of the
    /// module's memory (valid until the next call that can grow the memory).
    pub fn render_frame(&mut self) {
        self.inner.bus.gpu.render_into(&mut self.frame);
    }

    pub fn frame_ptr(&self) -> *const u8 {
        self.frame.as_ptr()
    }

    pub fn frame_len(&self) -> usize {
        self.frame.len()
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

    /// Switch a pad between digital and analog mode, like the Analog button.
    pub fn set_analog_mode(&mut self, port: u32, analog: bool) {
        self.inner.bus.controllers.set_analog_mode(port as usize, analog);
    }

    /// Plug a pad into a slot (slot 0 is plugged in from the start) or take it out.
    pub fn set_pad_connected(&mut self, port: u32, connected: bool) {
        self.inner.bus.controllers.set_connected(port as usize, connected);
    }

    /// What the game asks the vibration motors to do: the small motor
    /// (0 or 255) in bits 8-15 and the large motor (0-255) in bits 0-7.
    pub fn rumble(&self, port: u32) -> u32 {
        let (small, large) = self.inner.bus.controllers.rumble(port as usize);
        ((small as u32) << 8) | large as u32
    }

    /// Save state as bytes (Uint8Array from JS).
    pub fn save_state(&mut self) -> Vec<u8> {
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
