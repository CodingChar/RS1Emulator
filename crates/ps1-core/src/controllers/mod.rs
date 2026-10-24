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
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ControllerState {
    /// Whether a pad is plugged in.
    pub connected: bool,
    #[serde(with = "buttons_bits")]
    pub buttons: Buttons,
    pub left_stick_x: i8,
    pub left_stick_y: i8,
    pub right_stick_x: i8,
    pub right_stick_y: i8,
}

/// Where a serial exchange with a port's device currently stands.
#[derive(Serialize, Deserialize)]
enum Session {
    /// Waiting for the address byte that picks pad or memory card.
    Start,
    /// A digital pad poll; `step` is the next byte of the sequence.
    Pad { step: usize },
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

    /// Send one byte to the device on `port` (0 or 1). Returns the byte it
    /// answers and whether it pulsed /ACK, or `None` if nothing answered.
    pub fn exchange(&mut self, port: usize, tx: u8) -> Option<(u8, bool)> {
        match std::mem::replace(&mut self.session, Session::Silent) {
            Session::Start => match tx {
                0x01 if self.ports[port].connected => {
                    self.session = Session::Pad { step: 1 };
                    Some((0xFF, true))
                }
                0x81 if self.memory_cards[port].is_some() => {
                    self.session = Session::Card(CardSession::new());
                    Some((0xFF, true))
                }
                _ => None,
            },
            Session::Pad { step } => {
                let response = pad::digital_exchange(step, &self.ports[port].buttons);
                if !pad::is_last_step(step) {
                    self.session = Session::Pad { step: step + 1 };
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
}
