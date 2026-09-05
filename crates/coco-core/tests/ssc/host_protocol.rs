//! The host-byte protocol as the firmware runs it: direct AY access, sound
//! data, register strings, text passthrough. Every test here needs both
//! SSC ROMs and skips without them; each byte is handed over the way real
//! software does it, polling BUSY* in between.

use coco_core::SystemBus;
use coco_core::ay8913::reg as ay_reg;
use coco_core::ssc::{cmd, group, terminator};
use mc6809::Bus;

use super::common::{
    FF7E, QUIET, pump, send, settle, settle_bytes, skip, try_bus_with_ssc_selected,
    try_coco3_bus_with_ssc,
};

fn ay(b: &mut SystemBus, reg: u8) -> u8 {
    b.cart.as_ssc().unwrap().ay_read(reg)
}

#[test]
fn direct_access_end_to_end_pokes_ay_and_drives_audio() {
    let Some(mut b) = try_bus_with_ssc_selected() else {
        return skip("direct_access_end_to_end_pokes_ay_and_drives_audio");
    };
    // A period long enough that box-filtered samples swing between near-
    // silent and near-full-scale — the same rationale as `TEST_TONE_PERIOD`.
    const TONE_A_PERIOD: u16 = 1000;
    /// Mixer value enabling only channel A's tone generator.
    const MIXER_TONE_A_ONLY: u8 = 0x3E;

    send(&mut b, cmd::DIRECT_ACCESS_TOGGLE);
    for (reg, val) in [
        (ay_reg::TONE_A_FINE, (TONE_A_PERIOD & 0xFF) as u8),
        (ay_reg::TONE_A_COARSE, (TONE_A_PERIOD >> 8) as u8),
        (ay_reg::MIXER, MIXER_TONE_A_ONLY),
        (ay_reg::VOL_A, 0x0F),
    ] {
        send(&mut b, reg);
        send(&mut b, val);
    }
    send(&mut b, terminator::SOUND); // $FF where a register is expected exits
    settle(&mut b);

    assert_eq!(
        ay(&mut b, ay_reg::TONE_A_FINE),
        (TONE_A_PERIOD & 0xFF) as u8
    );
    assert_eq!(
        ay(&mut b, ay_reg::TONE_A_COARSE),
        (TONE_A_PERIOD >> 8) as u8
    );
    assert_eq!(ay(&mut b, ay_reg::MIXER), MIXER_TONE_A_ONLY);
    assert_eq!(ay(&mut b, ay_reg::VOL_A), 0x0F);

    let sample = pump(&mut b, 2_000);
    assert!(sample > 0.0, "the tone must reach the mux output: {sample}");
    assert_eq!(b.read(FF7E) & QUIET, 0, "bit 5 clears while sound plays");

    send(&mut b, cmd::STOP_ALL_SOUND);
    settle(&mut b);
    assert_eq!(ay(&mut b, ay_reg::VOL_A), 0, "$00 silences the channel");
    pump(&mut b, 20_000);
    assert_eq!(
        b.read(FF7E) & QUIET,
        QUIET,
        "bit 5 sets once the envelope decays"
    );
}

#[test]
fn direct_access_ff_is_a_plain_value_where_a_value_is_expected() {
    let Some(mut b) = try_coco3_bus_with_ssc() else {
        return skip("direct_access_ff_is_a_plain_value_where_a_value_is_expected");
    };
    send(&mut b, cmd::DIRECT_ACCESS_TOGGLE);
    send(&mut b, ay_reg::TONE_B_FINE);
    send(&mut b, 0xFF); // a value, not the terminator
    send(&mut b, terminator::SOUND);
    settle(&mut b);
    assert_eq!(ay(&mut b, ay_reg::TONE_B_FINE), 0xFF);
    // Back in command mode: a fresh direct-access session works.
    send(&mut b, cmd::DIRECT_ACCESS_TOGGLE);
    send(&mut b, ay_reg::VOL_B);
    send(&mut b, 0x03);
    send(&mut b, terminator::SOUND);
    settle(&mut b);
    assert_eq!(ay(&mut b, ay_reg::VOL_B), 0x03);
}

#[test]
fn sound_data_load_and_execute_programs_the_tone_registers() {
    let Some(mut b) = try_coco3_bus_with_ssc() else {
        return skip("sound_data_load_and_execute_programs_the_tone_registers");
    };
    const AMP: u8 = 0x0A;
    const COARSE: u8 = 0x05;
    const FINE: u8 = 100;
    const DURATION: u8 = 0x10;
    let byte0 = (group::TONE_A << group::OPCODE_SHIFT) | AMP;

    send(&mut b, cmd::LOAD_SOUND_INDIVIDUAL_START); // buffer 0
    for byte in [byte0, COARSE, FINE, DURATION, terminator::SOUND] {
        send(&mut b, byte);
    }
    send(&mut b, cmd::EXEC_SOUND_INDIVIDUAL_START);
    settle(&mut b);

    assert_eq!(ay(&mut b, ay_reg::TONE_A_COARSE), COARSE);
    assert_eq!(ay(&mut b, ay_reg::TONE_A_FINE), FINE);
    assert_eq!(ay(&mut b, ay_reg::VOL_A), AMP);
    assert_eq!(
        ay(&mut b, ay_reg::MIXER) & 0b0000_0001,
        0,
        "tone A enabled in the mixer"
    );
}

#[test]
fn ascii_text_bytes_are_consumed_then_normal_commands_still_work() {
    let Some(mut b) = try_coco3_bus_with_ssc() else {
        return skip("ascii_text_bytes_are_consumed_then_normal_commands_still_work");
    };
    for &byte in b"HELLO" {
        send(&mut b, byte);
    }
    // A command afterward, before any carriage return, still dispatches.
    send(&mut b, cmd::DIRECT_ACCESS_TOGGLE);
    send(&mut b, ay_reg::VOL_A);
    send(&mut b, 0x07);
    send(&mut b, terminator::SOUND);
    settle(&mut b);
    assert_eq!(ay(&mut b, ay_reg::VOL_A), 0x07);
}

#[test]
fn speech_load_command_does_not_desync_later_commands() {
    let Some(mut b) = try_coco3_bus_with_ssc() else {
        return skip("speech_load_command_does_not_desync_later_commands");
    };
    send(&mut b, cmd::LOAD_SPEECH_CONSECUTIVE_START); // $80: buffers 0..=7
    for &byte in b"HELLO WORLD" {
        send(&mut b, byte);
    }
    send(&mut b, terminator::SPEECH);

    send(&mut b, cmd::DIRECT_ACCESS_TOGGLE);
    send(&mut b, ay_reg::VOL_C);
    send(&mut b, 0x0A);
    send(&mut b, terminator::SOUND);
    settle(&mut b);
    assert_eq!(ay(&mut b, ay_reg::VOL_C), 0x0A);
}

#[test]
fn register_string_load_and_execute_applies_all_pairs() {
    let Some(mut b) = try_coco3_bus_with_ssc() else {
        return skip("register_string_load_and_execute_applies_all_pairs");
    };
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
    settle(&mut b);

    for &(register, value) in &pairs {
        assert_eq!(ay(&mut b, register), value, "register {register:#04x}");
    }
}

#[test]
fn consecutive_load_spills_into_the_next_buffer() {
    let Some(mut b) = try_coco3_bus_with_ssc() else {
        return skip("consecutive_load_spills_into_the_next_buffer");
    };
    // 40 register pairs = 80 bytes from buffer 0: the tail lands in buffer 1.
    send(&mut b, cmd::LOAD_REGISTER_CONSECUTIVE_START); // $A8
    for _ in 0..39 {
        send(&mut b, ay_reg::TONE_C_FINE);
        send(&mut b, 0x11);
    }
    send(&mut b, ay_reg::VOL_C);
    send(&mut b, 0x0C);
    send(&mut b, terminator::SOUND);

    send(&mut b, cmd::EXEC_REGISTER_INDIVIDUAL_START + 1); // buffer 1 alone
    settle_bytes(&mut b, 84);
    assert_eq!(
        ay(&mut b, ay_reg::VOL_C),
        0x0C,
        "the spilled pair executed from buffer 1"
    );
}
