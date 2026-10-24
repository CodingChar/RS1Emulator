/// Geometry Transformation Engine (GTE).
///
/// The GTE is a coprocessor (Coprocessor 2) that handles 3D geometry
/// transformations: rotation, translation, lighting, perspective projection,
/// and matrix-vector multiplication. Used heavily by PS1 games.
///
/// DuckStation implements this in `gte.cpp`. Here we define the register map
/// and stubs; full instruction emulation is a follow-up task.
pub struct Gte {
    /// Coprocessor 2 control registers (32 x 32-bit).
    pub regs: [u32; 32],
}

impl Gte {
    pub fn new() -> Self {
        Self { regs: [0u32; 32] }
    }

    pub fn reset(&mut self) {
        self.regs = [0u32; 32];
    }

    /// Execute a GTE instruction (opcode 0x10 with COP2 funct).
    pub fn exec(&mut self, _instr: u32) {
        // TODO: implement GTE instructions (RTPS, NCLIP, MVMVA, etc.)
    }
}
