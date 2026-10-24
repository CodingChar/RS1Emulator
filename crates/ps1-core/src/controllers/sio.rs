//! Serial I/O port 0 (SIO0): the link to the two controller / memory card
//! ports.
//!
//! Registers at `0x1F801040`:
//!
//! | offset | name      | meaning                                         |
//! |--------|-----------|-------------------------------------------------|
//! | `+0`   | JOY_DATA  | read: received byte; write: byte to send        |
//! | `+4`   | JOY_STAT  | status flags (see `STAT_*`)                     |
//! | `+8`   | JOY_MODE  | baud multiplier and character length           |
//! | `+A`   | JOY_CTRL  | enables, port select, interrupt enables        |
//! | `+E`   | JOY_BAUD  | baud rate reload value                          |
//!
//! Writing JOY_DATA starts the transfer of one byte; a moment later the
//! device's answer shows up in the receive FIFO, and if the device pulses
//! /ACK the port raises IRQ7.
//!
//! Mirrors DuckStation's `sio.cpp`.

use std::collections::VecDeque;

use super::ControllerManager;
use crate::io::{extract, merge};

// ----- JOY_STAT bits -----
const STAT_TX_READY: u32 = 1 << 0;
const STAT_RX_NOT_EMPTY: u32 = 1 << 1;
const STAT_TX_IDLE: u32 = 1 << 2;
const STAT_ACK_LEVEL: u32 = 1 << 7;
const STAT_IRQ: u32 = 1 << 9;

// ----- JOY_CTRL bits -----
const CTRL_TX_ENABLE: u16 = 1 << 0;
/// /JOYn output: 1 selects the device (drives the line low).
const CTRL_SELECT: u16 = 1 << 1;
const CTRL_RX_ENABLE: u16 = 1 << 2;
/// Write-only strobe: clears the IRQ flag and error bits.
const CTRL_ACKNOWLEDGE: u16 = 1 << 4;
/// Write-only strobe: resets the port.
const CTRL_RESET: u16 = 1 << 6;
const CTRL_RX_INT_MODE_SHIFT: u16 = 8;
const CTRL_TX_INT_ENABLE: u16 = 1 << 10;
const CTRL_RX_INT_ENABLE: u16 = 1 << 11;
const CTRL_ACK_INT_ENABLE: u16 = 1 << 12;
/// 0 = port 1, 1 = port 2.
const CTRL_PORT_SELECT: u16 = 1 << 13;
const CTRL_STROBES: u16 = CTRL_ACKNOWLEDGE | CTRL_RESET;

/// The receive FIFO holds 8 bytes.
const RX_FIFO_SIZE: usize = 8;
/// Cycles between the end of a byte and the device's /ACK pulse.
const ACK_DELAY: u32 = 100;
/// Cycles /ACK stays asserted.
const ACK_WIDTH: u32 = 100;
/// Baud timer multipliers selected by the low two bits of JOY_MODE.
const BAUD_MULTIPLIER: [u32; 4] = [1, 1, 16, 64];

/// A byte on its way to the device and back.
struct Transfer {
    /// Cycles until the exchange completes.
    remaining: u32,
    /// The device's answer, or `None` when nothing answered.
    response: Option<(u8, bool)>,
}

pub struct Sio0 {
    mode: u16,
    control: u16,
    baud: u16,
    rx_fifo: VecDeque<u8>,
    /// A byte written while the transmitter was busy or disabled.
    tx_pending: Option<u8>,
    transfer: Option<Transfer>,
    ack_delay: u32,
    ack_hold: u32,
    /// JOY_STAT bit 9: an interrupt condition occurred and was not acknowledged.
    irq_flag: bool,
    /// An edge for the interrupt controller to pick up.
    irq: bool,
}

impl Sio0 {
    pub fn new() -> Self {
        Self {
            mode: 0,
            control: 0,
            baud: 0,
            rx_fifo: VecDeque::with_capacity(RX_FIFO_SIZE),
            tx_pending: None,
            transfer: None,
            ack_delay: 0,
            ack_hold: 0,
            irq_flag: false,
            irq: false,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Whether IRQ7 was raised since the last call.
    pub fn take_irq(&mut self) -> bool {
        std::mem::take(&mut self.irq)
    }

    fn status(&self) -> u32 {
        let mut stat = 0;
        if self.tx_pending.is_none() {
            stat |= STAT_TX_READY;
        }
        if !self.rx_fifo.is_empty() {
            stat |= STAT_RX_NOT_EMPTY;
        }
        if self.transfer.is_none() && self.tx_pending.is_none() {
            stat |= STAT_TX_IDLE;
        }
        if self.ack_hold > 0 {
            stat |= STAT_ACK_LEVEL;
        }
        if self.irq_flag {
            stat |= STAT_IRQ;
        }
        stat
    }

    /// Register read; `off` is relative to `0x1F801040` (0..0x10).
    pub fn read(&mut self, off: u32, width: u32) -> u32 {
        match off & !3 {
            0 => {
                // JOY_DATA pops the FIFO: `width` bytes, oldest in the low byte.
                let first = self.rx_fifo.pop_front().unwrap_or(0xFF) as u32;
                (1..width).fold(first, |acc, i| {
                    acc | ((self.rx_fifo.pop_front().unwrap_or(0) as u32) << (8 * i))
                })
            }
            4 => extract(self.status(), off, width),
            8 => extract(self.mode as u32 | ((self.control as u32) << 16), off, width),
            12 => extract((self.baud as u32) << 16, off, width),
            _ => 0,
        }
    }

    /// Register write; `off` is relative to `0x1F801040` (0..0x10).
    pub fn write(&mut self, off: u32, width: u32, value: u32, devices: &mut ControllerManager) {
        match off & !3 {
            0 => {
                self.tx_pending = Some(value as u8);
                self.try_start_transfer(devices);
            }
            4 => {} // JOY_STAT is read-only
            8 => {
                // MODE occupies the low half; CTRL the high half of this word.
                let word = merge(self.mode as u32 | ((self.control as u32) << 16), off, width, value);
                self.mode = word as u16;
                if off & 3 >= 2 || width == 4 {
                    self.write_control((word >> 16) as u16, devices);
                }
            }
            12 => {
                // +C is unused; BAUD sits in the high half of this word.
                let word = merge((self.baud as u32) << 16, off, width, value);
                if off & 3 >= 2 || width == 4 {
                    self.baud = (word >> 16) as u16;
                }
            }
            _ => {}
        }
    }

    fn write_control(&mut self, value: u16, devices: &mut ControllerManager) {
        let was_selected = self.control & CTRL_SELECT != 0;
        let old_port = self.control & CTRL_PORT_SELECT;

        if value & CTRL_RESET != 0 {
            let (mode, baud) = (self.mode, self.baud);
            self.reset();
            self.mode = mode;
            self.baud = baud;
            devices.deselect();
            return;
        }
        if value & CTRL_ACKNOWLEDGE != 0 {
            self.irq_flag = false;
        }
        self.control = value & !CTRL_STROBES;

        // The device sees a new chip select whenever the line is released,
        // asserted again, or moved to the other port.
        let selected = self.control & CTRL_SELECT != 0;
        if !selected || !was_selected || old_port != self.control & CTRL_PORT_SELECT {
            devices.deselect();
        }
        self.try_start_transfer(devices);
    }

    fn port(&self) -> usize {
        (self.control & CTRL_PORT_SELECT != 0) as usize
    }

    /// Cycles one byte takes on the wire.
    fn byte_cycles(&self) -> u32 {
        let reload = (self.baud as u32).max(1);
        8 * reload * BAUD_MULTIPLIER[(self.mode & 3) as usize]
    }

    fn try_start_transfer(&mut self, devices: &mut ControllerManager) {
        if self.transfer.is_some() || self.control & CTRL_TX_ENABLE == 0 {
            return;
        }
        let Some(tx) = self.tx_pending.take() else { return };
        let response = devices.exchange(self.port(), tx);
        self.transfer = Some(Transfer { remaining: self.byte_cycles(), response });
    }

    /// Advance the port by one CPU cycle.
    pub fn step(&mut self, devices: &mut ControllerManager) {
        if self.ack_hold > 0 {
            self.ack_hold -= 1;
        }
        if self.ack_delay > 0 {
            self.ack_delay -= 1;
            if self.ack_delay == 0 {
                self.ack_hold = ACK_WIDTH;
                if self.control & CTRL_ACK_INT_ENABLE != 0 {
                    self.request_irq();
                }
            }
        }

        let Some(transfer) = &mut self.transfer else { return };
        transfer.remaining = transfer.remaining.saturating_sub(1);
        if transfer.remaining > 0 {
            return;
        }
        let response = transfer.response;
        self.transfer = None;
        self.finish_transfer(response);
        self.try_start_transfer(devices);
    }

    fn finish_transfer(&mut self, response: Option<(u8, bool)>) {
        // With nothing plugged in the data line floats high.
        let (byte, ack) = response.unwrap_or((0xFF, false));

        // Bytes are received while /JOYn is asserted, or whenever RX is on.
        if self.control & (CTRL_RX_ENABLE | CTRL_SELECT) != 0 && self.rx_fifo.len() < RX_FIFO_SIZE {
            self.rx_fifo.push_back(byte);
            let threshold = 1usize << ((self.control >> CTRL_RX_INT_MODE_SHIFT) & 3);
            if self.control & CTRL_RX_INT_ENABLE != 0 && self.rx_fifo.len() >= threshold {
                self.request_irq();
            }
        }
        if self.control & CTRL_TX_INT_ENABLE != 0 {
            self.request_irq();
        }
        if ack {
            self.ack_delay = ACK_DELAY;
        }
    }

    /// Latch the interrupt flag, signalling the controller on the 0 -> 1 edge.
    fn request_irq(&mut self) {
        if !self.irq_flag {
            self.irq_flag = true;
            self.irq = true;
        }
    }
}

impl Default for Sio0 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controllers::memory_card::MemoryCard;
    use crate::controllers::Buttons;

    const DATA: u32 = 0;
    const STAT: u32 = 4;
    const MODE: u32 = 8;
    const CTRL: u32 = 0xA;
    const BAUD: u32 = 0xE;

    /// What the BIOS writes to JOY_CTRL to talk to port 1: TX on, select,
    /// ACK interrupt enabled.
    const BIOS_CTRL: u32 = 0x1003;

    struct Rig {
        sio: Sio0,
        devices: ControllerManager,
    }

    impl Rig {
        fn new() -> Self {
            let mut rig = Self { sio: Sio0::new(), devices: ControllerManager::new() };
            rig.sio.write(MODE, 2, 0x000D, &mut rig.devices);
            rig.sio.write(BAUD, 2, 0x0088, &mut rig.devices);
            rig
        }

        fn write(&mut self, off: u32, width: u32, value: u32) {
            self.sio.write(off, width, value, &mut self.devices);
        }

        fn read(&mut self, off: u32, width: u32) -> u32 {
            self.sio.read(off, width)
        }

        fn step(&mut self, cycles: u32) {
            for _ in 0..cycles {
                self.sio.step(&mut self.devices);
            }
        }

        fn select(&mut self, ctrl: u32) {
            self.write(CTRL, 2, ctrl);
        }

        /// Send one byte and wait for the answer. Returns it, or `None` if
        /// no byte arrived in time.
        fn transfer(&mut self, tx: u8) -> Option<u8> {
            self.write(DATA, 1, tx as u32);
            for _ in 0..20_000 {
                if self.read(STAT, 4) & STAT_RX_NOT_EMPTY != 0 {
                    return Some(self.read(DATA, 1) as u8);
                }
                self.step(1);
            }
            None
        }
    }

    #[test]
    fn power_on_status() {
        let mut rig = Rig::new();
        assert_eq!(rig.read(STAT, 4), STAT_TX_READY | STAT_TX_IDLE);
        assert_eq!(rig.read(DATA, 1), 0xFF, "empty FIFO");
    }

    #[test]
    fn mode_control_and_baud_are_readable() {
        let mut rig = Rig::new();
        assert_eq!(rig.read(MODE, 2), 0x000D);
        assert_eq!(rig.read(BAUD, 2), 0x0088);
        rig.select(0x1003);
        assert_eq!(rig.read(CTRL, 2), 0x1003);
        // A 32-bit read of the MODE/CTRL word sees both.
        assert_eq!(rig.read(MODE, 4), 0x1003_000D);
    }

    #[test]
    fn strobe_bits_are_not_stored() {
        let mut rig = Rig::new();
        rig.select(0x1003 | 0x10);
        assert_eq!(rig.read(CTRL, 2), 0x1003);
    }

    #[test]
    fn a_byte_takes_a_while_to_come_back() {
        let mut rig = Rig::new();
        rig.select(BIOS_CTRL);
        rig.write(DATA, 1, 0x01);
        assert_eq!(rig.read(STAT, 4) & STAT_TX_IDLE, 0, "transmitting");
        rig.step(8 * 0x88 - 1);
        assert_eq!(rig.read(STAT, 4) & STAT_RX_NOT_EMPTY, 0, "not yet");
        rig.step(1);
        assert_ne!(rig.read(STAT, 4) & STAT_RX_NOT_EMPTY, 0, "arrived");
        assert_ne!(rig.read(STAT, 4) & STAT_TX_IDLE, 0);
    }

    #[test]
    fn polling_the_pad_through_the_registers() {
        let mut rig = Rig::new();
        rig.devices.set_input(0, (Buttons::START | Buttons::CIRCLE).bits(), 0, 0, 0, 0);
        rig.select(BIOS_CTRL);

        assert_eq!(rig.transfer(0x01), Some(0xFF));
        assert_eq!(rig.transfer(0x42), Some(0x41));
        assert_eq!(rig.transfer(0x00), Some(0x5A));
        assert_eq!(rig.transfer(0x00), Some(!0x08));
        assert_eq!(rig.transfer(0x00), Some(!0x20));
    }

    #[test]
    fn rx_fifo_orders_bytes() {
        let mut rig = Rig::new();
        rig.select(BIOS_CTRL);
        rig.write(DATA, 1, 0x01);
        rig.step(10_000);
        rig.write(DATA, 1, 0x42);
        rig.step(10_000);
        assert_eq!(rig.read(DATA, 1), 0xFF);
        assert_eq!(rig.read(DATA, 1), 0x41);
        assert_eq!(rig.read(STAT, 4) & STAT_RX_NOT_EMPTY, 0);
    }

    #[test]
    fn a_word_read_returns_several_queued_bytes() {
        let mut rig = Rig::new();
        rig.select(BIOS_CTRL);
        rig.write(DATA, 1, 0x01);
        rig.step(10_000);
        rig.write(DATA, 1, 0x42);
        rig.step(10_000);
        assert_eq!(rig.read(DATA, 2), 0x41FF);
    }

    #[test]
    fn the_device_acknowledges_with_an_interrupt() {
        let mut rig = Rig::new();
        rig.select(BIOS_CTRL);
        rig.write(DATA, 1, 0x01);
        rig.step(8 * 0x88);
        assert!(!rig.sio.take_irq(), "the ACK pulse comes a little later");
        rig.step(ACK_DELAY);
        assert_ne!(rig.read(STAT, 4) & STAT_ACK_LEVEL, 0, "/ACK is low");
        assert_ne!(rig.read(STAT, 4) & STAT_IRQ, 0);
        assert!(rig.sio.take_irq());
        assert!(!rig.sio.take_irq(), "one edge");
        rig.step(ACK_WIDTH);
        assert_eq!(rig.read(STAT, 4) & STAT_ACK_LEVEL, 0, "released");
    }

    #[test]
    fn the_last_byte_of_a_poll_is_not_acknowledged() {
        let mut rig = Rig::new();
        rig.select(BIOS_CTRL);
        for byte in [0x01, 0x42, 0x00, 0x00] {
            rig.transfer(byte);
        }
        rig.step(1000);
        rig.write(CTRL, 2, 0x10 | BIOS_CTRL); // acknowledge the pending IRQ
        rig.sio.take_irq();
        rig.transfer(0x00);
        rig.step(1000);
        assert!(!rig.sio.take_irq(), "no /ACK after the final byte");
        assert_eq!(rig.read(STAT, 4) & STAT_IRQ, 0);
    }

    #[test]
    fn no_interrupt_unless_the_ack_interrupt_is_enabled() {
        let mut rig = Rig::new();
        rig.select(0x0003); // TX + select, no interrupt enables
        rig.transfer(0x01);
        rig.step(1000);
        assert!(!rig.sio.take_irq());
        assert_eq!(rig.read(STAT, 4) & STAT_IRQ, 0);
    }

    #[test]
    fn acknowledging_clears_the_irq_flag_and_allows_the_next_one() {
        let mut rig = Rig::new();
        rig.select(BIOS_CTRL);
        rig.transfer(0x01);
        rig.step(1000);
        assert!(rig.sio.take_irq());
        assert_ne!(rig.read(STAT, 4) & STAT_IRQ, 0);

        rig.transfer(0x42);
        rig.step(1000);
        assert!(!rig.sio.take_irq(), "the flag was never cleared: no new edge");

        rig.write(CTRL, 2, 0x10 | BIOS_CTRL);
        assert_eq!(rig.read(STAT, 4) & STAT_IRQ, 0);
        rig.transfer(0x00);
        rig.step(1000);
        assert!(rig.sio.take_irq());
    }

    #[test]
    fn rx_interrupt_fires_at_the_configured_threshold() {
        let mut rig = Rig::new();
        // RX interrupt enabled, threshold of 2 bytes (mode 1).
        rig.select(0x0003 | CTRL_RX_INT_ENABLE as u32 | (1 << CTRL_RX_INT_MODE_SHIFT));
        rig.write(DATA, 1, 0x01);
        rig.step(10_000);
        assert!(!rig.sio.take_irq(), "one byte in the FIFO is below the threshold");
        rig.write(DATA, 1, 0x42);
        rig.step(10_000);
        assert!(rig.sio.take_irq(), "second byte in the FIFO");
    }

    #[test]
    fn tx_interrupt_fires_when_a_byte_is_sent() {
        let mut rig = Rig::new();
        rig.select(0x0003 | CTRL_TX_INT_ENABLE as u32);
        rig.write(DATA, 1, 0x01);
        assert!(!rig.sio.take_irq());
        rig.step(10_000);
        assert!(rig.sio.take_irq());
    }

    #[test]
    fn an_empty_port_reads_as_ff_without_an_ack() {
        let mut rig = Rig::new();
        rig.select(BIOS_CTRL | CTRL_PORT_SELECT as u32);
        assert_eq!(rig.transfer(0x01), Some(0xFF));
        assert_eq!(rig.transfer(0x42), Some(0xFF));
        rig.step(1000);
        assert!(!rig.sio.take_irq());
    }

    #[test]
    fn port_two_reaches_its_own_pad() {
        let mut rig = Rig::new();
        rig.devices.ports[1].connected = true;
        rig.devices.set_input(1, Buttons::SELECT.bits(), 0, 0, 0, 0);
        rig.select(BIOS_CTRL | CTRL_PORT_SELECT as u32);
        rig.transfer(0x01);
        assert_eq!(rig.transfer(0x42), Some(0x41));
        rig.transfer(0x00);
        assert_eq!(rig.transfer(0x00), Some(!0x01));
    }

    #[test]
    fn deselecting_between_polls_restarts_the_protocol() {
        let mut rig = Rig::new();
        rig.select(BIOS_CTRL);
        rig.transfer(0x01);
        rig.transfer(0x42);
        rig.select(0x1001); // release /JOYn
        rig.select(BIOS_CTRL);
        assert_eq!(rig.transfer(0x01), Some(0xFF));
        assert_eq!(rig.transfer(0x42), Some(0x41));
    }

    #[test]
    fn nothing_is_received_while_deselected_and_rx_is_off() {
        let mut rig = Rig::new();
        rig.select(0x1001); // TX on, not selected, RX off
        rig.write(DATA, 1, 0x01);
        rig.step(10_000);
        assert_eq!(rig.read(STAT, 4) & STAT_RX_NOT_EMPTY, 0);
    }

    #[test]
    fn a_byte_written_with_tx_disabled_waits_for_it() {
        let mut rig = Rig::new();
        rig.select(0x0002); // select only
        rig.write(DATA, 1, 0x01);
        assert_eq!(rig.read(STAT, 4) & STAT_TX_READY, 0, "byte is queued");
        rig.step(10_000);
        assert_eq!(rig.read(STAT, 4) & STAT_RX_NOT_EMPTY, 0);
        rig.select(0x0003);
        rig.step(10_000);
        assert_ne!(rig.read(STAT, 4) & STAT_RX_NOT_EMPTY, 0);
    }

    #[test]
    fn a_second_byte_written_during_a_transfer_is_queued_and_sent_after() {
        let mut rig = Rig::new();
        rig.select(BIOS_CTRL);
        rig.write(DATA, 1, 0x01);
        rig.write(DATA, 1, 0x42);
        assert_eq!(rig.read(STAT, 4) & STAT_TX_READY, 0, "TX FIFO full");
        rig.step(3 * 8 * 0x88);
        assert_eq!(rig.read(DATA, 1), 0xFF);
        assert_eq!(rig.read(DATA, 1), 0x41);
    }

    #[test]
    fn reset_clears_the_fifo_and_the_control_register() {
        let mut rig = Rig::new();
        rig.select(BIOS_CTRL);
        rig.transfer(0x01);
        rig.write(DATA, 1, 0x42);
        rig.write(CTRL, 2, 0x40); // reset
        assert_eq!(rig.read(CTRL, 2), 0);
        assert_eq!(rig.read(STAT, 4), STAT_TX_READY | STAT_TX_IDLE);
        assert_eq!(rig.read(MODE, 2), 0x000D, "mode and baud survive");
    }

    #[test]
    fn baud_rate_sets_the_transfer_time() {
        let mut rig = Rig::new();
        rig.write(BAUD, 2, 0x0010);
        rig.select(BIOS_CTRL);
        rig.write(DATA, 1, 0x01);
        rig.step(8 * 0x10 - 1);
        assert_eq!(rig.read(STAT, 4) & STAT_RX_NOT_EMPTY, 0);
        rig.step(1);
        assert_ne!(rig.read(STAT, 4) & STAT_RX_NOT_EMPTY, 0);

        // The x16 multiplier of MODE bits 0-1 = 2.
        let mut rig = Rig::new();
        rig.write(MODE, 2, 0x000E);
        rig.write(BAUD, 2, 0x0010);
        rig.select(BIOS_CTRL);
        rig.write(DATA, 1, 0x01);
        rig.step(8 * 0x10 * 16 - 1);
        assert_eq!(rig.read(STAT, 4) & STAT_RX_NOT_EMPTY, 0);
        rig.step(1);
        assert_ne!(rig.read(STAT, 4) & STAT_RX_NOT_EMPTY, 0);
    }

    #[test]
    fn registers_accept_32_bit_writes() {
        let mut rig = Rig::new();
        rig.write(MODE, 4, 0x1003_000D);
        assert_eq!(rig.read(MODE, 2), 0x000D);
        assert_eq!(rig.read(CTRL, 2), 0x1003);
        rig.write(0xC, 4, 0x0042_0000); // BAUD sits in the high half of +C
        assert_eq!(rig.read(BAUD, 2), 0x0042);
        assert_eq!(rig.read(CTRL, 2), 0x1003, "unchanged");
    }

    #[test]
    fn reading_a_memory_card_through_the_port() {
        let mut rig = Rig::new();
        rig.devices.insert_memory_card(0, MemoryCard::formatted());
        rig.select(BIOS_CTRL);

        let mut reply = Vec::new();
        for byte in [0x81, b'R', 0, 0, 0, 1] {
            reply.push(rig.transfer(byte).unwrap());
        }
        assert_eq!(reply, vec![0xFF, 0x08, 0x5A, 0x5D, 0x00, 0x00]);
        for byte in [0, 0, 0, 0] {
            reply.push(rig.transfer(byte).unwrap());
        }
        assert_eq!(&reply[6..], &[0x5C, 0x5D, 0x00, 0x01]);
        // First byte of frame 1 (a free directory entry).
        assert_eq!(rig.transfer(0), Some(0xA0));
    }

    #[test]
    fn a_card_in_port_one_does_not_answer_on_port_two() {
        let mut rig = Rig::new();
        rig.devices.insert_memory_card(0, MemoryCard::formatted());
        rig.select(BIOS_CTRL | CTRL_PORT_SELECT as u32);
        assert_eq!(rig.transfer(0x81), Some(0xFF));
        assert_eq!(rig.transfer(b'S'), Some(0xFF), "floating line: no card");
    }
}
