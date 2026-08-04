//! Shared dot-matrix "paper" model for the DMP printer family
//! ( "Family context": DMP-105 today, DMP-130/Epson
//! dialects later share this raster, not the per-model control-code
//! interpreters).
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
//! - **Vertical** ([`Y_UNITS_PER_INCH`]): 1/72", matching every documented
//!   vertical fact directly — the three text line-feed pitches (1/6", 1/8",
//!   1/12" — `dmp105-protocol.md` §4 T9) and the fixed graphics line feed
//!   (7/72" — §5) are all already whole numbers of 1/72" (12, 9, 6, and 7
//!   respectively), so no finer unit is needed to keep them exact.
//! - **Horizontal** ([`X_UNITS_PER_INCH`]): see its doc comment — a derived
//!   internal choice, not a hardware register, so the three pitch densities
//!   (`dmp105-protocol.md` §1 Appendix G) share one exact integer grid.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Vertical fixed-point resolution: 1/72" per unit. Not itself a hardware
/// register — it's the finest unit that keeps every documented vertical fact
/// in `dmp105-protocol.md` (§4 T9's 1/6"/1/8"/1/12" line-feed pitches, §5's
/// fixed 7/72" graphics line feed) an exact integer count of units.
pub const Y_UNITS_PER_INCH: u32 = 72;

/// Horizontal fixed-point resolution: 1/3600". Derived, not a hardware fact:
/// `dmp105-protocol.md` Appendix G (p.59) gives 960/1152/1600 dots over an
/// (arithmetically derived, see `dmp105.rs`'s `Pitch`) constant 8" print
/// width, i.e. 120/144/200 dots per inch for Normal/Compressed/Condensed
/// pitch. 3600 is the LCM of 120, 144, and 200, so each pitch's per-dot
/// spacing (3600/120=30, 3600/144=25, 3600/200=18) is an exact integer
/// number of these units — one common fixed-point grid all three pitches
/// (and mid-line pitch changes) can share without rounding.
pub const X_UNITS_PER_INCH: u32 = 3600;

/// Snapshot of how much paper has been printed on: the furthest dot row
/// reached and how much ink has been laid down, cheap enough to poll every
/// frame from a live-updating frontend view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PaperExtent {
    /// Highest `y` (1/72" units) any dot has been marked at; 0 if the paper
    /// is blank.
    pub max_y: u32,
    /// Total dots marked so far (not deduplicated — a dot re-struck at the
    /// same position, e.g. via the repeat code, counts twice, matching real
    /// ink laid down twice).
    pub dot_count: usize,
}

/// A continuous dot-matrix paper roll: an abstract raster of impressions
/// (dot columns at [`X_UNITS_PER_INCH`], rows at [`Y_UNITS_PER_INCH`]), not
/// pixels — rendering style (dot bleed, tractor-feed strips, page
/// perforations) is entirely a frontend concern (T5).
///
/// Storage is "Vec-of-bands": one row (`y`) maps to the sorted-or-not list of
/// `x` columns marked on it, via a `BTreeMap` so a frontend asking for a
/// visible window (`dots_in_range`) gets an efficient range scan rather than
/// a linear filter over the whole roll.
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

    /// Every dot in the inclusive row range `y0..=y1`, as `(x, y)` pairs —
    /// enough for a frontend to render a visible scroll window without
    /// walking the whole roll.
    pub fn dots_in_range(&self, y0: u32, y1: u32) -> Vec<(u32, u32)> {
        self.rows
            .range(y0..=y1)
            .flat_map(|(&y, xs)| xs.iter().map(move |&x| (x, y)))
            .collect()
    }

    /// The row range touched since the last call (or since construction),
    /// then reset — the simplest "what changed" signal a live-updating
    /// frontend view needs: redraw at least that band, nothing below its
    /// floor could have changed (the print head only ever advances `y`
    /// forward within a print job — see `dmp105.rs`).
    pub fn take_dirty(&mut self) -> Option<(u32, u32)> {
        let range = self.dirty_min.zip(self.dirty_max);
        self.dirty_min = None;
        self.dirty_max = None;
        range
    }

    /// Tear off: discard every dot printed so far. Does not rebase future
    /// `y` coordinates to 0 — the print head's own position (owned by the
    /// interpreter, not `Paper`) keeps advancing along the same continuous
    /// axis it always has, matching the roll's "no page concept" design.
    /// Rebasing a *view* to start fresh after tear-off is a T5 rendering
    /// choice, not modeled here.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.dirty_min = None;
        self.dirty_max = None;
    }
}

#[cfg(test)]
#[path = "printer_test.rs"]
mod tests;
