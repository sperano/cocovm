//! Audio / Sound Activity Circuit integration.

use coco_core::ay8913::reg as ay_reg;
use coco_core::ssc::SoundSpeechCartridge;
use mc6809::Bus;

use super::common::{bus_with_ssc_selected, coco3_bus_with_ssc, pump, FF7E, PROBE_DT};

const PIA0_CRA: u16 = 0xFF01;
const PIA0_CRB: u16 = 0xFF03;
const PIA1_CRA: u16 = 0xFF21;
const PIA1_CRB: u16 = 0xFF23;
/// Control value: data register selected, C2 set/reset output low/high.
const CR_C2_LOW: u8 = 0x34;
const CR_C2_HIGH: u8 = 0x3C;
/// Control value selecting the DDR (bit 2 clear) -- unused here since this
/// suite only drives Cx2 (SNDEN/SEL1/SEL2), never the DAC's data pins.
const CR_DDR: u8 = 0x30;

/// Tone period long enough that each `pump` call's box-filtered sample sits
/// near one extreme (near-silent or near-full-scale) for many consecutive
/// calls, with a sharp jump at each half-cycle boundary — the AC content the
/// SAC's high-pass filter needs to detect. A short period (much faster than
/// one `pump` call's ~25-internal-step window) instead averages to a nearly
/// constant per-call level, which a DC-blocking filter can't tell from
/// silence — an earlier version of this test used too short a period and the
/// SAC never triggered. ~112 Hz at the AY's ~223.7 kHz internal-step clock.
const TEST_TONE_PERIOD: u16 = 1000;

/// Loud AY tone A, gated on regardless of noise (mixer bit 3: noise disabled
/// for channel A).
fn drive_loud_tone(ssc: &mut SoundSpeechCartridge) {
    ssc.ay_write(ay_reg::TONE_A_FINE, (TEST_TONE_PERIOD & 0xFF) as u8);
    ssc.ay_write(ay_reg::TONE_A_COARSE, (TEST_TONE_PERIOD >> 8) as u8);
    ssc.ay_write(ay_reg::VOL_A, 0x0F);
    ssc.ay_write(ay_reg::MIXER, 0b0000_1000);
}

fn silence(ssc: &mut SoundSpeechCartridge) {
    ssc.ay_write(ay_reg::VOL_A, 0);
}

#[test]
fn loud_tone_through_the_cartridge_mux_produces_nonzero_output_and_clears_quiet() {
    let mut b = bus_with_ssc_selected();
    let ssc = b.cart.as_ssc().unwrap();
    drive_loud_tone(ssc);

    let sample = pump(&mut b, 2_000);
    assert!(sample > 0.0, "loud tone routed through the cartridge mux must be audible: {sample}");
    assert_eq!(b.read(FF7E) & 0x20, 0x00, "bit 5 (QUIET) must clear while sound is playing");
}

#[test]
fn silencing_the_ay_eventually_sets_quiet_again() {
    let mut b = bus_with_ssc_selected();
    let ssc = b.cart.as_ssc().unwrap();
    drive_loud_tone(ssc);
    pump(&mut b, 2_000);
    assert_eq!(b.read(FF7E) & 0x20, 0x00, "must be active before silencing");

    let ssc = b.cart.as_ssc().unwrap();
    silence(ssc);
    pump(&mut b, 20_000);
    assert_eq!(b.read(FF7E) & 0x20, 0x20, "bit 5 (QUIET) must set again once the envelope decays");
}

#[test]
fn sac_tracks_activity_even_when_the_mux_is_not_on_the_cartridge() {
    // Same setup as `bus_with_ssc_selected`, but leave the mux on the DAC
    // (SEL2:SEL1 = 00) instead of the cartridge input.
    let mut b = coco3_bus_with_ssc();
    b.write(PIA1_CRA, CR_DDR);
    b.write(PIA1_CRB, CR_DDR);
    b.write(PIA1_CRA, CR_C2_LOW);
    b.write(PIA1_CRB, CR_C2_HIGH); // SNDEN high
    b.write(PIA0_CRA, CR_C2_LOW);
    b.write(PIA0_CRB, CR_C2_LOW); // SEL2:SEL1 = 00: DAC, not the cartridge

    let ssc = b.cart.as_ssc().unwrap();
    drive_loud_tone(ssc);
    let mut heard_cart_audio = false;
    for _ in 0..2_000 {
        b.cart.tick(100);
        if b.sound_probe(PROBE_DT)[0] > 0.0 {
            heard_cart_audio = true;
        }
    }
    // The DAC data register was never written (stays 0), so any nonzero
    // sample here could only have come from the cartridge leaking through
    // the (supposedly closed) mux gate.
    assert!(!heard_cart_audio, "cartridge audio must not reach the speaker when the mux isn't on it");
    assert_eq!(
        b.read(FF7E) & 0x20,
        0x00,
        "the SAC must still observe the cartridge's own (pre-mux) output and report it active"
    );
}
