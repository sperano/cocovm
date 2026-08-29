//! Unit coverage for `SystemBus` address decode with a synthetic ROM: the ROM
//! window, always-ROM vector fetch, the disabled/enabled MMU translation, and the
//! write-8 / read-low-6 MMU register asymmetry (`DESIGN.md` §3).

use coco_core::cart::Cart;
use coco_core::config::BLOCK_SIZE;
use coco_core::gime::{DISABLED_MMU_BASE, MMU_READ_MASK, init0, init1};
use coco_core::{MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

const ROM_SIZE: usize = 32 * 1024;

/// A 32K ROM whose every byte equals its low-address byte, so a read reveals the
/// offset it came from.
fn marked_rom() -> Box<[u8]> {
    (0..ROM_SIZE)
        .map(|i| i as u8)
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn bus(mem: MemorySize) -> SystemBus {
    SystemBus::new(MachineVariant::Coco3, mem, marked_rom())
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

// ---- INIT0 MC1:MC0 ROM map -----------------------------------------------------

/// A cartridge whose ROM window answers with a recognizable constant.
struct MarkerCart;
impl coco_core::cart::Cartridge for MarkerCart {
    fn read(&mut self, _addr: u16) -> u8 {
        0xFF
    }
    fn write(&mut self, _addr: u16, _val: u8) {}
    fn rom_read(&mut self, _addr: u16) -> u8 {
        0xAA
    }
}

#[test]
fn mc_16k_split_routes_upper_half_to_cartridge() {
    let mut b = bus(MemorySize::K512);
    // Power-on INIT0 = $00 -> MC=00: 16K internal + 16K external. The empty
    // slot answers open-bus $00 (MAME trace-diff verified), not internal ROM.
    assert_eq!(b.read(0x8123), 0x23, "lower half stays internal");
    assert_eq!(b.read(0xC123), 0x00, "upper half is the (empty) cartridge");

    b.cart = Cart::custom(MarkerCart);
    assert_eq!(b.read(0xC123), 0xAA, "upper half reads the cartridge ROM");
    assert_eq!(b.read(0x8123), 0x23, "lower half still internal");
}

#[test]
fn mc_32k_internal_keeps_upper_half_internal() {
    let mut b = bus(MemorySize::K512);
    b.cart = Cart::custom(MarkerCart);
    // The cold-start value: MC=10 (32K internal) — what a diskless boot runs.
    b.write(0xFF90, init0::MC1);
    assert_eq!(b.read(0xC123), 0x23, "upper half reads internal ROM");
}

#[test]
fn mc_32k_external_maps_whole_window_except_vectors() {
    let mut b = bus(MemorySize::K512);
    b.cart = Cart::custom(MarkerCart);
    b.write(0xFF90, init0::MC1 | init0::MC0);
    assert_eq!(b.read(0x8123), 0xAA, "lower half external under MC=11");
    assert_eq!(b.read(0xFDFF), 0xAA, "top of window external");
    assert_eq!(b.read(0xFFFE), 0xFE, "vectors always internal ROM");
}

// ---- $FFE0-$FFFF hardwired-ROM window (fact 4: MAME coco3.cpp:53-58 / Astle) ---

#[test]
fn hardwired_window_reads_internal_rom_with_32k_external_cart() {
    let mut b = bus(MemorySize::K512);
    b.cart = Cart::custom(MarkerCart);
    b.write(0xFF90, init0::MC1 | init0::MC0); // MC=11: 32K external
    assert_eq!(
        b.read(0x8000),
        0xAA,
        "external cart drives the rest of the window"
    );
    for addr in 0xFFE0u32..=0xFFFF {
        assert_eq!(
            b.read(addr as u16),
            marked_rom()[(addr - 0x8000) as usize],
            "addr {addr:#06x} must read internal ROM, not the cart"
        );
    }
}

#[test]
fn hardwired_window_reads_internal_rom_even_in_all_ram_mode() {
    let mut b = bus(MemorySize::K512);
    b.cart = Cart::custom(MarkerCart);
    b.write(0xFF90, init0::MC1 | init0::MC0); // MC=11: 32K external
    b.gime.all_ram = true; // SAM TY set: ROM disabled everywhere else
    assert_eq!(
        b.read(0x8000),
        0x00,
        "all-RAM mode: $8000 now reads RAM, not ROM or cart"
    );
    for addr in 0xFFE0u32..=0xFFFF {
        assert_eq!(
            b.read(addr as u16),
            marked_rom()[(addr - 0x8000) as usize],
            "all-RAM mode: {addr:#06x} must still read internal ROM"
        );
    }
}

#[test]
fn hardwired_window_writes_are_dropped_not_shadowed_to_ram() {
    let mut b = bus(MemorySize::K512);
    let before = b.read(0xFFFE);
    b.write(0xFFFE, 0x00);
    assert_eq!(
        b.read(0xFFFE),
        before,
        "write to the hardwired window must be a no-op"
    );
}

#[test]
fn constant_page_fe00_is_ram_when_mc3_set() {
    // With INIT0 MC3 set (how BASIC runs: reset writes $0A, boot writes $CC —
    // both have bit 3 set), $FE00-$FEFF is the constant interrupt-trampoline
    // RAM page even though ROM is mapped: write/read must round-trip.
    let mut b = bus(MemorySize::K512);
    b.write(0xFF90, init0::MC3);
    b.write(0xFE00, 0x5A);
    b.write(0xFEFF, 0xA5);
    assert_eq!(b.read(0xFE00), 0x5A);
    assert_eq!(b.read(0xFEFF), 0xA5);
    // The immediately preceding byte still reads ROM (writes fall through to
    // shadow RAM).
    // MC=10 (32K internal) so $FDFF is internal ROM, not the external window.
    b.write(0xFF90, init0::MC3 | init0::MC1);
    b.write(0xFDFF, 0x11);
    assert_eq!(b.read(0xFDFF), marked_rom()[0xFDFF - 0x8000]);
}

#[test]
fn vector_page_follows_rom_map_when_mc3_clear() {
    // INIT0 MC3=0: $FE00-$FEFF follows the normal map like the rest of the
    // window (MAME gime.cpp update_memory bank 8 — `force_ram` only when MC3
    // is set). In 16K+16K mode the page is the tail of the external bank —
    // the ONLY way a pak's last $200 bytes are addressable (CTS stops at
    // $FDFF); Sokoban keeps its palette tables there and copies them out through
    // $FE88 reads. In 32K-internal mode it reads coco3.rom offset $7Exx.
    let mut b = bus(MemorySize::K512);
    b.cart = Cart::custom(MarkerCart);
    b.write(0xFF90, 0x00); // MC3=0, MC=00: 16K internal + 16K external
    assert_eq!(b.read(0xFE88), 0xAA, "vector page reads the cartridge");
    b.write(0xFF90, init0::MC1); // MC3=0, MC=10: 32K internal
    assert_eq!(
        b.read(0xFE88),
        marked_rom()[0x7E88],
        "vector page reads internal ROM offset $7E88"
    );
    // MC3 set flips the same address to the constant RAM page.
    b.write(0xFF90, init0::MC3 | init0::MC1);
    b.write(0xFE88, 0x21);
    assert_eq!(b.read(0xFE88), 0x21, "MC3 diverts the page to constant RAM");
}

#[test]
fn vector_page_is_mapped_ram_in_all_ram_mode_when_mc3_clear() {
    let mut b = bus(MemorySize::K512);
    b.cart = Cart::custom(MarkerCart);
    b.write(0xFF90, 0x00); // MC3=0
    b.gime.all_ram = true; // SAM TY: no ROM anywhere in the window
    b.write(0xFE88, 0x7E);
    assert_eq!(b.read(0xFE88), 0x7E, "all-RAM mode: plain mapped RAM");
}

#[test]
fn all_ram_mode_exposes_ram_under_rom() {
    let mut b = bus(MemorySize::K512);
    b.gime.all_ram = true; // SAM map-type = RAM: ROM disabled
    b.write(0x8000, 0x5A);
    assert_eq!(b.read(0x8000), 0x5A); // now RAM, not ROM offset 0
}

// ---- INIT0 MC2 SCS* gate ($FF40-$FF5F) -----------------------------------------

/// Records the last value written to the SCS window or `$FF7F`, answering
/// `id` until then — same read-your-last-write shape as `tests/mpi.rs`'s
/// `TestCart`, so a bus read proves whether a write reached the cartridge.
struct RecordingCart {
    id: u8,
    last_write: Option<u8>,
    last_control_write: Option<u8>,
}
impl RecordingCart {
    fn new(id: u8) -> Self {
        Self {
            id,
            last_write: None,
            last_control_write: None,
        }
    }
}
impl coco_core::cart::Cartridge for RecordingCart {
    fn read(&mut self, addr: u16) -> u8 {
        self.peek(addr)
    }
    fn peek(&self, _addr: u16) -> u8 {
        self.last_write.unwrap_or(self.id)
    }
    fn write(&mut self, _addr: u16, val: u8) {
        self.last_write = Some(val);
    }
    fn control_read(&mut self) -> u8 {
        self.last_control_write.unwrap_or(self.id)
    }
    fn control_write(&mut self, val: u8) {
        self.last_control_write = Some(val);
    }
}

const SCS_BASE: u16 = 0xFF40;
const SCS_LAST: u16 = 0xFF5F;
const CART_EXT_BASE: u16 = 0xFF60;
const CART_EXT_LAST: u16 = 0xFF7E;
const MPI_CONTROL: u16 = 0xFF7F;

#[test]
fn scs_window_reads_zero_and_drops_writes_when_mc2_clear() {
    let mut b = bus(MemorySize::K512);
    b.cart = Cart::custom(RecordingCart::new(0xA5));
    // INIT0 defaults to $00 on power-on: MC2 clear, gate closed.
    for addr in SCS_BASE..=SCS_LAST {
        assert_eq!(
            b.read(addr),
            0x00,
            "closed gate must read a hard 0 at {addr:#06x}"
        );
    }
    b.write(SCS_BASE, 0x11);
    assert_eq!(
        b.read(SCS_BASE),
        0x00,
        "a write while closed must never reach the cartridge"
    );
}

#[test]
fn scs_window_restores_normal_dispatch_when_mc2_set() {
    let mut b = bus(MemorySize::K512);
    b.cart = Cart::custom(RecordingCart::new(0xA5));
    b.write(INIT0_REG, init0::MC2);
    assert_eq!(
        b.read(SCS_BASE),
        0xA5,
        "open gate must reach the cartridge's id"
    );
    b.write(SCS_BASE, 0x77);
    assert_eq!(
        b.read(SCS_BASE),
        0x77,
        "open gate must let the write reach the cartridge"
    );
}

#[test]
fn debugger_peek_follows_the_scs_gate() {
    let mut b = bus(MemorySize::K512);
    b.cart = Cart::custom(RecordingCart::new(0xA5));
    // Closed (power-on default) and open must agree with a real read.
    assert_eq!(b.peek(SCS_BASE), b.read(SCS_BASE));
    assert_eq!(b.peek(SCS_BASE), 0x00);
    b.write(INIT0_REG, init0::MC2);
    assert_eq!(b.peek(SCS_BASE), b.read(SCS_BASE));
    assert_eq!(b.peek(SCS_BASE), 0xA5);
}

#[test]
fn cart_extension_window_ignores_mc2() {
    let mut b = bus(MemorySize::K512);
    b.cart = Cart::custom(RecordingCart::new(0xA5));
    // MC2 stays clear (power-on default): $FF60-$FF7E is outside the SCS*
    // decode, so it must still reach the cartridge either way.
    assert_eq!(b.read(CART_EXT_BASE), 0xA5);
    b.write(CART_EXT_BASE, 0x33);
    assert_eq!(b.read(CART_EXT_LAST), 0x33, "any address across the range");
}

#[test]
fn mpi_control_register_ignores_mc2() {
    let mut b = bus(MemorySize::K512);
    b.cart = Cart::custom(RecordingCart::new(0xA5));
    // MC2 stays clear (power-on default): $FF7F is the MPI's own select
    // register, not part of the SCS* decode, so it must still reach the cart.
    assert_eq!(b.read(MPI_CONTROL), 0xA5);
    b.write(MPI_CONTROL, 0x99);
    assert_eq!(b.read(MPI_CONTROL), 0x99);
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
