/// Digital / analog pad handling.
///
/// Separated from `ControllerManager` for clarity; maps raw button/axis
/// input into the PS1 controller protocol (command 0x01 / 0x42 / 0x43).
///
/// Mirrors DuckStation's `digital_controller.cpp` / `analog_controller.cpp`.
pub struct Pad {
    pub port: usize,
    pub mode: u8, // 0x01 digital, 0x73 analog with vibration, etc.
}

impl Pad {
    pub fn new(port: usize) -> Self {
        Self { port, mode: 0x01 }
    }

    /// Respond to a controller command byte from the console.
    pub fn handle_command(&mut self, _cmd: u8) -> u8 {
        // TODO: implement pad protocol (ack, data bytes, mode switching).
        0xFF
    }
}
