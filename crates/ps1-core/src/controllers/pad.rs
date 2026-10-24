//! Digital pad protocol.
//!
//! After the console selects a port and sends the address byte `0x01`, the
//! pad answers one byte per byte sent, acknowledging all but the last:
//!
//! | step | console sends | pad answers                      |
//! |------|---------------|----------------------------------|
//! | 0    | `0x01`        | `0xFF` (high impedance)          |
//! | 1    | `0x42` (poll) | `0x41` (digital pad, 1 halfword) |
//! | 2    | `0x00`        | `0x5A`                           |
//! | 3    | `0x00`        | buttons, low byte (0 = pressed)  |
//! | 4    | `0x00`        | buttons, high byte               |
//!
//! Mirrors DuckStation's `digital_controller.cpp`.

use super::Buttons;

/// Identification byte of a digital pad: type 4, one halfword of data.
pub const DIGITAL_PAD_ID: u8 = 0x41;
/// The byte that precedes the button data.
pub const DATA_START: u8 = 0x5A;
/// The last step of the poll sequence.
const LAST_STEP: usize = 4;

/// The pad's answer to the byte at `step` of a poll (step 0, the address
/// byte, is handled by the caller). Returns `(response, acknowledge)`, or
/// `None` once the sequence is over.
pub fn digital_exchange(step: usize, buttons: &Buttons) -> Option<(u8, bool)> {
    // Buttons are reported active-low. A digital pad has no stick buttons,
    // so L3 and R3 (bits 1 and 2) always read as released.
    let released = !buttons.bits() | 0x0006;
    match step {
        1 => Some((DIGITAL_PAD_ID, true)),
        2 => Some((DATA_START, true)),
        3 => Some((released as u8, true)),
        LAST_STEP => Some(((released >> 8) as u8, false)),
        _ => None,
    }
}

/// Whether the poll sequence is complete after `step`.
pub fn is_last_step(step: usize) -> bool {
    step >= LAST_STEP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_sequence_with_nothing_pressed() {
        let buttons = Buttons::empty();
        assert_eq!(digital_exchange(1, &buttons), Some((0x41, true)));
        assert_eq!(digital_exchange(2, &buttons), Some((0x5A, true)));
        assert_eq!(digital_exchange(3, &buttons), Some((0xFF, true)));
        assert_eq!(digital_exchange(4, &buttons), Some((0xFF, false)));
        assert_eq!(digital_exchange(5, &buttons), None);
    }

    #[test]
    fn pressed_buttons_read_as_zero_bits() {
        let buttons = Buttons::CROSS | Buttons::START | Buttons::UP;
        let (low, _) = digital_exchange(3, &buttons).unwrap();
        let (high, _) = digital_exchange(4, &buttons).unwrap();
        assert_eq!(low, !(0x08 | 0x10) & 0xFF);
        assert_eq!(high, !0x40);
    }

    #[test]
    fn stick_buttons_are_not_reported_by_a_digital_pad() {
        let (low, _) = digital_exchange(3, &(Buttons::L3 | Buttons::R3)).unwrap();
        assert_eq!(low, 0xFF);
    }

    #[test]
    fn last_step() {
        assert!(!is_last_step(3));
        assert!(is_last_step(4));
    }
}
