//! Host byte protocol (SOUND half): command dispatch, buffer RAM load/execute,
//! direct access, and the sound-data engine. Speech/allophone/SP0256 stays a
//! parse-and-discard no-op -- covered here only to the extent of proving it
//! doesn't desync the state machine for later commands.

use coco_core::SystemBus;
use coco_core::ay8913::reg as ay_reg;
use coco_core::ssc::{cmd, group, ram, terminator, timing};
use mc6809::Bus;

use super::common::{CLEAR_BUSY, FF7E, bus_with_ssc_selected, coco3_bus_with_ssc, pump};

/// Writes one byte to `$FF7E` and ticks the cart's clock well past the
/// synthetic busy-hold window ([`CLEAR_BUSY`]), so the next write is accepted
/// rather than dropped by the busy-lost gate.
fn send(b: &mut SystemBus, byte: u8) {
    b.write(FF7E, byte);
    b.cart.tick(CLEAR_BUSY);
}

#[test]
fn direct_access_end_to_end_pokes_ay_and_drives_audio() {
    let mut b = bus_with_ssc_selected();
    send(&mut b, cmd::DIRECT_ACCESS_TOGGLE);

    // A period long enough that box-filtered samples swing between near-
    // silent and near-full-scale -- same rationale as `TEST_TONE_PERIOD`.
    const TONE_A_PERIOD: u16 = 1000;
    /// Mixer value enabling only channel A's tone generator: bit0 (tone A
    /// disable) clear, every other disable bit set.
    const MIXER_TONE_A_ONLY: u8 = 0x3E;

    send(&mut b, ay_reg::TONE_A_FINE);
    send(&mut b, (TONE_A_PERIOD & 0xFF) as u8);
    send(&mut b, ay_reg::TONE_A_COARSE);
    send(&mut b, (TONE_A_PERIOD >> 8) as u8);
    send(&mut b, ay_reg::MIXER);
    send(&mut b, MIXER_TONE_A_ONLY);
    send(&mut b, ay_reg::VOL_A);
    send(&mut b, 0x0F);
    send(&mut b, terminator::SOUND); // $FF at a pair-start position exits direct-access mode

    {
        let ssc = b.cart.as_ssc().unwrap();
        assert_eq!(
            ssc.ay_read(ay_reg::TONE_A_FINE),
            (TONE_A_PERIOD & 0xFF) as u8
        );
        assert_eq!(
            ssc.ay_read(ay_reg::TONE_A_COARSE),
            (TONE_A_PERIOD >> 8) as u8
        );
        assert_eq!(ssc.ay_read(ay_reg::MIXER), MIXER_TONE_A_ONLY);
        assert_eq!(ssc.ay_read(ay_reg::VOL_A), 0x0F);
    }

    let sample = pump(&mut b, 2_000);
    assert!(
        sample > 0.0,
        "direct-access-programmed tone must be audible: {sample}"
    );
    assert_eq!(
        b.read(FF7E) & 0x20,
        0x00,
        "SAC bit5 (QUIET) must clear while sound plays"
    );

    send(&mut b, cmd::STOP_ALL_SOUND);
    {
        let ssc = b.cart.as_ssc().unwrap();
        assert_eq!(
            ssc.ay_read(ay_reg::VOL_A),
            0,
            "$00 stop-all-sound must zero every channel volume"
        );
    }
    pump(&mut b, 20_000);
    assert_eq!(
        b.read(FF7E) & 0x20,
        0x20,
        "SAC bit5 (QUIET) must set again once the envelope decays"
    );
}

#[test]
fn sound_data_load_and_execute_individual_tone_event() {
    let mut b = coco3_bus_with_ssc();

    const AMP: u8 = 0x0A;
    const COARSE: u8 = 0x05;
    const FINE: u8 = 100;
    const DURATION: u8 = 0x10;
    let byte0 = (group::TONE_A << group::OPCODE_SHIFT) | AMP; // M=0: no chained envelope
    let stream = [byte0, COARSE, FINE, DURATION, terminator::SOUND];

    send(&mut b, cmd::LOAD_SOUND_INDIVIDUAL_START); // buffer 0
    for &byte in &stream {
        send(&mut b, byte);
    }

    send(&mut b, cmd::EXEC_SOUND_INDIVIDUAL_START); // buffer 0: runs the event synchronously

    {
        let ssc = b.cart.as_ssc().unwrap();
        assert_eq!(ssc.ay_read(ay_reg::TONE_A_COARSE), COARSE);
        assert_eq!(ssc.ay_read(ay_reg::TONE_A_FINE), FINE);
        assert_eq!(ssc.ay_read(ay_reg::VOL_A), AMP);
        let mixer = ssc.ay_read(ay_reg::MIXER);
        assert_eq!(
            mixer & 0b0000_1001,
            0b0000_1000,
            "tone A enabled (bit0=0), noise A disabled (bit3=1)"
        );
    }

    b.cart
        .tick(timing::duration_cycles(DURATION, timing::DEFAULT_TIMER_BASE) + 1);

    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(
        ssc.ay_read(ay_reg::VOL_A),
        AMP,
        "end-of-stream must not silence the AY"
    );
}

#[test]
fn noise_reuse_flag_carries_forward_previous_amplitude() {
    let mut b = coco3_bus_with_ssc();

    const FIRST_AMP: u8 = 0x0C;
    const SECOND_AMP: u8 = 0x03; // must be ignored: the second group sets the reuse flag
    const FIRST_PERIOD: u8 = 0x0A;
    const SECOND_PERIOD: u8 = 0x0B;
    const FIRST_DURATION: u8 = 5;
    const SECOND_DURATION: u8 = 7;

    let noise_opcode = group::NOISE_A << group::OPCODE_SHIFT;
    let group1 = [noise_opcode | FIRST_AMP, FIRST_PERIOD, FIRST_DURATION];
    let group2 = [
        noise_opcode | SECOND_AMP,
        group::NOISE_REUSE_FLAG | SECOND_PERIOD,
        SECOND_DURATION,
    ];

    send(&mut b, cmd::LOAD_SOUND_INDIVIDUAL_START); // buffer 0
    for &byte in group1.iter().chain(group2.iter()) {
        send(&mut b, byte);
    }
    send(&mut b, terminator::SOUND);

    send(&mut b, cmd::EXEC_SOUND_INDIVIDUAL_START); // buffer 0: first group runs synchronously

    {
        let ssc = b.cart.as_ssc().unwrap();
        assert_eq!(
            ssc.ay_read(ay_reg::VOL_A),
            FIRST_AMP,
            "first group's own amplitude"
        );
    }

    b.cart
        .tick(timing::duration_cycles(FIRST_DURATION, timing::DEFAULT_TIMER_BASE) + 1);

    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(
        ssc.ay_read(ay_reg::VOL_A),
        FIRST_AMP,
        "the reuse flag must carry forward the FIRST group's amplitude, not the second group's own bits"
    );
    assert_eq!(
        ssc.ay_read(ay_reg::NOISE_PERIOD),
        SECOND_PERIOD,
        "the period must still come from the second group"
    );
    assert_ne!(
        FIRST_AMP, SECOND_AMP,
        "test setup: amplitudes must differ to be distinguishable"
    );
}

#[test]
fn tone_plus_envelope_pair_uses_the_envelope_groups_own_duration() {
    let mut b = coco3_bus_with_ssc();

    const TONE_AMP: u8 = 9;
    const TONE_DURATION: u8 = 3; // deliberately small
    const ENV_SHAPE: u8 = 0x0D;
    const ENV_COARSE: u8 = 4;
    const ENV_FINE: u8 = 5;
    const ENV_DURATION: u8 = 50; // deliberately large and different from TONE_DURATION
    const MARKER_AMP: u8 = 7;

    let tone = [
        (group::TONE_A << group::OPCODE_SHIFT) | group::M_FLAG | TONE_AMP,
        1,
        2,
        TONE_DURATION,
    ];
    let envelope = [
        (group::ENVELOPE_LOW << group::OPCODE_SHIFT) | ENV_SHAPE,
        ENV_COARSE,
        ENV_FINE,
        ENV_DURATION,
    ];
    // A plain (M=0) tone-B marker group: its VOL_B write only happens once
    // the engine actually advances past the chained tone+envelope event.
    let marker = [(group::TONE_B << group::OPCODE_SHIFT) | MARKER_AMP, 6, 7, 0];

    send(&mut b, cmd::LOAD_SOUND_INDIVIDUAL_START); // buffer 0
    for &byte in tone.iter().chain(envelope.iter()).chain(marker.iter()) {
        send(&mut b, byte);
    }
    send(&mut b, terminator::SOUND);

    // Dispatch the EXEC command directly (no trailing busy-clear tick) so
    // the subsequent cycle counts below are exact against `duration_cycles`.
    b.write(FF7E, cmd::EXEC_SOUND_INDIVIDUAL_START); // buffer 0

    {
        let ssc = b.cart.as_ssc().unwrap();
        assert_eq!(ssc.ay_read(ay_reg::TONE_A_COARSE), 1);
        assert_eq!(ssc.ay_read(ay_reg::TONE_A_FINE), 2);
        assert_eq!(ssc.ay_read(ay_reg::VOL_A), TONE_AMP | group::M_FLAG);
        assert_eq!(
            ssc.ay_read(ay_reg::ENV_SHAPE),
            ENV_SHAPE,
            "chained envelope group must program ENV_SHAPE"
        );
        assert_eq!(ssc.ay_read(ay_reg::ENV_COARSE), ENV_COARSE);
        assert_eq!(ssc.ay_read(ay_reg::ENV_FINE), ENV_FINE);
        assert_eq!(
            ssc.ay_read(ay_reg::VOL_B),
            0,
            "marker group must not have run yet"
        );
    }

    // Enough cycles for the TONE group's own duration, but nowhere near the
    // ENVELOPE group's -- if the engine wrongly used the tone's duration,
    // the marker would already have fired here.
    b.cart
        .tick(timing::duration_cycles(TONE_DURATION, timing::DEFAULT_TIMER_BASE) + 1);
    {
        let ssc = b.cart.as_ssc().unwrap();
        assert_eq!(
            ssc.ay_read(ay_reg::VOL_B),
            0,
            "combined tone+envelope event must be scheduled by the ENVELOPE group's duration, not the tone's"
        );
    }

    // Now cross the ENVELOPE group's own duration.
    b.cart
        .tick(timing::duration_cycles(ENV_DURATION, timing::DEFAULT_TIMER_BASE) + 1_000);
    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(
        ssc.ay_read(ay_reg::VOL_B),
        MARKER_AMP,
        "marker group must fire once the envelope's duration elapses"
    );
}

#[test]
fn timer_base_scales_sound_event_duration() {
    const AMP_A: u8 = 5;
    const DURATION: u8 = 10;
    const MARKER_AMP: u8 = 6;

    for &base in &[1u8, 250u8] {
        let mut b = coco3_bus_with_ssc();
        let tone = [
            (group::TONE_A << group::OPCODE_SHIFT) | AMP_A,
            1,
            2,
            DURATION,
        ];
        let marker = [(group::TONE_B << group::OPCODE_SHIFT) | MARKER_AMP, 3, 4, 0];

        send(&mut b, cmd::LOAD_SOUND_INDIVIDUAL_START); // buffer 0
        for &byte in tone.iter().chain(marker.iter()) {
            send(&mut b, byte);
        }
        send(&mut b, terminator::SOUND);

        send(&mut b, cmd::LOAD_TIMER_BASE);
        send(&mut b, base);

        // Dispatch directly (no trailing busy-clear tick) so the following
        // cycle counts are exact.
        b.write(FF7E, cmd::EXEC_SOUND_INDIVIDUAL_START); // buffer 0

        let expected = timing::duration_cycles(DURATION, base);
        b.cart.tick(expected - 1);
        {
            let ssc = b.cart.as_ssc().unwrap();
            assert_eq!(
                ssc.ay_read(ay_reg::VOL_B),
                0,
                "marker must not fire before the base-scaled duration elapses (base={base})"
            );
        }

        b.cart.tick(2);
        let ssc = b.cart.as_ssc().unwrap();
        assert_eq!(
            ssc.ay_read(ay_reg::VOL_B),
            MARKER_AMP,
            "marker must fire once the base-scaled duration elapses (base={base})"
        );
    }
}

#[test]
fn busy_lost_second_byte_is_discarded_before_hold_window_elapses() {
    let mut b = coco3_bus_with_ssc();
    b.write(FF7E, cmd::DIRECT_ACCESS_TOGGLE); // latches $AF, busy set
    // Before busy clears (no tick in between), a second, different byte
    // must be dropped entirely -- not latched, not dispatched.
    b.write(FF7E, 0x42);

    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(
        ssc.host_latch(),
        cmd::DIRECT_ACCESS_TOGGLE,
        "the second byte sent while busy must be discarded, not latched"
    );
}

#[test]
fn ascii_text_bytes_are_consumed_and_discarded_then_normal_commands_still_work() {
    let mut b = coco3_bus_with_ssc();
    for &byte in b"HELLO" {
        send(&mut b, byte);
    }
    send(&mut b, terminator::SPEECH); // 0x0D: discarded like any other byte in this mode

    // A normal command afterward must still dispatch correctly.
    send(&mut b, cmd::DIRECT_ACCESS_TOGGLE);
    send(&mut b, ay_reg::VOL_A);
    send(&mut b, 0x07);
    send(&mut b, terminator::SOUND);

    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(
        ssc.ay_read(ay_reg::VOL_A),
        0x07,
        "a direct-access command after ASCII passthrough must still work"
    );
}

#[test]
fn individual_load_without_terminator_ends_at_capacity_and_reprocesses_overflow_byte() {
    let mut b = coco3_bus_with_ssc();
    send(&mut b, cmd::LOAD_SOUND_INDIVIDUAL_START); // buffer 0, 64-byte cap

    for i in 0..ram::BUFFER_SIZE {
        send(&mut b, (i % 0x100) as u8); // never 0xFF: BUFFER_SIZE (64) < 0xFF
    }

    // The 65th accepted byte: capacity was exhausted without ever seeing a
    // terminator, so this byte is reprocessed as a fresh command instead of
    // being stored.
    send(&mut b, cmd::DIRECT_ACCESS_TOGGLE);
    send(&mut b, ay_reg::VOL_B);
    send(&mut b, 0x05);
    send(&mut b, terminator::SOUND);

    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(
        ssc.ay_read(ay_reg::VOL_B),
        0x05,
        "the overflow byte must have been reprocessed as $AF"
    );
}

#[test]
fn sound_data_execute_never_runs_an_incomplete_trailing_group() {
    let mut b = coco3_bus_with_ssc();

    // Buffer 1 (individual: offsets 64..128, EXEC cap 128). Pad with 14
    // tone groups (4 bytes each) + 2 noise groups (3 bytes each) = 62
    // bytes, then a dangling 2-byte opcode+amp fragment -- landing exactly
    // 2 bytes before the EXEC cap, where a tone group (needing 4 bytes)
    // cannot fully fit.
    send(&mut b, cmd::LOAD_SOUND_INDIVIDUAL_START + 1); // buffer 1

    const PAD_TONE_GROUPS: usize = 14;
    const PAD_NOISE_GROUPS: usize = 2;
    for _ in 0..PAD_TONE_GROUPS {
        // amp/coarse/fine/duration all 0: harmless filler whose duration
        // (duration_cycles(0, _) == 0) instantly expires, so a single-cycle
        // tick advances past it.
        for &byte in &[group::TONE_A << group::OPCODE_SHIFT, 0, 0, 0] {
            send(&mut b, byte);
        }
    }
    for _ in 0..PAD_NOISE_GROUPS {
        for &byte in &[group::NOISE_A << group::OPCODE_SHIFT, 0, 0] {
            send(&mut b, byte);
        }
    }
    assert_eq!(
        PAD_TONE_GROUPS * 4 + PAD_NOISE_GROUPS * 3,
        62,
        "test setup arithmetic"
    );

    // Dangling group: a tone-B opcode+amplitude byte, plus one filler byte
    // -- 2 of the 4 bytes a tone group needs. A distinctive amplitude makes
    // a wrongly-executed partial group detectable via VOL_B.
    send(&mut b, (group::TONE_B << group::OPCODE_SHIFT) | 0x0F);
    send(&mut b, 0x00); // would-be "coarse" byte if the group were complete

    // The buffer is now exactly full (64/64 bytes); the load ends on
    // capacity exhaustion when the next byte (the EXEC command) arrives,
    // which is reprocessed as that command.
    send(&mut b, cmd::EXEC_SOUND_INDIVIDUAL_START + 1); // buffer 1

    // Drain every padding group one at a time, plus a generous margin --
    // ticks after the engine goes inactive are no-ops.
    for _ in 0..(PAD_TONE_GROUPS + PAD_NOISE_GROUPS + 4) {
        b.cart.tick(1);
    }

    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(
        ssc.ay_read(ay_reg::VOL_B),
        0,
        "an incomplete trailing tone-B group must never be parsed or played"
    );
}

#[test]
fn speech_load_command_parses_and_discards_without_desyncing_later_commands() {
    let mut b = coco3_bus_with_ssc();
    send(&mut b, cmd::LOAD_SPEECH_CONSECUTIVE_START); // $80: load speech string, buffers 0..=7
    for &byte in b"HELLO WORLD" {
        send(&mut b, byte);
    }
    send(&mut b, terminator::SPEECH); // ends the speech load

    send(&mut b, cmd::DIRECT_ACCESS_TOGGLE);
    send(&mut b, ay_reg::VOL_C);
    send(&mut b, 0x0A);
    send(&mut b, terminator::SOUND);

    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(
        ssc.ay_read(ay_reg::VOL_C),
        0x0A,
        "a normal command after a speech LOAD must still work"
    );
}

#[test]
fn register_string_load_and_execute_applies_all_pairs() {
    let mut b = coco3_bus_with_ssc();
    let pairs: [(u8, u8); 3] = [
        (ay_reg::TONE_A_FINE, 0x33),
        (ay_reg::TONE_A_COARSE, 0x02),
        (ay_reg::VOL_A, 0x0D),
    ];

    send(&mut b, cmd::LOAD_REGISTER_INDIVIDUAL_START); // $B8: buffer 0
    for &(register, value) in &pairs {
        send(&mut b, register);
        send(&mut b, value);
    }
    send(&mut b, terminator::SOUND);

    send(&mut b, cmd::EXEC_REGISTER_INDIVIDUAL_START); // $F8: buffer 0

    let ssc = b.cart.as_ssc().unwrap();
    for &(register, value) in &pairs {
        assert_eq!(
            ssc.ay_read(register),
            value,
            "register {register:#04x} must have been applied"
        );
    }
}
