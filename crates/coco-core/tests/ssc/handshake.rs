//! Handshake ($FF7D/$FF7E): status bits, the busy flag the firmware clears,
//! the reset line's edge semantics (MAME `coco_ssc.cpp`).

use coco_core::ay8913::reg as ay_reg;
use mc6809::Bus;

use super::common::{
    FF7D, FF7E, NOT_BUSY, SPEECH_READY, coco3_bus_with_ssc, send, settle, skip,
    try_coco3_bus_with_ssc, wait_not_busy,
};

#[test]
fn ff7d_always_reads_ff() {
    let mut b = coco3_bus_with_ssc();
    assert_eq!(b.read(FF7D), 0xFF);
    b.write(FF7D, 0x01);
    assert_eq!(b.read(FF7D), 0xFF);
    b.write(FF7D, 0x00);
    assert_eq!(b.read(FF7D), 0xFF);
}

#[test]
fn ff7e_status_base_bits_and_speech_ready_are_set_at_power_on() {
    let mut b = coco3_bus_with_ssc();
    let status = b.read(FF7E);
    assert_eq!(status & 0x1F, 0x1F, "bits 4-0 always read set");
    assert_eq!(
        status & SPEECH_READY,
        SPEECH_READY,
        "the chip idles in reset"
    );
    assert_eq!(
        status & NOT_BUSY,
        NOT_BUSY,
        "not busy before any $FF7E write"
    );
}

#[test]
fn ff7e_write_sets_busy_until_the_firmware_takes_the_byte() {
    let Some(mut b) = try_coco3_bus_with_ssc() else {
        return skip("ff7e_write_sets_busy_until_the_firmware_takes_the_byte");
    };
    b.write(FF7E, b'B');
    assert_eq!(b.read(FF7E) & NOT_BUSY, 0, "busy right after the write");
    let waited = wait_not_busy(&mut b);
    assert!(waited > 0, "the firmware needs at least one instruction");
}

#[test]
fn ff7e_latch_holds_the_newest_byte() {
    // The manual's "you lose data" while busy: a byte the firmware hasn't
    // read yet is overwritten, never the other way round (MAME `ff7d_write`).
    let mut b = coco3_bus_with_ssc();
    b.write(FF7E, 0xAF);
    b.write(FF7E, 0x42);
    let ssc = b.cart.as_ssc().expect("a SoundSpeechCartridge is inserted");
    assert_eq!(ssc.host_latch(), 0x42);
    assert!(ssc.busy());
}

#[test]
fn ff7d_falling_edge_clears_busy_resets_the_ay_and_reboots_the_firmware() {
    let Some(mut b) = try_coco3_bus_with_ssc() else {
        return skip("ff7d_falling_edge_clears_busy_resets_the_ay_and_reboots_the_firmware");
    };
    let ssc = b.cart.as_ssc().unwrap();
    ssc.ay_write(ay_reg::TONE_A_FINE, 0x55);
    assert_eq!(ssc.ay_read(ay_reg::TONE_A_FINE), 0x55);

    // Make it busy with a harmless byte: the latch survives the reset and
    // the rebooted firmware reads it (MAME leaves INT3 asserted too).
    b.write(FF7E, 0x00);
    assert_eq!(b.read(FF7E) & NOT_BUSY, 0);

    b.write(FF7D, 0x01); // assert SP0256 reset
    b.write(FF7D, 0x00); // falling edge

    assert_eq!(
        b.read(FF7E) & NOT_BUSY,
        NOT_BUSY,
        "falling edge clears busy at once"
    );
    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(
        ssc.ay_read(ay_reg::TONE_A_FINE),
        0,
        "the AY's registers reset"
    );
    assert!(ssc.firmware().pending_reset(), "the TMS7040 restarts");

    // After its reboot the firmware answers the protocol again.
    b.cart.tick(super::common::BOOT_CYCLES);
    send(&mut b, 0xAF);
    send(&mut b, ay_reg::VOL_C);
    send(&mut b, 0x09);
    send(&mut b, 0xFF);
    settle(&mut b);
    assert_eq!(b.cart.as_ssc().unwrap().ay_read(ay_reg::VOL_C), 0x09);
}

#[test]
fn ff7d_bit0_set_alone_does_not_reset_the_ay() {
    let mut b = coco3_bus_with_ssc();
    b.cart.tick(100); // let the power-on reset run
    let ssc = b.cart.as_ssc().unwrap();
    ssc.ay_write(ay_reg::TONE_A_FINE, 0x7A);

    b.write(FF7D, 0x01); // the line was already high: no edge

    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(ssc.ay_read(ay_reg::TONE_A_FINE), 0x7A);
    assert!(!ssc.firmware().pending_reset());
}

#[test]
fn power_on_first_bit0_clear_write_is_a_falling_edge() {
    // MAME primes the reset line high at power-on, so the manual's
    // "POKE 1 then 0" is an edge even without the 1 (`m_reset_line = 1`).
    let mut b = coco3_bus_with_ssc();
    let ssc = b.cart.as_ssc().unwrap();
    ssc.ay_write(ay_reg::TONE_A_FINE, 0x11);

    b.write(FF7D, 0x00);

    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(ssc.ay_read(ay_reg::TONE_A_FINE), 0, "the AY reset");
    assert!(ssc.firmware().pending_reset(), "the TMS7040 restarts");
}
