pub mod pad;

use bitflags::bitflags;

bitflags! {
    /// PS1 controller button mask.
    #[derive(Clone, Debug, Default)]
    pub struct Buttons: u16 {
        const SELECT   = 0x0001;
        const L3       = 0x0002;
        const R3       = 0x0004;
        const START    = 0x0008;
        const UP       = 0x0010;
        const RIGHT    = 0x0020;
        const DOWN     = 0x0040;
        const LEFT     = 0x0080;
        const L2       = 0x0100;
        const R2       = 0x0200;
        const L1       = 0x0400;
        const R1       = 0x0800;
        const TRIANGLE = 0x1000;
        const CIRCLE   = 0x2000;
        const CROSS    = 0x4000;
        const SQUARE   = 0x8000;
    }
}

/// State of a single controller port.
#[derive(Clone, Debug, Default)]
pub struct ControllerState {
    pub buttons: Buttons,
    pub left_stick_x: i8,
    pub left_stick_y: i8,
    pub right_stick_x: i8,
    pub right_stick_y: i8,
}

/// Manages up to 8 controller ports (multitap support).
pub struct ControllerManager {
    pub ports: [ControllerState; 8],
}

impl ControllerManager {
    pub fn new() -> Self {
        Self {
            ports: core::array::from_fn(|_| ControllerState::default()),
        }
    }

    pub fn set_input(
        &mut self,
        port: usize,
        buttons: u16,
        left_stick_x: i8,
        left_stick_y: i8,
        right_stick_x: i8,
        right_stick_y: i8,
    ) {
        if port < self.ports.len() {
            let st = &mut self.ports[port];
            st.buttons = Buttons::from_bits_truncate(buttons);
            st.left_stick_x = left_stick_x;
            st.left_stick_y = left_stick_y;
            st.right_stick_x = right_stick_x;
            st.right_stick_y = right_stick_y;
        }
    }

    pub fn state(&self, port: usize) -> Option<&ControllerState> {
        self.ports.get(port)
    }
}

impl Default for ControllerManager {
    fn default() -> Self {
        Self::new()
    }
}
