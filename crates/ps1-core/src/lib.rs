//! PS1 Core Emulation Library.
//!
//! Pure Rust emulation core for the Sony PlayStation (PS1).
//! No WASM or DOM dependencies; testable on any host platform.
//!
//! Architecture reference: DuckStation (design only — no code copied).

pub mod bus;
pub mod cpu;
pub mod controllers;
pub mod cdrom;
pub mod disc;
pub mod dma;
pub mod exe;
pub mod gte;
pub mod gpu;
pub mod interrupt;
pub mod io;
pub mod mdec;
pub mod profile;
mod serde_arrays;
pub mod memory;
pub mod save_state;
pub mod spu;
pub mod timers;
pub mod system;
pub mod xa;

pub use system::System;
pub use types::{Ps1Error, Ps1Result};

mod types;
