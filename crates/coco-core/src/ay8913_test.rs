use super::*;

// ---- Register masking ---------------------------------------------------

#[test]
fn tone_coarse_register_masks_to_4_bits() {
    let mut ay = AY8913::new();
    ay.write_reg(reg::TONE_A_COARSE, 0xFF);
    assert_eq!(ay.read_reg(reg::TONE_A_COARSE), 0x0F);
}

#[test]
fn noise_period_register_masks_to_5_bits() {
    let mut ay = AY8913::new();
    ay.write_reg(reg::NOISE_PERIOD, 0xFF);
    assert_eq!(ay.read_reg(reg::NOISE_PERIOD), 0x1F);
}

// ---- Tone generator -------------------------------------------------------

#[test]
fn tone_toggles_once_per_period_internal_steps() {
    let mut ay = AY8913::new();
    const TONE_PERIOD: u16 = 100;
    ay.write_reg(reg::TONE_A_FINE, (TONE_PERIOD & 0xFF) as u8);
    ay.write_reg(reg::TONE_A_COARSE, (TONE_PERIOD >> 8) as u8);

    const INTERNAL_STEPS: u32 = 10_000;
    let mut edges = 0u32;
    let mut prev = ay.tone[0].output;
    for _ in 0..INTERNAL_STEPS {
        ay.internal_step();
        if ay.tone[0].output != prev {
            edges += 1;
            prev = ay.tone[0].output;
        }
    }
    // A toggle happens once every TONE_PERIOD internal steps (f = clock/(16*TONE_PERIOD) in master-clock terms).
    let expected = INTERNAL_STEPS / u32::from(TONE_PERIOD);
    assert!(
        edges.abs_diff(expected) <= 1,
        "edges={edges} expected={expected}"
    );
}

// ---- Noise LFSR -------------------------------------------------------------

#[test]
fn lfsr_matches_bit0_xor_bit3_recurrence_from_seed() {
    let mut ay = AY8913::new();
    assert_eq!(ay.rng, NOISE_SEED, "seed must be non-zero at power-on");
    let mut expected = NOISE_SEED;
    for _ in 0..32 {
        let bit0 = expected & 1;
        let bit3 = (expected >> 3) & 1;
        expected = (expected >> 1) | ((bit0 ^ bit3) << 16);
        ay.shift_noise();
        assert_eq!(ay.rng, expected);
    }
}

// ---- Envelope shapes --------------------------------------------------------

/// Runs the envelope generator for `steps` internal steps at envelope period 1
/// (effective period [`ENVELOPE_STEP_MULTIPLIER`] after the classic-AY pacing multiplier).
fn run_envelope(ay: &mut AY8913, shape_byte: u8, steps: u32) -> u8 {
    ay.write_reg(reg::ENV_FINE, 1);
    ay.write_reg(reg::ENV_COARSE, 0);
    ay.write_reg(reg::ENV_SHAPE, shape_byte);
    let period = ay.env_period() * ENVELOPE_STEP_MULTIPLIER;
    for _ in 0..steps {
        ay.envelope.step_once(period);
    }
    ay.envelope.volume()
}

#[test]
fn shape_0d_attacks_then_holds_at_max() {
    let mut ay = AY8913::new();
    ay.write_reg(reg::ENV_FINE, 1);
    ay.write_reg(reg::ENV_COARSE, 0);
    ay.write_reg(reg::ENV_SHAPE, 0x0D);
    assert_eq!(ay.envelope.volume(), 0, "attack shape starts at the bottom");
    let full_ramp = 16 * ENVELOPE_STEP_MULTIPLIER;
    let period = ay.env_period() * ENVELOPE_STEP_MULTIPLIER;
    for _ in 0..full_ramp + 4 {
        ay.envelope.step_once(period);
    }
    assert_eq!(ay.envelope.volume(), 0x0F, "0x0D holds at the max level");
    // Holding: further steps must not change it.
    for _ in 0..full_ramp {
        ay.envelope.step_once(period);
    }
    assert_eq!(ay.envelope.volume(), 0x0F);
}

#[test]
fn shape_00_family_decays_then_holds_at_zero() {
    let mut ay = AY8913::new();
    let full_ramp = 16 * ENVELOPE_STEP_MULTIPLIER;
    assert_eq!(
        run_envelope(&mut ay, 0x00, 0),
        0x0F,
        "decay shape starts at the top"
    );
    let v = run_envelope(&mut ay, 0x00, full_ramp + 4);
    assert_eq!(v, 0x00, "0x00 decays to and holds at 0");
}

#[test]
fn shape_08_is_a_repeating_sawtooth() {
    let mut ay = AY8913::new();
    let full_ramp = 16 * ENVELOPE_STEP_MULTIPLIER;
    let v_start = run_envelope(&mut ay, 0x08, 0);
    assert_eq!(v_start, 0x0F);
    let v_mid = run_envelope(&mut ay, 0x08, full_ramp / 2);
    assert!(
        v_mid < v_start,
        "midway through the ramp it must have decayed"
    );
    let v_wrapped = run_envelope(&mut ay, 0x08, full_ramp);
    assert_eq!(
        v_wrapped, 0x0F,
        "one full ramp must wrap back to the top, not hold (repeating, not one-shot)"
    );
}

// ---- Volume DAC table --------------------------------------------------------

#[test]
fn volume_table_is_monotonic_nondecreasing_and_normalized() {
    let table = build_volume_table();
    assert_eq!(table[0], 0.0, "quietest step must be exactly silent");
    assert_eq!(table[15], 1.0, "loudest step must be exactly full scale");
    for pair in table.windows(2) {
        assert!(
            pair[1] >= pair[0],
            "volume table must be nondecreasing: {table:?}"
        );
    }
}

// ---- Mixer gating --------------------------------------------------------------

#[test]
fn mixer_disable_bits_gate_the_channel() {
    let mut ay = AY8913::new();
    ay.write_reg(reg::VOL_A, 0x0F); // full scale, fixed level
    ay.write_reg(reg::TONE_A_FINE, 4);
    ay.write_reg(reg::TONE_A_COARSE, 0);

    // Both disabled (active-low bits set) forces the gate constant true (MAME: output is 1, not 0, when both are disabled).
    ay.write_reg(reg::MIXER, 0b0000_1001);
    ay.step(2_000 * MASTER_CLOCK_DIVIDER);
    let constant = ay.drain();

    // Tone enabled, noise still off: gate follows the ~50%-duty square wave, so the average output drops.
    ay.write_reg(reg::MIXER, 0b0000_1000);
    ay.step(2_000 * MASTER_CLOCK_DIVIDER);
    let toggling = ay.drain();

    assert!(
        toggling < constant - 0.1,
        "gating the tone in must reduce the average output: toggling={toggling} constant={constant}"
    );
}
