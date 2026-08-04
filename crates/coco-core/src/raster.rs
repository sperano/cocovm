//! Canonical 640×240 raster geometry (Option B).
//!
//! One fixed-size RGBA canvas for every GIME-native mode, matching MAME's
//! coco3 visible window (`coco3.cpp` `set_raw(..., 912, 0, 640, 262, 1, 240)`).
//! Every legal mode reaches it by an INTEGER horizontal scale: active content
//! is 512 px (non-wide, 64 px border each side) or 640 px (wide, no border) —
//! MAME `gime.cpp` `render_scanline` (`wide = !legacy && ($FF99 & 0x04)`;
//! note `update_geometry` tests bit 3 instead, but only for field-sync
//! timing — the pixel path uses bit 2, which matches the emitted widths).
//! Vertically, the $FF99 LPF field places the active body inside the 240
//! visible rows (MAME `update_geometry`); the machine's scanline counter is
//! already aligned with MAME's physical raster (`config.rs` fs edges), so
//! canvas row == machine line for lines 0..240.

/// Canonical canvas width: MAME's coco3 visible width.
pub const CANVAS_W: usize = 640;
/// Canonical canvas height: MAME's coco3 visible lines.
pub const CANVAS_H: usize = 240;

/// Active-content width of non-wide modes; the rest of the 640 is border.
pub const NON_WIDE_ACTIVE_W: usize = 512;
/// Horizontal border width each side of a non-wide mode's 512 px body.
pub const NON_WIDE_BORDER_X: usize = (CANVAS_W - NON_WIDE_ACTIVE_W) / 2;

/// Vertical placement of the active body inside the 240 visible rows,
/// indexed by the $FF99 LPF field: `(top border rows, body rows)`.
///
/// 192/200/225 come from MAME `update_geometry` (25/23/8 top-border lines;
/// each sums with its bottom border to exactly 240). LPF=%10 is the glitched
/// "zero/infinite" count on real silicon (MAME uses a sentinel; Lomont: the
/// visible result depends on where in the raster the write lands); 210
/// centered is this crate's existing sane approximation (`gime::LPF_LINES`).
pub const fn vertical_window(lpf: usize) -> (usize, usize) {
    match lpf {
        0 => (25, 192),
        1 => (23, 200),
        2 => ((CANVAS_H - 210) / 2, 210),
        _ => (8, 225),
    }
}
