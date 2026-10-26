pub mod memory_card;
pub mod pad;
pub mod sio;

use serde::{Deserialize, Serialize};
use bitflags::bitflags;
use memory_card::{CardSession, MemoryCard};

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

/// Saves a button mask as its bits.
mod buttons_bits {
    use super::Buttons;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(buttons: &Buttons, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u16(buttons.bits())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Buttons, D::Error> {
        Ok(Buttons::from_bits_truncate(u16::deserialize(deserializer)?))
    }
}

/// State of a single controller port.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ControllerState {
    /// Whether a pad is plugged in.
    pub connected: bool,
    #[serde(with = "buttons_bits")]
    pub buttons: Buttons,
    pub left_stick_x: i8,
    pub left_stick_y: i8,
    pub right_stick_x: i8,
    pub right_stick_y: i8,
    /// A DualShock (understands the configuration commands) rather than a
    /// plain digital pad.
    pub dualshock: bool,
    /// The analog mode (the red LED): sticks are reported.
    pub analog_mode: bool,
    /// The game locked the mode, so the Analog button no longer toggles it.
    pub analog_locked: bool,
    /// In the DualShock configuration mode (replies start with `0xF3`).
    pub config_mode: bool,
    /// What each data byte of a poll does to the vibration motors: 0 small
    /// motor, 1 large motor, anything else nothing.
    pub rumble_map: [u8; 6],
    /// Current motor strengths: (small, large).
    pub rumble: [u8; 2],
}

impl Default for ControllerState {
    fn default() -> Self {
        Self {
            connected: false,
            buttons: Buttons::empty(),
            left_stick_x: 0,
            left_stick_y: 0,
            right_stick_x: 0,
            right_stick_y: 0,
            dualshock: true,
            analog_mode: false,
            analog_locked: false,
            config_mode: false,
            rumble_map: [0xFF; 6],
            rumble: [0; 2],
        }
    }
}

/// Where a serial exchange with a port's device currently stands.
#[derive(Serialize, Deserialize)]
enum Session {
    /// Waiting for the address byte that picks pad or memory card.
    Start,
    /// A pad conversation: the command and the next byte of the sequence.
    Pad(pad::PadExchange),
    Card(CardSession),
    /// Nothing is listening until the port is deselected.
    Silent,
}

/// The devices on the two serial ports: pads (up to 8 slots, for multitap)
/// and memory cards.
#[derive(Serialize, Deserialize)]
pub struct ControllerManager {
    pub ports: [ControllerState; 8],
    /// Memory cards in slots 1 and 2.
    pub memory_cards: [Option<MemoryCard>; 2],
    session: Session,
}

impl ControllerManager {
    pub fn new() -> Self {
        let mut ports: [ControllerState; 8] = core::array::from_fn(|_| ControllerState::default());
        // Like a fresh console: a digital pad in slot 1, nothing else.
        ports[0].connected = true;
        Self { ports, memory_cards: [None, None], session: Session::Start }
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

    /// Switch the pad between digital and analog mode, as the Analog button
    /// does. Ignored while the game has locked the mode.
    pub fn set_analog_mode(&mut self, port: usize, analog: bool) {
        if let Some(state) = self.ports.get_mut(port) {
            if !state.analog_locked {
                state.analog_mode = analog;
            }
        }
    }

    /// Plug a pad into a slot or take it out.
    pub fn set_connected(&mut self, port: usize, connected: bool) {
        if let Some(state) = self.ports.get_mut(port) {
            state.connected = connected;
        }
    }

    /// Choose between a DualShock and a plain digital pad.
    pub fn set_dualshock(&mut self, port: usize, dualshock: bool) {
        if let Some(state) = self.ports.get_mut(port) {
            state.dualshock = dualshock;
        }
    }

    /// The strength of the vibration motors the game asked for: (small, large).
    pub fn rumble(&self, port: usize) -> (u8, u8) {
        self.ports.get(port).map_or((0, 0), |p| (p.rumble[0], p.rumble[1]))
    }

    pub fn state(&self, port: usize) -> Option<&ControllerState> {
        self.ports.get(port)
    }

    pub fn insert_memory_card(&mut self, port: usize, card: MemoryCard) {
        if let Some(slot) = self.memory_cards.get_mut(port) {
            *slot = Some(card);
        }
    }

    pub fn remove_memory_card(&mut self, port: usize) -> Option<MemoryCard> {
        self.memory_cards.get_mut(port)?.take()
    }

    pub fn memory_card(&self, port: usize) -> Option<&MemoryCard> {
        self.memory_cards.get(port)?.as_ref()
    }

    pub fn memory_card_mut(&mut self, port: usize) -> Option<&mut MemoryCard> {
        self.memory_cards.get_mut(port)?.as_mut()
    }

    /// The console released /JOYn: the next byte starts a fresh exchange.
    pub fn deselect(&mut self) {
        self.session = Session::Start;
    }

    /// Power the memory cards up again, as a console reset does: they report
    /// a new card until the console writes to them.
    pub fn reset_memory_cards(&mut self) {
        for card in self.memory_cards.iter_mut().flatten() {
            card.reset();
        }
    }

    /// Send one byte to the device on `port` (0 or 1). Returns the byte it
    /// answers and whether it pulsed /ACK, or `None` if nothing answered.
    pub fn exchange(&mut self, port: usize, tx: u8) -> Option<(u8, bool)> {
        match std::mem::replace(&mut self.session, Session::Silent) {
            Session::Start => match tx {
                0x01 if self.ports[port].connected => {
                    self.session = Session::Pad(pad::PadExchange::new());
                    Some((0xFF, true))
                }
                0x81 if self.memory_cards[port].is_some() => {
                    self.session = Session::Card(CardSession::new());
                    Some((0xFF, true))
                }
                _ => None,
            },
            Session::Pad(mut exchange) => {
                let response = self.ports[port].pad_exchange(&mut exchange, tx);
                if !exchange.finished() {
                    self.session = Session::Pad(exchange);
                }
                response
            }
            Session::Card(mut session) => {
                let card = self.memory_cards[port].as_mut()?;
                let response = session.exchange(card, tx);
                if response.is_some() {
                    self.session = Session::Card(session);
                }
                response
            }
            Session::Silent => None,
        }
    }
}

impl Default for ControllerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_console_has_a_pad_in_slot_one_only() {
        let manager = ControllerManager::new();
        assert!(manager.state(0).unwrap().connected);
        assert!(!manager.state(1).unwrap().connected);
        assert!(manager.memory_card(0).is_none());
        assert!(manager.memory_card(1).is_none());
    }

    #[test]
    fn polling_the_pad_follows_the_protocol() {
        let mut manager = ControllerManager::new();
        manager.set_input(0, (Buttons::CROSS | Buttons::LEFT).bits(), 0, 0, 0, 0);
        let replies: Vec<_> = [0x01, 0x42, 0x00, 0x00, 0x00]
            .iter()
            .map(|b| manager.exchange(0, *b))
            .collect();
        assert_eq!(
            replies,
            vec![
                Some((0xFF, true)),
                Some((0x41, true)),
                Some((0x5A, true)),
                Some((!0x80, true)),
                Some((!0x40, false)),
            ]
        );
        // The sequence is over: nothing answers until the port is deselected.
        assert_eq!(manager.exchange(0, 0x00), None);
    }

    #[test]
    fn deselecting_restarts_the_sequence() {
        let mut manager = ControllerManager::new();
        manager.exchange(0, 0x01);
        manager.exchange(0, 0x42);
        manager.deselect();
        assert_eq!(manager.exchange(0, 0x01), Some((0xFF, true)));
        assert_eq!(manager.exchange(0, 0x42), Some((0x41, true)));
    }

    #[test]
    fn a_missing_pad_never_answers() {
        let mut manager = ControllerManager::new();
        assert_eq!(manager.exchange(1, 0x01), None);
        assert_eq!(manager.exchange(1, 0x42), None, "the port stays silent");
        manager.deselect();
        manager.ports[1].connected = true;
        assert_eq!(manager.exchange(1, 0x01), Some((0xFF, true)));
    }

    #[test]
    fn unknown_address_bytes_get_no_answer() {
        let mut manager = ControllerManager::new();
        assert_eq!(manager.exchange(0, 0x55), None);
        assert_eq!(manager.exchange(0, 0x42), None);
    }

    #[test]
    fn memory_cards_answer_the_card_address() {
        let mut manager = ControllerManager::new();
        assert_eq!(manager.exchange(0, 0x81), None, "no card inserted");
        manager.deselect();

        manager.insert_memory_card(0, MemoryCard::formatted());
        assert_eq!(manager.exchange(0, 0x81), Some((0xFF, true)));
        assert_eq!(manager.exchange(0, b'S'), Some((0x08, true)));
        assert_eq!(manager.exchange(0, 0), Some((0x5A, true)));
        // The pad on the same port does not interfere.
        manager.deselect();
        assert_eq!(manager.exchange(0, 0x01), Some((0xFF, true)));
    }

    #[test]
    fn cards_can_be_removed_and_swapped() {
        let mut manager = ControllerManager::new();
        manager.insert_memory_card(1, MemoryCard::blank());
        assert!(manager.memory_card(1).is_some());
        assert!(manager.memory_card_mut(1).is_some());
        assert!(manager.remove_memory_card(1).is_some());
        assert!(manager.memory_card(1).is_none());
        assert!(manager.remove_memory_card(1).is_none());
        assert!(manager.remove_memory_card(5).is_none(), "no such slot");
    }

    #[test]
    fn set_input_truncates_unknown_button_bits_and_ignores_bad_ports() {
        let mut manager = ControllerManager::new();
        manager.set_input(0, 0xFFFF, 1, -2, 3, -4);
        let state = manager.state(0).unwrap();
        assert_eq!(state.buttons.bits(), 0xFFFF);
        assert_eq!((state.left_stick_x, state.left_stick_y), (1, -2));
        assert_eq!((state.right_stick_x, state.right_stick_y), (3, -4));
        manager.set_input(9, 1, 0, 0, 0, 0); // out of range: no panic
        assert!(manager.state(9).is_none());
    }

    #[test]
    fn resetting_the_console_powers_the_cards_up_again() {
        let mut manager = ControllerManager::new();
        manager.insert_memory_card(0, MemoryCard::formatted());
        manager.insert_memory_card(1, MemoryCard::blank());
        let flag = |manager: &mut ControllerManager, port: usize| {
            manager.deselect();
            manager.exchange(port, 0x81);
            manager.exchange(port, b'S').unwrap().0
        };
        // Write a sector to card 1 through the protocol, which clears its "new" flag.
        manager.deselect();
        let data = [7u8; 128];
        let checksum = data.iter().fold(0u8, |a, b| a ^ b);
        let mut bytes = vec![0x81, b'W', 0, 0, 0, 0];
        bytes.extend_from_slice(&data);
        bytes.push(checksum);
        bytes.extend_from_slice(&[0, 0, 0]);
        for byte in bytes {
            manager.exchange(0, byte);
        }
        assert_eq!(flag(&mut manager, 0), 0x00);
        assert_eq!(flag(&mut manager, 1), 0x08);

        manager.reset_memory_cards();
        assert_eq!(flag(&mut manager, 0), 0x08, "card 1 looks new again");
        assert_eq!(manager.memory_card(0).unwrap().data()[..128], data, "with its contents");
    }
}
