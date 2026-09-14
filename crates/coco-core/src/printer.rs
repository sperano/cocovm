//! Shared dot-matrix "paper" model for the DMP printer family.
//! The DMP-105 uses this raster today. Future DMP-130/Epson dialects can share
//! it without sharing the per-model control-code interpreters.
//!
//! The paper is a continuous roll: no page/form-feed concept exists in any
//! documented DMP-105 behavior (`dmp105-protocol.md` §3, "FF (0x0C): VERIFIED
//! ABSENT"), so [`Paper`] never introduces one either — an 11" page boundary
//! is purely a frontend rendering choice (T5), not modeled here.
//!
//! Two independent fixed-point axes, chosen so every documented pitch/feed
//! value converts to an exact integer — no floats anywhere in position
//! accounting:
//!
//! - **Vertical** ([`Y_UNITS_PER_INCH`]): 1/432 inch, the least common
//!   multiple of the DMP-105 and DMP-130's documented feed denominators
//!   (48, 72, 144, and 216). This is software precision, not a motor claim.
//! - **Horizontal** ([`X_UNITS_PER_INCH`]): see its doc comment — a derived
//!   internal choice, not a hardware register, so the three pitch densities
//!   (`dmp105-protocol.md` §1 Appendix G) share one exact integer grid.

use std::collections::BTreeMap;
use std::mem;
use std::ops::ControlFlow;

use serde::{Deserialize, Serialize};

/// Exact software grid for both printers' documented vertical feed commands.
/// See `docs/dmp130-protocol.md`; not a physical stepper resolution.
pub const Y_UNITS_PER_INCH: u32 = 432;

/// Horizontal fixed-point resolution: 1/3600". Derived, not a hardware fact:
/// `dmp105-protocol.md` Appendix G (p.59) gives 960/1152/1600 dots over an
/// (arithmetically derived, see `dmp105.rs`'s `Pitch`) constant 8" print
/// width, that is, 120/144/200 dots per inch for Normal/Compressed/Condensed
/// pitch. 3600 is the LCM of 120, 144, and 200, so each pitch's per-dot
/// spacing (3600/120=30, 3600/144=25, 3600/200=18) is an exact integer
/// number of these units — one common fixed-point grid all three pitches
/// (and mid-line pitch changes) can share without rounding.
pub const X_UNITS_PER_INCH: u32 = 3600;

/// Conservative per-row allowance for the B-tree node, key, allocator
/// metadata, and unused node slots that `size_of::<BTreeMap>()` cannot see.
const BTREE_ROW_ALLOCATION_OVERHEAD_ESTIMATE: usize = 128;
/// Covers the sparsely occupied root node independently of row count.
const BTREE_BASE_ALLOCATION_OVERHEAD_ESTIMATE: usize = 512;

/// Snapshot of how much paper has been printed on: the furthest dot row
/// reached and how much ink has been laid down. The frontend can poll it every
/// frame for a live-updating view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PaperExtent {
    /// Highest `y` (1/432" units) any dot has been marked at; 0 if the paper
    /// is blank.
    pub max_y: u32,
    /// Total dots marked so far (not deduplicated — a dot re-struck at the
    /// same position, such as through the repeat code, counts twice, matching real
    /// ink laid down twice).
    pub dot_count: usize,
}

/// A continuous dot-matrix paper roll: an abstract raster of impressions
/// (dot columns at [`X_UNITS_PER_INCH`], rows at [`Y_UNITS_PER_INCH`]), not
/// pixels — rendering style (dot bleed, tractor-feed strips, page
/// perforations) is entirely a frontend concern (T5).
///
/// Storage is "Vec-of-bands": one row (`y`) maps to its list of marked `x`
/// columns. A `BTreeMap` lets the frontend scan a visible window efficiently
/// instead of filtering the whole roll.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Paper {
    rows: BTreeMap<u32, Vec<u32>>,
    /// Frontend redraw hint (the row range touched since the last
    /// [`Paper::take_dirty`]), not paper content — skipped, `None` default
    /// is safe: after a snapshot restore the paper window has no prior
    /// frame to diff against anyway, so it always repaints in full
    /// regardless of what `take_dirty` would have reported.
    #[serde(skip)]
    dirty_min: Option<u32>,
    #[serde(skip)]
    dirty_max: Option<u32>,
}

impl Paper {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one dot impression at absolute position (`x`, `y`).
    pub fn mark(&mut self, x: u32, y: u32) {
        self.rows.entry(y).or_default().push(x);
        self.dirty_min = Some(self.dirty_min.map_or(y, |m| m.min(y)));
        self.dirty_max = Some(self.dirty_max.map_or(y, |m| m.max(y)));
    }

    /// How much paper has been printed on so far.
    pub fn extent(&self) -> PaperExtent {
        PaperExtent {
            max_y: self.rows.keys().next_back().copied().unwrap_or(0),
            dot_count: self.rows.values().map(Vec::len).sum(),
        }
    }

    /// Estimates owned storage, including allocated dot-vector capacity and
    /// a conservative allowance for each row's B-tree storage.
    ///
    /// This value is a memory-budget guard, not an allocator measurement.
    pub fn estimated_owned_bytes(&self) -> usize {
        self.rows.values().fold(
            mem::size_of::<Self>().saturating_add(BTREE_BASE_ALLOCATION_OVERHEAD_ESTIMATE),
            |total, dots| {
                total
                    .saturating_add(BTREE_ROW_ALLOCATION_OVERHEAD_ESTIMATE)
                    .saturating_add(dots.capacity().saturating_mul(mem::size_of::<u32>()))
            },
        )
    }

    /// Every dot in the inclusive row range `y0..=y1`, as `(x, y)` pairs — for
    /// rendering a visible scroll window without walking the whole roll.
    pub fn dots_in_range(&self, y0: u32, y1: u32) -> Vec<(u32, u32)> {
        let mut dots = Vec::new();
        self.visit_dots_in_range(y0, y1, |x, y| dots.push((x, y)));
        dots
    }

    /// Calls `visit` for every dot in the inclusive row range `y0..=y1`.
    ///
    /// Rendering uses this form to avoid allocating a second, page-local copy
    /// of the paper's dots.
    pub fn visit_dots_in_range(&self, y0: u32, y1: u32, mut visit: impl FnMut(u32, u32)) {
        let _ = self.try_visit_dots_in_range(y0, y1, |x, y| {
            visit(x, y);
            ControlFlow::Continue(())
        });
    }

    /// Visits dots until `visit` requests an early break.
    pub fn try_visit_dots_in_range(
        &self,
        y0: u32,
        y1: u32,
        mut visit: impl FnMut(u32, u32) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        for (&y, xs) in self.rows.range(y0..=y1) {
            for &x in xs {
                visit(x, y)?;
            }
        }
        ControlFlow::Continue(())
    }

    /// The row range touched since the last call (or since construction), then
    /// reset — the minimal "what changed" signal for a live-updating frontend view.
    pub fn take_dirty(&mut self) -> Option<(u32, u32)> {
        let range = self.dirty_min.zip(self.dirty_max);
        self.dirty_min = None;
        self.dirty_max = None;
        range
    }

    /// Tear off: discard every dot printed so far. Does not rebase future `y`
    /// coordinates to 0 — the print head keeps advancing along the same axis.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.dirty_min = None;
        self.dirty_max = None;
    }
}

#[cfg(test)]
#[path = "printer_test.rs"]
mod tests;
