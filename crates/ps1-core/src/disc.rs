//! Disc images.
//!
//! A [`Disc`] is a list of tracks backed by one or more byte sources. It hands
//! the CD-ROM controller raw 2352-byte sectors addressed by absolute LBA
//! (LBA 0 is the first sector of the first track, i.e. 00:02:00 on the
//! disc's clock).
//!
//! Supported layouts: a raw 2352-byte `.bin`, a cooked 2048-byte `.iso`, and
//! `.cue` sheets pointing at one or several such files.

use crate::types::{Ps1Error, Ps1Result};

/// Bytes in a raw CD sector (sync, header, subheader, data, EDC/ECC).
pub const RAW_SECTOR_SIZE: usize = 2352;
/// Bytes of user data in a Mode 1 / Mode 2 Form 1 sector.
pub const DATA_SECTOR_SIZE: usize = 2048;
pub const SECTORS_PER_SECOND: u32 = 75;
pub const SECTORS_PER_MINUTE: u32 = 60 * SECTORS_PER_SECOND;
/// The two-second lead-in: the clock reads 00:02:00 at LBA 0.
pub const LEAD_IN_SECTORS: u32 = 150;

/// 12-byte sync pattern that starts every raw data sector.
const SYNC: [u8; 12] = [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0];

/// A random-access byte store holding (part of) a disc image.
pub trait SectorSource {
    /// Total length in bytes.
    fn len(&self) -> u64;
    /// Fill `buf` from `offset`. Returns false if the range cannot be read.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> bool;
}

/// An image held in memory.
pub struct MemorySource(pub Vec<u8>);

impl SectorSource for MemorySource {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        let start = offset as usize;
        match self.0.get(start..start + buf.len()) {
            Some(src) => {
                buf.copy_from_slice(src);
                true
            }
            None => false,
        }
    }
}

/// An image read on demand from a file, so large discs are not loaded whole.
pub struct FileSource {
    file: std::fs::File,
    len: u64,
}

impl FileSource {
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self { file, len })
    }
}

impl SectorSource for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        use std::io::{Read, Seek, SeekFrom};
        self.file.seek(SeekFrom::Start(offset)).is_ok() && self.file.read_exact(buf).is_ok()
    }
}

/// How the sectors of a file are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectorFormat {
    /// 2352 bytes: sync + header + payload.
    Raw2352,
    /// 2048 bytes of user data only (cooked ISO).
    Cooked2048,
    /// 2336 bytes: Mode 2 payload without sync and header.
    Mode2_2336,
}

impl SectorFormat {
    fn size(self) -> u64 {
        match self {
            SectorFormat::Raw2352 => 2352,
            SectorFormat::Cooked2048 => 2048,
            SectorFormat::Mode2_2336 => 2336,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    Data,
    Audio,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    /// 1-based track number.
    pub number: u8,
    pub kind: TrackKind,
    /// LBA of the track's INDEX 01.
    pub start_lba: u32,
    /// LBA where the track's pregap (INDEX 00) starts; `start_lba` when it has none.
    pub pregap_lba: u32,
}

/// A contiguous run of sectors on the disc: part of a file, or silence for a
/// gap (a CUE `PREGAP` or `POSTGAP`) that the image does not contain.
struct Segment {
    /// Index into `Disc::sources`; `None` for silence.
    source: Option<usize>,
    format: SectorFormat,
    /// What the sectors are, for the header a silent data sector needs.
    kind: TrackKind,
    /// LBA of the segment's first sector.
    first_lba: u32,
    sectors: u32,
    /// Sector of the file where the segment starts.
    file_sector: u32,
}

/// Replacement subchannel Q data for one sector, from a `.sbi` file (the
/// copy protection of some discs lives in damaged Q data).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SbiEntry {
    lba: u32,
    /// Control/ADR, track, index, relative MSF, a zero byte, absolute MSF (BCD).
    q: [u8; 10],
}

pub struct Disc {
    sources: Vec<Box<dyn SectorSource>>,
    segments: Vec<Segment>,
    tracks: Vec<Track>,
    total_sectors: u32,
    sbi: Vec<SbiEntry>,
}

/// The subchannel Q data of a sector: where the head is on the disc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubQ {
    /// Control nibble in the high half (4 = data track), ADR in the low half.
    pub control: u8,
    pub track: u8,
    /// 0 in a pregap, 1 in the track proper.
    pub index: u8,
    /// Position within the track, in sectors: it counts down through a pregap.
    pub relative: u32,
    /// Position on the disc's clock (the lead-in's two seconds included).
    pub absolute: u32,
}

/// Region of a licensed disc, as reported by GetID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// Japan (SCEI).
    Japan,
    /// North America (SCEA).
    NorthAmerica,
    /// Europe (SCEE).
    Europe,
}

impl Region {
    /// The four ASCII letters GetID returns.
    pub fn id(self) -> [u8; 4] {
        match self {
            Region::Japan => *b"SCEI",
            Region::NorthAmerica => *b"SCEA",
            Region::Europe => *b"SCEE",
        }
    }
}

impl Disc {
    /// A single-track data disc from a raw image: 2352-byte sectors if the
    /// size allows, otherwise 2048-byte (ISO).
    pub fn from_image(data: Vec<u8>) -> Ps1Result<Disc> {
        Self::from_image_source(Box::new(MemorySource(data)))
    }

    /// Like [`Disc::from_image`] for an image read on demand from any source.
    pub fn from_image_source(source: Box<dyn SectorSource>) -> Ps1Result<Disc> {
        let len = source.len();
        let format = if len % RAW_SECTOR_SIZE as u64 == 0 && len != 0 {
            SectorFormat::Raw2352
        } else if len % DATA_SECTOR_SIZE as u64 == 0 && len != 0 {
            SectorFormat::Cooked2048
        } else {
            return Err(Ps1Error::InvalidDiscFormat(format!(
                "{len} bytes is not a whole number of 2352- or 2048-byte sectors"
            )));
        };
        Self::single_track(source, format)
    }

    /// A single-track data disc from any source.
    pub fn single_track(source: Box<dyn SectorSource>, format: SectorFormat) -> Ps1Result<Disc> {
        let sectors = (source.len() / format.size()) as u32;
        if sectors == 0 {
            return Err(Ps1Error::InvalidDiscFormat("empty image".into()));
        }
        Ok(Disc {
            sources: vec![source],
            segments: vec![Segment {
                source: Some(0),
                format,
                kind: TrackKind::Data,
                first_lba: 0,
                sectors,
                file_sector: 0,
            }],
            tracks: vec![Track { number: 1, kind: TrackKind::Data, start_lba: 0, pregap_lba: 0 }],
            total_sectors: sectors,
            sbi: Vec::new(),
        })
    }

    /// Build a disc from a CUE sheet. `open` loads the file named in each
    /// `FILE` entry (relative to wherever the sheet lives).
    ///
    /// `INDEX 00` marks a pregap that is part of the file; `PREGAP` and
    /// `POSTGAP` add silent sectors the file does not hold, which shift
    /// everything after them (the track's INDEX 01 lands after its pregap).
    pub fn from_cue(
        cue: &str,
        mut open: impl FnMut(&str) -> Result<Box<dyn SectorSource>, String>,
    ) -> Ps1Result<Disc> {
        let invalid = |msg: String| Ps1Error::InvalidDiscFormat(msg);

        struct PendingTrack {
            number: u8,
            kind: TrackKind,
            format: SectorFormat,
            /// Sectors into the file.
            index00: Option<u32>,
            index01: Option<u32>,
            pregap: u32,
            postgap: u32,
        }
        // (file name, tracks declared in it)
        let mut files: Vec<(String, Vec<PendingTrack>)> = Vec::new();

        for line in cue.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let upper = line.to_ascii_uppercase();
            if upper.starts_with("FILE") {
                let start = line.find('"').ok_or_else(|| invalid("FILE without quotes".into()))? + 1;
                let end = line.rfind('"').filter(|e| *e >= start).ok_or_else(|| invalid("FILE without quotes".into()))?;
                files.push((line[start..end].to_string(), Vec::new()));
            } else if upper.starts_with("TRACK") {
                let mut parts = line.split_whitespace().skip(1);
                let number: u8 = parts
                    .next()
                    .and_then(|n| n.parse().ok())
                    .ok_or_else(|| invalid("bad TRACK number".into()))?;
                let (kind, format) = match parts.next().map(str::to_ascii_uppercase).as_deref() {
                    Some("AUDIO") => (TrackKind::Audio, SectorFormat::Raw2352),
                    Some("MODE1/2352") | Some("MODE2/2352") => (TrackKind::Data, SectorFormat::Raw2352),
                    Some("MODE1/2048") => (TrackKind::Data, SectorFormat::Cooked2048),
                    Some("MODE2/2336") => (TrackKind::Data, SectorFormat::Mode2_2336),
                    other => return Err(invalid(format!("unsupported track mode {other:?}"))),
                };
                let file = files.last_mut().ok_or_else(|| invalid("TRACK before FILE".into()))?;
                file.1.push(PendingTrack { number, kind, format, index00: None, index01: None, pregap: 0, postgap: 0 });
            } else if upper.starts_with("INDEX") {
                let mut parts = line.split_whitespace().skip(1);
                let index: u32 = parts.next().and_then(|n| n.parse().ok()).ok_or_else(|| invalid("bad INDEX".into()))?;
                let msf = parts.next().ok_or_else(|| invalid("INDEX without time".into()))?;
                if index <= 1 {
                    let frames = parse_cue_time(msf).ok_or_else(|| invalid(format!("bad INDEX time {msf}")))?;
                    let track = files
                        .last_mut()
                        .and_then(|f| f.1.last_mut())
                        .ok_or_else(|| invalid("INDEX before TRACK".into()))?;
                    if index == 0 {
                        track.index00 = Some(frames);
                    } else {
                        track.index01 = Some(frames);
                    }
                }
            } else if upper.starts_with("PREGAP") || upper.starts_with("POSTGAP") {
                let msf = line.split_whitespace().nth(1).ok_or_else(|| invalid("gap without a length".into()))?;
                let frames = parse_cue_time(msf).ok_or_else(|| invalid(format!("bad gap length {msf}")))?;
                let track = files
                    .last_mut()
                    .and_then(|f| f.1.last_mut())
                    .ok_or_else(|| invalid("gap before TRACK".into()))?;
                if upper.starts_with("PREGAP") {
                    track.pregap = frames;
                } else {
                    track.postgap = frames;
                }
            }
            // REM, CATALOG, FLAGS... carry nothing we need.
        }

        let mut sources: Vec<Box<dyn SectorSource>> = Vec::new();
        let mut segments = Vec::new();
        let mut tracks = Vec::new();
        let mut next_lba = 0u32;
        for (name, pending) in files {
            let first = pending
                .first()
                .ok_or_else(|| invalid(format!("FILE {name} has no tracks")))?;
            let format = first.format;
            if pending.iter().any(|t| t.format != format) {
                return Err(invalid(format!("FILE {name} mixes sector sizes")));
            }
            let source = open(&name).map_err(|e| invalid(format!("{name}: {e}")))?;
            let file_sectors = (source.len() / format.size()) as u32;
            let source_index = sources.len();
            sources.push(source);

            let mut cursor = 0u32; // next unplaced sector of the file
            for (i, track) in pending.iter().enumerate() {
                let index01 = track.index01.ok_or_else(|| invalid(format!("track {} has no INDEX 01", track.number)))?;
                // The track's part of the file runs to where the next track's pregap
                // (or, without one, the next track) starts.
                let end = pending
                    .get(i + 1)
                    .map(|next| next.index00.or(next.index01).unwrap_or(file_sectors))
                    .unwrap_or(file_sectors)
                    .min(file_sectors);
                let pregap_start = next_lba;
                if track.pregap > 0 {
                    segments.push(silence(track.kind, next_lba, track.pregap));
                    next_lba += track.pregap;
                }
                let begin = cursor.min(end);
                let in_file_pregap = track.index00.map_or(0, |index00| index01.saturating_sub(index00.max(begin)));
                let start_lba = next_lba + index01.saturating_sub(begin);
                let pregap_lba = if track.pregap > 0 {
                    pregap_start
                } else {
                    start_lba - in_file_pregap
                };
                tracks.push(Track { number: track.number, kind: track.kind, start_lba, pregap_lba });
                let length = end - begin;
                if length > 0 {
                    segments.push(Segment {
                        source: Some(source_index),
                        format,
                        kind: track.kind,
                        first_lba: next_lba,
                        sectors: length,
                        file_sector: begin,
                    });
                    next_lba += length;
                }
                cursor = end;
                if track.postgap > 0 {
                    segments.push(silence(track.kind, next_lba, track.postgap));
                    next_lba += track.postgap;
                }
            }
        }
        if tracks.is_empty() {
            return Err(invalid("the CUE sheet has no tracks".into()));
        }
        Ok(Disc { sources, segments, tracks, total_sectors: next_lba, sbi: Vec::new() })
    }

    /// Build a disc from a CloneCD control file (`.ccd`) and its image (`.img`,
    /// raw 2352-byte sectors from the start of the disc). The sub-channel file
    /// (`.sub`) is not used; `[TRACK n]` entries give each track's mode and indexes.
    pub fn from_ccd(ccd: &str, image: Box<dyn SectorSource>) -> Ps1Result<Disc> {
        let invalid = |msg: String| Ps1Error::InvalidDiscFormat(msg);
        let sectors = (image.len() / RAW_SECTOR_SIZE as u64) as u32;
        if sectors == 0 || image.len() % RAW_SECTOR_SIZE as u64 != 0 {
            return Err(invalid("a CloneCD image holds whole 2352-byte sectors".into()));
        }
        struct Entry {
            number: u8,
            mode: u32,
            index0: Option<u32>,
            index1: Option<u32>,
        }
        let mut entries: Vec<Entry> = Vec::new();
        for line in ccd.lines().map(str::trim) {
            let upper = line.to_ascii_uppercase();
            if let Some(rest) = upper.strip_prefix("[TRACK ") {
                let number = rest
                    .trim_end_matches(']')
                    .trim()
                    .parse()
                    .map_err(|_| invalid(format!("bad track header {line}")))?;
                entries.push(Entry { number, mode: 1, index0: None, index1: None });
            } else if let Some((key, value)) = upper.split_once('=') {
                let Some(entry) = entries.last_mut() else { continue };
                let value: Option<u32> = value.trim().parse().ok();
                match key.trim() {
                    "MODE" => entry.mode = value.unwrap_or(1),
                    "INDEX 0" => entry.index0 = value,
                    "INDEX 1" => entry.index1 = value,
                    _ => {}
                }
            }
        }
        let mut tracks = Vec::new();
        for entry in &entries {
            let index1 = entry
                .index1
                .ok_or_else(|| invalid(format!("track {} has no INDEX 1", entry.number)))?;
            let kind = if entry.mode == 0 { TrackKind::Audio } else { TrackKind::Data };
            let pregap_lba = entry.index0.filter(|&i0| i0 < index1).unwrap_or(index1);
            tracks.push(Track { number: entry.number, kind, start_lba: index1, pregap_lba });
        }
        if tracks.is_empty() {
            return Err(invalid("the CCD file lists no tracks".into()));
        }
        Ok(Disc {
            sources: vec![image],
            segments: vec![Segment {
                source: Some(0),
                format: SectorFormat::Raw2352,
                kind: tracks[0].kind,
                first_lba: 0,
                sectors,
                file_sector: 0,
            }],
            tracks,
            total_sectors: sectors,
            sbi: Vec::new(),
        })
    }

    /// Apply the replacement subchannel data of a `.sbi` file: the drive then
    /// reports those sectors' Q data as the file says (the protection of some
    /// European discs reads them back).
    pub fn load_sbi(&mut self, data: &[u8]) -> Ps1Result<()> {
        let invalid = |msg: &str| Ps1Error::InvalidDiscFormat(msg.into());
        if data.len() < 4 || &data[..4] != b"SBI\0" {
            return Err(invalid("not an SBI file"));
        }
        let mut entries = Vec::new();
        for entry in data[4..].chunks(14) {
            if entry.len() < 14 {
                return Err(invalid("truncated SBI entry"));
            }
            let bcd = |b: u8| from_bcd(b).ok_or_else(|| invalid("bad time in an SBI entry"));
            let frames = (bcd(entry[0])? as u32) * SECTORS_PER_MINUTE
                + (bcd(entry[1])? as u32) * SECTORS_PER_SECOND
                + bcd(entry[2])? as u32;
            // Entry type 1: the 10 bytes of Q data follow.
            if entry[3] != 1 {
                return Err(invalid("unsupported SBI entry type"));
            }
            let mut q = [0u8; 10];
            q.copy_from_slice(&entry[4..14]);
            entries.push(SbiEntry { lba: frames.saturating_sub(LEAD_IN_SECTORS), q });
        }
        entries.sort_by_key(|e| e.lba);
        self.sbi = entries;
        Ok(())
    }

    pub fn total_sectors(&self) -> u32 {
        self.total_sectors
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// The track a sector belongs to; a pregap belongs to the track it leads into.
    pub fn track_at(&self, lba: u32) -> &Track {
        self.tracks
            .iter()
            .rev()
            .find(|t| t.pregap_lba <= lba)
            .unwrap_or(&self.tracks[0])
    }

    /// What the drive reads from the subchannel Q of the sector at `lba`.
    pub fn subchannel_q(&self, lba: u32) -> SubQ {
        if let Ok(position) = self.sbi.binary_search_by_key(&lba, |e| e.lba) {
            let q = self.sbi[position].q;
            let bcd = |b: u8| from_bcd(b).unwrap_or(0) as u32;
            let time = |m: u8, s: u8, f: u8| bcd(m) * SECTORS_PER_MINUTE + bcd(s) * SECTORS_PER_SECOND + bcd(f);
            return SubQ {
                control: q[0],
                track: from_bcd(q[1]).unwrap_or(0),
                index: from_bcd(q[2]).unwrap_or(0),
                relative: time(q[3], q[4], q[5]),
                absolute: time(q[7], q[8], q[9]),
            };
        }
        let track = self.track_at(lba);
        let control = match track.kind {
            TrackKind::Data => 0x41,
            TrackKind::Audio => 0x01,
        };
        let (index, relative) = if lba >= track.start_lba {
            (1, lba - track.start_lba)
        } else {
            // In the pregap the time counts down to the start of the track.
            (0, track.start_lba - lba - 1)
        };
        SubQ { control, track: track.number, index, relative, absolute: lba + LEAD_IN_SECTORS }
    }

    /// Read the raw 2352-byte sector at `lba`. Cooked sectors get a
    /// synthesized sync/header so the controller sees a uniform layout, and gaps
    /// read as silence. Returns false when `lba` is outside the disc or the
    /// source fails.
    pub fn read_raw_sector(&mut self, lba: u32, out: &mut [u8; RAW_SECTOR_SIZE]) -> bool {
        let Some(segment) = self
            .segments
            .iter()
            .find(|s| lba >= s.first_lba && lba < s.first_lba + s.sectors)
        else {
            return false;
        };
        let Some(source_index) = segment.source else {
            out.fill(0);
            if segment.kind == TrackKind::Data {
                write_header(lba, out);
            }
            return true;
        };
        let format = segment.format;
        let offset = (segment.file_sector + lba - segment.first_lba) as u64 * format.size();
        let source = &mut self.sources[source_index];
        match format {
            SectorFormat::Raw2352 => source.read_at(offset, out),
            SectorFormat::Cooked2048 => {
                let mut data = [0u8; DATA_SECTOR_SIZE];
                if !source.read_at(offset, &mut data) {
                    return false;
                }
                out.fill(0);
                write_header(lba, out);
                // Mode 2 Form 1: 8 subheader bytes (zero), then the data.
                out[24..24 + DATA_SECTOR_SIZE].copy_from_slice(&data);
                true
            }
            SectorFormat::Mode2_2336 => {
                let mut payload = [0u8; 2336];
                if !source.read_at(offset, &mut payload) {
                    return false;
                }
                out.fill(0);
                write_header(lba, out);
                out[16..].copy_from_slice(&payload);
                true
            }
        }
    }

    /// Identify the disc's region from the licence text in the system area.
    pub fn region(&mut self) -> Option<Region> {
        let mut sector = [0u8; RAW_SECTOR_SIZE];
        if !self.read_raw_sector(4, &mut sector) {
            return None;
        }
        let contains = |needle: &[u8]| sector.windows(needle.len()).any(|w| w == needle);
        if !contains(b"Sony Computer Entertainment") {
            return None;
        }
        if contains(b"Amer") {
            Some(Region::NorthAmerica)
        } else if contains(b"Euro") {
            Some(Region::Europe)
        } else if contains(b"Inc.") {
            Some(Region::Japan)
        } else {
            None
        }
    }
}

fn silence(kind: TrackKind, first_lba: u32, sectors: u32) -> Segment {
    Segment { source: None, format: SectorFormat::Raw2352, kind, first_lba, sectors, file_sector: 0 }
}

/// Sync pattern plus the BCD minute/second/frame header and mode byte.
fn write_header(lba: u32, out: &mut [u8; RAW_SECTOR_SIZE]) {
    out[..12].copy_from_slice(&SYNC);
    let (m, s, f) = lba_to_msf(lba + LEAD_IN_SECTORS);
    out[12] = to_bcd(m);
    out[13] = to_bcd(s);
    out[14] = to_bcd(f);
    out[15] = 2; // Mode 2
}

fn parse_cue_time(text: &str) -> Option<u32> {
    let mut parts = text.split(':').map(|p| p.parse::<u32>().ok());
    let (m, s, f) = (parts.next()??, parts.next()??, parts.next()??);
    (parts.next().is_none() && s < 60 && f < 75).then(|| m * SECTORS_PER_MINUTE + s * SECTORS_PER_SECOND + f)
}

/// Split a sector count (an absolute position) into minute, second, frame.
pub fn lba_to_msf(frames: u32) -> (u8, u8, u8) {
    (
        (frames / SECTORS_PER_MINUTE) as u8,
        ((frames / SECTORS_PER_SECOND) % 60) as u8,
        (frames % SECTORS_PER_SECOND) as u8,
    )
}

pub fn to_bcd(n: u8) -> u8 {
    ((n / 10) << 4) | (n % 10)
}

pub fn from_bcd(n: u8) -> Option<u8> {
    let (hi, lo) = (n >> 4, n & 0xF);
    (hi < 10 && lo < 10).then_some(hi * 10 + lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cooked image of `sectors` sectors whose first byte is the sector number.
    fn iso(sectors: u32) -> Vec<u8> {
        let mut data = vec![0u8; sectors as usize * DATA_SECTOR_SIZE];
        for s in 0..sectors as usize {
            data[s * DATA_SECTOR_SIZE] = s as u8;
        }
        data
    }

    /// A raw image of `sectors` sectors whose payload byte 24 is the sector number.
    fn raw(sectors: u32) -> Vec<u8> {
        let mut data = vec![0u8; sectors as usize * RAW_SECTOR_SIZE];
        for s in 0..sectors as usize {
            data[s * RAW_SECTOR_SIZE..s * RAW_SECTOR_SIZE + 12].copy_from_slice(&SYNC);
            data[s * RAW_SECTOR_SIZE + 24] = s as u8;
        }
        data
    }

    #[test]
    fn bcd_and_msf_helpers() {
        assert_eq!(to_bcd(59), 0x59);
        assert_eq!(from_bcd(0x59), Some(59));
        assert_eq!(from_bcd(0x5A), None);
        assert_eq!(lba_to_msf(150), (0, 2, 0));
        assert_eq!(lba_to_msf(75 * 60 * 3 + 75 * 4 + 5), (3, 4, 5));
        assert_eq!(parse_cue_time("00:02:00"), Some(150));
        assert_eq!(parse_cue_time("01:00:74"), Some(75 * 60 + 74));
        assert_eq!(parse_cue_time("00:60:00"), None);
        assert_eq!(parse_cue_time("00:00"), None);
    }

    #[test]
    fn raw_images_are_served_verbatim() {
        let mut disc = Disc::from_image(raw(4)).unwrap();
        assert_eq!(disc.total_sectors(), 4);
        let mut sector = [0u8; RAW_SECTOR_SIZE];
        assert!(disc.read_raw_sector(2, &mut sector));
        assert_eq!(sector[24], 2);
        assert_eq!(&sector[..12], &SYNC);
        assert!(!disc.read_raw_sector(4, &mut sector), "past the end");
    }

    #[test]
    fn cooked_images_get_a_synthesized_header() {
        let mut disc = Disc::from_image(iso(3)).unwrap();
        let mut sector = [0u8; RAW_SECTOR_SIZE];
        assert!(disc.read_raw_sector(1, &mut sector));
        assert_eq!(&sector[..12], &SYNC);
        // LBA 1 is 00:02:01 on the disc clock.
        assert_eq!(&sector[12..16], &[0x00, 0x02, 0x01, 0x02]);
        assert_eq!(sector[24], 1, "user data sits after the subheader");
    }

    #[test]
    fn odd_sizes_are_rejected() {
        assert!(matches!(
            Disc::from_image(vec![0; 1000]),
            Err(Ps1Error::InvalidDiscFormat(_))
        ));
        assert!(Disc::from_image(Vec::new()).is_err());
    }

    #[test]
    fn single_file_cue() {
        let cue = "FILE \"game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n";
        let mut disc = Disc::from_cue(cue, |name| {
            assert_eq!(name, "game.bin");
            Ok(Box::new(MemorySource(raw(5))))
        })
        .unwrap();
        assert_eq!(disc.total_sectors(), 5);
        assert_eq!(disc.tracks().len(), 1);
        assert_eq!(disc.tracks()[0].kind, TrackKind::Data);
        let mut sector = [0u8; RAW_SECTOR_SIZE];
        assert!(disc.read_raw_sector(3, &mut sector));
        assert_eq!(sector[24], 3);
    }

    #[test]
    fn multi_file_cue_places_tracks_back_to_back() {
        let cue = "FILE \"data.bin\" BINARY\n TRACK 01 MODE2/2352\n  INDEX 01 00:00:00\n\
                   FILE \"audio.bin\" BINARY\n TRACK 02 AUDIO\n  INDEX 00 00:00:00\n  INDEX 01 00:00:02\n";
        let mut disc = Disc::from_cue(cue, |name| {
            Ok(Box::new(MemorySource(raw(if name == "data.bin" { 10 } else { 20 }))))
        })
        .unwrap();
        assert_eq!(disc.total_sectors(), 30);
        assert_eq!(disc.tracks()[1], Track { number: 2, kind: TrackKind::Audio, start_lba: 12, pregap_lba: 10 });
        assert_eq!(disc.track_at(5).number, 1);
        assert_eq!(disc.track_at(9).number, 1);
        assert_eq!(disc.track_at(11).number, 2, "track 2's pregap belongs to track 2");
        assert_eq!(disc.track_at(12).number, 2);

        // LBA 13 is sector 3 of the second file.
        let mut sector = [0u8; RAW_SECTOR_SIZE];
        assert!(disc.read_raw_sector(13, &mut sector));
        assert_eq!(sector[24], 3);
    }

    #[test]
    fn bad_cue_sheets_are_rejected() {
        let open = |_: &str| -> Result<Box<dyn SectorSource>, String> { Ok(Box::new(MemorySource(raw(1)))) };
        assert!(Disc::from_cue("", open).is_err());
        assert!(Disc::from_cue("TRACK 01 MODE2/2352\n", open).is_err());
        assert!(Disc::from_cue("FILE \"a.bin\" BINARY\n TRACK 01 MODE9/9999\n", open).is_err());
        assert!(Disc::from_cue("FILE \"a.bin\" BINARY\n TRACK 01 MODE2/2352\n", open).is_err());
        let missing = |_: &str| -> Result<Box<dyn SectorSource>, String> { Err("not found".into()) };
        assert!(Disc::from_cue("FILE \"a.bin\" BINARY\n TRACK 01 MODE2/2352\n INDEX 01 00:00:00\n", missing).is_err());
    }

    #[test]
    fn region_comes_from_the_licence_sector() {
        for (text, expected) in [
            (&b"          Licensed  by          Sony Computer Entertainment Amer  ica "[..], Some(Region::NorthAmerica)),
            (&b"          Licensed  by          Sony Computer Entertainment Euro  pe   "[..], Some(Region::Europe)),
            (&b"          Licensed  by          Sony Computer Entertainment Inc."[..], Some(Region::Japan)),
            (&b"no licence here"[..], None),
        ] {
            let mut data = iso(8);
            data[4 * DATA_SECTOR_SIZE..4 * DATA_SECTOR_SIZE + text.len()].copy_from_slice(text);
            let mut disc = Disc::from_image(data).unwrap();
            assert_eq!(disc.region(), expected);
        }
        assert_eq!(Region::NorthAmerica.id(), *b"SCEA");
    }

    #[test]
    fn region_is_none_on_a_tiny_disc() {
        let mut disc = Disc::from_image(iso(2)).unwrap();
        assert_eq!(disc.region(), None);
    }

    // ----- Gaps, subchannel, CloneCD and SBI -----

    fn open_raw(sectors: &'static [u32]) -> impl FnMut(&str) -> Result<Box<dyn SectorSource>, String> {
        let mut next = 0;
        move |_| {
            let count = sectors[next];
            next += 1;
            Ok(Box::new(MemorySource(raw(count))) as Box<dyn SectorSource>)
        }
    }

    #[test]
    fn a_pregap_command_adds_silent_sectors_before_the_track() {
        let cue = "FILE \"d.bin\" BINARY
 TRACK 01 MODE2/2352
  INDEX 01 00:00:00
                   FILE \"a.bin\" BINARY
 TRACK 02 AUDIO
  PREGAP 00:00:02
  INDEX 01 00:00:00
";
        let mut disc = Disc::from_cue(cue, open_raw(&[10, 5])).unwrap();
        assert_eq!(disc.total_sectors(), 17, "the file has 5 sectors, the gap adds 2");
        assert_eq!(disc.tracks()[1], Track { number: 2, kind: TrackKind::Audio, start_lba: 12, pregap_lba: 10 });
        let mut sector = [0xAAu8; RAW_SECTOR_SIZE];
        assert!(disc.read_raw_sector(10, &mut sector));
        assert!(sector.iter().all(|b| *b == 0), "audio silence");
        assert!(disc.read_raw_sector(12, &mut sector));
        assert_eq!(sector[24], 0, "the file's first sector comes after the gap");
        assert!(disc.read_raw_sector(14, &mut sector));
        assert_eq!(sector[24], 2);
    }

    #[test]
    fn a_postgap_command_adds_silent_sectors_after_the_track() {
        let cue = "FILE \"d.bin\" BINARY
 TRACK 01 MODE2/2352
  INDEX 01 00:00:00
  POSTGAP 00:00:03
";
        let mut disc = Disc::from_cue(cue, open_raw(&[4])).unwrap();
        assert_eq!(disc.total_sectors(), 7);
        let mut sector = [0xAAu8; RAW_SECTOR_SIZE];
        assert!(disc.read_raw_sector(5, &mut sector));
        assert_eq!(&sector[..12], &SYNC, "a data gap keeps its header");
        assert!(sector[24..].iter().all(|b| *b == 0));
    }

    #[test]
    fn one_file_with_several_tracks_keeps_the_pregap_inside_it() {
        let cue = "FILE \"all.bin\" BINARY
 TRACK 01 MODE2/2352
  INDEX 01 00:00:00
                   TRACK 02 AUDIO
  INDEX 00 00:00:20
  INDEX 01 00:00:22
";
        let mut disc = Disc::from_cue(cue, open_raw(&[30])).unwrap();
        assert_eq!(disc.total_sectors(), 30);
        assert_eq!(disc.tracks()[1].start_lba, 22);
        assert_eq!(disc.tracks()[1].pregap_lba, 20);
        let mut sector = [0u8; RAW_SECTOR_SIZE];
        assert!(disc.read_raw_sector(25, &mut sector));
        assert_eq!(sector[24], 25, "sectors keep their place in the file");
    }

    #[test]
    fn the_subchannel_counts_down_through_a_pregap() {
        let cue = "FILE \"all.bin\" BINARY
 TRACK 01 MODE2/2352
  INDEX 01 00:00:00
                   TRACK 02 AUDIO
  INDEX 00 00:00:20
  INDEX 01 00:00:22
";
        let disc = Disc::from_cue(cue, open_raw(&[30])).unwrap();
        let q = disc.subchannel_q(5);
        assert_eq!((q.track, q.index, q.relative, q.control), (1, 1, 5, 0x41));
        assert_eq!(q.absolute, 5 + LEAD_IN_SECTORS);
        let q = disc.subchannel_q(20);
        assert_eq!((q.track, q.index, q.relative, q.control), (2, 0, 1, 0x01));
        let q = disc.subchannel_q(21);
        assert_eq!((q.track, q.index, q.relative), (2, 0, 0), "the last pregap sector reads 00:00:00");
        let q = disc.subchannel_q(22);
        assert_eq!((q.track, q.index, q.relative), (2, 1, 0));
    }

    #[test]
    fn a_ccd_file_gives_the_tracks_of_a_raw_image() {
        let ccd = "[CloneCD]
Version=3
[Disc]
TocEntries=4
                   [TRACK 1]
MODE=2
INDEX 1=0
[TRACK 2]
MODE=0
INDEX 0=20
INDEX 1=22
";
        let mut disc = Disc::from_ccd(ccd, Box::new(MemorySource(raw(30)))).unwrap();
        assert_eq!(disc.total_sectors(), 30);
        assert_eq!(disc.tracks()[0].kind, TrackKind::Data);
        assert_eq!(disc.tracks()[1], Track { number: 2, kind: TrackKind::Audio, start_lba: 22, pregap_lba: 20 });
        let mut sector = [0u8; RAW_SECTOR_SIZE];
        assert!(disc.read_raw_sector(7, &mut sector));
        assert_eq!(sector[24], 7);
        assert!(Disc::from_ccd("[CloneCD]
", Box::new(MemorySource(raw(3)))).is_err(), "no tracks");
        assert!(Disc::from_ccd(ccd, Box::new(MemorySource(vec![0; 100]))).is_err(), "not whole sectors");
    }

    #[test]
    fn an_sbi_file_replaces_the_q_data_of_the_sectors_it_lists() {
        let mut disc = Disc::from_image(raw(20)).unwrap();
        let mut sbi = b"SBI ".to_vec();
        // 00:02:10 is LBA 10; type 1; Q: control 41, track 01, index 01, rel 00:00:10,
        // zero, abs 00:02:10.
        sbi.extend([0x00, 0x02, 0x10, 0x01, 0x41, 0x01, 0x01, 0x00, 0x00, 0x10, 0x00, 0x00, 0x02, 0x99]);
        disc.load_sbi(&sbi).unwrap();
        let q = disc.subchannel_q(10);
        assert_eq!((q.control, q.track, q.index), (0x41, 1, 1));
        assert_eq!(q.relative, 10);
        assert_eq!(q.absolute, 2 * 75 + 99, "the file's value wins even when it is not what a clean disc says");
        assert_eq!(disc.subchannel_q(11).absolute, 11 + LEAD_IN_SECTORS, "other sectors are untouched");
        assert!(disc.load_sbi(b"nope").is_err());
        assert!(disc.load_sbi(b"SBI  ").is_err(), "truncated entry");
    }
}
