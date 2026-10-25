//! Serial I/O port 1 (SIO1): the console's RS-232 link at `0x1F801050`.
//!
//! Nothing is plugged into it here, but the registers behave: a byte written to
//! `SIO_TX_DATA` leaves through the port (the host collects it with
//! [`Sio1::take_output`], which is how a debug BIOS's TTY text can be read) and
//! bytes the host feeds in with [`Sio1::push_input`] arrive in the 8-byte receive
//! FIFO. Transfers are instant: the transmitter is always ready.
//!
//! | offset | name      | meaning                                         |
//! |--------|-----------|-------------------------------------------------|
//! | `+0`   | SIO_DATA  | read: received byte; write: byte to send        |
//! | `+4`   | SIO_STAT  | status flags                                    |
//! | `+8`   | SIO_MODE  | baud multiplier, character length, parity       |
//! | `+A`   | SIO_CTRL  | enables, handshake lines, interrupt enables     |
//! | `+C`   | SIO_MISC  | (undocumented, read back)                       |
//! | `+E`   | SIO_BAUD  | baud rate reload value                          |

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use crate::io::{extract, merge};

// ----- SIO_STAT bits -----
const STAT_TX_READY: u32 = 1 << 0;
const STAT_RX_NOT_EMPTY: u32 = 1 << 1;
const STAT_TX_IDLE: u32 = 1 << 2;
const STAT_RX_OVERRUN: u32 = 1 << 4;
const STAT_IRQ: u32 = 1 << 9;

// ----- SIO_CTRL bits -----
const CTRL_TX_ENABLE: u16 = 1 << 0;
const CTRL_RX_ENABLE: u16 = 1 << 2;
/// Write-only strobe: clears the IRQ flag and the error bits.
const CTRL_ACKNOWLEDGE: u16 = 1 << 4;
/// Write-only strobe: resets the port.
const CTRL_RESET: u16 = 1 << 6;
const CTRL_RX_INT_MODE_SHIFT: u16 = 8;
const CTRL_TX_INT_ENABLE: u16 = 1 << 10;
const CTRL_RX_INT_ENABLE: u16 = 1 << 11;
const CTRL_STROBES: u16 = CTRL_ACKNOWLEDGE | CTRL_RESET;

const RX_FIFO_SIZE: usize = 8;
/// The host's view of what was sent stops growing here.
const OUTPUT_LIMIT: usize = 1 << 20;

#[derive(Serialize, Deserialize)]
pub struct Sio1 {
    mode: u16,
    control: u16,
    misc: u16,
    baud: u16,
    rx_fifo: VecDeque<u8>,
    /// SIO_STAT bit 4: a byte arrived with the FIFO full.
    overrun: bool,
    /// SIO_STAT bit 9: an interrupt condition occurred and was not acknowledged.
    irq_flag: bool,
    /// An edge for the interrupt controller to pick up.
    irq: bool,
    /// Bytes the console sent, for the host.
    #[serde(skip)]
    output: Vec<u8>,
}

impl Sio1 {
    pub fn new() -> Self {
        Self {
            mode: 0,
            control: 0,
            misc: 0,
            baud: 0,
            rx_fifo: VecDeque::with_capacity(RX_FIFO_SIZE),
            overrun: false,
            irq_flag: false,
            irq: false,
            output: Vec::new(),
        }
    }

    /// Power-on state; what the console already sent stays available to the host.
    pub fn reset(&mut self) {
        let output = std::mem::take(&mut self.output);
        *self = Self::new();
        self.output = output;
    }

    /// Whether IRQ8 was raised since the last call.
    pub fn take_irq(&mut self) -> bool {
        std::mem::take(&mut self.irq)
    }

    /// The bytes the console has sent since the last call.
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }

    /// A byte arriving from the other end of the cable.
    pub fn push_input(&mut self, byte: u8) {
        if self.control & CTRL_RX_ENABLE == 0 {
            return;
        }
        if self.rx_fifo.len() >= RX_FIFO_SIZE {
            self.overrun = true;
            return;
        }
        self.rx_fifo.push_back(byte);
        // The interrupt fires once the FIFO holds 1, 2, 4 or 8 bytes.
        let threshold = 1usize << ((self.control >> CTRL_RX_INT_MODE_SHIFT) & 3);
        if self.control & CTRL_RX_INT_ENABLE != 0 && self.rx_fifo.len() >= threshold {
            self.raise_irq();
        }
    }

    fn raise_irq(&mut self) {
        if !self.irq_flag {
            self.irq = true;
        }
        self.irq_flag = true;
    }

    fn status(&self) -> u32 {
        let mut stat = STAT_TX_READY | STAT_TX_IDLE;
        if !self.rx_fifo.is_empty() {
            stat |= STAT_RX_NOT_EMPTY;
        }
        if self.overrun {
            stat |= STAT_RX_OVERRUN;
        }
        if self.irq_flag {
            stat |= STAT_IRQ;
        }
        stat
    }

    /// Register read; `off` is relative to `0x1F801050`.
    pub fn read(&mut self, off: u32, width: u32) -> u32 {
        let word = match off & 0xC {
            0 => {
                // A wider read takes the following bytes of the FIFO too.
                let mut word = 0;
                for i in 0..width.min(4) {
                    word |= (self.rx_fifo.pop_front().unwrap_or(0) as u32) << (8 * i);
                }
                return word;
            }
            4 => self.status(),
            8 => self.mode as u32 | (((self.control & !CTRL_STROBES) as u32) << 16),
            _ => self.misc as u32 | ((self.baud as u32) << 16),
        };
        extract(word, off, width)
    }

    /// Register write; `off` is relative to `0x1F801050`.
    pub fn write(&mut self, off: u32, width: u32, value: u32) {
        match off & 0xC {
            0 => {
                if self.control & CTRL_TX_ENABLE != 0 {
                    for i in 0..width.min(4) {
                        if self.output.len() < OUTPUT_LIMIT {
                            self.output.push((value >> (8 * i)) as u8);
                        }
                    }
                    if self.control & CTRL_TX_INT_ENABLE != 0 {
                        self.raise_irq();
                    }
                }
            }
            4 => {}
            8 => {
                let before = self.mode as u32 | ((self.control as u32) << 16);
                let word = merge(before, off, width, value);
                self.mode = word as u16;
                // Only a write that reaches the control half changes it.
                if (off & 3) + width.min(4) > 2 {
                    self.write_control((word >> 16) as u16);
                }
            }
            _ => {
                let word = merge(self.misc as u32 | ((self.baud as u32) << 16), off, width, value);
                self.misc = word as u16;
                self.baud = (word >> 16) as u16;
            }
        }
    }

    /// Store SIO_CTRL; the acknowledge and reset bits act instead of being kept.
    fn write_control(&mut self, value: u16) {
        if value & CTRL_RESET != 0 {
            self.reset();
            return;
        }
        if value & CTRL_ACKNOWLEDGE != 0 {
            self.irq_flag = false;
            self.overrun = false;
        }
        self.control = value & !CTRL_STROBES;
    }
}

impl Default for Sio1 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled() -> Sio1 {
        let mut sio = Sio1::new();
        sio.write(0xA, 2, (CTRL_TX_ENABLE | CTRL_RX_ENABLE) as u32);
        sio
    }

    #[test]
    fn the_transmitter_is_always_ready() {
        let mut sio = Sio1::new();
        assert_eq!(sio.read(4, 4) & (STAT_TX_READY | STAT_TX_IDLE), STAT_TX_READY | STAT_TX_IDLE);
        assert_eq!(sio.read(4, 4) & STAT_RX_NOT_EMPTY, 0);
    }

    #[test]
    fn bytes_written_with_the_transmitter_on_reach_the_host() {
        let mut sio = Sio1::new();
        sio.write(0, 1, 'x' as u32);
        assert!(sio.take_output().is_empty(), "the transmitter is off");
        sio.write(0xA, 2, CTRL_TX_ENABLE as u32);
        for byte in "Hi".bytes() {
            sio.write(0, 1, byte as u32);
        }
        assert_eq!(sio.take_output(), b"Hi".to_vec());
        assert!(sio.take_output().is_empty(), "taken once");
    }

    #[test]
    fn received_bytes_queue_in_an_eight_byte_fifo() {
        let mut sio = enabled();
        for byte in 1..=10u8 {
            sio.push_input(byte);
        }
        assert_ne!(sio.read(4, 4) & STAT_RX_NOT_EMPTY, 0);
        assert_ne!(sio.read(4, 4) & STAT_RX_OVERRUN, 0, "the ninth byte did not fit");
        let received: Vec<u32> = (0..8).map(|_| sio.read(0, 1)).collect();
        assert_eq!(received, (1..=8).collect::<Vec<u32>>());
        assert_eq!(sio.read(4, 4) & STAT_RX_NOT_EMPTY, 0);
        assert_eq!(sio.read(0, 1), 0, "an empty FIFO reads 0");
    }

    #[test]
    fn nothing_arrives_while_the_receiver_is_off() {
        let mut sio = Sio1::new();
        sio.push_input(7);
        assert_eq!(sio.read(4, 4) & STAT_RX_NOT_EMPTY, 0);
    }

    #[test]
    fn the_receive_interrupt_waits_for_the_threshold_and_acknowledge_clears_it() {
        let mut sio = Sio1::new();
        // RX enabled, RX interrupt enabled, interrupt after 2 bytes.
        sio.write(0xA, 2, (CTRL_RX_ENABLE | CTRL_RX_INT_ENABLE | (1 << CTRL_RX_INT_MODE_SHIFT)) as u32);
        sio.push_input(1);
        assert!(!sio.take_irq());
        sio.push_input(2);
        assert!(sio.take_irq());
        assert_ne!(sio.read(4, 4) & STAT_IRQ, 0);
        sio.write(0xA, 2, (CTRL_RX_ENABLE | CTRL_RX_INT_ENABLE | CTRL_ACKNOWLEDGE) as u32);
        assert_eq!(sio.read(4, 4) & STAT_IRQ, 0);
        assert_eq!(sio.read(0xA, 2) as u16 & CTRL_ACKNOWLEDGE, 0, "the strobe is not stored");
    }

    #[test]
    fn the_transmit_interrupt_fires_when_a_byte_is_sent() {
        let mut sio = Sio1::new();
        sio.write(0xA, 2, (CTRL_TX_ENABLE | CTRL_TX_INT_ENABLE) as u32);
        assert!(!sio.take_irq());
        sio.write(0, 1, 0x41);
        assert!(sio.take_irq());
    }

    #[test]
    fn mode_and_baud_read_back_and_reset_clears_them() {
        let mut sio = Sio1::new();
        sio.write(8, 2, 0x004E);
        sio.write(0xE, 2, 0x0088);
        sio.write(0xC, 2, 0x0123);
        assert_eq!(sio.read(8, 2), 0x004E);
        assert_eq!(sio.read(0xE, 2), 0x0088);
        assert_eq!(sio.read(0xC, 2), 0x0123);
        sio.write(0xA, 2, CTRL_RESET as u32);
        assert_eq!(sio.read(8, 2), 0);
        assert_eq!(sio.read(0xE, 2), 0);
    }

    #[test]
    fn a_word_write_reaches_mode_and_control_together() {
        let mut sio = Sio1::new();
        sio.write(8, 4, ((CTRL_TX_ENABLE as u32) << 16) | 0x4E);
        assert_eq!(sio.read(8, 2), 0x4E);
        assert_eq!(sio.read(0xA, 2), CTRL_TX_ENABLE as u32);
        // A byte write to the mode half leaves the control register alone.
        sio.write(8, 1, 0x0D);
        assert_eq!(sio.read(0xA, 2), CTRL_TX_ENABLE as u32);
    }
}
