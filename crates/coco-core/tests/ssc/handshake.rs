//! Handshake ($FF7D/$FF7E).

use coco_core::ay8913::reg as ay_reg;
use mc6809::Bus;

use super::common::{FF7D, FF7E, coco3_bus_with_ssc};

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
fn ff7e_write_sets_busy_and_it_clears_after_the_hold_window() {
    let mut b = coco3_bus_with_ssc();
    // Not busy at power-on: bit 7 set.
    assert_eq!(
        b.read(FF7E) & 0x80,
        0x80,
        "must not be busy before any $FF7E write"
    );

    b.write(FF7E, 0x42);
    assert_eq!(
        b.read(FF7E) & 0x80,
        0x00,
        "busy immediately after a $FF7E write"
    );

    // Advance the cart's clock past the synthetic hold window.
    b.cart.tick(1_000);
    assert_eq!(
        b.read(FF7E) & 0x80,
        0x80,
        "busy must clear after the hold window elapses"
    );
}

#[test]
fn ff7e_status_base_bits_and_speech_ready_are_always_set() {
    let mut b = coco3_bus_with_ssc();
    let status = b.read(FF7E);
    assert_eq!(status & 0x1F, 0x1F, "bits 4-0 always read set");
    assert_eq!(
        status & 0x40,
        0x40,
        "bit 6 (SP0256 SBY) always set: no speech ROM fitted"
    );
}

#[test]
fn ff7d_falling_edge_clears_busy_and_resets_the_ay() {
    let mut b = coco3_bus_with_ssc();
    let ssc = b.cart.as_ssc().expect("a SoundSpeechCartridge is inserted");
    ssc.ay_write(ay_reg::TONE_A_FINE, 0x55);
    assert_eq!(ssc.ay_read(ay_reg::TONE_A_FINE), 0x55);

    b.write(FF7E, 0x99); // make it busy
    assert_eq!(b.read(FF7E) & 0x80, 0x00);

    b.write(FF7D, 0x01); // assert SP0256 reset
    b.write(FF7D, 0x00); // falling edge

    assert_eq!(
        b.read(FF7E) & 0x80,
        0x80,
        "falling edge must clear busy immediately"
    );
    let ssc = b.cart.as_ssc().expect("a SoundSpeechCartridge is inserted");
    assert_eq!(
        ssc.ay_read(ay_reg::TONE_A_FINE),
        0,
        "falling edge must reset the AY's registers"
    );
}

#[test]
fn ff7d_bit0_set_alone_does_not_reset_the_ay() {
    let mut b = coco3_bus_with_ssc();
    let ssc = b.cart.as_ssc().expect("a SoundSpeechCartridge is inserted");
    ssc.ay_write(ay_reg::TONE_A_FINE, 0x7A);

    b.write(FF7D, 0x01); // assert only — no preceding 1 can make this a falling edge

    let ssc = b.cart.as_ssc().expect("a SoundSpeechCartridge is inserted");
    assert_eq!(
        ssc.ay_read(ay_reg::TONE_A_FINE),
        0x7A,
        "bit0=1 alone must not reset the AY"
    );
}

#[test]
fn power_on_first_bit0_clear_write_is_not_a_falling_edge() {
    // MAME primes the reset line so the very first $FF7D write, even if it's
    // bit0=0, isn't itself treated as a 1-then-0 transition.
    let mut b = coco3_bus_with_ssc();
    let ssc = b.cart.as_ssc().expect("a SoundSpeechCartridge is inserted");
    ssc.ay_write(ay_reg::TONE_A_FINE, 0x11);

    b.write(FF7D, 0x00);

    let ssc = b.cart.as_ssc().expect("a SoundSpeechCartridge is inserted");
    assert_eq!(
        ssc.ay_read(ay_reg::TONE_A_FINE),
        0x11,
        "the first bit0=0 write must not reset the AY"
    );
}
