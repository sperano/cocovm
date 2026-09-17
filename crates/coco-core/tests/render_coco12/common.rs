use coco_core::raster::{CANVAS_W, NON_WIDE_BORDER_X};
use coco_core::video::{BYTES_PER_PIXEL, CELL_H, CELL_W, VDG_ACTIVE_TOP, VDG_XSCALE};
use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize, VDGVariant, VideoStandard};
use mc6809::Bus;

/// Text/graphics base this suite uses throughout: SAM F1 set (`$FFC9`) moves
/// the display base to $0400 (2 * 512), keeping it clear of the parked `BRA *`
/// at $0000.
pub const SCREEN_BASE: u16 = 0x0400;

pub fn coco2_config() -> MachineConfig {
    MachineConfig {
        variant: MachineVariant::Coco2,
        video: VideoStandard::NTSC,
        memory: MemorySize::K64,
        monitor: None,
        vdg: Some(VDGVariant::MC6847),
    }
}

/// A CoCo 1 (forced plain MC6847 by [`MachineConfig::validate`]).
pub fn coco1_config() -> MachineConfig {
    MachineConfig {
        variant: MachineVariant::Coco1,
        video: VideoStandard::NTSC,
        memory: MemorySize::K32,
        monitor: None,
        vdg: Some(VDGVariant::MC6847),
    }
}

/// A CoCo 2 with the MC6847T1 installed.
pub fn coco2_t1_config() -> MachineConfig {
    MachineConfig {
        vdg: Some(VDGVariant::MC6847T1),
        ..coco2_config()
    }
}

/// A machine with a zeroed 16K synthetic ROM (so the reset vector, mirrored
/// from $BFFE/$BFFF, resolves to $0000) and the CPU parked on `BRA *` there,
/// so it never executes anything with side effects — matching
/// `tests/render.rs`'s `text_renderer_follows_sam_page_register`.
pub fn boot_parked_machine_with(config: MachineConfig) -> Machine {
    let rom = vec![0u8; 16 * 1024].into_boxed_slice();
    let mut m = Machine::new(config, rom);
    m.bus.write(0x0000, 0x20); // BRA
    m.bus.write(0x0001, 0xFE); // -2
    m.bus.write(0xFFC9, 0); // SAM F1 set: display base -> $0400
    m
}

pub fn boot_parked_machine() -> Machine {
    boot_parked_machine_with(coco2_config())
}

/// A raw canvas pixel.
pub fn px(fb: &[u8], x: usize, y: usize) -> [u8; 4] {
    let i = (y * CANVAS_W + x) * BYTES_PER_PIXEL;
    fb[i..i + 4].try_into().unwrap()
}

/// The left canvas pixel of VDG dot (`x`, `y`) within the active area: every
/// variant renders legacy modes on the canonical 640×240 raster, the 512 px
/// body behind a 64 px border (dots doubled), top at row 25 (LPF=%00).
pub fn dot(fb: &[u8], x: usize, y: usize) -> [u8; 4] {
    px(fb, NON_WIDE_BORDER_X + x * VDG_XSCALE, VDG_ACTIVE_TOP + y)
}

// --- MC6847 versus MC6847T1 font and lowercase tests (`crates/coco-core/src/font6847.rs`) ---
//
// The expected glyph bit patterns that follow are copied from and cross-checked
// against the unit tests alongside `src/font6847.rs`'s `MC6847_FONT` and
// `MC6847T1_FONT` tables: 'O' (code $0F) at plain-font index 15 / T1-font
// index 15, and lowercase 'a' at T1-font index 64+1=65 (`font6847.rs`'s
// module doc: index 64-95 = lowercase, entry 65 = 'a', screen code $01).

/// VDG screen code for 'O' (`@`=$00, so 'O' = $0F).
pub const CODE_O: u8 = 0x0F;
/// VDG screen code for 'A' (`@`=$00, so 'A' = $01).
pub const CODE_A: u8 = 0x01;

/// `MC6847_FONT[15]` ('O'): square shape (`font6847.rs::tests::plain_o_is_square`).
pub const PLAIN_O_GLYPH: [u8; CELL_H] = [
    0x00, 0x00, 0x00, 0x3E, 0x22, 0x22, 0x22, 0x22, 0x22, 0x3E, 0x00, 0x00,
];
/// `MC6847T1_FONT[15]` ('O'): rounded shape (`font6847.rs::tests::t1_o_is_rounded`).
pub const T1_O_GLYPH: [u8; CELL_H] = [
    0x00, 0x1C, 0x22, 0x22, 0x22, 0x22, 0x22, 0x1C, 0x00, 0x00, 0x00, 0x00,
];

/// Sample the 8×12 cell at (row, col) into a bit grid: `true` where the
/// dot equals `on_color`, `false` where it equals `off_color` (panics on
/// any other color — every glyph dot must be one or the other).
pub fn sample_cell(
    fb: &[u8],
    row: usize,
    col: usize,
    on_color: [u8; 4],
    off_color: [u8; 4],
) -> [[bool; CELL_W]; CELL_H] {
    let mut out = [[false; CELL_W]; CELL_H];
    for (cy, row_out) in out.iter_mut().enumerate() {
        for (cx, bit) in row_out.iter_mut().enumerate() {
            let p = dot(fb, col * CELL_W + cx, row * CELL_H + cy);
            *bit = if p == on_color {
                true
            } else if p == off_color {
                false
            } else {
                panic!("unexpected colour {p:?} at cell ({row},{col}) px ({cx},{cy})");
            };
        }
    }
    out
}

/// Decode a raw font row byte array into the same bit-grid shape
/// [`sample_cell`] produces (leftmost pixel = bit mask `0x80 >> col`,
/// matching `paint_legacy_text_line`).
pub fn glyph_bits(glyph: &[u8; CELL_H]) -> [[bool; CELL_W]; CELL_H] {
    let mut out = [[false; CELL_W]; CELL_H];
    for (cy, &bits) in glyph.iter().enumerate() {
        for (cx, bit) in out[cy].iter_mut().enumerate() {
            *bit = bits & (0x80 >> cx) != 0;
        }
    }
    out
}
