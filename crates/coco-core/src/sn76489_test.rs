use super::*;

/// 4 MHz — the GMC's crystal.
const CRYSTAL_HZ: f64 = 4_000_000.0;
/// A `sample()` interval worth exactly one internal tick.
const ONE_TICK: f64 = CRYSTAL_CYCLES_PER_TICK / CRYSTAL_HZ;

fn chip() -> SN76489A {
    SN76489A::new(CRYSTAL_HZ)
}

/// Silence every channel (attenuation code 15 on registers 1/3/5/7).
fn mute_all(psg: &mut SN76489A) {
    for cmd in [0x9F, 0xBF, 0xDF, 0xFF] {
        psg.write(cmd);
    }
}

#[test]
fn latch_plus_data_byte_form_the_ten_bit_tone_period() {
    let mut psg = chip();
    // Tone 0 period 0x264: latch low nibble 4, then data byte 0x26.
    psg.write(0x84);
    psg.write(0x26);
    assert_eq!(psg.regs[0], 0x264);
    assert_eq!(psg.period[0], 0x264);
}

#[test]
fn data_only_byte_reuses_the_last_latched_register() {
    let mut psg = chip();
    psg.write(0x84); // latch tone 0, nibble 4
    psg.write(0x26); // data -> tone 0 upper bits
    psg.write(0x01); // still tone 0: replaces upper bits only
    assert_eq!(psg.regs[0], 0x014);
}

#[test]
fn period_zero_counts_as_0x400() {
    let mut psg = chip();
    psg.write(0x80); // tone 0 period = 0
    psg.write(0x00);
    assert_eq!(psg.regs[0], 0);
    assert_eq!(psg.period[0], u32::from(PERIOD_ZERO_COUNTS_AS));
}

#[test]
fn tone_flip_flop_toggles_every_period_ticks() {
    const PERIOD: u16 = 100;
    let mut psg = chip();
    mute_all(&mut psg);
    psg.write(0x90); // tone 0 attenuation 0 (max)
    psg.write(0x80 | (PERIOD & 0x0F) as u8);
    psg.write((PERIOD >> 4) as u8);

    // Tick one at a time (directly, so the count is exact); record the
    // interval between output changes.
    psg.tick();
    let mut last = psg.level();
    let mut transitions = Vec::new();
    for t in 0..2000u32 {
        psg.tick();
        let level = psg.level();
        if level != last {
            transitions.push(t);
            last = level;
        }
    }
    assert!(transitions.len() >= 2, "tone never toggled");
    for pair in transitions.windows(2) {
        assert_eq!(
            pair[1] - pair[0],
            u32::from(PERIOD),
            "half-period must be exactly the period register value in ticks"
        );
    }
}

#[test]
fn attenuation_is_2db_per_step_and_code_15_is_silence() {
    let psg = chip();
    assert_eq!(psg.vol_table[0], CHANNEL_FULL_SCALE as f32);
    assert_eq!(psg.vol_table[15], 0.0);
    for code in 0..14 {
        let ratio = psg.vol_table[code + 1] / psg.vol_table[code];
        assert!(
            (f64::from(ratio) - 1.0 / ATTENUATION_STEP).abs() < 1e-6,
            "step {code} ratio {ratio}"
        );
    }
}

#[test]
fn volume_write_applies_immediately_from_the_data_nibble() {
    let mut psg = chip();
    psg.write(0x95); // tone 0 attenuation 5
    assert_eq!(psg.volume[0], psg.vol_table[5]);
    psg.write(0x03); // DATA-only to the same register
    assert_eq!(psg.volume[0], psg.vol_table[3]);
}

#[test]
fn noise_rate_field_selects_32_64_128_tick_periods() {
    let mut psg = chip();
    for (rate, want) in [(0u8, 32u32), (1, 64), (2, 128)] {
        psg.write(0xE0 | rate);
        assert_eq!(psg.period[NOISE_CHANNEL], want);
    }
}

#[test]
fn noise_rate_3_mirrors_tone_2_doubled_and_tracks_live_retunes() {
    let mut psg = chip();
    psg.write(0xC8); // tone 2 period low nibble 8
    psg.write(0x02); // upper bits -> period 0x028
    psg.write(0xE3); // noise: rate follows tone 2
    assert_eq!(psg.period[NOISE_CHANNEL], 0x28 << 1);
    psg.write(0xC4); // retune tone 2 low nibble -> 0x024
    assert_eq!(psg.period[NOISE_CHANNEL], 0x24 << 1);
}

#[test]
fn any_noise_control_write_reseeds_the_lfsr() {
    let mut psg = chip();
    psg.write(0xE0);
    for _ in 0..100 {
        psg.shift_lfsr();
    }
    assert_ne!(psg.lfsr, LFSR_FEEDBACK);
    psg.write(0xE4); // latch write to reg 6
    assert_eq!(psg.lfsr, LFSR_FEEDBACK);
    for _ in 0..100 {
        psg.shift_lfsr();
    }
    psg.write(0x04); // DATA-only continuation to reg 6 reseeds too
    assert_eq!(psg.lfsr, LFSR_FEEDBACK);
}

#[test]
fn periodic_noise_is_one_set_bit_circulating_over_15_shifts() {
    let mut psg = chip();
    psg.write(0xE0); // periodic (bit 2 clear), reseeds LFSR
    let mut outputs = Vec::new();
    for _ in 0..60 {
        psg.shift_lfsr();
        outputs.push(psg.lfsr & 1);
    }
    let ones: Vec<usize> = (0..outputs.len()).filter(|&i| outputs[i] == 1).collect();
    assert!(ones.len() >= 3, "periodic noise never reached bit 0");
    for pair in ones.windows(2) {
        assert_eq!(pair[1] - pair[0], 15, "periodic noise period must be 15");
    }
}

#[test]
fn white_noise_diverges_from_periodic() {
    let sequence = |mode_cmd: u8| {
        let mut psg = chip();
        psg.write(mode_cmd);
        (0..64)
            .map(|_| {
                psg.shift_lfsr();
                psg.lfsr & 1
            })
            .collect::<Vec<_>>()
    };
    assert_ne!(sequence(0xE0), sequence(0xE4));
}

#[test]
fn power_on_state_hums_at_max_volume() {
    // MAME device_start: attenuation 0 (max) everywhere, tone periods
    // $400 — the chip is audible before software touches it.
    let mut psg = chip();
    let mut heard = false;
    for _ in 0..3000 {
        if psg.sample(ONE_TICK) > 0.0 {
            heard = true;
            break;
        }
    }
    assert!(heard);
}

#[test]
fn mean_sampling_of_a_fast_tone_settles_near_half_volume() {
    let mut psg = chip();
    mute_all(&mut psg);
    psg.write(0x90); // tone 0 max volume
    psg.write(0x81); // period 1: toggles every tick
    psg.write(0x00);
    // 1000 ticks per sample: the box filter averages the 125 kHz square
    // wave to ~vol/2 instead of aliasing.
    let level = psg.sample(1000.0 * ONE_TICK);
    let half = psg.vol_table[0] / 2.0;
    assert!((level - half).abs() < 0.01, "expected ~{half}, got {level}");
}
