//! Digital pad and DualShock protocol.
//!
//! After the console selects a port and sends the address byte `0x01`, the
//! pad answers one byte per byte sent, acknowledging all but the last. The
//! second byte is the command; a poll (`0x42`) looks like this:
//!
//! | step | console sends | digital pad answers | DualShock (analog) answers |
//! |------|---------------|---------------------|----------------------------|
//! | 0    | `0x01`        | `0xFF`              | `0xFF`                     |
//! | 1    | `0x42`        | `0x41`              | `0x73`                     |
//! | 2    | `0x00`        | `0x5A`              | `0x5A`                     |
//! | 3    | motor 1       | buttons, low byte   | buttons, low byte          |
//! | 4    | motor 2       | buttons, high byte  | buttons, high byte         |
//! | 5-8  | `0x00`        |                     | right X, right Y, left X, left Y |
//!
//! Buttons are active low. A DualShock starts in digital mode; games switch
//! it to analog by entering the *configuration mode* (`0x43`), setting the
//! mode (`0x44`), maybe querying it (`0x45`, `0x46`, `0x47`, `0x4C`), mapping
//! the vibration motors (`0x4D`) and leaving the configuration mode. In
//! configuration mode every reply starts with ID `0xF3`.
//!
//! Mirrors DuckStation's `digital_controller.cpp` and `analog_controller.cpp`.

use serde::{Deserialize, Serialize};

use super::ControllerState;

/// Identification bytes: type in the high nibble, halfwords of data in the low.
pub const ID_DIGITAL: u8 = 0x41;
pub const ID_ANALOG: u8 = 0x73;
pub const ID_CONFIG: u8 = 0xF3;
/// The byte that precedes the button data.
pub const DATA_START: u8 = 0x5A;

const CMD_POLL: u8 = 0x42;
const CMD_CONFIG: u8 = 0x43;
const CMD_SET_MODE: u8 = 0x44;
const CMD_STATUS: u8 = 0x45;
const CMD_CONSTANT_1: u8 = 0x46;
const CMD_CONSTANT_2: u8 = 0x47;
const CMD_CONSTANT_3: u8 = 0x4C;
const CMD_MOTOR_MAP: u8 = 0x4D;

/// Where a pad exchange stands, kept by the controller manager between bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadExchange {
    /// The next byte of the sequence (the address byte is step 0).
    pub step: usize,
    pub command: u8,
    /// Data bytes the reply carries after `0x5A`, fixed when the command
    /// arrives so a mode change inside the command does not alter it.
    pub length: usize,
    /// The first parameter byte of the command (`0x46` and `0x4C` use it).
    pub parameter: u8,
}

impl PadExchange {
    pub fn new() -> Self {
        Self { step: 1, ..Self::default() }
    }

    /// Whether the sequence is over after the byte just exchanged.
    pub fn finished(&self) -> bool {
        self.step > 2 + self.length
    }
}

impl ControllerState {
    /// The ID byte for the pad's current mode.
    fn pad_id(&self) -> u8 {
        if self.config_mode {
            ID_CONFIG
        } else if self.analog_mode {
            ID_ANALOG
        } else {
            ID_DIGITAL
        }
    }

    /// The six input bytes of a poll: buttons (active low), then the sticks.
    fn input_bytes(&self) -> [u8; 6] {
        // Without analog mode L3 and R3 (bits 1 and 2) read as released.
        let mut released = !self.buttons.bits();
        if !(self.analog_mode || self.config_mode) {
            released |= 0x0006;
        }
        let stick = |value: i8| (value as i16 + 128) as u8;
        [
            released as u8,
            (released >> 8) as u8,
            stick(self.right_stick_x),
            stick(self.right_stick_y),
            stick(self.left_stick_x),
            stick(self.left_stick_y),
        ]
    }

    /// Whether the pad understands `command` in its current mode.
    fn pad_understands(&self, command: u8) -> bool {
        match command {
            CMD_POLL => true,
            CMD_CONFIG => self.dualshock,
            CMD_SET_MODE..=CMD_MOTOR_MAP => self.dualshock && self.config_mode,
            _ => false,
        }
    }

    /// Answer the byte `tx` sent at the exchange's current step. Returns the
    /// pad's byte and whether it pulsed /ACK, or `None` when it is done.
    pub fn pad_exchange(&mut self, exchange: &mut PadExchange, tx: u8) -> Option<(u8, bool)> {
        let step = exchange.step;
        exchange.step += 1;
        match step {
            1 => {
                exchange.command = tx;
                exchange.length = if self.config_mode || self.analog_mode { 6 } else { 2 };
                // An unknown command is answered with the ID and nothing more.
                if !self.pad_understands(tx) {
                    exchange.length = 0;
                    exchange.step = 3;
                    return Some((self.pad_id(), false));
                }
                Some((self.pad_id(), true))
            }
            2 => Some((DATA_START, exchange.length > 0)),
            _ if step > 2 + exchange.length => None,
            _ => {
                let index = step - 3;
                let response = self.pad_data(exchange, index, tx);
                Some((response, index + 1 < exchange.length))
            }
        }
    }

    /// Data byte `index` of the reply, given the byte `tx` received with it.
    fn pad_data(&mut self, exchange: &mut PadExchange, index: usize, tx: u8) -> u8 {
        if index == 0 {
            exchange.parameter = tx;
        }
        match exchange.command {
            CMD_POLL => {
                // The motor bytes follow the pad's own mapping.
                match self.rumble_map[index.min(5)] {
                    0 => self.rumble[0] = if tx & 1 != 0 { 0xFF } else { 0 },
                    1 => self.rumble[1] = tx,
                    _ => {}
                }
                self.input_bytes()[index]
            }
            CMD_CONFIG => {
                if index == 0 {
                    // 1 enters the configuration mode, 0 leaves it.
                    self.config_mode = tx == 1;
                }
                self.input_bytes()[index]
            }
            CMD_SET_MODE => {
                match index {
                    0 if !self.analog_locked => self.analog_mode = tx == 1,
                    1 => self.analog_locked = tx == 3,
                    _ => {}
                }
                0
            }
            CMD_STATUS => [0x01, 0x02, self.analog_mode as u8, 0x02, 0x01, 0x00][index],
            CMD_CONSTANT_1 => {
                if exchange.parameter == 0 {
                    [0x00, 0x00, 0x01, 0x02, 0x00, 0x0A][index]
                } else {
                    [0x00, 0x00, 0x01, 0x01, 0x01, 0x14][index]
                }
            }
            CMD_CONSTANT_2 => [0x00, 0x00, 0x02, 0x00, 0x01, 0x00][index],
            CMD_CONSTANT_3 => {
                if exchange.parameter == 0 {
                    [0x00, 0x00, 0x00, 0x04, 0x00, 0x00][index]
                } else {
                    [0x00, 0x00, 0x00, 0x07, 0x00, 0x00][index]
                }
            }
            CMD_MOTOR_MAP => {
                let previous = self.rumble_map[index];
                self.rumble_map[index] = tx;
                previous
            }
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Buttons, ControllerManager};

    /// Send `bytes` to port 0 as one exchange and collect the replies.
    fn exchange(manager: &mut ControllerManager, bytes: &[u8]) -> Vec<Option<(u8, bool)>> {
        manager.deselect();
        bytes.iter().map(|b| manager.exchange(0, *b)).collect()
    }

    fn reply_bytes(replies: &[Option<(u8, bool)>]) -> Vec<u8> {
        replies.iter().map(|r| r.map_or(0xEE, |(byte, _)| byte)).collect()
    }

    fn enter_config(manager: &mut ControllerManager) {
        exchange(manager, &[0x01, 0x43, 0x00, 0x01, 0x00]);
    }

    fn exit_config(manager: &mut ControllerManager) {
        exchange(manager, &[0x01, 0x43, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn a_digital_poll_has_two_data_bytes() {
        let mut manager = ControllerManager::new();
        manager.set_input(0, (Buttons::CROSS | Buttons::LEFT).bits(), 10, 20, 30, 40);
        let replies = exchange(&mut manager, &[0x01, 0x42, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(
            replies,
            vec![
                Some((0xFF, true)),
                Some((0x41, true)),
                Some((0x5A, true)),
                Some((!0x80, true)),
                Some((!0x40, false)),
                None,
            ]
        );
    }

    #[test]
    fn stick_buttons_are_hidden_from_a_digital_pad() {
        let mut manager = ControllerManager::new();
        manager.set_input(0, (Buttons::L3 | Buttons::R3).bits(), 0, 0, 0, 0);
        let replies = reply_bytes(&exchange(&mut manager, &[0x01, 0x42, 0x00, 0x00, 0x00]));
        assert_eq!(replies[3], 0xFF);
    }

    #[test]
    fn config_mode_enter_set_analog_exit_switches_the_pad_to_analog() {
        let mut manager = ControllerManager::new();
        enter_config(&mut manager);
        // In configuration mode the ID is F3 and replies are six bytes long.
        let replies = exchange(&mut manager, &[0x01, 0x44, 0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(reply_bytes(&replies), vec![0xFF, 0xF3, 0x5A, 0, 0, 0, 0, 0, 0]);
        exit_config(&mut manager);

        manager.set_input(0, Buttons::START.bits(), -128, 127, 0, 1);
        let replies = exchange(&mut manager, &[0x01, 0x42, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            reply_bytes(&replies),
            vec![0xFF, 0x73, 0x5A, !0x08, 0xFF, 0x80, 0x81, 0x00, 0xFF],
            "right X/Y then left X/Y, 0x80 at rest"
        );
        // Only the last byte is not acknowledged.
        assert_eq!(replies[7], Some((0x00, true)));
        assert_eq!(replies[8], Some((0xFF, false)));
    }

    #[test]
    fn a_locked_mode_ignores_further_changes() {
        let mut manager = ControllerManager::new();
        enter_config(&mut manager);
        exchange(&mut manager, &[0x01, 0x44, 0x00, 0x01, 0x03, 0, 0, 0, 0]);
        exchange(&mut manager, &[0x01, 0x44, 0x00, 0x00, 0x00, 0, 0, 0, 0]);
        exit_config(&mut manager);
        let replies = reply_bytes(&exchange(&mut manager, &[0x01, 0x42, 0]));
        assert_eq!(replies[1], 0x73);
    }

    #[test]
    fn the_analog_led_can_be_toggled_by_the_frontend_unless_locked() {
        let mut manager = ControllerManager::new();
        manager.set_analog_mode(0, true);
        assert_eq!(reply_bytes(&exchange(&mut manager, &[0x01, 0x42, 0]))[1], 0x73);
        manager.set_analog_mode(0, false);
        assert_eq!(reply_bytes(&exchange(&mut manager, &[0x01, 0x42, 0]))[1], 0x41);
    }

    #[test]
    fn analog_mode_reports_the_stick_buttons() {
        let mut manager = ControllerManager::new();
        manager.set_analog_mode(0, true);
        manager.set_input(0, Buttons::L3.bits(), 0, 0, 0, 0);
        let bytes = reply_bytes(&exchange(&mut manager, &[0x01, 0x42, 0, 0]));
        assert_eq!(bytes[3], !0x02);
    }

    #[test]
    fn config_queries_return_the_dualshock_constants() {
        let mut manager = ControllerManager::new();
        enter_config(&mut manager);
        let zeros = [0u8; 9];
        let mut query = |command: u8, parameter: u8| {
            let mut bytes = zeros;
            bytes[0] = 0x01;
            bytes[1] = command;
            bytes[3] = parameter;
            reply_bytes(&exchange(&mut manager, &bytes))[3..].to_vec()
        };
        assert_eq!(query(0x45, 0), vec![0x01, 0x02, 0x00, 0x02, 0x01, 0x00]);
        assert_eq!(query(0x46, 0), vec![0x00, 0x00, 0x01, 0x02, 0x00, 0x0A]);
        assert_eq!(query(0x46, 1), vec![0x00, 0x00, 0x01, 0x01, 0x01, 0x14]);
        assert_eq!(query(0x47, 0), vec![0x00, 0x00, 0x02, 0x00, 0x01, 0x00]);
        assert_eq!(query(0x4C, 0), vec![0x00, 0x00, 0x00, 0x04, 0x00, 0x00]);
        assert_eq!(query(0x4C, 1), vec![0x00, 0x00, 0x00, 0x07, 0x00, 0x00]);
    }

    #[test]
    fn config_commands_are_ignored_outside_config_mode() {
        let mut manager = ControllerManager::new();
        let replies = exchange(&mut manager, &[0x01, 0x45, 0x00, 0x00]);
        assert_eq!(replies[1], Some((0x41, false)), "ID only, no acknowledge");
        assert_eq!(replies[2], None);
    }

    #[test]
    fn a_digital_only_pad_does_not_enter_config_mode() {
        let mut manager = ControllerManager::new();
        manager.ports[0].dualshock = false;
        let replies = exchange(&mut manager, &[0x01, 0x43, 0x00, 0x01, 0x00]);
        assert_eq!(replies[2], None);
        let poll = reply_bytes(&exchange(&mut manager, &[0x01, 0x42, 0]));
        assert_eq!(poll[1], 0x41);
    }

    #[test]
    fn the_vibration_motors_follow_the_mapping_the_game_sets() {
        let mut manager = ControllerManager::new();
        // Before mapping, poll bytes drive nothing.
        exchange(&mut manager, &[0x01, 0x42, 0x00, 0x01, 0x80]);
        assert_eq!(manager.rumble(0), (0, 0));

        // Map byte 0 to the small motor and byte 1 to the large one.
        enter_config(&mut manager);
        let replies = exchange(&mut manager, &[0x01, 0x4D, 0x00, 0x00, 0x01, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(reply_bytes(&replies)[3..], [0xFF; 6], "the previous mapping: none");
        exit_config(&mut manager);

        // A DualShock in analog mode polls 9 bytes; the motor bytes are 3 and 4.
        manager.set_analog_mode(0, true);
        exchange(&mut manager, &[0x01, 0x42, 0x00, 0x01, 0x80, 0, 0, 0, 0]);
        assert_eq!(manager.rumble(0), (0xFF, 0x80));
        exchange(&mut manager, &[0x01, 0x42, 0x00, 0x00, 0x00, 0, 0, 0, 0]);
        assert_eq!(manager.rumble(0), (0, 0));
    }
}
