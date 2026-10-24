use super::*;

const BLOCK_ADDRESS: usize = 0x1000;

/// A looping block whose 28 samples all equal 7 << 12 (28672).
fn put_constant_block(spu: &mut Spu) {
    spu.sound_ram[BLOCK_ADDRESS] = 0x00; // shift 0, filter 0
    spu.sound_ram[BLOCK_ADDRESS + 1] = 0x03; // loop end + repeat
    for i in 0..14 {
        spu.sound_ram[BLOCK_ADDRESS + 2 + i] = 0x77;
    }
}

/// Voice 0 playing the constant block at full volume, envelope held at maximum.
fn spu_with_tone() -> Spu {
    let mut spu = Spu::new();
    put_constant_block(&mut spu);
    spu.write(0x1AA, 2, 0xC000); // SPU enabled, unmuted
    spu.write(MAIN_VOLUME_LEFT, 2, 0x3FFF);
    spu.write(MAIN_VOLUME_RIGHT, 2, 0x3FFF);
    spu.write(0x00, 2, 0x3FFF); // voice 0 volume left
    spu.write(0x02, 2, 0x3FFF); // voice 0 volume right
    spu.write(0x04, 2, 0x1000); // pitch: 44.1 kHz
    spu.write(0x06, 2, (BLOCK_ADDRESS / 8) as u32); // start address
    spu.write(0x08, 2, 0x000F); // fastest attack, sustain level at the top
    spu.write(0x0A, 2, 0x0000); // sustain rises, release shift 0
    spu
}

fn run_samples(spu: &mut Spu, count: usize) {
    for _ in 0..count {
        spu.tick_sample((0, 0));
    }
}

fn last_frame(spu: &Spu) -> (f32, f32) {
    let n = spu.output_buffer.len();
    (spu.output_buffer[n - 2], spu.output_buffer[n - 1])
}

#[test]
fn silent_without_any_voice() {
    let mut spu = Spu::new();
    spu.write(0x1AA, 2, 0xC000);
    run_samples(&mut spu, 10);
    assert_eq!(spu.output_buffer.len(), 20);
    assert!(spu.output_buffer.iter().all(|s| *s == 0.0));
}

#[test]
fn a_keyed_on_voice_plays_a_steady_tone() {
    let mut spu = spu_with_tone();
    spu.write(KEY_ON_LOW, 2, 1);
    run_samples(&mut spu, 40);
    let (left, right) = last_frame(&spu);
    assert!((0.85..0.88).contains(&left), "{left}");
    assert!((0.85..0.88).contains(&right), "{right}");
    assert!(spu.voice_active(0));
    assert!(!spu.voice_active(1));
}

#[test]
fn nothing_plays_before_the_key_on() {
    let mut spu = spu_with_tone();
    run_samples(&mut spu, 20);
    assert!(spu.output_buffer.iter().all(|s| *s == 0.0));
}

#[test]
fn key_off_releases_the_voice_into_silence() {
    let mut spu = spu_with_tone();
    spu.write(KEY_ON_LOW, 2, 1);
    run_samples(&mut spu, 40);
    spu.write(KEY_OFF_LOW, 2, 1);
    run_samples(&mut spu, 40);
    assert_eq!(last_frame(&spu), (0.0, 0.0));
    assert!(!spu.voice_active(0));
}

#[test]
fn muting_silences_the_output_but_not_the_voices() {
    let mut spu = spu_with_tone();
    spu.write(0x1AA, 2, 0x8000); // enabled, muted
    spu.write(KEY_ON_LOW, 2, 1);
    run_samples(&mut spu, 40);
    assert_eq!(last_frame(&spu), (0.0, 0.0));
    assert!(spu.voice_active(0));
}

#[test]
fn main_volume_scales_the_mix() {
    let mut spu = spu_with_tone();
    spu.write(MAIN_VOLUME_LEFT, 2, 0x1FFF); // half
    spu.write(KEY_ON_LOW, 2, 1);
    run_samples(&mut spu, 40);
    let (left, right) = last_frame(&spu);
    assert!((0.42..0.45).contains(&left), "{left}");
    assert!((0.85..0.88).contains(&right), "{right}");
}

#[test]
fn looping_blocks_set_the_end_flag() {
    let mut spu = spu_with_tone();
    spu.write(KEY_ON_LOW, 2, 1);
    assert_eq!(spu.read(ENDX_LOW, 2), 0);
    run_samples(&mut spu, 80);
    assert_eq!(spu.read(ENDX_LOW, 2) & 1, 1);
    spu.write(KEY_ON_LOW, 2, 1);
    run_samples(&mut spu, 1);
    assert_eq!(spu.read(ENDX_LOW, 2) & 1, 0, "key on clears the flag");
}

#[test]
fn one_shot_blocks_stop_the_voice() {
    let mut spu = spu_with_tone();
    spu.sound_ram[BLOCK_ADDRESS + 1] = 0x01; // loop end without repeat
    spu.write(KEY_ON_LOW, 2, 1);
    run_samples(&mut spu, 80);
    assert!(!spu.voice_active(0));
    assert_eq!(last_frame(&spu), (0.0, 0.0));
}

#[test]
fn pitch_sets_how_fast_the_block_is_consumed() {
    let mut fast = spu_with_tone();
    fast.sound_ram[BLOCK_ADDRESS + 1] = 0x01;
    fast.write(0x04, 2, 0x2000); // double speed
    fast.write(KEY_ON_LOW, 2, 1);
    let mut slow = spu_with_tone();
    slow.sound_ram[BLOCK_ADDRESS + 1] = 0x01;
    slow.write(0x04, 2, 0x0800); // half speed
    slow.write(KEY_ON_LOW, 2, 1);
    run_samples(&mut fast, 20);
    run_samples(&mut slow, 20);
    assert!(!fast.voice_active(0), "28 samples at double speed last 14 ticks");
    assert!(slow.voice_active(0));
}

#[test]
fn cd_audio_is_mixed_in_when_enabled() {
    let mut spu = Spu::new();
    spu.write(0x1AA, 2, 0xC001); // enabled, unmuted, CD audio on
    spu.write(MAIN_VOLUME_LEFT, 2, 0x3FFF);
    spu.write(MAIN_VOLUME_RIGHT, 2, 0x3FFF);
    spu.write(CD_VOLUME_LEFT, 2, 0x7FFF);
    spu.write(CD_VOLUME_RIGHT, 2, 0x7FFF);
    spu.tick_sample((16384, -16384));
    let (left, right) = last_frame(&spu);
    assert!((0.49..0.51).contains(&left), "{left}");
    assert!((-0.51..-0.49).contains(&right), "{right}");

    spu.write(0x1AA, 2, 0xC000);
    spu.tick_sample((16384, -16384));
    assert_eq!(last_frame(&spu), (0.0, 0.0));
}

#[test]
fn noise_voices_produce_changing_samples() {
    let mut spu = spu_with_tone();
    spu.write(NOISE_LOW, 2, 1);
    spu.write(0x1AA, 2, 0xC000 | (0xF << 10)); // the fastest noise clock
    spu.write(KEY_ON_LOW, 2, 1);
    run_samples(&mut spu, 200);
    let samples: Vec<f32> = spu.output_buffer.chunks(2).skip(40).map(|f| f[0]).collect();
    let distinct = samples.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(distinct > 20, "noise should vary, got {distinct} changes");
}

#[test]
fn pitch_modulation_follows_the_previous_voice() {
    let mut spu = spu_with_tone();
    // Voice 1 plays the same block, modulated by voice 0 (which is silent).
    spu.write(0x10, 2, 0x3FFF);
    spu.write(0x12, 2, 0x3FFF);
    spu.write(0x14, 2, 0x1000);
    spu.write(0x16, 2, (BLOCK_ADDRESS / 8) as u32);
    spu.write(0x18, 2, 0x000F);
    spu.write(PITCH_MOD_LOW, 2, 2);
    spu.sound_ram[BLOCK_ADDRESS + 1] = 0x01;
    spu.write(KEY_ON_LOW, 2, 2);
    run_samples(&mut spu, 60);
    // A silent modulator gives a factor of 0x8000 / 0x8000 = 1: normal speed,
    // so the one-shot block has ended by now.
    assert!(!spu.voice_active(1));
}

#[test]
fn the_irq_fires_when_a_voice_reads_the_watched_address() {
    let mut spu = spu_with_tone();
    spu.write(0x1AA, 2, 0xC040); // IRQ enabled
    spu.write(IRQ_ADDRESS, 2, (BLOCK_ADDRESS / 8) as u32);
    assert!(!spu.take_irq());
    spu.write(KEY_ON_LOW, 2, 1);
    run_samples(&mut spu, 2);
    assert!(spu.take_irq());
    assert_ne!(spu.read(SPUSTAT_OFFSET, 2) & 0x40, 0);
    assert!(!spu.take_irq(), "one interrupt per trigger");

    spu.write(0x1AA, 2, 0xC000); // clearing the enable bit acknowledges
    assert_eq!(spu.read(SPUSTAT_OFFSET, 2) & 0x40, 0);
}

#[test]
fn the_irq_fires_on_transfers_to_the_watched_address() {
    let mut spu = Spu::new();
    spu.write(0x1AA, 2, 0xC040);
    spu.write(IRQ_ADDRESS, 2, 0x10);
    spu.write(TRANSFER_ADDRESS_OFFSET, 2, 0x10);
    spu.write(TRANSFER_FIFO_OFFSET, 2, 0x1234);
    assert!(spu.take_irq());
}

#[test]
fn status_reports_the_dma_mode() {
    let mut spu = Spu::new();
    spu.write(0x1AA, 2, 0x0020); // DMA write
    assert_eq!(spu.read(SPUSTAT_OFFSET, 2) & 0x3C0, 0x180);
    spu.write(0x1AA, 2, 0x0030); // DMA read
    assert_eq!(spu.read(SPUSTAT_OFFSET, 2) & 0x3C0, 0x280);
}

#[test]
fn current_envelope_level_is_readable() {
    let mut spu = spu_with_tone();
    spu.write(KEY_ON_LOW, 2, 1);
    run_samples(&mut spu, 40);
    assert_eq!(spu.read(0x0C, 2), 0x7FFF);
}

#[test]
fn the_repeat_address_register_follows_loop_start_flags() {
    let mut spu = spu_with_tone();
    spu.sound_ram[BLOCK_ADDRESS + 1] = 0x07; // loop start + end + repeat
    spu.write(KEY_ON_LOW, 2, 1);
    run_samples(&mut spu, 2);
    assert_eq!(spu.read(0x0E, 2) as usize, BLOCK_ADDRESS / 8);
}

#[test]
fn reverb_echoes_the_input_when_enabled() {
    let mut spu = Spu::new();
    spu.write(0x1AA, 2, 0xC080); // enabled, unmuted, reverb on
    spu.write(MAIN_VOLUME_LEFT, 2, 0x3FFF);
    spu.write(MAIN_VOLUME_RIGHT, 2, 0x3FFF);
    spu.write(REVERB_VOLUME_LEFT, 2, 0x7FFF);
    spu.write(REVERB_VOLUME_RIGHT, 2, 0x7FFF);
    spu.write(REVERB_START, 2, 0xE000); // work area at 0x70000
    let set = |spu: &mut Spu, index: u32, value: u32| spu.write(REVERB_REGS + index * 2, 2, value);
    set(&mut spu, 0, 1); // dAPF1: 8 bytes
    set(&mut spu, 1, 1); // dAPF2: 8 bytes
    set(&mut spu, 2, 0x7FFF); // vIIR
    set(&mut spu, 3, 0x7FFF); // vCOMB1
    set(&mut spu, 10, 0x10); // mLSAME
    set(&mut spu, 12, 0x10); // mLCOMB1 reads the cell mLSAME writes
    set(&mut spu, 26, 0x20); // mLAPF1
    set(&mut spu, 28, 0x30); // mLAPF2
    set(&mut spu, 30, 0x7FFF); // vLIN
    set(&mut spu, 31, 0x7FFF); // vRIN

    // One voice-less impulse: feed it through the CD input.
    spu.write(0x1AA, 2, 0xC085); // CD audio and CD reverb on as well
    spu.write(CD_VOLUME_LEFT, 2, 0x7FFF);
    spu.write(CD_VOLUME_RIGHT, 2, 0x7FFF);
    spu.write(0x1AA, 2, 0xC085);
    spu.tick_sample((20000, 0));
    spu.tick_sample((0, 0));
    for _ in 0..40 {
        spu.tick_sample((0, 0));
    }
    let echoes = spu.output_buffer.chunks(2).skip(2).filter(|f| f[0].abs() > 0.1).count();
    assert!(echoes >= 1, "the reverb should return the impulse later");
}
