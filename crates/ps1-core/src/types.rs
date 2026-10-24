use thiserror::Error;

/// Result type used throughout the PS1 core.
pub type Ps1Result<T> = std::result::Result<T, Ps1Error>;

/// Errors produced by the PS1 emulation core.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum Ps1Error {
    #[error("BIOS not loaded")]
    BiosNotLoaded,
    #[error("No disc inserted")]
    NoDiscInserted,
    #[error("Invalid CD-ROM image format: {0}")]
    InvalidDiscFormat(String),
    #[error("Memory access out of range: addr=0x{0:08X}")]
    MemoryOutOfRange(u32),
    #[error("Unimplemented CPU opcode: {0}")]
    UnimplementedOpcode(u32),
    #[error("Save state version mismatch")]
    SaveStateVersionMismatch,
    #[error("IO error: {0}")]
    Io(String),
}
