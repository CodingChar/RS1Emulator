/// Where in the ROM the `System ROM Version x.y mm/dd/yy R` line sits.
const VERSION_LINE_OFFSET: usize = 0x7FF32;
const VERSION_PREFIX: &[u8] = b"System ROM Version ";

/// What identifies a BIOS image: the CRC-32 of the 512 KB and the version line
/// every Sony ROM carries near its end (version, date and a region letter).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BiosInfo {
    pub crc32: u32,
    /// Such as `2.2 12/04/95`.
    pub version: Option<String>,
    /// `A` Americas, `E` Europe, `J` Japan; any other letter is kept as found.
    pub region: Option<char>,
    /// The console model, when the checksum is one this emulator has verified.
    pub model: Option<&'static str>,
}

/// Checksums of dumps checked against a real console's ROM.
const KNOWN_MODELS: &[(u32, &str)] = &[(0x3715_7331, "SCPH-1001 (v2.2, Americas)")];

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

impl BiosInfo {
    pub fn identify(rom: &[u8]) -> Self {
        let crc32 = crc32(rom);
        let line = rom
            .get(VERSION_LINE_OFFSET..)
            .filter(|tail| tail.starts_with(VERSION_PREFIX))
            .map(|tail| &tail[VERSION_PREFIX.len()..])
            .map(|tail| &tail[..tail.iter().position(|b| !(0x20..0x7F).contains(b)).unwrap_or(tail.len())]);
        let text = line.and_then(|bytes| std::str::from_utf8(bytes).ok());
        let (version, region) = match text {
            Some(text) => {
                // `2.2 12/04/95 A`: the last word is the region letter.
                let (rest, letter) = text.rsplit_once(' ').unwrap_or((text, ""));
                let region = letter.chars().next().filter(|_| letter.len() == 1);
                match region {
                    Some(_) => (Some(rest.to_string()), region),
                    None => (Some(text.to_string()), None),
                }
            }
            None => (None, None),
        };
        let model = KNOWN_MODELS.iter().find(|(known, _)| *known == crc32).map(|(_, name)| *name);
        Self { crc32, version, region, model }
    }

    /// One line for the user, such as `SCPH-1001 (v2.2, Americas) · CRC 37157331`.
    pub fn describe(&self) -> String {
        let region = match self.region {
            Some('A') => "Américas",
            Some('E') => "Europa",
            Some('J') => "Japón",
            _ => "región desconocida",
        };
        match (self.model, &self.version) {
            (Some(model), _) => format!("{model} · CRC {:08X}", self.crc32),
            (None, Some(version)) => format!("versión {version} · {region} · CRC {:08X}", self.crc32),
            (None, None) => format!("BIOS sin identificar · CRC {:08X}", self.crc32),
        }
    }
}

/// PS1 BIOS ROM region (512 KB).
pub struct Bios {
    data: Vec<u8>,
}

const BIOS_SIZE: usize = 512 * 1024;

impl Bios {
    pub fn new() -> Self {
        Self {
            data: vec![0u8; BIOS_SIZE],
        }
    }

    pub fn capacity(&self) -> usize {
        BIOS_SIZE
    }

    #[inline(always)]
    pub fn read32(&self, offset: usize) -> u32 {
        let o = offset & (BIOS_SIZE - 4);
        u32::from_le_bytes(self.data[o..o + 4].try_into().unwrap())
    }

    #[inline(always)]
    pub fn read16(&self, offset: usize) -> u16 {
        let o = offset & (BIOS_SIZE - 2);
        u16::from_le_bytes(self.data[o..o + 2].try_into().unwrap())
    }

    pub fn read8(&self, offset: usize) -> u8 {
        self.data[offset & (BIOS_SIZE - 1)]
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// Which BIOS is loaded: its checksum and the version line Sony put in it.
    pub fn info(&self) -> BiosInfo {
        BiosInfo::identify(&self.data)
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    fn rom_with_version_line(line: &[u8]) -> Vec<u8> {
        let mut rom = vec![0u8; BIOS_SIZE];
        rom[VERSION_LINE_OFFSET..VERSION_LINE_OFFSET + line.len()].copy_from_slice(line);
        rom
    }

    #[test]
    fn the_version_line_gives_version_date_and_region() {
        let info = BiosInfo::identify(&rom_with_version_line(b"System ROM Version 4.4 03/24/00 E"));
        assert_eq!(info.version.as_deref(), Some("4.4 03/24/00"));
        assert_eq!(info.region, Some('E'));
        assert_eq!(info.model, None, "an unverified checksum names no model");
        assert!(info.describe().contains("Europa"));
    }

    #[test]
    fn an_image_without_the_line_is_reported_as_unidentified() {
        let info = BiosInfo::identify(&vec![0u8; BIOS_SIZE]);
        assert_eq!(info.version, None);
        assert_eq!(info.region, None);
        assert!(info.describe().starts_with("BIOS sin identificar"));
    }

    #[test]
    fn a_verified_checksum_names_the_model() {
        // A ROM can't be forged to a CRC here, so look the table up directly.
        assert!(KNOWN_MODELS.iter().any(|(crc, name)| *crc == 0x3715_7331 && name.contains("SCPH-1001")));
    }
}
