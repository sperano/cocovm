//! Canonical 744×243 raster geometry (Option B, `docs/plan-per-scanline-video.md`).
//!
//! One fixed-size RGBA canvas for every GIME-native mode, covering the full
//! visible NTSC picture: the same time window the legacy VDG framebuffer
//! spans (`video.rs` — 186 of 228 VDG clocks per line, 243 of 262 lines,
//! MAME `mc6847.cpp`), sampled at the GIME dot clock, which is exactly twice
//! the VDG pixel rate. Hence [`CANVAS_W`]` = 2 × `[`video::FB_W`] and
//! [`CANVAS_H`]` = `[`video::FB_H`]. MAME's own coco3 visible window
//! (`coco3.cpp` `set_raw(..., 912, 0, 640, 262, 1, 240)`) crops the border
//! to the widest active span; it embeds in this canvas at the fixed offset
//! `(`[`WIDE_BORDER_X`]`, 0)`, rows `0..240`, so trace-diffing a frame
//! against MAME is still a crop, never a rescale.
//!
//! Every legal mode reaches the canvas by an INTEGER horizontal scale:
//! active content is 512 px behind a [`NON_WIDE_BORDER_X`] border or 640 px
//! behind a [`WIDE_BORDER_X`] border — MAME `gime.cpp` `render_scanline`
//! (`wide = !legacy && ($FF99 & 0x04)`; note `update_geometry` tests bit 3
//! instead, but only for field-sync timing — the pixel path uses bit 2,
//! which matches the emitted widths). Vertically, the $FF99 LPF field
//! places the active body inside the 243 visible rows (MAME
//! `update_geometry`); the machine's scanline counter is already aligned
//! with MAME's physical raster (`config.rs` fs edges), so canvas row ==
//! machine line for lines 0..243.

use crate::video;

/// Canonical canvas width: the full visible NTSC line at the GIME dot
/// clock — twice the legacy VDG framebuffer's width, same time window.
pub const CANVAS_W: usize = 2 * video::FB_W;
/// Canonical canvas height: the visible NTSC field (25 + 192 + 26 lines),
/// shared with the legacy framebuffer.
pub const CANVAS_H: usize = video::FB_H;

/// Active-content width of wide modes ($FF99 HRES bit 2 set): MAME's whole
/// coco3 visible window, a 640-dot span inside this canvas's visible line.
pub const WIDE_ACTIVE_W: usize = 640;
/// Horizontal border width each side of a wide mode's 640 px body. 52 GIME
/// dots — narrower in *time* than `video::BORDER_X`'s 58 VDG pixels (which
/// are 2 dots each): the wide body swallows most of the line.
pub const WIDE_BORDER_X: usize = (CANVAS_W - WIDE_ACTIVE_W) / 2;

/// Active-content width of non-wide modes; the rest of the line is border.
pub const NON_WIDE_ACTIVE_W: usize = 512;
/// Horizontal border width each side of a non-wide mode's 512 px body.
pub const NON_WIDE_BORDER_X: usize = (CANVAS_W - NON_WIDE_ACTIVE_W) / 2;

/// Vertical placement of the active body inside the 243 visible rows,
/// indexed by the $FF99 LPF field: `(top border rows, body rows)`.
///
/// 192/200/225 come from MAME `update_geometry` (25/23/8 top-border lines;
/// the bottom border is the visible remainder — 26/20/10 rows). LPF=%10 is
/// the glitched "zero/infinite" count on real silicon (MAME uses a
/// sentinel; Lomont: the visible result depends on where in the raster the
/// write lands); 210 centered is this crate's existing sane approximation
/// (`gime::LPF_LINES`).
pub const fn vertical_window(lpf: usize) -> (usize, usize) {
    match lpf {
        0 => (25, 192),
        1 => (23, 200),
        2 => ((CANVAS_H - 210) / 2, 210),
        _ => (8, 225),
    }
}
