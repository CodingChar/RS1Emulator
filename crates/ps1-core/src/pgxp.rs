//! PGXP (Parallel/Precision Geometry Transform Pipeline).
//!
//! The GTE projects 3D vertices to screen space with 16.16 fixed-point maths and
//! then truncates the result to whole pixels, which is what makes PlayStation
//! 3D wobble and warp textures. PGXP keeps the untruncated result next to the
//! integer one and lets the rasterizer use it. The technique is the one of
//! ICatButt's PGXP (as in Beetle PSX and DuckStation's "Memory" mode):
//!
//! 1. RTPS/RTPT save the precise `x`, `y` and the depth `w` of each projected
//!    vertex next to the SXY FIFO entry (see `Gte::precise_screen`).
//! 2. The tag follows the value through the CPU: `swc2`/`mfc2` copy the tag of an
//!    SXY register, `lw`/`sw` move it between general registers and memory
//!    (the cache below, indexed by word address), `lwc2`/`mtc2` bring it back.
//!    Other instructions do not track tags, so a tag is always checked against
//!    the integer value it was made for: if software changed the value, the tag
//!    simply does not match any more and is ignored.
//! 3. When the GPU receives a polygon through DMA it asks for the tag of each
//!    vertex word by the address the word came from; a vertex with a matching
//!    tag is drawn with its precise position, and with perspective-correct
//!    texturing if all its vertices have a depth.
//!
//! The tags are not part of the emulated machine: they never change what the
//! game computes, and they are not saved in save states.

/// Words of main RAM and of the scratchpad that can hold a tagged value.
const RAM_WORDS: usize = 0x20_0000 / 4;
const SCRATCHPAD_WORDS: usize = 0x400 / 4;
const SCRATCHPAD_START: u32 = 0x1F80_0000;

/// A vertex as the GTE computed it before truncation, together with the
/// packed integer coordinates (`x | y << 16`) that went into the SXY register.
/// Without a positive depth the vertex is not valid.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PreciseVertex {
    pub x: f32,
    pub y: f32,
    /// The depth the vertex was projected from (SZ3), always positive.
    pub w: f32,
    pub value: u32,
}

impl PreciseVertex {
    pub const NONE: Self = Self { x: 0.0, y: 0.0, w: 0.0, value: 0 };

    pub fn is_valid(&self) -> bool {
        self.w > 0.0
    }

    /// This vertex if it was made for `value`, otherwise nothing.
    #[inline]
    pub fn matching(self, value: u32) -> Self {
        if self.w > 0.0 && self.value == value { self } else { Self::NONE }
    }
}

/// The tags kept in memory: one per word of RAM and scratchpad, allocated
/// only while PGXP is on.
pub struct Memory {
    enabled: bool,
    tags: Vec<PreciseVertex>,
}

impl Memory {
    pub fn new() -> Self {
        Self { enabled: false, tags: Vec::new() }
    }

    /// Whether the CPU and the DMA should track tags.
    #[inline(always)]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled == self.enabled {
            return;
        }
        self.enabled = enabled;
        self.tags = if enabled { vec![PreciseVertex::NONE; RAM_WORDS + SCRATCHPAD_WORDS] } else { Vec::new() };
    }

    /// Forget every tag (a reset or a loaded state).
    pub fn clear(&mut self) {
        self.tags.fill(PreciseVertex::NONE);
    }

    /// Where a physical address keeps its tag, for RAM (with its mirrors) and the scratchpad.
    #[inline(always)]
    fn index(phys: u32) -> Option<usize> {
        if phys < 0x80_0000 {
            Some(((phys & 0x1F_FFFC) >> 2) as usize)
        } else if phys.wrapping_sub(SCRATCHPAD_START) < 0x400 {
            Some(RAM_WORDS + ((phys & 0x3FC) >> 2) as usize)
        } else {
            None
        }
    }

    /// The tag of the word at `phys` whose current content is `value`.
    #[inline]
    pub fn lookup(&self, phys: u32, value: u32) -> PreciseVertex {
        match Self::index(phys) {
            Some(index) => self.tags.get(index).copied().unwrap_or(PreciseVertex::NONE).matching(value),
            None => PreciseVertex::NONE,
        }
    }

    /// Record the tag of the word just written at `phys` (`NONE` clears it).
    #[inline]
    pub fn store(&mut self, phys: u32, tag: PreciseVertex) {
        if let Some(slot) = Self::index(phys).and_then(|index| self.tags.get_mut(index)) {
            *slot = tag;
        }
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

/// The tags the GTE keeps for its SXY FIFO. It is not machine state, so two
/// GTEs compare equal whatever it holds.
#[derive(Clone, Debug, Default)]
pub struct GteTags {
    pub enabled: bool,
    /// Tags of SXY0, SXY1 and SXY2.
    pub sxy: [PreciseVertex; 3],
}

impl PartialEq for GteTags {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for GteTags {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(value: u32) -> PreciseVertex {
        PreciseVertex { x: 1.5, y: -2.25, w: 300.0, value }
    }

    #[test]
    fn a_tag_is_found_by_address_and_checked_against_the_value() {
        let mut memory = Memory::new();
        memory.set_enabled(true);
        memory.store(0x1000, tag(0x0010_0020));
        assert_eq!(memory.lookup(0x1000, 0x0010_0020), tag(0x0010_0020));
        assert_eq!(memory.lookup(0x1000, 0x0010_0021), PreciseVertex::NONE, "the value changed");
        assert_eq!(memory.lookup(0x1004, 0x0010_0020), PreciseVertex::NONE, "another address");
    }

    #[test]
    fn the_mirrors_of_ram_share_their_tags_and_the_scratchpad_has_its_own() {
        let mut memory = Memory::new();
        memory.set_enabled(true);
        memory.store(0x40, tag(7));
        assert_eq!(memory.lookup(0x20_0040, 7), tag(7), "RAM mirror");
        memory.store(0x1F80_0040, tag(9));
        assert_eq!(memory.lookup(0x1F80_0040, 9), tag(9));
        assert_eq!(memory.lookup(0x40, 9), PreciseVertex::NONE);
        assert_eq!(memory.lookup(0x1F80_1000, 9), PreciseVertex::NONE, "I/O has no tags");
    }

    #[test]
    fn storing_none_forgets_the_tag_and_turning_off_frees_it() {
        let mut memory = Memory::new();
        memory.set_enabled(true);
        memory.store(8, tag(1));
        memory.store(8, PreciseVertex::NONE);
        assert_eq!(memory.lookup(8, 1), PreciseVertex::NONE);
        memory.store(8, tag(1));
        memory.set_enabled(false);
        assert_eq!(memory.lookup(8, 1), PreciseVertex::NONE);
        memory.set_enabled(true);
        assert_eq!(memory.lookup(8, 1), PreciseVertex::NONE, "tags do not survive being turned off");
    }

    #[test]
    fn a_vertex_needs_a_positive_depth() {
        let invalid = PreciseVertex { w: 0.0, ..tag(5) };
        assert!(!invalid.is_valid());
        assert_eq!(invalid.matching(5), PreciseVertex::NONE);
    }
}
