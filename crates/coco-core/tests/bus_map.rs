//! Unit coverage for `SystemBus` address decode with a synthetic ROM: the ROM
//! window, always-ROM vector fetch, the disabled/enabled MMU translation, and the
//! write-8 / read-low-6 MMU register asymmetry (`DESIGN.md` §3).

use coco_core::config::BLOCK_SIZE;
use coco_core::gime::{init0, init1, DISABLED_MMU_BASE, MMU_READ_MASK};
use coco_core::{MemorySize, SystemBus};
use mc6809::Bus;

const ROM_SIZE: usize = 32 * 1024;

/// A 32K ROM whose every byte equals its low-address byte, so a read reveals the
/// offset it came from.
fn marked_rom() -> Box<[u8]> {
    (0..ROM_SIZE).map(|i| i as u8).collect::<Vec<_>>().into_boxed_slice()
}

fn bus(mem: MemorySize) -> SystemBus {
    SystemBus::new(mem, marked_rom())
}

// ---- ROM window --------------------------------------------------------------

#[test]
fn rom_window_maps_8000_to_offset_zero() {
    let mut b = bus(MemorySize::K512);
    assert_eq!(b.read(0x8000), 0x00); // offset $0000
    assert_eq!(b.read(0x8123), 0x23); // offset $0123 -> low byte $23
    assert_eq!(b.read(0xBF00), 0x00); // offset $3F00 -> low byte $00
}

#[test]
fn vectors_read_from_rom_even_in_io_page() {
    let mut b = bus(MemorySize::K512);
    // $FFFE -> ROM offset $7FFE -> low byte $FE.
    assert_eq!(b.read(0xFFFE), 0xFE);
    assert_eq!(b.read(0xFFFF), 0xFF);
}

#[test]
fn io_page_below_vectors_is_not_rom() {
    let mut b = bus(MemorySize::K512);
    // $FF90 is INIT0 (defaults 0), not ROM offset $7F90.
    assert_eq!(b.read(0xFF90), 0x00);
}

#[test]
fn constant_page_fe00_is_ram_not_rom() {
    // $FE00-$FEFF is the interrupt-trampoline page: RAM even though it sits inside
    // the $8000-$FFFF ROM window. Writing then reading must round-trip through RAM.
    let mut b = bus(MemorySize::K512);
    b.write(0xFE00, 0x5A);
    b.write(0xFEFF, 0xA5);
    assert_eq!(b.read(0xFE00), 0x5A);
    assert_eq!(b.read(0xFEFF), 0xA5);
    // The byte just below still reads ROM (writes fall through to shadow RAM).
    b.write(0xFDFF, 0x11);
    assert_eq!(b.read(0xFDFF), marked_rom()[0xFDFF - 0x8000]);
}

#[test]
fn all_ram_mode_exposes_ram_under_rom() {
    let mut b = bus(MemorySize::K512);
    b.gime.all_ram = true; // SAM map-type = RAM: ROM disabled
    b.write(0x8000, 0x5A);
    assert_eq!(b.read(0x8000), 0x5A); // now RAM, not ROM offset 0
}

// ---- MMU translation ---------------------------------------------------------

#[test]
fn disabled_mmu_maps_to_high_window() {
    let mut b = bus(MemorySize::K512);
    assert!(!b.gime.mmu_enabled);
    // Logical $0000 -> physical $70000 in the disabled-MMU window.
    b.write(0x0000, 0xAB);
    assert_eq!(b.ram[DISABLED_MMU_BASE], 0xAB);
    assert_eq!(b.read(0x0000), 0xAB);
}

#[test]
fn small_machine_aliases_high_window_into_top_blocks() {
    // 128K has no physical $70000; the mask relocates it to the top 64K ($10000).
    let mut b = bus(MemorySize::K128);
    b.write(0x0000, 0xCD);
    assert_eq!(b.ram[DISABLED_MMU_BASE % b.ram.len()], 0xCD);
}

#[test]
fn enabled_mmu_uses_task_block() {
    let mut b = bus(MemorySize::K512);
    b.write(0xFFA0, 0x05); // task0 slot0 -> physical block 5
    b.write(INIT0_REG, init0::MMUEN); // enable MMU
    b.write(0x0000, 0x99);
    assert_eq!(b.ram[5 * BLOCK_SIZE], 0x99);
}

#[test]
fn init1_selects_second_task_set() {
    let mut b = bus(MemorySize::K512);
    b.write(0xFFA8, 0x07); // task1 slot0 -> block 7
    b.write(INIT0_REG, init0::MMUEN);
    b.write(INIT1_REG, init1::TR); // select task 1
    assert_eq!(b.gime.task, 1);
    b.write(0x0000, 0x77);
    assert_eq!(b.ram[7 * BLOCK_SIZE], 0x77);
}

// ---- MMU register readback asymmetry ----------------------------------------

#[test]
fn mmu_register_write_8_read_low_6() {
    let mut b = bus(MemorySize::K512);
    b.write(0xFFA3, 0xFF); // full 8 bits stored
    assert_eq!(b.gime.mmu[0][3], 0xFF);
    assert_eq!(b.read(0xFFA3), MMU_READ_MASK); // only low 6 read back
}

const INIT0_REG: u16 = 0xFF90;
const INIT1_REG: u16 = 0xFF91;

// ---- GIME palette → RGB conversion (RGBrgb, 2 bits/channel × 0x55) ----------

#[test]
fn gime_rgb_color_decodes_registers() {
    use coco_core::GIME;
    assert_eq!(GIME::rgb_color(0x00), [0x00, 0x00, 0x00, 0xFF]); // black
    assert_eq!(GIME::rgb_color(0x3F), [0xFF, 0xFF, 0xFF, 0xFF]); // white
    assert_eq!(GIME::rgb_color(0x12), [0x00, 0xFF, 0x00, 0xFF]); // pure green (BASIC text)
    assert_eq!(GIME::rgb_color(0x09), [0x00, 0x00, 0xFF, 0xFF]); // pure blue
    assert_eq!(GIME::rgb_color(0x24), [0xFF, 0x00, 0x00, 0xFF]); // pure red
}
