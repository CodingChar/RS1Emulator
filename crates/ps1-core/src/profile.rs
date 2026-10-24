//! Optional sampling instrumentation, compiled in with the `profile` feature.
//!
//! The emulation thread records what it is doing in an atomic with
//! [`enter`]; a sampler thread (see `examples/bench.rs`) reads it at a fixed
//! interval, which gives the share of wall-clock time per area. Without the
//! feature everything here compiles to nothing.

/// What the emulation thread can be doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Area {
    /// Interpreter and RAM/BIOS accesses.
    Cpu = 0,
    /// Bringing the devices up to date.
    Devices,
    Spu,
    /// Register accesses to the GPU, timers, CD-ROM, SIO, SPU, ...
    Mmio,
    Dma,
    GpuCommand,
    GpuTriangle,
    GpuRectangle,
    GpuLine,
    GpuFill,
    GpuCopy,
    GpuUpload,
    Mdec,
    CdSector,
    Display,
    Gte,
}

pub const AREA_NAMES: [&str; 16] = [
    "cpu", "devices", "spu", "mmio", "dma", "gpu-command", "gpu-triangle", "gpu-rectangle",
    "gpu-line", "gpu-fill", "gpu-copy", "gpu-upload", "mdec", "cd-sector", "display", "gte",
];

#[cfg(feature = "profile")]
mod enabled {
    use super::Area;
    use std::sync::atomic::{AtomicU8, Ordering};

    /// The area the emulation thread is in right now.
    pub static CURRENT: AtomicU8 = AtomicU8::new(0);

    /// Restores the previous area when dropped.
    pub struct Scope(u8);

    pub fn enter(area: Area) -> Scope {
        Scope(CURRENT.swap(area as u8, Ordering::Relaxed))
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            CURRENT.store(self.0, Ordering::Relaxed);
        }
    }
}

#[cfg(feature = "profile")]
pub use enabled::{enter, Scope, CURRENT};

#[cfg(not(feature = "profile"))]
pub struct Scope;

#[cfg(not(feature = "profile"))]
#[inline(always)]
pub fn enter(_area: Area) -> Scope {
    Scope
}
