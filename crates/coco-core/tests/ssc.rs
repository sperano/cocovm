//! Tandy Sound/Speech Cartridge (SSC): the `$FF7D`/`$FF7E` handshake, bus
//! routing (standard SCS-slot behaviour plus the Multi-Pak's `$FF60-$FF7E`
//! broadcast — see `crate::cart::MultiPak`'s doc comment), and the
//! AY-3-8913's audio/Sound Activity Circuit integration. Facts per
//! `docs/ssc-spec.md` / MAME `coco_ssc.cpp`. AY-3-8913 core coverage lives in
//! `crates/coco-core/src/ay8913.rs`'s own inline tests.

use coco_core::ay8913::reg as ay_reg;
use coco_core::cart::{EmptySlot, MultiPak};
use coco_core::ssc::{Ssc, cmd, group, ram, reg as ssc_reg, terminator, timing};
use coco_core::{MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

/// Generator step for `sound_probe` (the AY drain is call-count based, so
/// this only feeds the (absent) crystal generators).
const PROBE_DT: f64 = 1.0 / 62_866.0;

const FF7D: u16 = ssc_reg::RESET;
const FF7E: u16 = ssc_reg::DATA;

/// Synthetic hold time for `busy` after a `$FF7E` write (see
/// `crates/coco-core/src/ssc.rs`'s `BUSY_HOLD_CYCLES` doc comment) — not
/// exported, so tests need their own large-enough tick count to clear busy
/// between protocol bytes. Comfortably larger than the real constant (100).
const CLEAR_BUSY: u32 = 1_000;

fn bus_with_ssc(variant: MachineVariant, memory: MemorySize) -> SystemBus {
    let mut b = SystemBus::new(variant, memory, vec![0u8; 32 * 1024].into_boxed_slice());
    b.cart = Ssc::new().into();
    b
}

fn coco3_bus_with_ssc() -> SystemBus {
    bus_with_ssc(MachineVariant::Coco3, MemorySize::K512)
}

// ============================================================================
// Handshake ($FF7D/$FF7E).
// ============================================================================

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
    assert_eq!(b.read(FF7E) & 0x80, 0x80, "must not be busy before any $FF7E write");

    b.write(FF7E, 0x42);
    assert_eq!(b.read(FF7E) & 0x80, 0x00, "busy immediately after a $FF7E write");

    // Advance the cart's clock past the synthetic hold window.
    b.cart.tick(1_000);
    assert_eq!(b.read(FF7E) & 0x80, 0x80, "busy must clear after the hold window elapses");
}

#[test]
fn ff7e_status_base_bits_and_speech_ready_are_always_set() {
    let mut b = coco3_bus_with_ssc();
    let status = b.read(FF7E);
    assert_eq!(status & 0x1F, 0x1F, "bits 4-0 always read set");
    assert_eq!(status & 0x40, 0x40, "bit 6 (SP0256 SBY) always set: no SP0256 emulated");
}

#[test]
fn ff7d_falling_edge_clears_busy_and_resets_the_ay() {
    let mut b = coco3_bus_with_ssc();
    let ssc = b.cart.as_ssc().expect("an Ssc is inserted");
    ssc.ay_write(ay_reg::TONE_A_FINE, 0x55);
    assert_eq!(ssc.ay_read(ay_reg::TONE_A_FINE), 0x55);

    b.write(FF7E, 0x99); // make it busy
    assert_eq!(b.read(FF7E) & 0x80, 0x00);

    b.write(FF7D, 0x01); // assert SP0256 reset
    b.write(FF7D, 0x00); // falling edge

    assert_eq!(b.read(FF7E) & 0x80, 0x80, "falling edge must clear busy immediately");
    let ssc = b.cart.as_ssc().expect("an Ssc is inserted");
    assert_eq!(ssc.ay_read(ay_reg::TONE_A_FINE), 0, "falling edge must reset the AY's registers");
}

#[test]
fn ff7d_bit0_set_alone_does_not_reset_the_ay() {
    let mut b = coco3_bus_with_ssc();
    let ssc = b.cart.as_ssc().expect("an Ssc is inserted");
    ssc.ay_write(ay_reg::TONE_A_FINE, 0x7A);

    b.write(FF7D, 0x01); // assert only -- no preceding 1 that could make this a falling edge

    let ssc = b.cart.as_ssc().expect("an Ssc is inserted");
    assert_eq!(ssc.ay_read(ay_reg::TONE_A_FINE), 0x7A, "bit0=1 alone must not reset the AY");
}

#[test]
fn power_on_first_bit0_clear_write_is_not_a_falling_edge() {
    // MAME primes the reset line so the very first $FF7D write, even if it's
    // bit0=0, isn't itself treated as a 1-then-0 transition.
    let mut b = coco3_bus_with_ssc();
    let ssc = b.cart.as_ssc().expect("an Ssc is inserted");
    ssc.ay_write(ay_reg::TONE_A_FINE, 0x11);

    b.write(FF7D, 0x00);

    let ssc = b.cart.as_ssc().expect("an Ssc is inserted");
    assert_eq!(ssc.ay_read(ay_reg::TONE_A_FINE), 0x11, "the first bit0=0 write must not reset the AY");
}

// ============================================================================
// Bus routing.
// ============================================================================

#[test]
fn empty_slot_still_reads_open_bus_across_ff60_to_ff7e() {
    let mut b = SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    b.cart = EmptySlot.into();
    for addr in 0xFF60u16..=0xFF7E {
        assert_eq!(b.read(addr), 0xFF, "addr {addr:#06x} must be open bus with an empty slot");
    }
}

#[test]
fn ssc_reaches_ff7d_ff7e_on_the_coco1_2_plain_sam_path() {
    let mut b = bus_with_ssc(MachineVariant::Coco1, MemorySize::K64);
    assert_eq!(b.read(FF7D), 0xFF);

    b.write(FF7E, 0x12);
    assert_eq!(b.read(FF7E) & 0x80, 0x00, "busy must be set on the plain-SAM (CoCo 1/2) path too");
}

#[test]
fn ssc_in_a_non_scs_selected_mpi_slot_still_receives_ff7d_ff7e() {
    let mut b = SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    let mut mp = MultiPak::new(3); // switch on slot 4 (index 3)
    mp.insert(1, Ssc::new()); // SSC lives in slot 2 (index 1)
    b.cart = mp.into();

    // Re-point the SCS/CTS select at slot 0, definitely not the SSC's slot 1.
    b.write(0xFF7F, 0x00);
    assert_eq!(
        b.cart.as_multipak().unwrap().scs_slot(),
        0,
        "test setup: slot 0 must be SCS-selected, not the SSC's slot 1"
    );

    // $FF7D/$FF7E is outside the standard SCS window ($FF40-$FF5F), so the
    // MPI must broadcast it to every slot regardless of SCS selection.
    assert_eq!(b.read(FF7D), 0xFF);
    b.write(FF7E, 0x77);
    assert_eq!(
        b.read(FF7E) & 0x80,
        0x00,
        "busy must be set even though the SSC's slot isn't SCS-selected"
    );
}

// ============================================================================
// Audio / Sound Activity Circuit integration.
// ============================================================================

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

/// A bus with an `Ssc` inserted and the sound mux routed to the cartridge
/// input (SEL2:SEL1 = 10, SNDEN high) — same PIA-poking pattern as
/// `tests/sound.rs`'s `bus()` helper.
fn bus_with_ssc_selected() -> SystemBus {
    let mut b = coco3_bus_with_ssc();
    b.write(PIA1_CRB, CR_DDR);
    b.write(PIA1_CRB, CR_C2_HIGH); // SNDEN high
    b.write(PIA0_CRA, CR_C2_LOW); // SEL1 = 0
    b.write(PIA0_CRB, CR_C2_HIGH); // SEL2 = 1 -> mux 10: cartridge
    b
}

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
fn drive_loud_tone(ssc: &mut Ssc) {
    ssc.ay_write(ay_reg::TONE_A_FINE, (TEST_TONE_PERIOD & 0xFF) as u8);
    ssc.ay_write(ay_reg::TONE_A_COARSE, (TEST_TONE_PERIOD >> 8) as u8);
    ssc.ay_write(ay_reg::VOL_A, 0x0F);
    ssc.ay_write(ay_reg::MIXER, 0b0000_1000);
}

fn silence(ssc: &mut Ssc) {
    ssc.ay_write(ay_reg::VOL_A, 0);
}

/// Advance the cart's clock and pump `count` `sound_probe` calls, returning
/// the last sample.
fn pump(b: &mut SystemBus, count: u32) -> f32 {
    let mut last = 0.0;
    for _ in 0..count {
        b.cart.tick(100);
        last = b.sound_probe(PROBE_DT)[0];
    }
    last
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

// ============================================================================
// Host byte protocol (SOUND half): command dispatch, buffer RAM load/execute,
// direct access, and the sound-data engine. Speech/allophone/SP0256 stays a
// parse-and-discard no-op -- covered here only to the extent of proving it
// doesn't desync the state machine for later commands.
// ============================================================================

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
        assert_eq!(ssc.ay_read(ay_reg::TONE_A_FINE), (TONE_A_PERIOD & 0xFF) as u8);
        assert_eq!(ssc.ay_read(ay_reg::TONE_A_COARSE), (TONE_A_PERIOD >> 8) as u8);
        assert_eq!(ssc.ay_read(ay_reg::MIXER), MIXER_TONE_A_ONLY);
        assert_eq!(ssc.ay_read(ay_reg::VOL_A), 0x0F);
    }

    let sample = pump(&mut b, 2_000);
    assert!(sample > 0.0, "direct-access-programmed tone must be audible: {sample}");
    assert_eq!(b.read(FF7E) & 0x20, 0x00, "SAC bit5 (QUIET) must clear while sound plays");

    send(&mut b, cmd::STOP_ALL_SOUND);
    {
        let ssc = b.cart.as_ssc().unwrap();
        assert_eq!(ssc.ay_read(ay_reg::VOL_A), 0, "$00 stop-all-sound must zero every channel volume");
    }
    pump(&mut b, 20_000);
    assert_eq!(b.read(FF7E) & 0x20, 0x20, "SAC bit5 (QUIET) must set again once the envelope decays");
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
        assert_eq!(mixer & 0b0000_1001, 0b0000_1000, "tone A enabled (bit0=0), noise A disabled (bit3=1)");
    }

    b.cart.tick(timing::duration_cycles(DURATION, timing::DEFAULT_TIMER_BASE) + 1);

    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(ssc.ay_read(ay_reg::VOL_A), AMP, "end-of-stream must not silence the AY");
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
    let group2 = [noise_opcode | SECOND_AMP, group::NOISE_REUSE_FLAG | SECOND_PERIOD, SECOND_DURATION];

    send(&mut b, cmd::LOAD_SOUND_INDIVIDUAL_START); // buffer 0
    for &byte in group1.iter().chain(group2.iter()) {
        send(&mut b, byte);
    }
    send(&mut b, terminator::SOUND);

    send(&mut b, cmd::EXEC_SOUND_INDIVIDUAL_START); // buffer 0: first group runs synchronously

    {
        let ssc = b.cart.as_ssc().unwrap();
        assert_eq!(ssc.ay_read(ay_reg::VOL_A), FIRST_AMP, "first group's own amplitude");
    }

    b.cart.tick(timing::duration_cycles(FIRST_DURATION, timing::DEFAULT_TIMER_BASE) + 1);

    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(
        ssc.ay_read(ay_reg::VOL_A),
        FIRST_AMP,
        "the reuse flag must carry forward the FIRST group's amplitude, not the second group's own bits"
    );
    assert_eq!(ssc.ay_read(ay_reg::NOISE_PERIOD), SECOND_PERIOD, "the period must still come from the second group");
    assert_ne!(FIRST_AMP, SECOND_AMP, "test setup: amplitudes must differ to be distinguishable");
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

    let tone = [(group::TONE_A << group::OPCODE_SHIFT) | group::M_FLAG | TONE_AMP, 1, 2, TONE_DURATION];
    let envelope = [(group::ENVELOPE_LOW << group::OPCODE_SHIFT) | ENV_SHAPE, ENV_COARSE, ENV_FINE, ENV_DURATION];
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
        assert_eq!(ssc.ay_read(ay_reg::ENV_SHAPE), ENV_SHAPE, "chained envelope group must program ENV_SHAPE");
        assert_eq!(ssc.ay_read(ay_reg::ENV_COARSE), ENV_COARSE);
        assert_eq!(ssc.ay_read(ay_reg::ENV_FINE), ENV_FINE);
        assert_eq!(ssc.ay_read(ay_reg::VOL_B), 0, "marker group must not have run yet");
    }

    // Enough cycles for the TONE group's own duration, but nowhere near the
    // ENVELOPE group's -- if the engine wrongly used the tone's duration,
    // the marker would already have fired here.
    b.cart.tick(timing::duration_cycles(TONE_DURATION, timing::DEFAULT_TIMER_BASE) + 1);
    {
        let ssc = b.cart.as_ssc().unwrap();
        assert_eq!(
            ssc.ay_read(ay_reg::VOL_B),
            0,
            "combined tone+envelope event must be scheduled by the ENVELOPE group's duration, not the tone's"
        );
    }

    // Now cross the ENVELOPE group's own duration.
    b.cart.tick(timing::duration_cycles(ENV_DURATION, timing::DEFAULT_TIMER_BASE) + 1_000);
    let ssc = b.cart.as_ssc().unwrap();
    assert_eq!(ssc.ay_read(ay_reg::VOL_B), MARKER_AMP, "marker group must fire once the envelope's duration elapses");
}

#[test]
fn timer_base_scales_sound_event_duration() {
    const AMP_A: u8 = 5;
    const DURATION: u8 = 10;
    const MARKER_AMP: u8 = 6;

    for &base in &[1u8, 250u8] {
        let mut b = coco3_bus_with_ssc();
        let tone = [(group::TONE_A << group::OPCODE_SHIFT) | AMP_A, 1, 2, DURATION];
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
    assert_eq!(ssc.ay_read(ay_reg::VOL_A), 0x07, "a direct-access command after ASCII passthrough must still work");
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
    assert_eq!(ssc.ay_read(ay_reg::VOL_B), 0x05, "the overflow byte must have been reprocessed as $AF");
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
    assert_eq!(PAD_TONE_GROUPS * 4 + PAD_NOISE_GROUPS * 3, 62, "test setup arithmetic");

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
    assert_eq!(ssc.ay_read(ay_reg::VOL_C), 0x0A, "a normal command after a speech LOAD must still work");
}

#[test]
fn register_string_load_and_execute_applies_all_pairs() {
    let mut b = coco3_bus_with_ssc();
    let pairs: [(u8, u8); 3] = [(ay_reg::TONE_A_FINE, 0x33), (ay_reg::TONE_A_COARSE, 0x02), (ay_reg::VOL_A, 0x0D)];

    send(&mut b, cmd::LOAD_REGISTER_INDIVIDUAL_START); // $B8: buffer 0
    for &(register, value) in &pairs {
        send(&mut b, register);
        send(&mut b, value);
    }
    send(&mut b, terminator::SOUND);

    send(&mut b, cmd::EXEC_REGISTER_INDIVIDUAL_START); // $F8: buffer 0

    let ssc = b.cart.as_ssc().unwrap();
    for &(register, value) in &pairs {
        assert_eq!(ssc.ay_read(register), value, "register {register:#04x} must have been applied");
    }
}
