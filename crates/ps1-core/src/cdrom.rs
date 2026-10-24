use std::collections::VecDeque;

use crate::disc::{
    from_bcd, lba_to_msf, to_bcd, Disc, TrackKind, DATA_SECTOR_SIZE, LEAD_IN_SECTORS,
    RAW_SECTOR_SIZE, SECTORS_PER_MINUTE, SECTORS_PER_SECOND,
};
use crate::types::Ps1Result;
use crate::xa::{self, Frame, XaDecoder};

// ----- Status byte returned by most commands -----
const STAT_ERROR: u8 = 0x01;
const STAT_MOTOR_ON: u8 = 0x02;
const STAT_ID_ERROR: u8 = 0x08;
const STAT_READING: u8 = 0x20;
const STAT_SEEKING: u8 = 0x40;
const STAT_PLAYING: u8 = 0x80;

// ----- Interrupt numbers (bits 0-2 of the interrupt flag register) -----
/// A sector of data is ready.
const INT1_DATA_READY: u8 = 1;
/// The second response of a command completed.
const INT2_COMPLETE: u8 = 2;
/// The first response of a command (acknowledge).
const INT3_ACKNOWLEDGE: u8 = 3;
/// The end of the disc was reached.
const INT4_DATA_END: u8 = 4;
/// A command failed.
const INT5_ERROR: u8 = 5;
/// The interrupt flag/enable registers hold five bits.
const INTERRUPT_MASK: u8 = 0x1F;

// ----- Error codes returned with INT5 -----
const ERROR_INVALID_ARGUMENT: u8 = 0x10;
const ERROR_WRONG_PARAMETER_COUNT: u8 = 0x20;
const ERROR_INVALID_COMMAND: u8 = 0x40;
const ERROR_NOT_READY: u8 = 0x80;

// ----- Mode register (Setmode) -----
/// Sectors hold the full 2340 bytes after the sync pattern instead of 2048.
const MODE_WHOLE_SECTOR: u8 = 1 << 5;
/// Decode XA-ADPCM audio sectors instead of delivering them.
const MODE_XA_ENABLE: u8 = 1 << 6;
/// Only decode XA sectors matching the Setfilter file and channel.
const MODE_XA_FILTER: u8 = 1 << 3;
/// Double speed (150 sectors per second).
const MODE_DOUBLE_SPEED: u8 = 1 << 7;

// ----- Timing, in CPU cycles (33.8688 MHz) -----
/// Delay before the first response of most commands.
const ACK_DELAY: u32 = 0xC4E1;
const INIT_COMPLETE_DELAY: u32 = 0x13CE7;
const GET_ID_DELAY: u32 = 0x4A00;
const PAUSE_DELAY_WHILE_READING: u32 = 0x21181;
const PAUSE_DELAY_IDLE: u32 = 0x1DF2;
const MOTOR_DELAY: u32 = 0x5_0000;
const SEEK_DELAY: u32 = 0x1_0000;
const TOC_DELAY: u32 = 0x10_0000;
/// Extra wait before the first sector of a read, for the head to settle.
const READ_START_DELAY: u32 = 0x1_0000;
const CPU_CLOCK: u32 = 33_868_800;

/// Parameter and response FIFOs are 16 bytes deep.
const FIFO_SIZE: usize = 16;

/// BIOS version reported by `Test 20h`: 1994-09-19, version C0 (SCPH-1001).
const CONTROLLER_VERSION: [u8; 4] = [0x94, 0x09, 0x19, 0xC0];

/// A CD-DA sector holds 588 stereo frames.
const AUDIO_FRAMES_PER_SECTOR: usize = RAW_SECTOR_SIZE / 4;
/// Upper bound on queued audio frames (about two seconds).
const AUDIO_QUEUE_LIMIT: usize = 44_100 * 2;

/// Keep at most this many entries in [`CdRom::command_log`].
const COMMAND_LOG_LIMIT: usize = 256;

/// A command the CPU issued, kept for debugging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandRecord {
    pub command: u8,
    pub parameters: Vec<u8>,
}

/// A response waiting for its delivery time.
struct Scheduled {
    /// CPU cycles to wait after the previous response was delivered.
    delay: u32,
    interrupt: u8,
    data: Vec<u8>,
}

/// CD-ROM drive and controller.
///
/// Four registers at `0x1F801800` whose meaning depends on the index selected
/// in the first one. Commands are written to register 1, parameters to
/// register 2, and the controller answers through the response FIFO and one
/// of five interrupts. Sector data is requested with the BFRD bit and read
/// from the data FIFO, usually by DMA channel 3.
///
/// Mirrors DuckStation's `cdrom.cpp`.
pub struct CdRom {
    pub disc: Option<Disc>,
    /// Position of the read head (LBA of the next sector to read).
    pub current_sector: u32,
    pub total_sectors: u32,
    /// The first commands issued since reset, with their parameters.
    pub command_log: Vec<CommandRecord>,
    /// Total number of commands issued since reset.
    pub command_count: u64,

    // Registers.
    index: u8,
    interrupt_enable: u8,
    interrupt_flag: u8,
    parameters: Vec<u8>,
    response: VecDeque<u8>,
    data: Vec<u8>,
    data_pos: usize,
    busy: bool,
    irq: bool,
    irq_line: bool,

    // Pending responses; `countdown` is the wait for the one at the front.
    queue: VecDeque<Scheduled>,
    countdown: u32,

    // Drive state.
    motor_on: bool,
    reading: bool,
    mode: u8,
    filter_file: u8,
    filter_channel: u8,
    seek_target: Option<u32>,
    read_countdown: u32,
    /// Sector delivered with the latest INT1, waiting for a BFRD request.
    sector_buffer: Vec<u8>,
    /// Header and subheader of the latest sector (for GetlocL).
    last_header: Option<[u8; 8]>,

    // Audio output towards the SPU, at 44.1 kHz.
    audio: VecDeque<Frame>,
    xa: XaDecoder,
    /// A CD-DA track is playing; `play_sector` is the next sector to read.
    playing: bool,
    play_sector: u32,
    play_countdown: u32,
    /// CD to SPU volume matrix, 0x80 = 1.0: left-left, left-right, right-left, right-right.
    volume: [u8; 4],
}

impl CdRom {
    pub fn new() -> Self {
        Self {
            disc: None,
            current_sector: 0,
            total_sectors: 0,
            command_log: Vec::new(),
            command_count: 0,
            index: 0,
            interrupt_enable: 0,
            interrupt_flag: 0,
            parameters: Vec::new(),
            response: VecDeque::new(),
            data: Vec::new(),
            data_pos: 0,
            busy: false,
            irq: false,
            irq_line: false,
            queue: VecDeque::new(),
            countdown: 0,
            motor_on: false,
            reading: false,
            mode: 0,
            filter_file: 0,
            filter_channel: 0,
            seek_target: None,
            read_countdown: 0,
            sector_buffer: Vec::new(),
            last_header: None,
            audio: VecDeque::new(),
            xa: XaDecoder::new(),
            playing: false,
            play_sector: 0,
            play_countdown: 0,
            volume: [0x80, 0, 0, 0x80],
        }
    }

    /// Insert a raw disc image (2352-byte sectors, or 2048-byte ISO).
    pub fn insert_image(&mut self, data: Vec<u8>) -> Ps1Result<()> {
        self.insert_disc(Disc::from_image(data)?);
        Ok(())
    }

    pub fn insert_disc(&mut self, disc: Disc) {
        self.total_sectors = disc.total_sectors();
        self.current_sector = 0;
        self.disc = Some(disc);
        self.motor_on = true;
    }

    pub fn reset(&mut self) {
        let disc = self.disc.take();
        *self = Self::new();
        if let Some(disc) = disc {
            self.insert_disc(disc);
        }
    }

    /// Whether the CD-ROM interrupt line rose since the last call.
    pub fn take_irq(&mut self) -> bool {
        std::mem::take(&mut self.irq)
    }

    // ----- Registers -----

    fn status_register(&self) -> u8 {
        let mut status = self.index;
        if self.parameters.is_empty() {
            status |= 0x08; // parameter FIFO empty
        }
        if self.parameters.len() < FIFO_SIZE {
            status |= 0x10; // parameter FIFO not full
        }
        if !self.response.is_empty() {
            status |= 0x20; // response FIFO not empty
        }
        if self.data_pos < self.data.len() {
            status |= 0x40; // data FIFO not empty
        }
        if self.busy {
            status |= 0x80; // command in progress
        }
        status
    }

    fn read_register(&mut self, reg: u32) -> u8 {
        match reg {
            0 => self.status_register(),
            1 => self.response.pop_front().unwrap_or(0),
            2 => self.pop_data_byte(),
            _ => match self.index & 1 {
                0 => 0xE0 | self.interrupt_enable,
                _ => 0xE0 | self.interrupt_flag,
            },
        }
    }

    fn pop_data_byte(&mut self) -> u8 {
        let byte = self.data.get(self.data_pos).copied().unwrap_or(0);
        if self.data_pos < self.data.len() {
            self.data_pos += 1;
        }
        byte
    }

    /// Register read; `off` is relative to `0x1F801800`.
    pub fn read(&mut self, off: u32, width: u32) -> u32 {
        let reg = off & 3;
        // Only the data port is read in wider units (by the CPU or DMA).
        let count = if reg == 2 { width.max(1) } else { 1 };
        (0..count).fold(0u32, |acc, i| acc | ((self.read_register(reg) as u32) << (8 * i)))
    }

    /// Register write; `off` is relative to `0x1F801800`.
    pub fn write(&mut self, off: u32, _width: u32, value: u32) {
        let value = value as u8;
        match (off & 3, self.index) {
            (0, _) => self.index = value & 3,
            (1, 0) => self.execute(value),
            (2, 0) => {
                if self.parameters.len() < FIFO_SIZE {
                    self.parameters.push(value);
                }
            }
            (2, 1) => {
                self.interrupt_enable = value & INTERRUPT_MASK;
                self.update_irq_line();
            }
            (3, 0) => {
                // Request register: BFRD makes the sector buffer readable.
                if value & 0x80 != 0 {
                    if self.data_pos >= self.data.len() {
                        self.data = self.sector_buffer.clone();
                        self.data_pos = 0;
                    }
                } else {
                    self.data.clear();
                    self.data_pos = 0;
                }
            }
            (3, 1) => {
                // Acknowledge interrupts (write 1 to clear) / reset parameters.
                self.interrupt_flag &= !(value & INTERRUPT_MASK);
                if value & 0x40 != 0 {
                    self.parameters.clear();
                }
                self.update_irq_line();
            }
            // Audio volume matrix: CD left/right into the SPU left/right inputs.
            (2, 2) => self.volume[0] = value,
            (3, 2) => self.volume[1] = value,
            (2, 3) => self.volume[2] = value,
            (1, 3) => self.volume[3] = value,
            _ => {}
        }
    }

    /// Raise the CPU interrupt when an enabled interrupt becomes pending.
    fn update_irq_line(&mut self) {
        let line = self.interrupt_flag & self.interrupt_enable & INTERRUPT_MASK != 0;
        if line && !self.irq_line {
            self.irq = true;
        }
        self.irq_line = line;
    }

    /// DMA: read one word of sector data.
    pub fn dma_read_word(&mut self) -> u32 {
        (0..4).fold(0u32, |acc, i| acc | ((self.pop_data_byte() as u32) << (8 * i)))
    }

    // ----- Time -----

    /// Advance the drive by one CPU cycle.
    pub fn step(&mut self) {
        if !self.queue.is_empty() {
            self.countdown = self.countdown.saturating_sub(1);
            // A new interrupt waits until the previous one is acknowledged.
            if self.countdown == 0 && self.interrupt_flag == 0 {
                self.deliver_next_response();
            }
        }

        if self.playing {
            if self.play_countdown > 0 {
                self.play_countdown -= 1;
            } else {
                self.deliver_audio_sector();
            }
        }

        if self.reading {
            if self.read_countdown > 0 {
                self.read_countdown -= 1;
            } else if self.interrupt_flag == 0 && self.queue.is_empty() {
                self.deliver_sector();
            }
        }
    }

    fn deliver_next_response(&mut self) {
        let Some(item) = self.queue.pop_front() else { return };
        self.set_response(item.interrupt, &item.data);
        self.busy = false;
        self.countdown = self.queue.front().map_or(0, |next| next.delay);
    }

    fn set_response(&mut self, interrupt: u8, data: &[u8]) {
        self.response.clear();
        self.response.extend(data.iter().take(FIFO_SIZE));
        self.interrupt_flag = interrupt;
        self.update_irq_line();
    }

    /// Queue a response `delay` cycles after the previous one.
    fn schedule(&mut self, delay: u32, interrupt: u8, data: Vec<u8>) {
        if self.queue.is_empty() {
            self.countdown = delay;
        }
        self.queue.push_back(Scheduled { delay, interrupt, data });
    }

    fn cycles_per_sector(&self) -> u32 {
        let sectors_per_second = if self.mode & MODE_DOUBLE_SPEED != 0 {
            2 * SECTORS_PER_SECOND
        } else {
            SECTORS_PER_SECOND
        };
        CPU_CLOCK / sectors_per_second
    }

    /// Read the sector under the head and raise INT1 (or INT4 at the end).
    fn deliver_sector(&mut self) {
        let lba = self.current_sector;
        let mut raw = [0u8; RAW_SECTOR_SIZE];
        let readable = self
            .disc
            .as_mut()
            .is_some_and(|disc| lba < disc.total_sectors() && disc.read_raw_sector(lba, &mut raw));
        if !readable {
            self.reading = false;
            let stat = self.stat();
            self.set_response(INT4_DATA_END, &[stat]);
            return;
        }

        // XA audio sectors are decoded for the SPU instead of being delivered.
        if self.mode & MODE_XA_ENABLE != 0 && xa::is_audio_sector(&raw) {
            let selected = self.mode & MODE_XA_FILTER == 0
                || (raw[16] == self.filter_file && raw[17] == self.filter_channel);
            if selected && self.audio.len() < AUDIO_QUEUE_LIMIT {
                self.xa.decode_sector(&raw, &mut self.audio);
            }
            self.current_sector += 1;
            self.read_countdown = self.cycles_per_sector();
            return;
        }

        let mut header = [0u8; 8];
        header.copy_from_slice(&raw[12..20]);
        self.last_header = Some(header);
        self.sector_buffer = if self.mode & MODE_WHOLE_SECTOR != 0 {
            raw[12..].to_vec() // 2340 bytes after the sync pattern
        } else {
            raw[24..24 + DATA_SECTOR_SIZE].to_vec()
        };

        self.current_sector += 1;
        self.read_countdown = self.cycles_per_sector();
        let stat = self.stat();
        self.set_response(INT1_DATA_READY, &[stat]);
    }

    /// Read the next CD-DA sector under the play head into the audio queue.
    fn deliver_audio_sector(&mut self) {
        let lba = self.play_sector;
        let mut raw = [0u8; RAW_SECTOR_SIZE];
        let readable = self.disc.as_mut().is_some_and(|disc| {
            lba < disc.total_sectors()
                && disc.track_at(lba).kind == TrackKind::Audio
                && disc.read_raw_sector(lba, &mut raw)
        });
        if !readable {
            self.playing = false;
            return;
        }
        if self.audio.len() < AUDIO_QUEUE_LIMIT {
            for i in 0..AUDIO_FRAMES_PER_SECTOR {
                let left = i16::from_le_bytes([raw[i * 4], raw[i * 4 + 1]]);
                let right = i16::from_le_bytes([raw[i * 4 + 2], raw[i * 4 + 3]]);
                self.audio.push_back((left, right));
            }
        }
        self.play_sector += 1;
        self.current_sector = self.play_sector;
        self.play_countdown = self.cycles_per_sector();
    }

    /// The next frame of CD audio (CD-DA or XA) for the SPU, with the drive
    /// volume matrix applied; silence when nothing is playing.
    pub fn pop_audio_frame(&mut self) -> Frame {
        let Some((left, right)) = self.audio.pop_front() else {
            return (0, 0);
        };
        let [ll, lr, rl, rr] = self.volume.map(|v| v as i32);
        let out_left = (left as i32 * ll + right as i32 * rl) >> 7;
        let out_right = (left as i32 * lr + right as i32 * rr) >> 7;
        (out_left.clamp(-0x8000, 0x7FFF) as i16, out_right.clamp(-0x8000, 0x7FFF) as i16)
    }

    // ----- Commands -----

    /// The status byte most responses start with.
    fn stat(&self) -> u8 {
        let mut stat = 0;
        if self.motor_on {
            stat |= STAT_MOTOR_ON;
        }
        if self.reading {
            stat |= STAT_READING;
        }
        if self.playing {
            stat |= STAT_PLAYING;
        }
        stat
    }

    fn acknowledge(&mut self, data: Vec<u8>) {
        self.schedule(ACK_DELAY, INT3_ACKNOWLEDGE, data);
    }

    fn complete(&mut self, delay: u32, data: Vec<u8>) {
        self.schedule(delay, INT2_COMPLETE, data);
    }

    fn fail(&mut self, code: u8) {
        let stat = self.stat() | STAT_ERROR;
        self.schedule(ACK_DELAY, INT5_ERROR, vec![stat, code]);
    }

    /// Decode a BCD minute/second/frame triple into an LBA.
    fn parse_location(params: &[u8]) -> Option<u32> {
        let m = from_bcd(params[0])? as u32;
        let s = from_bcd(params[1])? as u32;
        let f = from_bcd(params[2])? as u32;
        if s >= 60 || f >= SECTORS_PER_SECOND {
            return None;
        }
        let frames = m * SECTORS_PER_MINUTE + s * SECTORS_PER_SECOND + f;
        // The clock starts at 00:02:00; earlier positions are the lead-in.
        Some(frames.saturating_sub(LEAD_IN_SECTORS))
    }

    fn execute(&mut self, command: u8) {
        let params = std::mem::take(&mut self.parameters);
        self.busy = true;
        self.command_count += 1;
        if self.command_log.len() < COMMAND_LOG_LIMIT {
            self.command_log.push(CommandRecord { command, parameters: params.clone() });
        }
        let stat = self.stat();

        // Parameter counts the controller insists on.
        let expected = match command {
            0x02 => Some(3),
            0x0D => Some(2),
            0x0E | 0x14 | 0x19 => Some(1),
            0x01 | 0x06 | 0x07 | 0x08 | 0x09 | 0x0A | 0x0B | 0x0C | 0x0F | 0x10 | 0x11 | 0x13
            | 0x15 | 0x16 | 0x1A | 0x1B | 0x1E => Some(0),
            _ => None,
        };
        if expected.is_some_and(|n| n != params.len()) {
            return self.fail(ERROR_WRONG_PARAMETER_COUNT);
        }

        match command {
            // Getstat
            0x01 => self.acknowledge(vec![stat]),
            // Setloc
            0x02 => match Self::parse_location(&params) {
                Some(lba) => {
                    self.seek_target = Some(lba);
                    self.acknowledge(vec![stat]);
                }
                None => self.fail(ERROR_INVALID_ARGUMENT),
            },
            // Play: CD-DA playback from a track, the Setloc position or the head
            0x03 => {
                if self.disc.is_none() {
                    return self.fail(ERROR_NOT_READY);
                }
                let track_start = params.first().filter(|t| **t != 0).and_then(|t| {
                    let number = from_bcd(*t)?;
                    self.disc.as_ref()?.tracks().iter().find(|t| t.number == number).map(|t| t.start_lba)
                });
                self.play_sector = track_start
                    .or_else(|| self.seek_target.take())
                    .unwrap_or(self.current_sector);
                self.seek_target = None;
                self.playing = true;
                self.reading = false;
                self.play_countdown = ACK_DELAY + READ_START_DELAY;
                self.acknowledge(vec![self.stat()]);
            }
            // Forward, Backward: fast-forward is not emulated.
            0x04 | 0x05 => self.acknowledge(vec![stat]),
            // ReadN, ReadS
            0x06 | 0x1B => {
                if self.disc.is_none() {
                    return self.fail(ERROR_NOT_READY);
                }
                if let Some(target) = self.seek_target.take() {
                    self.current_sector = target;
                }
                self.playing = false;
                self.reading = true;
                self.read_countdown = ACK_DELAY + READ_START_DELAY;
                self.acknowledge(vec![self.stat()]);
            }
            // MotorOn
            0x07 => {
                self.motor_on = true;
                self.acknowledge(vec![self.stat()]);
                self.complete(MOTOR_DELAY, vec![self.stat()]);
            }
            // Stop
            0x08 => {
                self.reading = false;
                self.playing = false;
                self.acknowledge(vec![stat]);
                self.motor_on = false;
                self.complete(MOTOR_DELAY, vec![self.stat()]);
            }
            // Pause
            0x09 => {
                let delay = if self.reading { PAUSE_DELAY_WHILE_READING } else { PAUSE_DELAY_IDLE };
                self.reading = false;
                self.playing = false;
                self.acknowledge(vec![stat]);
                self.complete(delay, vec![self.stat()]);
            }
            // Init
            0x0A => {
                self.motor_on = true;
                self.reading = false;
                self.playing = false;
                self.mode = 0;
                self.acknowledge(vec![self.stat()]);
                self.complete(INIT_COMPLETE_DELAY, vec![self.stat()]);
            }
            // Mute, Demute
            0x0B | 0x0C => self.acknowledge(vec![stat]),
            // Setfilter
            0x0D => {
                self.filter_file = params[0];
                self.filter_channel = params[1];
                self.acknowledge(vec![stat]);
            }
            // Setmode
            0x0E => {
                self.mode = params[0];
                self.acknowledge(vec![stat]);
            }
            // Getparam
            0x0F => self.acknowledge(vec![stat, self.mode, 0, self.filter_file, self.filter_channel]),
            // GetlocL
            0x10 => match self.last_header {
                Some(header) => self.acknowledge(header.to_vec()),
                None => self.fail(ERROR_NOT_READY),
            },
            // GetlocP
            0x11 => {
                let (track, relative, absolute) = self.position_info();
                let (rm, rs, rf) = lba_to_msf(relative);
                let (am, as_, af) = lba_to_msf(absolute);
                self.acknowledge(vec![
                    to_bcd(track),
                    1,
                    to_bcd(rm),
                    to_bcd(rs),
                    to_bcd(rf),
                    to_bcd(am),
                    to_bcd(as_),
                    to_bcd(af),
                ]);
            }
            // SetSession
            0x12 => {
                self.acknowledge(vec![stat]);
                self.complete(SEEK_DELAY, vec![stat]);
            }
            // GetTN
            0x13 => match &self.disc {
                Some(disc) => {
                    let last = disc.tracks().last().map_or(1, |t| t.number);
                    self.acknowledge(vec![stat, to_bcd(1), to_bcd(last)]);
                }
                None => self.fail(ERROR_NOT_READY),
            },
            // GetTD
            0x14 => match (&self.disc, from_bcd(params[0])) {
                (Some(disc), Some(track)) => {
                    let start = if track == 0 {
                        Some(disc.total_sectors())
                    } else {
                        disc.tracks().iter().find(|t| t.number == track).map(|t| t.start_lba)
                    };
                    match start {
                        Some(lba) => {
                            let (m, s, _) = lba_to_msf(lba + LEAD_IN_SECTORS);
                            self.acknowledge(vec![stat, to_bcd(m), to_bcd(s)]);
                        }
                        None => self.fail(ERROR_INVALID_ARGUMENT),
                    }
                }
                (None, _) => self.fail(ERROR_NOT_READY),
                (_, None) => self.fail(ERROR_INVALID_ARGUMENT),
            },
            // SeekL, SeekP
            0x15 | 0x16 => {
                if self.disc.is_none() {
                    return self.fail(ERROR_NOT_READY);
                }
                self.reading = false;
                self.playing = false;
                if let Some(target) = self.seek_target.take() {
                    self.current_sector = target;
                }
                self.acknowledge(vec![self.stat() | STAT_SEEKING]);
                self.complete(SEEK_DELAY, vec![self.stat()]);
            }
            // Test
            0x19 => match params[0] {
                0x20 => self.acknowledge(CONTROLLER_VERSION.to_vec()),
                _ => self.fail(ERROR_INVALID_ARGUMENT),
            },
            // GetID
            0x1A => self.get_id(stat),
            // ReadTOC
            0x1E => {
                if self.disc.is_none() {
                    return self.fail(ERROR_NOT_READY);
                }
                self.acknowledge(vec![stat]);
                self.complete(TOC_DELAY, vec![self.stat()]);
            }
            _ => self.fail(ERROR_INVALID_COMMAND),
        }
    }

    fn get_id(&mut self, stat: u8) {
        let region = self.disc.as_mut().and_then(|disc| disc.region());
        let has_disc = self.disc.is_some();
        self.acknowledge(vec![stat]);
        match (has_disc, region) {
            // Licensed data disc.
            (true, Some(region)) => {
                let mut id = vec![self.stat(), 0x00, 0x20, 0x00];
                id.extend_from_slice(&region.id());
                self.complete(GET_ID_DELAY, id);
            }
            // Data disc without a recognisable licence.
            (true, None) => {
                self.schedule(GET_ID_DELAY, INT5_ERROR, vec![self.stat() | STAT_ERROR, 0x40, 0, 0, 0, 0, 0, 0]);
            }
            // Empty drive.
            (false, _) => {
                self.schedule(GET_ID_DELAY, INT5_ERROR, vec![STAT_ID_ERROR, 0x40, 0, 0, 0, 0, 0, 0]);
            }
        }
    }

    /// Track number, position within the track and absolute position of the
    /// sector last read, as `(track, relative, absolute)` clock values.
    fn position_info(&self) -> (u8, u32, u32) {
        let lba = self.current_sector.saturating_sub(1);
        match &self.disc {
            Some(disc) => {
                let track = disc.track_at(lba);
                (track.number, lba.saturating_sub(track.start_lba), lba + LEAD_IN_SECTORS)
            }
            None => (1, 0, lba + LEAD_IN_SECTORS),
        }
    }
}

impl Default for CdRom {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cooked image with a valid US licence sector and a recognisable
    /// payload: byte 0 of every sector is the sector number.
    fn us_disc(sectors: u32) -> Disc {
        let mut data = vec![0u8; sectors as usize * DATA_SECTOR_SIZE];
        for s in 0..sectors as usize {
            data[s * DATA_SECTOR_SIZE] = s as u8;
            data[s * DATA_SECTOR_SIZE + 1] = 0xA5;
        }
        let text = b"          Licensed  by          Sony Computer Entertainment Amer  ica ";
        data[4 * DATA_SECTOR_SIZE..4 * DATA_SECTOR_SIZE + text.len()].copy_from_slice(text);
        Disc::from_image(data).unwrap()
    }

    /// Drives the controller through its registers like the BIOS would.
    struct Drive {
        cd: CdRom,
        cycles: u64,
    }

    impl Drive {
        fn new() -> Self {
            Self { cd: CdRom::new(), cycles: 0 }
        }

        fn with_disc(sectors: u32) -> Self {
            let mut drive = Self::new();
            drive.cd.insert_disc(us_disc(sectors));
            drive
        }

        fn select(&mut self, index: u32) {
            self.cd.write(0, 1, index);
        }

        fn status(&mut self) -> u8 {
            self.cd.read(0, 1) as u8
        }

        /// Enable all interrupts, as the BIOS does.
        fn enable_interrupts(&mut self) {
            self.select(1);
            self.cd.write(2, 1, 0x1F);
        }

        fn command(&mut self, command: u8, params: &[u8]) {
            self.select(0);
            for p in params {
                self.cd.write(2, 1, *p as u32);
            }
            self.cd.write(1, 1, command as u32);
        }

        /// Step until an interrupt is pending; returns its number.
        fn wait_interrupt(&mut self) -> u8 {
            self.wait_interrupt_within(2_000_000)
        }

        fn wait_interrupt_within(&mut self, limit: u64) -> u8 {
            for _ in 0..limit {
                if self.flag() != 0 {
                    return self.flag();
                }
                self.cd.step();
                self.cycles += 1;
            }
            panic!("no interrupt within {limit} cycles");
        }

        fn flag(&mut self) -> u8 {
            self.select(1);
            (self.cd.read(3, 1) as u8) & 0x1F
        }

        fn ack(&mut self) {
            self.select(1);
            self.cd.write(3, 1, 0x1F);
        }

        /// Read the whole response FIFO.
        fn response(&mut self) -> Vec<u8> {
            let mut bytes = Vec::new();
            while self.status() & 0x20 != 0 {
                bytes.push(self.cd.read(1, 1) as u8);
            }
            bytes
        }

        /// Issue a command and return `(interrupt, response)` of its first answer.
        fn run(&mut self, command: u8, params: &[u8]) -> (u8, Vec<u8>) {
            self.command(command, params);
            let int = self.wait_interrupt();
            let response = self.response();
            self.ack();
            (int, response)
        }
    }

    #[test]
    fn idle_status_register() {
        let mut drive = Drive::new();
        assert_eq!(drive.status(), 0x18, "parameter FIFO empty and not full");
        drive.select(2);
        assert_eq!(drive.status() & 3, 2, "index bits");
    }

    #[test]
    fn parameter_fifo_flags() {
        let mut drive = Drive::new();
        drive.cd.write(2, 1, 0x11);
        assert_eq!(drive.status() & 0x18, 0x10, "not empty but not full");
        for _ in 0..15 {
            drive.cd.write(2, 1, 0);
        }
        assert_eq!(drive.status() & 0x18, 0x00, "full");
        // Reset through the acknowledge register.
        drive.select(1);
        drive.cd.write(3, 1, 0x40);
        assert_eq!(drive.status() & 0x18, 0x18);
    }

    #[test]
    fn getstat_answers_with_int3_after_a_delay() {
        let mut drive = Drive::with_disc(8);
        drive.enable_interrupts();
        drive.command(0x01, &[]);
        assert_ne!(drive.status() & 0x80, 0, "busy while the command runs");
        assert_eq!(drive.status() & 0x20, 0, "no response yet");

        let int = drive.wait_interrupt();
        assert_eq!(int, INT3_ACKNOWLEDGE);
        assert!(drive.cycles >= ACK_DELAY as u64, "took {} cycles", drive.cycles);
        assert_eq!(drive.status() & 0x80, 0, "no longer busy");
        assert_ne!(drive.status() & 0x20, 0, "response ready");
        assert_eq!(drive.response(), vec![STAT_MOTOR_ON]);
        assert_eq!(drive.response(), Vec::<u8>::new(), "FIFO drained");
    }

    #[test]
    fn cpu_interrupt_fires_only_when_enabled() {
        let mut drive = Drive::with_disc(8);
        drive.command(0x01, &[]);
        drive.wait_interrupt();
        assert!(!drive.cd.take_irq(), "interrupts were not enabled");

        // Enabling now makes the pending interrupt visible to the CPU.
        drive.enable_interrupts();
        assert!(drive.cd.take_irq());
        assert!(!drive.cd.take_irq(), "edge, not level");

        // Acknowledge, then a new interrupt raises it again.
        drive.ack();
        drive.command(0x01, &[]);
        drive.wait_interrupt();
        assert!(drive.cd.take_irq());
    }

    #[test]
    fn interrupt_registers_have_their_high_bits_set() {
        let mut drive = Drive::with_disc(8);
        drive.enable_interrupts();
        drive.select(0);
        assert_eq!(drive.cd.read(3, 1), 0xE0 | 0x1F, "enable register");
        drive.select(1);
        assert_eq!(drive.cd.read(3, 1), 0xE0, "flag register");
    }

    #[test]
    fn acknowledge_clears_only_the_written_bits() {
        let mut drive = Drive::with_disc(8);
        drive.command(0x01, &[]);
        drive.wait_interrupt();
        // The flag register holds the interrupt number (3 = 0b011); writing 1s
        // clears the corresponding bits.
        drive.select(1);
        drive.cd.write(3, 1, 0x01);
        assert_eq!(drive.flag(), 0b010);
        drive.cd.write(3, 1, 0x02);
        assert_eq!(drive.flag(), 0);
    }

    #[test]
    fn second_response_waits_for_the_first_to_be_acknowledged() {
        let mut drive = Drive::with_disc(8);
        drive.enable_interrupts();
        drive.command(0x0A, &[]); // Init
        assert_eq!(drive.wait_interrupt(), INT3_ACKNOWLEDGE);

        // Far longer than the INT2 delay, but the INT3 is still pending.
        for _ in 0..(INIT_COMPLETE_DELAY * 2) {
            drive.cd.step();
        }
        assert_eq!(drive.flag(), INT3_ACKNOWLEDGE);

        drive.ack();
        assert_eq!(drive.wait_interrupt(), INT2_COMPLETE);
        assert_eq!(drive.response(), vec![STAT_MOTOR_ON]);
    }

    #[test]
    fn test_command_reports_the_controller_version() {
        let mut drive = Drive::new();
        assert_eq!(drive.run(0x19, &[0x20]), (INT3_ACKNOWLEDGE, vec![0x94, 0x09, 0x19, 0xC0]));
        let (int, response) = drive.run(0x19, &[0x99]);
        assert_eq!(int, INT5_ERROR);
        assert_eq!(response[1], ERROR_INVALID_ARGUMENT);
    }

    #[test]
    fn unknown_commands_and_bad_parameter_counts_fail() {
        let mut drive = Drive::with_disc(8);
        let (int, response) = drive.run(0x77, &[]);
        assert_eq!(int, INT5_ERROR);
        assert_eq!(response, vec![STAT_MOTOR_ON | STAT_ERROR, ERROR_INVALID_COMMAND]);

        let (int, response) = drive.run(0x02, &[0x00, 0x02]); // Setloc needs 3
        assert_eq!(int, INT5_ERROR);
        assert_eq!(response[1], ERROR_WRONG_PARAMETER_COUNT);

        let (int, response) = drive.run(0x01, &[0x00]); // Getstat takes none
        assert_eq!(int, INT5_ERROR);
        assert_eq!(response[1], ERROR_WRONG_PARAMETER_COUNT);
    }

    #[test]
    fn get_id_identifies_a_licensed_us_disc() {
        let mut drive = Drive::with_disc(8);
        assert_eq!(drive.run(0x1A, &[]).0, INT3_ACKNOWLEDGE);
        assert_eq!(drive.wait_interrupt(), INT2_COMPLETE);
        assert_eq!(
            drive.response(),
            vec![0x02, 0x00, 0x20, 0x00, b'S', b'C', b'E', b'A']
        );
    }

    #[test]
    fn get_id_without_a_disc_reports_an_error() {
        let mut drive = Drive::new();
        assert_eq!(drive.run(0x1A, &[]).0, INT3_ACKNOWLEDGE);
        assert_eq!(drive.wait_interrupt(), INT5_ERROR);
        assert_eq!(drive.response(), vec![0x08, 0x40, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn get_id_with_an_unlicensed_disc_reports_an_error() {
        let mut drive = Drive::new();
        drive.cd.insert_disc(Disc::from_image(vec![0; 8 * DATA_SECTOR_SIZE]).unwrap());
        drive.run(0x1A, &[]);
        assert_eq!(drive.wait_interrupt(), INT5_ERROR);
    }

    #[test]
    fn setmode_and_getparam_round_trip() {
        let mut drive = Drive::with_disc(8);
        assert_eq!(drive.run(0x0E, &[0xA0]).0, INT3_ACKNOWLEDGE);
        drive.run(0x0D, &[0x01, 0x02]); // Setfilter
        let (int, response) = drive.run(0x0F, &[]);
        assert_eq!(int, INT3_ACKNOWLEDGE);
        assert_eq!(response, vec![STAT_MOTOR_ON, 0xA0, 0, 0x01, 0x02]);
    }

    #[test]
    fn table_of_contents_commands() {
        let mut drive = Drive::with_disc(100);
        assert_eq!(drive.run(0x13, &[]).1, vec![STAT_MOTOR_ON, 0x01, 0x01]);
        // Track 1 starts at 00:02:00; the lead-out of a 100-sector disc is at 250 = 00:03:25.
        assert_eq!(drive.run(0x14, &[0x01]).1, vec![STAT_MOTOR_ON, 0x00, 0x02]);
        assert_eq!(drive.run(0x14, &[0x00]).1, vec![STAT_MOTOR_ON, 0x00, 0x03]);
        assert_eq!(drive.run(0x14, &[0x05]).0, INT5_ERROR, "no track 5");
    }

    #[test]
    fn toc_commands_need_a_disc() {
        let mut drive = Drive::new();
        for command in [0x13u8, 0x15, 0x1E, 0x06] {
            assert_eq!(drive.run(command, &[]).0, INT5_ERROR, "command {command:#04x}");
        }
        assert_eq!(drive.run(0x14, &[0x01]).0, INT5_ERROR);
    }

    #[test]
    fn setloc_validates_its_arguments() {
        let mut drive = Drive::with_disc(8);
        assert_eq!(drive.run(0x02, &[0x00, 0x02, 0x00]).0, INT3_ACKNOWLEDGE);
        assert_eq!(drive.run(0x02, &[0x00, 0x60, 0x00]).0, INT5_ERROR, "60 seconds");
        assert_eq!(drive.run(0x02, &[0x00, 0x02, 0x75]).0, INT5_ERROR, "75 frames");
        assert_eq!(drive.run(0x02, &[0x0A, 0x02, 0x00]).0, INT5_ERROR, "A is not BCD");
    }

    #[test]
    fn seek_moves_the_head_and_completes_with_int2() {
        let mut drive = Drive::with_disc(32);
        drive.run(0x02, &[0x00, 0x02, 0x10]); // 00:02:10 = LBA 10
        let (int, response) = drive.run(0x15, &[]);
        assert_eq!(int, INT3_ACKNOWLEDGE);
        assert_eq!(response, vec![STAT_MOTOR_ON | STAT_SEEKING]);
        assert_eq!(drive.wait_interrupt(), INT2_COMPLETE);
        assert_eq!(drive.cd.current_sector, 10);
    }

    #[test]
    fn reading_delivers_one_sector_per_interrupt() {
        let mut drive = Drive::with_disc(32);
        drive.enable_interrupts();
        drive.run(0x02, &[0x00, 0x02, 0x08]); // LBA 8
        let (int, response) = drive.run(0x06, &[]); // ReadN
        assert_eq!(int, INT3_ACKNOWLEDGE);
        assert_eq!(response, vec![STAT_MOTOR_ON | STAT_READING]);

        for expected in [8u8, 9, 10] {
            assert_eq!(drive.wait_interrupt(), INT1_DATA_READY);
            assert_eq!(drive.response(), vec![STAT_MOTOR_ON | STAT_READING]);

            // Request the data and read it from the data FIFO.
            drive.select(0);
            drive.cd.write(3, 1, 0x80);
            assert_ne!(drive.status() & 0x40, 0, "data ready");
            assert_eq!(drive.cd.read(2, 1) as u8, expected);
            assert_eq!(drive.cd.read(2, 1) as u8, 0xA5);
            for _ in 2..DATA_SECTOR_SIZE {
                drive.cd.read(2, 1);
            }
            assert_eq!(drive.status() & 0x40, 0, "data FIFO drained");

            drive.select(0);
            drive.cd.write(3, 1, 0x00);
            drive.ack();
        }
    }

    #[test]
    fn dma_reads_whole_words_from_the_data_fifo() {
        let mut drive = Drive::with_disc(32);
        drive.run(0x02, &[0x00, 0x02, 0x03]);
        drive.run(0x06, &[]);
        assert_eq!(drive.wait_interrupt(), INT1_DATA_READY);
        drive.select(0);
        drive.cd.write(3, 1, 0x80);
        assert_eq!(drive.cd.dma_read_word(), 0x0000_A503);
        assert_eq!(drive.cd.dma_read_word(), 0, "next four payload bytes");
    }

    #[test]
    fn whole_sector_mode_returns_2340_bytes_after_the_sync() {
        let mut drive = Drive::with_disc(32);
        drive.run(0x0E, &[MODE_WHOLE_SECTOR]);
        drive.run(0x02, &[0x00, 0x02, 0x05]);
        drive.run(0x06, &[]);
        drive.wait_interrupt();
        drive.select(0);
        drive.cd.write(3, 1, 0x80);
        let mut bytes = Vec::new();
        while drive.status() & 0x40 != 0 {
            bytes.push(drive.cd.read(2, 1) as u8);
        }
        assert_eq!(bytes.len(), 2340);
        // MSF header of LBA 5 (00:02:05), then mode 2.
        assert_eq!(&bytes[..4], &[0x00, 0x02, 0x05, 0x02]);
    }

    #[test]
    fn reads_wait_for_the_previous_interrupt_to_be_acknowledged() {
        let mut drive = Drive::with_disc(32);
        drive.run(0x06, &[]);
        assert_eq!(drive.wait_interrupt(), INT1_DATA_READY);
        let position = drive.cd.current_sector;
        for _ in 0..(2 * 451_584) {
            drive.cd.step();
        }
        assert_eq!(drive.cd.current_sector, position, "no new sector while INT1 is pending");
        drive.ack();
        drive.wait_interrupt();
        assert_eq!(drive.cd.current_sector, position + 1);
    }

    #[test]
    fn double_speed_reads_twice_as_fast() {
        fn cycles_between_sectors(mode: u8) -> u64 {
            let mut drive = Drive::with_disc(32);
            drive.run(0x0E, &[mode]);
            drive.run(0x06, &[]);
            drive.wait_interrupt();
            drive.ack();
            let start = drive.cycles;
            drive.wait_interrupt();
            drive.cycles - start
        }
        let single = cycles_between_sectors(0);
        let double = cycles_between_sectors(MODE_DOUBLE_SPEED);
        assert!((single as i64 - 451_584).abs() < 50, "single speed: {single}");
        assert!((double as i64 - 225_792).abs() < 50, "double speed: {double}");
    }

    #[test]
    fn pause_stops_the_reading() {
        let mut drive = Drive::with_disc(32);
        drive.run(0x06, &[]);
        assert_eq!(drive.wait_interrupt(), INT1_DATA_READY);
        drive.ack();

        let (int, response) = drive.run(0x09, &[]);
        assert_eq!(int, INT3_ACKNOWLEDGE);
        assert_eq!(response, vec![STAT_MOTOR_ON | STAT_READING], "stat as of the command");
        assert_eq!(drive.wait_interrupt(), INT2_COMPLETE);
        assert_eq!(drive.response(), vec![STAT_MOTOR_ON], "no longer reading");
        drive.ack();

        let sector = drive.cd.current_sector;
        for _ in 0..(2 * 451_584) {
            drive.cd.step();
        }
        assert_eq!(drive.flag(), 0, "no more sectors");
        assert_eq!(drive.cd.current_sector, sector);
    }

    #[test]
    fn reading_past_the_end_signals_data_end() {
        let mut drive = Drive::with_disc(5);
        drive.run(0x02, &[0x00, 0x02, 0x04]); // last sector
        drive.run(0x06, &[]);
        assert_eq!(drive.wait_interrupt(), INT1_DATA_READY);
        drive.ack();
        assert_eq!(drive.wait_interrupt(), INT4_DATA_END);
        assert_eq!(drive.response(), vec![STAT_MOTOR_ON], "reading stopped");
    }

    #[test]
    fn getloc_l_reports_the_header_of_the_last_sector() {
        let mut drive = Drive::with_disc(32);
        assert_eq!(drive.run(0x10, &[]).0, INT5_ERROR, "nothing read yet");
        drive.run(0x02, &[0x00, 0x02, 0x07]);
        drive.run(0x06, &[]);
        drive.wait_interrupt();
        drive.ack();
        let (int, header) = drive.run(0x10, &[]);
        assert_eq!(int, INT3_ACKNOWLEDGE);
        assert_eq!(&header[..4], &[0x00, 0x02, 0x07, 0x02]);
        assert_eq!(header.len(), 8);
    }

    #[test]
    fn getloc_p_reports_track_and_positions() {
        let mut drive = Drive::with_disc(32);
        drive.run(0x02, &[0x00, 0x02, 0x07]);
        drive.run(0x06, &[]);
        drive.wait_interrupt();
        drive.ack();
        let (_, info) = drive.run(0x11, &[]);
        // Track 1, index 1, relative 00:00:07, absolute 00:02:07.
        assert_eq!(info, vec![0x01, 0x01, 0x00, 0x00, 0x07, 0x00, 0x02, 0x07]);
    }

    #[test]
    fn commands_are_logged() {
        let mut drive = Drive::with_disc(8);
        drive.run(0x02, &[0x00, 0x02, 0x00]);
        drive.run(0x01, &[]);
        assert_eq!(drive.cd.command_count, 2);
        assert_eq!(
            drive.cd.command_log,
            vec![
                CommandRecord { command: 0x02, parameters: vec![0x00, 0x02, 0x00] },
                CommandRecord { command: 0x01, parameters: vec![] },
            ]
        );
    }

    #[test]
    fn reset_keeps_the_disc_but_clears_the_controller() {
        let mut drive = Drive::with_disc(8);
        drive.run(0x0E, &[0xFF]);
        drive.cd.reset();
        assert!(drive.cd.disc.is_some());
        assert_eq!(drive.cd.mode, 0);
        assert_eq!(drive.status(), 0x18);
        assert_eq!(drive.run(0x01, &[]).1, vec![STAT_MOTOR_ON]);
    }

    #[test]
    fn raw_images_can_be_inserted_directly() {
        let mut cd = CdRom::new();
        assert!(cd.insert_image(vec![0; 3]).is_err());
        cd.insert_image(vec![0; 6 * DATA_SECTOR_SIZE]).unwrap();
        assert_eq!(cd.total_sectors, 6);
    }
}
