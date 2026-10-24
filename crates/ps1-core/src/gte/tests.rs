use super::*;

// Register numbers used below.
const R_RGBC: usize = 6;
const R_OTZ: usize = 7;
const R_IR0: usize = 8;
const R_IR1: usize = 9;
const R_IR2: usize = 10;
const R_IR3: usize = 11;
const R_SXY0: usize = 12;
const R_SXY1: usize = 13;
const R_SXY2: usize = 14;
const R_SXYP: usize = 15;
const R_SZ0: usize = 16;
const R_SZ1: usize = 17;
const R_SZ2: usize = 18;
const R_SZ3: usize = 19;
const R_RGB2: usize = 22;
const R_MAC0: usize = 24;
const R_MAC1: usize = 25;
const R_MAC2: usize = 26;
const R_MAC3: usize = 27;
const R_IRGB: usize = 28;
const R_ORGB: usize = 29;
const R_LZCS: usize = 30;
const R_LZCR: usize = 31;

// Control register numbers (cop2r32-63 minus 32).
const C_TR: usize = 5;
const C_BK: usize = 13;
const C_FC: usize = 21;
const C_OFX: usize = 24;
const C_OFY: usize = 25;
const C_H: usize = 26;
const C_DQA: usize = 27;
const C_DQB: usize = 28;
const C_ZSF3: usize = 29;
const C_ZSF4: usize = 30;
const C_FLAG: usize = 31;

// Command encodings (opcode in the low 6 bits).
const SF: u32 = 1 << 19;
const LM: u32 = 1 << 10;
const RTPS: u32 = 0x01;
const NCLIP: u32 = 0x06;
const OP: u32 = 0x0C;
const DPCS: u32 = 0x10;
const INTPL: u32 = 0x11;
const MVMVA: u32 = 0x12;
const NCDS: u32 = 0x13;
const CDP: u32 = 0x14;
const NCDT: u32 = 0x16;
const NCCS: u32 = 0x1B;
const CC: u32 = 0x1C;
const NCS: u32 = 0x1E;
const NCT: u32 = 0x20;
const SQR: u32 = 0x28;
const DCPL: u32 = 0x29;
const DPCT: u32 = 0x2A;
const AVSZ3: u32 = 0x2D;
const AVSZ4: u32 = 0x2E;
const RTPT: u32 = 0x30;
const GPF: u32 = 0x3D;
const GPL: u32 = 0x3E;
const NCCT: u32 = 0x3F;

fn mvmva(matrix: u32, vector: u32, translation: u32) -> u32 {
    MVMVA | (matrix << 17) | (vector << 15) | (translation << 13)
}

/// Pack two signed 16-bit values into a register.
fn pack(low: i32, high: i32) -> u32 {
    (low as u16 as u32) | ((high as u16 as u32) << 16)
}

fn sx(gte: &Gte, reg: usize) -> i16 {
    gte.read_data(reg) as i16
}

fn sy(gte: &Gte, reg: usize) -> i16 {
    (gte.read_data(reg) >> 16) as i16
}

fn mac(gte: &Gte, n: usize) -> i32 {
    gte.read_data(R_MAC1 - 1 + n) as i32
}

fn ir(gte: &Gte, n: usize) -> i32 {
    gte.read_data(R_IR1 - 1 + n) as i32
}

fn flag(gte: &Gte) -> u32 {
    gte.read_control(C_FLAG)
}

/// The last colour pushed: (r, g, b, code).
fn color(gte: &Gte) -> (u32, u32, u32, u32) {
    let c = gte.read_data(R_RGB2);
    (c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF, c >> 24)
}

/// Load a 3x3 matrix into the control registers at `base` (0 rotation,
/// 8 light, 16 colour).
fn set_matrix(gte: &mut Gte, base: usize, m: [[i32; 3]; 3]) {
    let flat: Vec<i32> = m.iter().flatten().copied().collect();
    for reg in 0..5 {
        let low = flat[2 * reg];
        let high = flat.get(2 * reg + 1).copied().unwrap_or(0);
        gte.write_control(base + reg, pack(low, high));
    }
}

fn set_vector(gte: &mut Gte, index: usize, v: [i32; 3]) {
    gte.write_data(2 * index, pack(v[0], v[1]));
    gte.write_data(2 * index + 1, v[2] as u32);
}

fn set_ir(gte: &mut Gte, v: [i32; 3]) {
    for (i, value) in v.into_iter().enumerate() {
        gte.write_data(R_IR1 + i, value as u32);
    }
}

fn identity(scale: i32) -> [[i32; 3]; 3] {
    [[scale, 0, 0], [0, scale, 0], [0, 0, scale]]
}

// ----- Registers -----

#[test]
fn signed_16_bit_data_registers_read_sign_extended() {
    let mut gte = Gte::new();
    for reg in [1, 3, 5, R_IR0, R_IR1, R_IR2, R_IR3] {
        gte.write_data(reg, 0x1234_8001);
        assert_eq!(gte.read_data(reg), 0xFFFF_8001, "register {reg}");
    }
}

#[test]
fn unsigned_16_bit_data_registers_read_zero_extended() {
    let mut gte = Gte::new();
    for reg in [R_OTZ, R_SZ0, R_SZ1, R_SZ2, R_SZ3] {
        gte.write_data(reg, 0xABCD_8001);
        assert_eq!(gte.read_data(reg), 0x8001, "register {reg}");
    }
}

#[test]
fn plain_data_registers_store_all_32_bits() {
    let mut gte = Gte::new();
    for reg in [0, 2, 4, R_RGBC, 20, 21, R_RGB2, R_MAC0, R_MAC1, R_MAC2, R_MAC3, R_LZCS] {
        gte.write_data(reg, 0xDEAD_BEEF);
        assert_eq!(gte.read_data(reg), 0xDEAD_BEEF, "register {reg}");
    }
}

#[test]
fn writing_sxyp_pushes_the_screen_fifo_and_reading_returns_the_newest() {
    let mut gte = Gte::new();
    gte.write_data(R_SXYP, 0x0001_0001);
    gte.write_data(R_SXYP, 0x0002_0002);
    gte.write_data(R_SXYP, 0x0003_0003);
    gte.write_data(R_SXYP, 0x0004_0004);
    assert_eq!(gte.read_data(R_SXY0), 0x0002_0002);
    assert_eq!(gte.read_data(R_SXY1), 0x0003_0003);
    assert_eq!(gte.read_data(R_SXY2), 0x0004_0004);
    assert_eq!(gte.read_data(R_SXYP), 0x0004_0004);
}

#[test]
fn irgb_expands_5_bit_components_and_orgb_packs_them_back() {
    let mut gte = Gte::new();
    gte.write_data(R_IRGB, (0x1F << 10) | (0x10 << 5) | 0x01);
    assert_eq!(ir(&gte, 1), 0x80);
    assert_eq!(ir(&gte, 2), 0x800);
    assert_eq!(ir(&gte, 3), 0xF80);
    assert_eq!(gte.read_data(R_ORGB), (0x1F << 10) | (0x10 << 5) | 0x01);
    assert_eq!(gte.read_data(R_IRGB), gte.read_data(R_ORGB), "IRGB reads like ORGB");

    // Out-of-range IR values clamp to 0..0x1F.
    set_ir(&mut gte, [-5, 0x7FFF, 0x100]);
    assert_eq!(gte.read_data(R_ORGB), (0x02 << 10) | (0x1F << 5));
    gte.write_data(R_ORGB, 0xFFFF);
    assert_eq!(gte.read_data(R_ORGB), (0x02 << 10) | (0x1F << 5), "ORGB is read-only");
}

#[test]
fn lzcr_counts_leading_sign_bits() {
    let mut gte = Gte::new();
    for (lzcs, expected) in [
        (0u32, 32),
        (1, 31),
        (0x0000_8000, 16),
        (0x7FFF_FFFF, 1),
        (0xFFFF_FFFF, 32),
        (0xFFFF_0000, 16),
        (0x8000_0000, 1),
    ] {
        gte.write_data(R_LZCS, lzcs);
        assert_eq!(gte.read_data(R_LZCR), expected, "lzcs {lzcs:#x}");
    }
    gte.write_data(R_LZCR, 5);
    assert_eq!(gte.read_data(R_LZCR), 1, "LZCR is read-only");
}

#[test]
fn single_value_control_registers_read_sign_extended() {
    let mut gte = Gte::new();
    for reg in [4, 12, 20, C_H, C_DQA, C_ZSF3, C_ZSF4] {
        gte.write_control(reg, 0x0001_8000);
        assert_eq!(gte.read_control(reg), 0xFFFF_8000, "control {reg}");
    }
    // Two-halves registers and 32-bit registers are untouched.
    gte.write_control(0, 0x8001_8000);
    assert_eq!(gte.read_control(0), 0x8001_8000);
    gte.write_control(C_OFX, 0x1234_5678);
    assert_eq!(gte.read_control(C_OFX), 0x1234_5678);
}

#[test]
fn flag_register_masks_writes_and_computes_the_summary_bit() {
    let mut gte = Gte::new();
    gte.write_control(C_FLAG, 0xFFFF_FFFF);
    assert_eq!(gte.control[C_FLAG], 0x7FFF_F000);
    assert_eq!(gte.read_control(C_FLAG), 0xFFFF_F000, "error bits set the summary");

    // Bits outside the error mask (12 and 31 aside) don't raise it.
    gte.write_control(C_FLAG, 1 << 12);
    assert_eq!(gte.read_control(C_FLAG), 1 << 12);
    gte.write_control(C_FLAG, 1 << 13);
    assert_eq!(gte.read_control(C_FLAG), (1 << 13) | (1 << 31));
}

#[test]
fn reset_clears_everything() {
    let mut gte = Gte::new();
    gte.write_data(R_RGBC, 1);
    gte.write_control(C_H, 2);
    gte.reset();
    assert_eq!(gte, Gte::new());
}

#[test]
fn unr_table_matches_the_hardware_seed_values() {
    assert_eq!(UNR_TABLE[0], 0xFF);
    assert_eq!(UNR_TABLE[1], 0xFD);
    assert_eq!(UNR_TABLE[2], 0xFB);
    assert_eq!(UNR_TABLE[0x80], 0x54);
    assert_eq!(UNR_TABLE[8], 0xEF);
    assert_eq!(UNR_TABLE[9], 0xEE);
    assert_eq!(UNR_TABLE[0x100], 0x00);
    assert!(UNR_TABLE.windows(2).all(|w| w[0] >= w[1]), "monotonically decreasing");
}

// ----- Division -----

#[test]
fn division_approximates_the_reciprocal() {
    let mut gte = Gte::new();
    for (h, sz3) in [(1000u32, 1000u32), (0x100, 0x200), (500, 300), (1, 1), (0x7FFF, 0xFFFF), (3, 7)] {
        let exact = (h as f64 * 65536.0 / sz3 as f64).min(0x1FFFF as f64);
        let n = gte.divide(h, sz3) as f64;
        assert!((n - exact).abs() <= 3.0, "{h}/{sz3}: got {n}, expected {exact}");
    }
    assert_eq!(gte.control[C_FLAG], 0);
}

#[test]
fn division_overflow_saturates_and_flags() {
    let mut gte = Gte::new();
    assert_eq!(gte.divide(600, 300), 0x1FFFF);
    assert_ne!(gte.control[C_FLAG] & FLAG_DIVIDE_OVERFLOW, 0);
    gte.control[C_FLAG] = 0;
    assert_eq!(gte.divide(1, 0), 0x1FFFF, "division by zero");
    assert_ne!(gte.control[C_FLAG] & FLAG_DIVIDE_OVERFLOW, 0);
    assert_ne!(gte.read_control(C_FLAG) & (1 << 31), 0);
}

// ----- Perspective transformation -----

fn projection_setup() -> Gte {
    let mut gte = Gte::new();
    set_matrix(&mut gte, 0, identity(0x1000));
    gte.write_control(C_H, 500);
    set_vector(&mut gte, 0, [100, 200, 300]);
    gte
}

#[test]
fn rtps_projects_a_point() {
    let mut gte = projection_setup();
    gte.execute(RTPS | SF);

    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (100, 200, 300));
    assert_eq!((ir(&gte, 1), ir(&gte, 2), ir(&gte, 3)), (100, 200, 300));
    assert_eq!(gte.read_data(R_SZ3), 300);

    // n = 500 / 300 in 1.16 fixed point (about 109227); x = n * 100 >> 16.
    assert_eq!(sx(&gte, R_SXY2), 166);
    assert_eq!(sy(&gte, R_SXY2), 333);
    let n = gte.read_data(R_MAC0); // depth cue not set up: DQB = 0
    assert_eq!(n, 0);
    assert_eq!(flag(&gte), 0);
}

#[test]
fn rtps_applies_screen_offset_and_depth_cueing() {
    let mut gte = projection_setup();
    gte.write_control(C_OFX, 160 << 16);
    gte.write_control(C_OFY, 120 << 16);
    gte.write_control(C_DQB, 0x0080_0000);
    gte.execute(RTPS | SF);

    assert_eq!(sx(&gte, R_SXY2), 160 + 166);
    assert_eq!(sy(&gte, R_SXY2), 120 + 333);
    // MAC0 = n * DQA + DQB (DQA = 0), IR0 = MAC0 >> 12.
    assert_eq!(gte.read_data(R_MAC0), 0x0080_0000);
    assert_eq!(gte.read_data(R_IR0), 0x800);
    assert_eq!(flag(&gte), 0);
}

#[test]
fn rtps_clamps_ir0_to_the_depth_cue_range() {
    let mut gte = projection_setup();
    gte.write_control(C_DQA, 0x100); // n * 256 is far more than 1.0
    gte.execute(RTPS | SF);
    assert_eq!(gte.read_data(R_IR0), 0x1000);
    assert_ne!(flag(&gte) & FLAG_IR0_SATURATED, 0);

    let mut gte = projection_setup();
    gte.write_control(C_DQB, (-0x0100_0000i32) as u32);
    gte.execute(RTPS | SF);
    assert_eq!(gte.read_data(R_IR0), 0);
    assert_ne!(flag(&gte) & FLAG_IR0_SATURATED, 0);
}

#[test]
fn rtps_pushes_the_depth_and_screen_fifos() {
    let mut gte = projection_setup();
    gte.execute(RTPS | SF);
    set_vector(&mut gte, 0, [10, 20, 400]);
    gte.execute(RTPS | SF);

    assert_eq!(gte.read_data(R_SZ2), 300);
    assert_eq!(gte.read_data(R_SZ3), 400);
    // The first point moved down the FIFO; the new one is 10 * (500 / 400) = 12.
    assert_eq!(sx(&gte, R_SXY1), 166);
    assert_eq!(sx(&gte, R_SXY2), 12);
    assert_eq!(gte.read_data(R_SZ0), 0);
}

#[test]
fn rtpt_projects_three_vectors() {
    let mut gte = projection_setup();
    set_vector(&mut gte, 1, [-100, 0, 500]);
    set_vector(&mut gte, 2, [0, 600, 250]);
    gte.execute(RTPT | SF);

    assert_eq!(gte.read_data(R_SZ1), 300);
    assert_eq!(gte.read_data(R_SZ2), 500);
    assert_eq!(gte.read_data(R_SZ3), 250);
    // V1: x = -100 * (500/500) = -100, give or take the table's rounding.
    assert!((-101..=-99).contains(&sx(&gte, R_SXY1)));
    assert_eq!(sy(&gte, R_SXY1), 0);
    // V2: y = 600 * (500/250) = 1200, which is outside the 11-bit range.
    assert_eq!(sy(&gte, R_SXY2), 0x3FF);
    assert_ne!(flag(&gte) & FLAG_SY_SATURATED, 0);
}

#[test]
fn rtps_uses_the_translation_vector() {
    let mut gte = projection_setup();
    gte.write_control(C_TR, 10);
    gte.write_control(C_TR + 1, (-20i32) as u32);
    gte.write_control(C_TR + 2, 100);
    gte.execute(RTPS | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (110, 180, 400));
}

#[test]
fn rtps_saturates_the_screen_position() {
    let mut gte = projection_setup();
    set_vector(&mut gte, 0, [1000, -1000, 100]); // H/SZ3 = 5: far outside the screen
    gte.execute(RTPS | SF);
    assert_eq!(sx(&gte, R_SXY2), 0x3FF);
    assert_eq!(sy(&gte, R_SXY2), -0x400);
    assert_ne!(flag(&gte) & FLAG_SX_SATURATED, 0);
    assert_ne!(flag(&gte) & FLAG_SY_SATURATED, 0);
    assert_ne!(read_flag_bit31(&gte), 0);
}

fn read_flag_bit31(gte: &Gte) -> u32 {
    flag(gte) & (1 << 31)
}

#[test]
fn rtps_with_zero_depth_overflows_the_divide() {
    let mut gte = projection_setup();
    set_vector(&mut gte, 0, [10, 10, 0]);
    gte.execute(RTPS | SF);
    assert_ne!(flag(&gte) & FLAG_DIVIDE_OVERFLOW, 0);
    assert_eq!(gte.read_data(R_SZ3), 0);
}

#[test]
fn rtps_saturates_ir3_and_the_depth() {
    let mut gte = projection_setup();
    set_vector(&mut gte, 0, [0, 0, -5]);
    gte.execute(RTPS | SF);
    assert_eq!(gte.read_data(R_SZ3), 0, "negative depth clamps to 0");
    assert_ne!(flag(&gte) & FLAG_SZ_OTZ_SATURATED, 0);

    let mut gte = projection_setup();
    set_vector(&mut gte, 0, [0, 0, 0x7FFF]);
    gte.write_control(C_TR + 2, 0x7FFF); // z = 0xFFFE
    gte.execute(RTPS | SF);
    assert_eq!(ir(&gte, 3), 0x7FFF);
    assert_ne!(flag(&gte) & (1 << 22), 0, "IR3 saturated");
    assert_eq!(gte.read_data(R_SZ3), 0xFFFE);
}

#[test]
fn rtps_with_shift_zero_still_derives_depth_from_the_unshifted_sum() {
    let mut gte = Gte::new();
    set_matrix(&mut gte, 0, identity(1)); // no scaling
    gte.write_control(C_H, 500);
    set_vector(&mut gte, 0, [3, 4, 0x2000]);
    gte.execute(RTPS); // sf = 0
    assert_eq!(mac(&gte, 3), 0x2000);
    assert_eq!(gte.read_data(R_SZ3), 2, "0x2000 >> 12");
}

// ----- Normal clip, average Z -----

#[test]
fn nclip_is_the_signed_triangle_area() {
    let mut gte = Gte::new();
    gte.write_data(R_SXYP, pack(0, 0));
    gte.write_data(R_SXYP, pack(10, 0));
    gte.write_data(R_SXYP, pack(0, 10));
    gte.execute(NCLIP);
    assert_eq!(gte.read_data(R_MAC0) as i32, 100);

    // Opposite winding.
    gte.write_data(R_SXYP, pack(0, 0));
    gte.write_data(R_SXYP, pack(0, 10));
    gte.write_data(R_SXYP, pack(10, 0));
    gte.execute(NCLIP);
    assert_eq!(gte.read_data(R_MAC0) as i32, -100);

    // Degenerate triangle.
    gte.write_data(R_SXYP, pack(1, 1));
    gte.write_data(R_SXYP, pack(2, 2));
    gte.write_data(R_SXYP, pack(3, 3));
    gte.execute(NCLIP);
    assert_eq!(gte.read_data(R_MAC0), 0);
}

#[test]
fn avsz3_and_avsz4_average_the_depths() {
    let mut gte = Gte::new();
    gte.write_control(C_ZSF3, 341);
    gte.write_control(C_ZSF4, 256);
    for (reg, z) in [(R_SZ0, 50), (R_SZ1, 100), (R_SZ2, 200), (R_SZ3, 300)] {
        gte.write_data(reg, z);
    }
    gte.execute(AVSZ3);
    assert_eq!(gte.read_data(R_MAC0), 341 * 600);
    assert_eq!(gte.read_data(R_OTZ), (341 * 600) >> 12);

    gte.execute(AVSZ4);
    assert_eq!(gte.read_data(R_MAC0), 256 * 650);
    assert_eq!(gte.read_data(R_OTZ), (256 * 650) >> 12);
}

#[test]
fn avsz_saturates_otz() {
    let mut gte = Gte::new();
    gte.write_control(C_ZSF3, 0x7FFF);
    for reg in [R_SZ1, R_SZ2, R_SZ3] {
        gte.write_data(reg, 0xFFFF);
    }
    gte.execute(AVSZ3);
    assert_eq!(gte.read_data(R_OTZ), 0xFFFF);
    assert_ne!(flag(&gte) & FLAG_SZ_OTZ_SATURATED, 0);

    gte.write_control(C_ZSF3, (-0x100i32) as u32);
    gte.execute(AVSZ3);
    assert_eq!(gte.read_data(R_OTZ), 0);
}

// ----- Vector arithmetic -----

#[test]
fn op_is_the_cross_product_with_the_rotation_diagonal() {
    let mut gte = Gte::new();
    set_matrix(&mut gte, 0, [[4, 0, 0], [0, 5, 0], [0, 0, 6]]);
    set_ir(&mut gte, [1, 2, 3]);
    gte.execute(OP);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (3, -6, 3));
    assert_eq!((ir(&gte, 1), ir(&gte, 2), ir(&gte, 3)), (3, -6, 3));

    // lm clamps negative results to zero and flags IR2.
    set_ir(&mut gte, [1, 2, 3]);
    gte.execute(OP | LM);
    assert_eq!(ir(&gte, 2), 0);
    assert_ne!(flag(&gte) & (1 << 23), 0);
}

#[test]
fn sqr_squares_ir() {
    let mut gte = Gte::new();
    set_ir(&mut gte, [3, -4, 5]);
    gte.execute(SQR);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (9, 16, 25));

    // sf shifts the result down by 12.
    set_ir(&mut gte, [0x1000, 0x2000, 0x10]);
    gte.execute(SQR | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (0x1000, 0x4000, 0));
}

#[test]
fn gpf_scales_ir_by_ir0_and_pushes_a_colour() {
    let mut gte = Gte::new();
    gte.write_data(R_RGBC, 0xAB00_0000);
    gte.write_data(R_IR0, 0x800); // 0.5
    set_ir(&mut gte, [0x100, 0x200, 0x7FF]);
    gte.execute(GPF | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (0x80, 0x100, 0x3FF));
    assert_eq!(color(&gte), (0x8, 0x10, 0x3F, 0xAB));
}

#[test]
fn gpl_adds_the_previous_mac() {
    let mut gte = Gte::new();
    gte.write_data(R_IR0, 0x800);
    set_ir(&mut gte, [0x100, 0x200, 0x400]);
    gte.write_data(R_MAC1, 0x10);
    gte.write_data(R_MAC2, 0x20);
    gte.write_data(R_MAC3, 0x30);
    gte.execute(GPL | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (0x10 + 0x80, 0x20 + 0x100, 0x30 + 0x200));
}

#[test]
fn colour_fifo_clamps_and_flags() {
    let mut gte = Gte::new();
    gte.write_data(R_IR0, 0x1000);
    set_ir(&mut gte, [0x7000, -0x100, 0x20]);
    gte.execute(GPF | SF);
    let (r, g, b, _) = color(&gte);
    assert_eq!((r, g, b), (0xFF, 0x00, 0x02));
    assert_ne!(flag(&gte) & (1 << 21), 0, "red saturated");
    assert_ne!(flag(&gte) & (1 << 20), 0, "green saturated");
    assert_eq!(flag(&gte) & (1 << 19), 0);
}

// ----- MVMVA -----

fn mvmva_setup() -> Gte {
    let mut gte = Gte::new();
    // Entry k is k * 0x100 and the vector is 0x10, so every product is
    // k * 0x1000: a whole number once shifted down by 12.
    set_matrix(
        &mut gte,
        0,
        [[0x100, 0x200, 0x300], [0x400, 0x500, 0x600], [0x700, 0x800, 0x900]],
    );
    set_vector(&mut gte, 0, [0x10, 0x10, 0x10]);
    gte.write_control(C_TR, 10);
    gte.write_control(C_TR + 1, 20);
    gte.write_control(C_TR + 2, 30);
    gte
}

#[test]
fn mvmva_multiplies_and_adds_the_translation() {
    let mut gte = mvmva_setup();
    gte.execute(mvmva(0, 0, 0) | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (16, 35, 54));
    assert_eq!((ir(&gte, 1), ir(&gte, 2), ir(&gte, 3)), (16, 35, 54));
}

#[test]
fn mvmva_without_the_shift_keeps_the_translation_scaled_by_0x1000() {
    let mut gte = mvmva_setup();
    gte.execute(mvmva(0, 0, 0));
    assert_eq!(mac(&gte, 1), 10 * 0x1000 + 6 * 0x1000);
    assert_eq!(ir(&gte, 1), 0x7FFF, "saturated");
}

#[test]
fn mvmva_without_translation() {
    let mut gte = mvmva_setup();
    gte.execute(mvmva(0, 0, 3) | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (6, 15, 24));
}

#[test]
fn mvmva_can_use_the_background_colour_and_other_vectors() {
    let mut gte = mvmva_setup();
    gte.write_control(C_BK, 100);
    gte.write_control(C_BK + 1, 200);
    gte.write_control(C_BK + 2, 300);
    set_vector(&mut gte, 1, [0x20, 0, 0]);
    gte.execute(mvmva(0, 1, 1) | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (102, 208, 314));

    // Vector 3 is IR.
    set_ir(&mut gte, [0x10, 0, 0x10]);
    gte.execute(mvmva(0, 3, 3) | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (4, 10, 16));
}

#[test]
fn mvmva_can_select_the_light_and_colour_matrices() {
    let mut gte = mvmva_setup();
    set_matrix(&mut gte, 8, identity(2));
    set_matrix(&mut gte, 16, identity(3));
    set_vector(&mut gte, 0, [5, 6, 7]);
    gte.execute(mvmva(1, 0, 3));
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (10, 12, 14));
    gte.execute(mvmva(2, 0, 3));
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (15, 18, 21));
}

#[test]
fn mvmva_shift_and_lm() {
    let mut gte = Gte::new();
    set_matrix(&mut gte, 0, identity(0x1000));
    set_vector(&mut gte, 0, [-0x40, 0x40, 0x7FFF]);
    gte.execute(mvmva(0, 0, 3) | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (-0x40, 0x40, 0x7FFF));
    assert_eq!(ir(&gte, 1), -0x40);

    gte.execute(mvmva(0, 0, 3) | SF | LM);
    assert_eq!(ir(&gte, 1), 0, "lm clamps negatives");
    assert_ne!(flag(&gte) & (1 << 24), 0);
    assert_eq!(mac(&gte, 1), -0x40, "MAC keeps the unclamped value");
}

#[test]
fn mvmva_saturates_ir_but_keeps_mac() {
    let mut gte = Gte::new();
    set_matrix(&mut gte, 0, [[0x7FFF, 0, 0], [0, 0, 0], [0, 0, 0]]);
    set_vector(&mut gte, 0, [0x7FFF, 0, 0]);
    gte.execute(mvmva(0, 0, 3));
    assert_eq!(mac(&gte, 1), 0x7FFF * 0x7FFF);
    assert_eq!(ir(&gte, 1), 0x7FFF);
    assert_ne!(flag(&gte) & (1 << 24), 0);
}

#[test]
fn mvmva_accumulator_overflow_is_flagged() {
    let mut gte = Gte::new();
    set_matrix(&mut gte, 0, [[0x7FFF; 3], [0; 3], [0; 3]]);
    set_vector(&mut gte, 0, [0x7FFF; 3]);
    gte.write_control(C_TR, 0x7FFF_FFFF); // 0x7FFFFFFF << 12 is just under 2^43
    gte.execute(mvmva(0, 0, 0));
    assert_ne!(flag(&gte) & (1 << 30), 0, "MAC1 positive overflow");
    assert_ne!(read_flag_bit31(&gte), 0);

    gte.write_control(C_TR, 0x8000_0000);
    gte.execute(mvmva(0, 0, 0));
    // -2^43 plus positive products is back in range: no negative overflow.
    assert_eq!(flag(&gte) & (1 << 27), 0);
    gte.write_control(C_TR, 0x8000_0000);
    set_vector(&mut gte, 0, [-0x8000; 3]);
    gte.execute(mvmva(0, 0, 0));
    assert_ne!(flag(&gte) & (1 << 27), 0, "MAC1 negative overflow");
}

#[test]
fn mvmva_with_the_far_colour_reproduces_the_hardware_quirk() {
    let mut gte = mvmva_setup();
    gte.write_control(C_FC, 1);
    gte.write_control(C_FC + 1, 1);
    gte.write_control(C_FC + 2, 1);
    gte.execute(mvmva(0, 0, 2) | SF);
    // Only the second and third columns contribute to the result.
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (5, 11, 17));
}

#[test]
fn mvmva_far_colour_still_raises_the_flags_of_the_dropped_part() {
    let mut gte = Gte::new();
    set_matrix(&mut gte, 0, identity(1));
    set_vector(&mut gte, 0, [0, 0, 0]);
    gte.write_control(C_FC, 0x7FFF); // 0x7FFF << 12 does not fit IR1 with sf = 0
    gte.execute(mvmva(0, 0, 2));
    assert_eq!(mac(&gte, 1), 0, "the far colour is dropped from the result");
    assert_ne!(flag(&gte) & (1 << 24), 0, "but it still saturates IR1");
}

#[test]
fn mvmva_garbage_matrix() {
    let mut gte = Gte::new();
    gte.write_data(R_RGBC, 0x0000_0010); // R = 16
    gte.write_data(R_IR0, 3);
    set_matrix(&mut gte, 0, [[0, 0, 7], [0, 9, 0], [0, 0, 0]]);
    set_vector(&mut gte, 0, [1, 1, 1]);
    gte.execute(mvmva(3, 0, 3));
    // Row 0: [-R<<4, R<<4, IR0] = [-256, 256, 3] -> 3; rows 1 and 2 sum RT13*3 and RT22*3.
    assert_eq!(mac(&gte, 1), 3);
    assert_eq!(mac(&gte, 2), 21);
    assert_eq!(mac(&gte, 3), 27);
}

// ----- Lighting -----

fn lighting_setup() -> Gte {
    let mut gte = Gte::new();
    set_matrix(&mut gte, 8, identity(0x1000)); // light matrix
    set_matrix(&mut gte, 16, identity(0x1000)); // light colour matrix
    set_vector(&mut gte, 0, [0x800, 0x400, 0x200]);
    gte.write_data(R_RGBC, 0x4C80_8080); // code 0x4C, white-ish 0x80
    gte
}

#[test]
fn ncs_lights_a_normal() {
    let mut gte = lighting_setup();
    gte.execute(NCS | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (0x800, 0x400, 0x200));
    assert_eq!(color(&gte), (0x80, 0x40, 0x20, 0x4C));
}

#[test]
fn ncs_adds_the_background_colour() {
    let mut gte = lighting_setup();
    gte.write_control(C_BK, 0x100);
    gte.write_control(C_BK + 1, 0x100);
    gte.write_control(C_BK + 2, 0x100);
    gte.execute(NCS | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (0x900, 0x500, 0x300));
}

#[test]
fn nct_lights_three_normals_and_fills_the_colour_fifo() {
    let mut gte = lighting_setup();
    set_vector(&mut gte, 1, [0x1000, 0, 0]);
    set_vector(&mut gte, 2, [0, 0x1000, 0]);
    gte.execute(NCT | SF);
    assert_eq!(gte.read_data(20) & 0xFFFFFF, 0x20_40_80, "first colour in RGB0");
    assert_eq!(gte.read_data(21) & 0xFFFFFF, 0x00_00_FF);
    assert_eq!(gte.read_data(22) & 0xFFFFFF, 0x00_FF_00);
}

#[test]
fn nccs_modulates_with_the_vertex_colour() {
    let mut gte = lighting_setup();
    gte.execute(NCCS | SF);
    // (0x80 * 0x800 << 4) >> 12 = 0x400, then / 16 for the FIFO.
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (0x400, 0x200, 0x100));
    assert_eq!(color(&gte), (0x40, 0x20, 0x10, 0x4C));
}

#[test]
fn ncct_runs_nccs_three_times() {
    let mut gte = lighting_setup();
    set_vector(&mut gte, 1, [0x800, 0x400, 0x200]);
    set_vector(&mut gte, 2, [0x800, 0x400, 0x200]);
    gte.execute(NCCT | SF);
    assert_eq!(color(&gte), (0x40, 0x20, 0x10, 0x4C));
    assert_eq!(gte.read_data(20), gte.read_data(22), "same vertex three times");
}

#[test]
fn cc_skips_the_light_matrix() {
    let mut gte = lighting_setup();
    set_ir(&mut gte, [0x800, 0x400, 0x200]);
    gte.execute(CC | SF);
    assert_eq!(color(&gte), (0x40, 0x20, 0x10, 0x4C));
}

#[test]
fn depth_cueing_blends_towards_the_far_colour() {
    // IR0 = 0: the vertex colour passes through.
    let mut gte = lighting_setup();
    gte.write_control(C_FC, 0x400);
    gte.write_control(C_FC + 1, 0x400);
    gte.write_control(C_FC + 2, 0x400);
    gte.execute(NCDS | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (0x400, 0x200, 0x100));

    // IR0 = 1.0: the far colour replaces it.
    let mut gte = lighting_setup();
    for i in 0..3 {
        gte.write_control(C_FC + i, 0x400);
    }
    gte.write_data(R_IR0, 0x1000);
    gte.execute(NCDS | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (0x400, 0x400, 0x400));
    assert_eq!(color(&gte), (0x40, 0x40, 0x40, 0x4C));
}

#[test]
fn ncdt_and_cdp_use_the_same_blend() {
    let mut gte = lighting_setup();
    for i in 0..3 {
        gte.write_control(C_FC + i, 0x400);
    }
    gte.write_data(R_IR0, 0x1000);
    gte.execute(NCDT | SF);
    assert_eq!(color(&gte), (0x40, 0x40, 0x40, 0x4C));

    let mut gte = lighting_setup();
    for i in 0..3 {
        gte.write_control(C_FC + i, 0x400);
    }
    gte.write_data(R_IR0, 0x1000);
    set_ir(&mut gte, [0x800, 0x400, 0x200]);
    gte.execute(CDP | SF);
    assert_eq!(color(&gte), (0x40, 0x40, 0x40, 0x4C));
}

#[test]
fn dpcs_blends_the_command_colour() {
    let mut gte = Gte::new();
    gte.write_data(R_RGBC, 0x2000_4080); // code 0x20, B=0, G=0x40, R=0x80
    for i in 0..3 {
        gte.write_control(C_FC + i, 0x400);
    }
    gte.write_data(R_IR0, 0x800);
    gte.execute(DPCS | SF);
    // R: 0x800 + (0x400 - 0x800) / 2 = 0x600 -> 0x60.
    // G already equals the far colour and B moves halfway up to it.
    assert_eq!(color(&gte), (0x60, 0x40, 0x20, 0x20));
}

#[test]
fn dpct_cycles_the_colour_fifo() {
    let mut gte = Gte::new();
    for i in 0..3 {
        gte.write_control(C_FC + i, 0);
    }
    gte.write_data(R_RGBC, 0x1100_0000);
    // IR0 = 0: colours pass through unchanged, so the FIFO rotates by three.
    for (reg, value) in [(20, 0x0A_0B_0C), (21, 0x0D_0E_0F), (22, 0x10_11_12)] {
        gte.write_data(reg, value);
    }
    gte.execute(DPCT | SF);
    assert_eq!(gte.read_data(20) & 0xFFFFFF, 0x0A_0B_0C);
    assert_eq!(gte.read_data(21) & 0xFFFFFF, 0x0D_0E_0F);
    assert_eq!(gte.read_data(22) & 0xFFFFFF, 0x10_11_12);
    assert_eq!(gte.read_data(22) >> 24, 0x11);
}

#[test]
fn dcpl_modulates_then_blends() {
    let mut gte = lighting_setup();
    set_ir(&mut gte, [0x800, 0x400, 0x200]);
    gte.execute(DCPL | SF);
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (0x400, 0x200, 0x100));
}

#[test]
fn intpl_interpolates_ir_towards_the_far_colour() {
    let mut gte = Gte::new();
    for i in 0..3 {
        gte.write_control(C_FC + i, 0x200);
    }
    gte.write_data(R_IR0, 0x800);
    set_ir(&mut gte, [0x400, 0x400, 0x400]);
    gte.execute(INTPL | SF);
    // 0x400 + (0x200 - 0x400) * 0.5 = 0x300.
    assert_eq!((mac(&gte, 1), mac(&gte, 2), mac(&gte, 3)), (0x300, 0x300, 0x300));
    assert_eq!(color(&gte).0, 0x30);
}

#[test]
fn commands_clear_the_flag_register_first() {
    let mut gte = Gte::new();
    gte.write_control(C_FLAG, 0x7FFF_F000);
    gte.execute(NCLIP);
    assert_eq!(flag(&gte), 0);
}

#[test]
fn mac0_overflow_is_flagged() {
    let mut gte = Gte::new();
    gte.write_data(R_SXYP, pack(0x7FFF, 0x7FFF));
    gte.write_data(R_SXYP, pack(-0x8000, 0x7FFF));
    gte.write_data(R_SXYP, pack(0x7FFF, -0x8000));
    gte.execute(NCLIP);
    // Sum of products is about 2^31 or more.
    assert_ne!(flag(&gte) & (FLAG_MAC0_POSITIVE | FLAG_MAC0_NEGATIVE), 0);
}

#[test]
fn undefined_commands_are_harmless() {
    let mut gte = projection_setup();
    let before = gte.clone();
    gte.execute(0x3A); // unassigned opcode
    assert_eq!(gte.data, before.data);
}
