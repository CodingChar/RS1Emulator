//! Memory card: 128 KB of flash organised as 1024 frames of 128 bytes, and
//! the serial protocol the console speaks to read and write it.
//!
//! After the address byte `0x81`, each exchange follows this pattern (the
//! card acknowledges every byte except the last):
//!
//! * **Read** `R` — `flag, 5A, 5D, 00, MSB, 5C, 5D, MSB, LSB, 128 data
//!   bytes, checksum, 'G'`.
//! * **Write** `W` — `flag, 5A, 5D, 00, MSB, <128 echoes>, <checksum echo>,
//!   5C, 5D, 'G'` (or `4E` on a bad checksum, `FF` on a bad sector).
//! * **Get ID** `S` — `flag, 5A, 5D, 5C, 5D, 04, 00, 00, 80`.
//!
//! Mirrors DuckStation's `memory_card.cpp`.
use serde::{Deserialize, Serialize};

pub const FRAME_SIZE: usize = 128;
pub const FRAME_COUNT: usize = 1024;
pub const CARD_SIZE: usize = FRAME_SIZE * FRAME_COUNT;

/// The card's status flag at power-on: "directory not read yet".
const FLAG_NEW_CARD: u8 = 0x08;

const WRITE_GOOD: u8 = 0x47; // 'G'
const WRITE_BAD_CHECKSUM: u8 = 0x4E; // 'N'
const WRITE_BAD_SECTOR: u8 = 0xFF;

#[derive(Serialize, Deserialize)]
pub struct MemoryCard {
    data: Vec<u8>,
    flag: u8,
    dirty: bool,
}

impl MemoryCard {
    /// A card of zeros: games will offer to format it.
    pub fn blank() -> Self {
        Self { data: vec![0; CARD_SIZE], flag: FLAG_NEW_CARD, dirty: false }
    }

    /// An empty card with a valid file system, as the BIOS formats it.
    pub fn formatted() -> Self {
        let mut card = Self::blank();
        let xor_checksum = |frame: &[u8]| frame[..FRAME_SIZE - 1].iter().fold(0u8, |a, b| a ^ b);

        // Frame 0: header "MC".
        card.data[0] = b'M';
        card.data[1] = b'C';
        card.data[FRAME_SIZE - 1] = xor_checksum(&card.data[..FRAME_SIZE]);

        // Frames 1-15: directory entries, all free.
        for frame in 1..16 {
            let entry = &mut card.data[frame * FRAME_SIZE..(frame + 1) * FRAME_SIZE];
            entry[0] = 0xA0;
            entry[8] = 0xFF;
            entry[9] = 0xFF;
            entry[FRAME_SIZE - 1] = xor_checksum(&*entry);
        }

        // Frames 16-35: broken sector list, empty.
        for frame in 16..36 {
            let entry = &mut card.data[frame * FRAME_SIZE..(frame + 1) * FRAME_SIZE];
            entry[..4].fill(0xFF);
            entry[8] = 0xFF;
            entry[9] = 0xFF;
            entry[FRAME_SIZE - 1] = xor_checksum(&*entry);
        }

        // Frame 63: write-test frame, a copy of the header.
        let header: Vec<u8> = card.data[..FRAME_SIZE].to_vec();
        card.data[63 * FRAME_SIZE..64 * FRAME_SIZE].copy_from_slice(&header);
        card
    }

    /// Load a raw card image (exactly 128 KB).
    pub fn from_bytes(data: Vec<u8>) -> Option<Self> {
        (data.len() == CARD_SIZE).then(|| Self { data, flag: FLAG_NEW_CARD, dirty: false })
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Whether the console wrote to the card since the flag was last cleared.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn frame(&self, index: usize) -> &[u8] {
        &self.data[index * FRAME_SIZE..(index + 1) * FRAME_SIZE]
    }
}

/// One serial exchange with a card, from the address byte to the last reply.
#[derive(Serialize, Deserialize)]
pub struct CardSession {
    step: usize,
    command: u8,
    sector: usize,
    sector_high: u8,
    previous_tx: u8,
    checksum: u8,
    received_checksum: u8,
    #[serde(with = "crate::serde_arrays")]
    buffer: [u8; FRAME_SIZE],
}

impl CardSession {
    /// A session whose address byte `0x81` was just accepted.
    pub fn new() -> Self {
        Self {
            step: 1,
            command: 0,
            sector: 0,
            sector_high: 0,
            previous_tx: 0,
            checksum: 0,
            received_checksum: 0,
            buffer: [0; FRAME_SIZE],
        }
    }

    /// Process one byte from the console. Returns `(response, acknowledge)`,
    /// or `None` when the card has nothing more to say.
    pub fn exchange(&mut self, card: &mut MemoryCard, tx: u8) -> Option<(u8, bool)> {
        let step = self.step;
        self.step += 1;
        let previous = std::mem::replace(&mut self.previous_tx, tx);

        match step {
            1 => {
                // Unknown commands leave the card silent for the rest of the exchange.
                self.command = if matches!(tx, b'R' | b'W' | b'S') { tx } else { 0 };
                (self.command != 0).then_some((card.flag, true))
            }
            2 if self.command != 0 => Some((0x5A, true)),
            3 if self.command != 0 => Some((0x5D, true)),
            _ => match self.command {
                b'S' => Self::id_step(step),
                b'R' => self.read_step(card, step, tx, previous),
                b'W' => self.write_step(card, step, tx, previous),
                _ => None,
            },
        }
    }

    fn id_step(step: usize) -> Option<(u8, bool)> {
        match step {
            4 => Some((0x5C, true)),
            5 => Some((0x5D, true)),
            6 => Some((0x04, true)),
            7 | 8 => Some((0x00, true)),
            9 => Some((0x80, false)),
            _ => None,
        }
    }

    fn read_step(&mut self, card: &MemoryCard, step: usize, tx: u8, previous: u8) -> Option<(u8, bool)> {
        match step {
            4 => {
                self.sector_high = tx;
                Some((0x00, true))
            }
            5 => {
                self.sector = ((self.sector_high as usize) << 8) | tx as usize;
                Some((previous, true))
            }
            6 => Some((0x5C, true)),
            7 => Some((0x5D, true)),
            8 => {
                if self.sector >= FRAME_COUNT {
                    self.command = 0; // bad sector: the card falls silent
                    return Some((0xFF, false));
                }
                self.buffer.copy_from_slice(card.frame(self.sector));
                self.checksum = self.sector_high ^ (self.sector as u8);
                Some((self.sector_high, true))
            }
            9 => Some((self.sector as u8, true)),
            10..=137 => {
                let byte = self.buffer[step - 10];
                self.checksum ^= byte;
                Some((byte, true))
            }
            138 => Some((self.checksum, true)),
            139 => Some((b'G', false)),
            _ => None,
        }
    }

    fn write_step(&mut self, card: &mut MemoryCard, step: usize, tx: u8, previous: u8) -> Option<(u8, bool)> {
        match step {
            4 => {
                self.sector_high = tx;
                Some((0x00, true))
            }
            5 => {
                self.sector = ((self.sector_high as usize) << 8) | tx as usize;
                self.checksum = self.sector_high ^ tx;
                Some((previous, true))
            }
            6..=133 => {
                self.buffer[step - 6] = tx;
                self.checksum ^= tx;
                Some((previous, true))
            }
            134 => {
                self.received_checksum = tx;
                Some((previous, true))
            }
            135 => Some((0x5C, true)),
            136 => Some((0x5D, true)),
            137 => {
                let status = if self.sector >= FRAME_COUNT {
                    WRITE_BAD_SECTOR
                } else if self.checksum != self.received_checksum {
                    WRITE_BAD_CHECKSUM
                } else {
                    let start = self.sector * FRAME_SIZE;
                    card.data[start..start + FRAME_SIZE].copy_from_slice(&self.buffer);
                    card.flag &= !FLAG_NEW_CARD;
                    card.dirty = true;
                    WRITE_GOOD
                };
                Some((status, false))
            }
            _ => None,
        }
    }
}

impl Default for CardSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a full exchange: the address byte is implicit (the session starts
    /// after it), `bytes` are what the console sends. Returns the responses.
    fn run(card: &mut MemoryCard, bytes: &[u8]) -> Vec<Option<(u8, bool)>> {
        let mut session = CardSession::new();
        bytes.iter().map(|b| session.exchange(card, *b)).collect()
    }

    fn read_command(sector: u16) -> Vec<u8> {
        let mut bytes = vec![b'R', 0, 0, (sector >> 8) as u8, sector as u8];
        bytes.extend(std::iter::repeat(0).take(4 + 128 + 2));
        bytes
    }

    fn write_command(sector: u16, data: &[u8; 128], checksum: u8) -> Vec<u8> {
        let mut bytes = vec![b'W', 0, 0, (sector >> 8) as u8, sector as u8];
        bytes.extend_from_slice(data);
        bytes.push(checksum);
        bytes.extend_from_slice(&[0, 0, 0]);
        bytes
    }

    fn checksum(sector: u16, data: &[u8; 128]) -> u8 {
        data.iter().fold((sector >> 8) as u8 ^ sector as u8, |a, b| a ^ b)
    }

    #[test]
    fn a_blank_card_is_zeros_with_the_new_card_flag() {
        let card = MemoryCard::blank();
        assert_eq!(card.data().len(), CARD_SIZE);
        assert!(card.data().iter().all(|b| *b == 0));
        assert!(!card.is_dirty());
    }

    #[test]
    fn a_formatted_card_has_a_valid_file_system() {
        let card = MemoryCard::formatted();
        let frame_ok = |i: usize| {
            let frame = &card.data()[i * FRAME_SIZE..(i + 1) * FRAME_SIZE];
            frame[..127].iter().fold(0u8, |a, b| a ^ b) == frame[127]
        };
        assert_eq!(&card.data()[..2], b"MC");
        assert!((0..36).all(frame_ok), "every header, directory and bad-sector frame is checksummed");
        assert_eq!(card.data()[FRAME_SIZE], 0xA0, "directory entry 1 is free");
        assert_eq!(&card.data()[63 * FRAME_SIZE..63 * FRAME_SIZE + 2], b"MC");
    }

    #[test]
    fn card_images_must_be_exactly_128_kb() {
        assert!(MemoryCard::from_bytes(vec![0; CARD_SIZE]).is_some());
        assert!(MemoryCard::from_bytes(vec![0; CARD_SIZE - 1]).is_none());
        assert!(MemoryCard::from_bytes(Vec::new()).is_none());
    }

    #[test]
    fn get_id_replies_with_the_card_signature() {
        let mut card = MemoryCard::blank();
        let responses = run(&mut card, &[b'S', 0, 0, 0, 0, 0, 0, 0, 0]);
        let bytes: Vec<u8> = responses.iter().map(|r| r.unwrap().0).collect();
        assert_eq!(bytes, vec![0x08, 0x5A, 0x5D, 0x5C, 0x5D, 0x04, 0x00, 0x00, 0x80]);
        assert!(responses[..8].iter().all(|r| r.unwrap().1), "acknowledged");
        assert!(!responses[8].unwrap().1, "no acknowledge on the last byte");
    }

    #[test]
    fn unknown_commands_get_no_answer() {
        let mut card = MemoryCard::blank();
        let responses = run(&mut card, &[b'X', 0, 0]);
        assert_eq!(responses, vec![None, None, None]);
    }

    #[test]
    fn reading_returns_the_frame_with_its_checksum() {
        let mut card = MemoryCard::formatted();
        let responses = run(&mut card, &read_command(1));
        let bytes: Vec<u8> = responses.iter().map(|r| r.unwrap().0).collect();

        assert_eq!(bytes[0], 0x08, "flag");
        assert_eq!(&bytes[1..9], &[0x5A, 0x5D, 0x00, 0x00, 0x5C, 0x5D, 0x00, 0x01]);
        assert_eq!(&bytes[9..137], card.frame(1));
        assert_eq!(bytes[137], 0x00 ^ 0x01 ^ card.frame(1).iter().fold(0u8, |a, b| a ^ b));
        assert_eq!(bytes[138], b'G');
        assert!(responses[..138].iter().all(|r| r.unwrap().1));
        assert!(!responses[138].unwrap().1);
    }

    #[test]
    fn reading_a_bad_sector_aborts() {
        let mut card = MemoryCard::blank();
        let responses = run(&mut card, &read_command(0x0400));
        assert_eq!(responses[7], Some((0xFF, false)));
        assert!(responses[8..].iter().all(|r| r.is_none()));
    }

    #[test]
    fn writing_stores_the_frame_and_clears_the_new_card_flag() {
        let mut card = MemoryCard::blank();
        let mut data = [0u8; 128];
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = i as u8 ^ 0x5A;
        }
        let sector = 0x0123 & 0x3FF;
        let responses = run(&mut card, &write_command(sector, &data, checksum(sector, &data)));
        let last = responses.last().unwrap().unwrap();

        assert_eq!(last, (b'G', false));
        assert_eq!(card.frame(sector as usize), &data);
        assert!(card.is_dirty());
        // The next read reports a card that has been written.
        let again = run(&mut card, &[b'S', 0, 0]);
        assert_eq!(again[0].unwrap().0, 0x00);
    }

    #[test]
    fn writing_echoes_the_previous_byte() {
        let mut card = MemoryCard::blank();
        let data = [0x11u8; 128];
        let responses = run(&mut card, &write_command(2, &data, checksum(2, &data)));
        let bytes: Vec<u8> = responses.iter().map(|r| r.unwrap().0).collect();
        // flag, 5A, 5D, 00 (MSB), MSB echo, then the LSB echoed when the first
        // data byte arrives, then each data byte echoed by the next one.
        assert_eq!(&bytes[..5], &[0x08, 0x5A, 0x5D, 0x00, 0x00]);
        assert_eq!(bytes[5], 2, "LSB echoed when the first data byte arrives");
        assert_eq!(bytes[6], 0x11);
        assert_eq!(&bytes[bytes.len() - 3..], &[0x5C, 0x5D, b'G']);
    }

    #[test]
    fn a_bad_checksum_rejects_the_write() {
        let mut card = MemoryCard::blank();
        let data = [0x22u8; 128];
        let responses = run(&mut card, &write_command(3, &data, checksum(3, &data) ^ 0xFF));
        assert_eq!(responses.last().unwrap().unwrap().0, 0x4E);
        assert!(card.data().iter().all(|b| *b == 0), "nothing written");
        assert!(!card.is_dirty());
    }

    #[test]
    fn writing_a_bad_sector_is_rejected() {
        let mut card = MemoryCard::blank();
        let data = [0u8; 128];
        let responses = run(&mut card, &write_command(0x0400, &data, checksum(0x0400, &data)));
        assert_eq!(responses.last().unwrap().unwrap().0, 0xFF);
        assert!(!card.is_dirty());
    }

    #[test]
    fn dirty_flag_can_be_cleared() {
        let mut card = MemoryCard::blank();
        let data = [1u8; 128];
        run(&mut card, &write_command(0, &data, checksum(0, &data)));
        assert!(card.is_dirty());
        card.clear_dirty();
        assert!(!card.is_dirty());
    }
}
