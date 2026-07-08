//! Phase 6 (first slice, `docs/coco12-plan.md`): a CoCo 2 running real
//! Extended Color BASIC 1.1 + Color BASIC 1.2 ROMs boots to the sign-on
//! banner and evaluates `PRINT 2+2` — the milestone that proves the SAM
//! primary memory map (Phase 2), the VDG-native colour/mode dispatch (Phase
//! 3), and the per-variant field loop (Phase 4) all work together on real
//! ROM code, not just unit tests.
//!
//! Skipped (not failed) if the ROMs aren't present locally, matching
//! `tests/boot.rs`/`tests/alive.rs`.

use std::path::PathBuf;

use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize, MonitorType, VideoStandard};
use mc6809::Bus;

/// Extended Color BASIC occupies the low 8K of the flat ROM image ($8000-$9FFF).
const EXTBAS_LEN: usize = 8 * 1024;
/// Color BASIC occupies the high 8K ($A000-$BFFF) — `SAM_BAS_ROM_OFFSET` in
/// `bus.rs`, duplicated here as a test-local constant (that one is private).
const BAS_OFFSET: usize = 8 * 1024;
/// Offset of the 6809 hardware vectors within an 8K Color BASIC ROM (the top
/// 32 bytes, $BFE0-$BFFF relative to the ROM's own $A000 base).
const BAS_VECTOR_OFFSET: usize = 0x1FE0;

fn try_load(name: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../roms")
        .join(name);
    std::fs::read(&path).ok()
}

/// Compose the flat 16K ROM image the plain-SAM bus path expects: extbas at
/// offset 0, bas at offset `BAS_OFFSET` (`docs/coco12-plan.md` "ROM files";
/// `bus.rs::SAM_BAS_ROM_OFFSET`). Returns `None` (test should skip) if either
/// file is missing.
fn load_coco2_rom() -> Option<Box<[u8]>> {
    let extbas = try_load("extbas11.rom")?;
    let bas = try_load("bas12.rom")?;
    assert_eq!(extbas.len(), EXTBAS_LEN, "extbas11.rom: unexpected size");
    assert_eq!(bas.len(), BAS_OFFSET, "bas12.rom: unexpected size");
    let mut image = extbas;
    image.extend_from_slice(&bas);
    Some(image.into_boxed_slice())
}

fn boot_machine() -> Option<(Machine, Vec<u8>)> {
    let extbas = try_load("extbas11.rom");
    let bas = try_load("bas12.rom");
    if extbas.is_none() || bas.is_none() {
        eprintln!(
            "skipping coco2_boot: extbas11.rom/bas12.rom not present in roms/ \
             (see docs/coco12-plan.md \"ROM files\")"
        );
        return None;
    }
    let bas = bas.unwrap();
    let rom = load_coco2_rom().expect("checked Some above");
    let config = MachineConfig {
        variant: MachineVariant::Coco2,
        video: VideoStandard::Ntsc,
        memory: MemorySize::K64,
        monitor: MonitorType::Rgb,
    };
    config
        .validate()
        .expect("Coco2/Ntsc/K64 must be a valid configuration");
    let m = Machine::new(config, rom);
    Some((m, bas))
}

fn screen_contains(m: &mut Machine, needle: &str) -> bool {
    m.text_screen_lines().iter().any(|l| l.contains(needle))
}

fn screen_dump(m: &mut Machine) -> String {
    m.text_screen_lines().join("\n")
}

fn tap_char(m: &mut Machine, c: char) {
    let (pos, shift) =
        coco_core::keyboard::char_key(c).unwrap_or_else(|| panic!("no key for {c:?}"));
    if shift {
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, true);
    }
    for _ in 0..3 {
        m.bus.keyboard.set(pos, true);
        m.run_field();
    }
    m.bus.keyboard.set(pos, false);
    m.bus.keyboard.set(coco_core::keyboard::SHIFT, false);
    for _ in 0..3 {
        m.run_field();
    }
}

fn type_line(m: &mut Machine, s: &str) {
    for c in s.chars().chain(std::iter::once('\r')) {
        tap_char(m, c);
    }
}

/// Run fields until the sign-on banner/`OK` prompt appears (real Color
/// BASIC's cold-start does a RAM-size probe, byte-by-byte across up to 64K,
/// before it can paint anything, hence the generous budget), then let the
/// housekeeping loop settle for a few more fields. Shared by every test below
/// that needs a machine sitting at the `OK` prompt, ready for direct-mode
/// input.
fn boot_to_prompt(m: &mut Machine) {
    const MAX_FIELDS: usize = 1500;
    let mut alive = false;
    for _ in 0..MAX_FIELDS {
        m.run_field();
        if screen_contains(m, "OK") && screen_contains(m, "EXTENDED COLOR BASIC 1.1") {
            alive = true;
            break;
        }
    }
    assert!(
        alive,
        "sign-on banner/OK prompt never appeared; screen:\n{}",
        screen_dump(m)
    );
    for _ in 0..10 {
        m.run_field();
    }
}

/// Field budget given to each direct-mode BASIC statement below to
/// tokenize/execute before the next one is typed (mirrors the settle time
/// `boot_to_prompt` already gives the cold-start banner).
const SETTLE_FIELDS: usize = 30;

fn run_fields(m: &mut Machine, n: usize) {
    for _ in 0..n {
        m.run_field();
    }
}

#[test]
fn coco2_boots_extended_color_basic_and_evaluates_print() {
    let Some((mut m, bas_rom)) = boot_machine() else {
        return;
    };

    // The 6809 hardware vectors always read through the SAM's $FFE0-$FFFF ->
    // $BFE0-$BFFF mirror onto Color BASIC's own ROM, regardless of the SAM's
    // TY/M1 state (`docs/coco12-plan.md`; `sam.rs::VECTOR_MIRROR_BASE`). Check
    // the reset vector explicitly: it must match the last two bytes of the
    // real bas12.rom dump, not just "some" value.
    let expected_reset_hi = bas_rom[BAS_VECTOR_OFFSET + 0x1E]; // $BFFE
    let expected_reset_lo = bas_rom[BAS_VECTOR_OFFSET + 0x1F]; // $BFFF
    assert_eq!(
        m.bus.read(0xFFFE),
        expected_reset_hi,
        "reset vector high byte via $FFFE mirror"
    );
    assert_eq!(
        m.bus.read(0xFFFF),
        expected_reset_lo,
        "reset vector low byte via $FFFF mirror"
    );
    assert_eq!(
        m.cpu.pc,
        u16::from_be_bytes([expected_reset_hi, expected_reset_lo]),
        "Machine::new's reset() must have fetched PC from that same vector"
    );

    boot_to_prompt(&mut m);
    type_line(&mut m, "PRINT 2+2");

    const ANSWER_FIELDS: usize = 120;
    let mut answered = false;
    for _ in 0..ANSWER_FIELDS {
        m.run_field();
        if screen_contains(&mut m, " 4") {
            answered = true;
            break;
        }
    }
    assert!(
        answered,
        "PRINT 2+2 never produced ' 4'; screen:\n{}",
        screen_dump(&mut m)
    );
}

/// Phase 6 acceptance test 1 (`docs/coco12-plan.md`): `PMODE 4,1:SCREEN 1,1`
/// switches PIA1 $FF22's A/G bit on, `Machine::video_mode_summary` reports
/// the CoCo-compatible graphics dispatch, and the framebuffer's border and
/// interior pixels resolve through the fixed VDG palette
/// (`render_coco12.rs`'s unit-level coverage of the same colour source,
/// exercised here end-to-end through real ROM code).
///
/// `PMODE`/`SCREEN` must run from a *running program*, not typed directly at
/// the `OK` prompt: verified empirically against the real ROMs (traced via a
/// temporary `sam_write`/`sam_io_write` probe during development, since
/// nothing in `docs/coco12-plan.md` documents it) — Color BASIC's idle loop
/// (waiting for a keystroke at the prompt) re-asserts the SAM V0-V2/F0-F6
/// strobes and PIA1 $FF22 back to its text-mode defaults every field, so a
/// direct-mode `SCREEN 1,1` (or a raw `POKE 65314,...`) is visibly clobbered
/// again before the next field boundary. A one-line program that ends in an
/// infinite loop keeps the CPU out of that idle loop, so the mode sticks —
/// matching the well-known real-hardware behaviour that `PMODE`/`SCREEN`
/// only "hold" once `RUN`, not typed live.
#[test]
fn coco2_pmode_switches_to_graphics_with_fixed_vdg_colors() {
    let Some((mut m, _bas_rom)) = boot_machine() else {
        return;
    };
    boot_to_prompt(&mut m);

    type_line(&mut m, "10 PMODE 4,1:SCREEN 1,1:PCLS");
    run_fields(&mut m, SETTLE_FIELDS);
    type_line(&mut m, "20 GOTO 20");
    run_fields(&mut m, SETTLE_FIELDS);
    type_line(&mut m, "RUN");
    run_fields(&mut m, 2 * SETTLE_FIELDS);

    assert!(
        m.bus.pia1.b.output & coco_core::video::VDG_AG != 0,
        "PIA1 $FF22 A/G bit should be set after SCREEN 1,1; screen:\n{}",
        screen_dump(&mut m)
    );
    assert!(
        m.video_mode_summary().contains("PMODE"),
        "video_mode_summary should report CoCo-compatible graphics: {}",
        m.video_mode_summary()
    );

    let css = m.bus.pia1.b.output & coco_core::video::VDG_CSS != 0;
    let border_index = coco_core::video::vdg_graphics_border_index(css);
    let expected_border = coco_core::video::VDG_FIXED_PALETTE[border_index];

    let px = |fb: &[u8], x: usize, y: usize| -> [u8; 4] {
        let i = (y * coco_core::video::FB_W + x) * coco_core::video::BYTES_PER_PIXEL;
        fb[i..i + 4].try_into().unwrap()
    };
    assert_eq!(
        px(&m.framebuffer, 0, 0),
        expected_border,
        "graphics border should be the fixed VDG colour for CSS={css}"
    );

    // RG6/PMODE4's 2-colour table: palette regs 8/9 (CSS=0) or 10/11 (CSS=1)
    // — see `video.rs::vdg_palette_indices`/`render_coco12.rs`. Whatever
    // PCLS filled the page with, every interior pixel must resolve to one of
    // those two fixed colours, not e.g. a GIME-palette leftover.
    let (off_index, on_index) = if css { (10, 11) } else { (8, 9) };
    let off = coco_core::video::VDG_FIXED_PALETTE[off_index];
    let on = coco_core::video::VDG_FIXED_PALETTE[on_index];
    let interior = px(
        &m.framebuffer,
        coco_core::video::BORDER,
        coco_core::video::BORDER,
    );
    assert!(
        interior == off || interior == on,
        "interior graphics pixel should be one of the fixed RG6 2-colour VDG \
         colours for CSS={css}: got {interior:?}, expected {off:?} or {on:?}"
    );
}

/// Phase 6 acceptance test 2 (`docs/coco12-plan.md`): `POKE 65497,0` ($FFD9,
/// SAM R1 strobe) doubles the CPU rate, and `POKE 65496,0` ($FFD8) restores
/// it — matching the bus-level coverage in `tests/sam.rs`/`tests/speed.rs`,
/// exercised here through real ROM code typed at the `OK` prompt, with a
/// working `PRINT` afterward proving the machine is still sane post-flip.
#[test]
fn coco2_speed_poke_toggles_sam_r1_and_keeps_running() {
    let Some((mut m, _bas_rom)) = boot_machine() else {
        return;
    };
    boot_to_prompt(&mut m);

    assert!(!m.bus.sam.r1, "R1 should be clear at cold boot");

    type_line(&mut m, "POKE 65497,0");
    run_fields(&mut m, SETTLE_FIELDS);
    assert!(m.bus.sam.r1, "R1 should be set after POKE 65497,0 ($FFD9)");

    type_line(&mut m, "POKE 65496,0");
    run_fields(&mut m, SETTLE_FIELDS);
    assert!(
        !m.bus.sam.r1,
        "R1 should be clear again after POKE 65496,0 ($FFD8)"
    );

    type_line(&mut m, "PRINT 2+2");
    const ANSWER_FIELDS: usize = 120;
    let mut answered = false;
    for _ in 0..ANSWER_FIELDS {
        m.run_field();
        if screen_contains(&mut m, " 4") {
            answered = true;
            break;
        }
    }
    assert!(
        answered,
        "PRINT 2+2 never produced ' 4' after the speed-poke round trip; screen:\n{}",
        screen_dump(&mut m)
    );
}

/// Phase 6 acceptance test 3 (`docs/coco12-plan.md`): on a live-booted
/// machine, strobing SAM TY set (`$FFDF`) with M1 already set (64K) switches
/// $A000-$BFFF from ROM to RAM — matching `sam.rs`'s
/// `ty1_with_m1_maps_all_ram_through_feff_banking_out_rom` unit test, but
/// through the bus with a real ROM image underneath, and with a working
/// `PRINT` afterward proving BASIC survives the round trip. Bus-level (not
/// typed), per the plan: BASIC itself runs from ROM, so this can't be probed
/// purely from BASIC without an assembly stub.
#[test]
fn coco2_ffdf_all_ram_flip_on_live_boot() {
    let Some((mut m, bas_rom)) = boot_machine() else {
        return;
    };
    boot_to_prompt(&mut m);

    const M1_SET: u16 = 0xFFDD;
    const TY_SET: u16 = 0xFFDF;
    const TY_CLEAR: u16 = 0xFFDE;
    const A000: u16 = 0xA000;

    let rom_byte = bas_rom[0]; // Color BASIC ROM's first byte, at $A000.
    assert_eq!(
        m.bus.read(A000),
        rom_byte,
        "before the flip, $A000 should read the real Color BASIC ROM byte"
    );

    m.bus.write(M1_SET, 0); // Force M1 (64K), regardless of BASIC's own sizing.
    m.bus.write(TY_SET, 0); // TY set: all-RAM, ROM disabled.

    const TEST_BYTE: u8 = 0xAB;
    m.bus.write(A000, TEST_BYTE);
    assert_eq!(
        m.bus.read(A000),
        TEST_BYTE,
        "with TY set, $A000 should be writable RAM, not read-only ROM"
    );

    m.bus.write(TY_CLEAR, 0); // Back to the ROM map.
    assert_eq!(
        m.bus.read(A000),
        rom_byte,
        "after clearing TY, $A000 should read the ROM byte again"
    );

    type_line(&mut m, "PRINT 2+2");
    const ANSWER_FIELDS: usize = 120;
    let mut answered = false;
    for _ in 0..ANSWER_FIELDS {
        m.run_field();
        if screen_contains(&mut m, " 4") {
            answered = true;
            break;
        }
    }
    assert!(
        answered,
        "PRINT 2+2 never produced ' 4' after the TY round trip; screen:\n{}",
        screen_dump(&mut m)
    );
}

// ============================================================================
// Phase 6 acceptance test 4: cassette CSAVE/CLOAD (`docs/coco12-plan.md`)
// ============================================================================
//
// The cassette deck (`Cassette`, `bus.rs`'s PIA1 record/playback wiring) is
// entirely machine-neutral — confirmed by inspection: `bus.rs::sam_io_write`'s
// PIA1 branch feeds `Cassette::record_dac` exactly like the GIME path's
// `io_write` does, and `Machine::pia1_pa_pins`'s cassette-input bit doesn't
// consult `self.config.variant` at all. This is a regression guard for that
// wiring on the plain-SAM bus path, mirroring `tests/cassette.rs`'s
// `csave_rewind_cload_round_trips_a_basic_program` almost line-for-line, with
// a CoCo 2 boot in place of the CoCo 3 one.

/// Tape leader/sync bytes (Service Manual §5.10) — matches `tests/cassette.rs`.
const LEADER: u8 = 0x55;
const SYNC: u8 = 0x3C;

/// Run until the cassette motor, having been on, stays off for a stretch
/// longer than any intra-operation pause, or until `max_fields` elapse.
fn run_until_motor_idle(m: &mut Machine, max_fields: usize) {
    /// 1.5 s of motor-off at 60 fields/s — longer than any mid-tape gap.
    const IDLE_FIELDS: usize = 90;
    let mut seen_on = false;
    let mut off_streak = 0;
    for _ in 0..max_fields {
        m.run_field();
        if m.bus.pia1.a.c2_output() {
            seen_on = true;
            off_streak = 0;
        } else if seen_on {
            off_streak += 1;
            if off_streak >= IDLE_FIELDS {
                return;
            }
        }
    }
}

/// Parse the framed blocks out of a decoded tape stream, asserting every
/// checksum — matches `tests/cassette.rs::parse_blocks`.
fn parse_blocks(tape: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < tape.len() {
        if tape[i] == LEADER {
            i += 1;
            continue;
        }
        assert_eq!(
            tape[i], SYNC,
            "expected sync at offset {i}, got ${:02X}",
            tape[i]
        );
        let block_type = tape[i + 1];
        let len = usize::from(tape[i + 2]);
        let payload = tape[i + 3..i + 3 + len].to_vec();
        let checksum = tape[i + 3 + len];
        let expected = payload
            .iter()
            .fold((block_type).wrapping_add(len as u8), |acc, &b| {
                acc.wrapping_add(b)
            });
        assert_eq!(
            checksum, expected,
            "bad checksum in block type ${block_type:02X}"
        );
        i += 3 + len + 1; // sync consumed through checksum; trailer is a LEADER
        blocks.push((block_type, payload));
    }
    blocks
}

#[test]
fn coco2_csave_rewind_cload_round_trips_a_basic_program() {
    const TAPE_OP_FIELDS: usize = 3000;
    const BLOCK_NAMEFILE: u8 = 0x00;
    const BLOCK_DATA: u8 = 0x01;
    const BLOCK_EOF: u8 = 0xFF;

    let Some((mut m, _bas_rom)) = boot_machine() else {
        return;
    };
    boot_to_prompt(&mut m);

    // Blank tape in the deck, record a program.
    m.bus.cassette.insert_tape(Vec::new());
    type_line(&mut m, "10 PRINT \"HI\"");
    type_line(&mut m, "CSAVE\"X\"");
    run_until_motor_idle(&mut m, TAPE_OP_FIELDS);
    assert!(
        screen_contains(&mut m, "OK"),
        "CSAVE never finished:\n{}",
        screen_dump(&mut m)
    );

    // Rewind finalizes the recording into the tape; check its structure.
    m.bus.cassette.rewind();
    let tape = m.bus.cassette.tape_bytes().to_vec();
    assert!(m.bus.cassette.dirty(), "a fresh recording must be dirty");
    let blocks = parse_blocks(&tape);
    assert_eq!(blocks[0].0, BLOCK_NAMEFILE);
    assert_eq!(&blocks[0].1[..8], b"X       ", "namefile name");
    assert_eq!(blocks[0].1.len(), 15, "namefile payload is 15 bytes");
    assert!(
        blocks[1..blocks.len() - 1]
            .iter()
            .all(|(t, _)| *t == BLOCK_DATA),
        "middle blocks are data blocks"
    );
    assert_eq!(blocks.last().unwrap().0, BLOCK_EOF);

    // Wipe BASIC's program, load it back from the tape, and run it.
    type_line(&mut m, "NEW");
    m.bus.cassette.rewind();
    type_line(&mut m, "CLOAD");
    run_until_motor_idle(&mut m, TAPE_OP_FIELDS);
    assert!(
        screen_contains(&mut m, "OK"),
        "CLOAD never finished:\n{}",
        screen_dump(&mut m)
    );
    assert!(
        !screen_contains(&mut m, "ERROR"),
        "CLOAD errored:\n{}",
        screen_dump(&mut m)
    );

    type_line(&mut m, "RUN");
    run_fields(&mut m, SETTLE_FIELDS);
    assert!(
        screen_contains(&mut m, "HI"),
        "the round-tripped program must print HI:\n{}",
        screen_dump(&mut m)
    );
}
