//! MC6883 SAM primary memory map (CoCo 1/2, no GIME): control-strobe latching,
//! the TY=0/TY=1 memory map, the $FFE0-$FFFF vector mirror, and P1 banking
//! (`docs/coco12-plan.md`; MAME `6883sam.cpp`). Style mirrors the GIME's own
//! `tests/sam_video.rs` (build a `SystemBus` directly, poke strobe addresses).

use coco_core::{MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

const ROM_SIZE: usize = 32 * 1024;

// ---- Strobe addresses ($FFC0-$FFDF): even clears, odd sets --------------------
const V0_CLEAR: u16 = 0xFFC0;
const V0_SET: u16 = 0xFFC1;
const V1_CLEAR: u16 = 0xFFC2;
const V1_SET: u16 = 0xFFC3;
const V2_CLEAR: u16 = 0xFFC4;
const V2_SET: u16 = 0xFFC5;
const F0_SET: u16 = 0xFFC7;
const F1_SET: u16 = 0xFFC9;
const F2_CLEAR: u16 = 0xFFCA;
const F2_SET: u16 = 0xFFCB;
const F6_SET: u16 = 0xFFD3;
const P1_CLEAR: u16 = 0xFFD4;
const P1_SET: u16 = 0xFFD5;
const R0_CLEAR: u16 = 0xFFD6;
const R0_SET: u16 = 0xFFD7;
const R1_CLEAR: u16 = 0xFFD8;
const R1_SET: u16 = 0xFFD9;
const M0_CLEAR: u16 = 0xFFDA;
const M0_SET: u16 = 0xFFDB;
const M1_CLEAR: u16 = 0xFFDC;
const M1_SET: u16 = 0xFFDD;
const TY_CLEAR: u16 = 0xFFDE;
const TY_SET: u16 = 0xFFDF;

/// A 32K ROM whose every byte equals its low-address byte, so a read reveals
/// the offset it came from — same trick as `bus_map.rs`'s `marked_rom`.
fn marked_rom() -> Box<[u8]> {
    (0..ROM_SIZE)
        .map(|i| i as u8)
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn bus(memory: MemorySize) -> SystemBus {
    SystemBus::new(MachineVariant::Coco1, memory, marked_rom())
}

// ---- Strobe latching ----------------------------------------------------------

#[test]
fn v_strobes_latch_independently_and_ignore_written_data() {
    let mut b = bus(MemorySize::K64);
    assert_eq!(b.sam.v_bits(), 0);
    b.write(V0_SET, 0xFF); // written data is ignored — only the address matters
    assert_eq!(b.sam.v_bits(), 0b001);
    b.write(V2_SET, 0x00);
    assert_eq!(b.sam.v_bits(), 0b101);
    b.write(V1_SET, 0x42);
    assert_eq!(b.sam.v_bits(), 0b111);
    b.write(V0_CLEAR, 0xFF);
    assert_eq!(b.sam.v_bits(), 0b110);
    b.write(V2_CLEAR, 0);
    b.write(V1_CLEAR, 0);
    assert_eq!(b.sam.v_bits(), 0);
}

#[test]
fn f_strobes_latch_independently_and_dont_disturb_v_bits() {
    let mut b = bus(MemorySize::K64);
    b.write(V0_SET, 0);
    b.write(F0_SET, 0);
    b.write(F1_SET, 0);
    b.write(F6_SET, 0);
    assert_eq!(b.sam.v_bits(), 0b001, "V bits untouched by F strobes");
    assert_eq!(b.sam.f, 0b100_0011);
    b.write(F2_SET, 0);
    assert_eq!(b.sam.f, 0b100_0111);
    b.write(F2_CLEAR, 0);
    assert_eq!(b.sam.f, 0b100_0011);
}

#[test]
fn p1_r0_r1_m0_m1_ty_strobes_latch_independently() {
    let mut b = bus(MemorySize::K64);
    assert!(!b.sam.p1 && !b.sam.r0 && !b.sam.r1 && !b.sam.m0 && !b.sam.m1 && !b.sam.ty);

    b.write(P1_SET, 0);
    assert!(b.sam.p1);
    b.write(P1_CLEAR, 0);
    assert!(!b.sam.p1);

    b.write(R0_SET, 0);
    assert!(b.sam.r0);
    b.write(R0_CLEAR, 0);
    assert!(!b.sam.r0);

    b.write(R1_SET, 0);
    assert!(b.sam.r1);
    b.write(R1_CLEAR, 0);
    assert!(!b.sam.r1);

    b.write(M0_SET, 0);
    assert!(b.sam.m0);
    b.write(M0_CLEAR, 0);
    assert!(!b.sam.m0);

    b.write(M1_SET, 0);
    assert!(b.sam.m1);
    b.write(M1_CLEAR, 0);
    assert!(!b.sam.m1);

    b.write(TY_SET, 0);
    assert!(b.sam.ty);
    b.write(TY_CLEAR, 0);
    assert!(!b.sam.ty);
}

// ---- TY=0: ROM map --------------------------------------------------------------

#[test]
fn ty0_reads_rom_at_extbas_and_bas_windows() {
    let mut b = bus(MemorySize::K64);
    assert_eq!(b.read(0x8000), 0x00); // extbas window, offset 0
    assert_eq!(b.read(0x8123), 0x23); // extbas offset $0123 -> low byte $23
    assert_eq!(b.read(0xA000), 0x00); // bas window, offset 0 -> rom[$2000]
    assert_eq!(b.read(0xA123), 0x23); // bas offset $0123 -> rom[$2123], low byte $23
}

#[test]
fn ty0_writes_to_rom_space_do_not_reach_ram_underneath() {
    let mut b = bus(MemorySize::K64);
    let before_ext = b.read(0x8123);
    let before_bas = b.read(0xA123);
    b.write(0x8123, 0x00);
    b.write(0xA123, 0x00);
    assert_eq!(
        b.read(0x8123),
        before_ext,
        "extbas write must not shadow RAM"
    );
    assert_eq!(b.read(0xA123), before_bas, "bas write must not shadow RAM");
}

// ---- TY=1 + M1: all-RAM ---------------------------------------------------------

#[test]
fn ty1_with_m1_maps_all_ram_through_feff_banking_out_rom() {
    let mut b = bus(MemorySize::K64);
    b.write(M1_SET, 0);
    b.write(TY_SET, 0);

    b.write(0x8123, 0x5A);
    assert_eq!(b.read(0x8123), 0x5A, "extbas window is now plain RAM");
    b.write(0xA123, 0xA5);
    assert_eq!(b.read(0xA123), 0xA5, "bas window is now plain RAM");
    b.write(0xC123, 0x11);
    assert_eq!(b.read(0xC123), 0x11, "cart window is now plain RAM");
    b.write(0xFEFF, 0x22);
    assert_eq!(b.read(0xFEFF), 0x22, "RAM extends through $FEFF");
}

#[test]
fn ty1_without_m1_stays_on_the_rom_map() {
    // The plan: "TY=1 (all-RAM) requires M1 set (64K)". Without M1, TY alone
    // is not modeled as effective all-RAM.
    let mut b = bus(MemorySize::K64);
    b.write(TY_SET, 0); // M1 stays clear
    assert_eq!(b.read(0x8123), 0x23, "still reads ROM, not RAM");
}

// ---- Vector mirror: $FFE0-$FFFF == $BFE0-$BFFF ---------------------------------

#[test]
fn vector_mirror_matches_bas_rom_top_when_ty_clear() {
    let mut b = bus(MemorySize::K64);
    for addr in 0xFFE0u32..=0xFFFF {
        let mirrored = addr - 0xFFE0 + 0xBFE0;
        assert_eq!(
            b.read(addr as u16),
            b.read(mirrored as u16),
            "addr {addr:#06x} must mirror {mirrored:#06x}"
        );
    }
}

#[test]
fn vector_mirror_stays_rom_even_in_all_ram_mode() {
    // Read the expected ROM byte with TY=0 first — once TY=1 flips $BFFE
    // itself over to plain RAM (it's inside the $0000-$FEFF all-RAM range),
    // it's no longer a like-for-like comparison, so capture the ROM value
    // before switching.
    let mut b = bus(MemorySize::K64);
    let rom_byte = b.read(0xBFFE);
    assert_eq!(b.read(0xFFFE), rom_byte, "mirrors $BFFE under TY=0");

    b.write(M1_SET, 0);
    b.write(TY_SET, 0); // all-RAM elsewhere ($BFFE included)...
    b.write(0xFFFE, 0x00); // ...but the mirror region is still ROM: write dropped.
    assert_eq!(
        b.read(0xFFFE),
        rom_byte,
        "vector mirror must stay the same ROM byte under TY=1"
    );
    assert_ne!(
        b.read(0xBFFE),
        rom_byte,
        "sanity: $BFFE itself really did flip to RAM under TY=1"
    );
}

// ---- P1 banking -----------------------------------------------------------------

#[test]
fn p1_banks_the_upper_32k_into_the_low_window() {
    let mut b = bus(MemorySize::K64);
    b.write(M1_SET, 0); // P1 only matters at TY=0 and 64K.
    b.write(P1_SET, 0);
    b.write(0x0123, 0x77); // logical $0123 now really addresses $8123.
    assert_eq!(b.ram[0x8123], 0x77, "P1 banks $0000-$7FFF to $8000-$FFFF");
    assert_eq!(b.read(0x0123), 0x77);

    b.write(P1_CLEAR, 0);
    b.write(0x0123, 0x11);
    assert_eq!(b.ram[0x0123], 0x11, "P1 clear: back to the low 32K");
}

#[test]
fn p1_is_inert_without_m1() {
    let mut b = bus(MemorySize::K64);
    b.write(P1_SET, 0); // M1 stays clear -> P1 has no effect.
    b.write(0x0123, 0x77);
    assert_eq!(b.ram[0x0123], 0x77, "no 64K -> P1 doesn't bank");
    assert_eq!(b.ram[0x8123], 0x00);
}

// ---- Display base / V bits -------------------------------------------------------

#[test]
fn display_base_after_setting_f2() {
    // "BASIC sets F2 -> $0400" (docs/coco12-plan.md) means the 7-bit F
    // register's *decimal value* becomes 2 (the classic default text-screen
    // base), which is bit F1 alone (value 2), not the individually-named F2
    // bit (value 4, which would give $0800). F1_SET sets that bit.
    let mut b = bus(MemorySize::K64);
    b.write(F1_SET, 0);
    assert_eq!(b.sam.f, 2);
    assert_eq!(b.sam.display_base(), 0x0400);
}

// ---- Open bus -------------------------------------------------------------------

#[test]
fn ff60_to_ffbf_is_open_bus_on_coco1_2() {
    let mut b = bus(MemorySize::K64);
    for addr in 0xFF60u16..=0xFFBF {
        assert_eq!(b.read(addr), 0xFF, "addr {addr:#06x} must be open bus");
    }
}

// ---- Speed poke -------------------------------------------------------------------

#[test]
fn cpu_fast_after_r0_or_r1_set() {
    let mut b = bus(MemorySize::K64);
    assert!(!b.sam.cpu_fast());
    b.write(R0_SET, 0);
    assert!(b.sam.cpu_fast());
    b.write(R0_CLEAR, 0);
    assert!(!b.sam.cpu_fast());
    b.write(R1_SET, 0);
    assert!(b.sam.cpu_fast());
    b.write(R0_SET, 0); // both set
    assert!(b.sam.cpu_fast());
    b.write(R1_CLEAR, 0);
    assert!(b.sam.cpu_fast(), "R0 alone still selects fast");
}

// ---- RAM smaller than 64K: truncated, not wrapped --------------------------------

#[test]
fn small_ram_reads_open_bus_and_drops_writes_past_installed_size() {
    let mut b = bus(MemorySize::K4);
    assert_eq!(b.ram.len(), 4 * 1024);
    // $0000-$0FFF is within the installed 4K.
    b.write(0x0100, 0x42);
    assert_eq!(b.read(0x0100), 0x42);
    // $1000 is past the installed 4K but still < $8000 (RAM-mapped logical
    // range under TY=0): must be open bus, not wrapped/mirrored.
    assert_eq!(b.read(0x1000), 0xFF, "out-of-range RAM read is open bus");
    b.write(0x1000, 0x99); // dropped: no panic, no effect observable via read
    assert_eq!(b.read(0x1000), 0xFF);
}
