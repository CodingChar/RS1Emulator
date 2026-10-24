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
}

/// One file of the image and where it sits on the disc.
struct Extent {
    source: Box<dyn SectorSource>,
    format: SectorFormat,
    /// LBA of the file's first sector.
    first_lba: u32,
    sectors: u32,
}

pub struct Disc {
    extents: Vec<Extent>,
    tracks: Vec<Track>,
    total_sectors: u32,
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
            extents: vec![Extent { source, format, first_lba: 0, sectors }],
            tracks: vec![Track { number: 1, kind: TrackKind::Data, start_lba: 0 }],
            total_sectors: sectors,
        })
    }

    /// Build a disc from a CUE sheet. `open` loads the file named in each
    /// `FILE` entry (relative to wherever the sheet lives).
    pub fn from_cue(
        cue: &str,
        mut open: impl FnMut(&str) -> Result<Box<dyn SectorSource>, String>,
    ) -> Ps1Result<Disc> {
        let invalid = |msg: String| Ps1Error::InvalidDiscFormat(msg);

        struct PendingTrack {
            number: u8,
            kind: TrackKind,
            format: SectorFormat,
            index01: Option<u32>,
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
                file.1.push(PendingTrack { number, kind, format, index01: None });
            } else if upper.starts_with("INDEX") {
                let mut parts = line.split_whitespace().skip(1);
                let index: u32 = parts.next().and_then(|n| n.parse().ok()).ok_or_else(|| invalid("bad INDEX".into()))?;
                let msf = parts.next().ok_or_else(|| invalid("INDEX without time".into()))?;
                if index == 1 {
                    let frames = parse_cue_time(msf).ok_or_else(|| invalid(format!("bad INDEX time {msf}")))?;
                    let track = files
                        .last_mut()
                        .and_then(|f| f.1.last_mut())
                        .ok_or_else(|| invalid("INDEX before TRACK".into()))?;
                    track.index01 = Some(frames);
                }
            }
            // PREGAP, POSTGAP, REM, CATALOG, FLAGS... carry nothing we need.
        }

        let mut extents = Vec::new();
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
            let sectors = (source.len() / format.size()) as u32;
            for track in &pending {
                let index01 = track.index01.ok_or_else(|| invalid(format!("track {} has no INDEX 01", track.number)))?;
                tracks.push(Track { number: track.number, kind: track.kind, start_lba: next_lba + index01 });
            }
            extents.push(Extent { source, format, first_lba: next_lba, sectors });
            next_lba += sectors;
        }
        if tracks.is_empty() {
            return Err(invalid("the CUE sheet has no tracks".into()));
        }
        Ok(Disc { extents, tracks, total_sectors: next_lba })
    }

    pub fn total_sectors(&self) -> u32 {
        self.total_sectors
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// The track a sector belongs to.
    pub fn track_at(&self, lba: u32) -> &Track {
        self.tracks
            .iter()
            .rev()
            .find(|t| t.start_lba <= lba)
            .unwrap_or(&self.tracks[0])
    }

    /// Read the raw 2352-byte sector at `lba`. Cooked sectors get a
    /// synthesized sync/header so the controller sees a uniform layout.
    /// Returns false when `lba` is outside the disc or the source fails.
    pub fn read_raw_sector(&mut self, lba: u32, out: &mut [u8; RAW_SECTOR_SIZE]) -> bool {
        let Some(extent) = self
            .extents
            .iter_mut()
            .find(|e| lba >= e.first_lba && lba < e.first_lba + e.sectors)
        else {
            return false;
        };
        let offset = (lba - extent.first_lba) as u64 * extent.format.size();
        match extent.format {
            SectorFormat::Raw2352 => extent.source.read_at(offset, out),
            SectorFormat::Cooked2048 => {
                let mut data = [0u8; DATA_SECTOR_SIZE];
                if !extent.source.read_at(offset, &mut data) {
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
                if !extent.source.read_at(offset, &mut payload) {
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
        assert_eq!(disc.tracks()[1], Track { number: 2, kind: TrackKind::Audio, start_lba: 12 });
        assert_eq!(disc.track_at(5).number, 1);
        assert_eq!(disc.track_at(11).number, 1, "still in track 2's pregap");
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
}
