//! Tandy DMP-105 dot-matrix printer interpreter: byte stream in (as decoded
//! by [`crate::bitbanger::BitBanger`]), abstract dot-raster paper out
//! (`crate::printer::Paper`). Every hardware fact cited here is sourced from
//! `docs/dmp105-protocol.md`; only entries that document marks VERIFIED are
//! implemented — see that document's own INFERRED/UNVERIFIABLE flags for
//! what's deliberately left out (e.g. exact BUSY assertion granularity, the
//! European character set's per-code glyph mapping).
//!
//! `docs/printer-plan.md` T4's "Family context": [`crate::printer`] holds the
//! DMP-family-shared paper model; this module holds everything specific to
//! the DMP-105's own control-code dialect, so a DMP-130/Epson dialect (V3)
//! can share the paper without inheriting 105-only parsing.
//!
//! # Position accounting
//!
//! `x` is in [`crate::printer::X_UNITS_PER_INCH`] units, `y` in
//! [`crate::printer::Y_UNITS_PER_INCH`] units — both exact fixed-point
//! integers, never floats (see `printer.rs`'s doc comment for why those
//! particular denominators were chosen).
//!
//! One documented fact this module does **not** attempt to reproduce as a
//! literal equality: `dmp105-protocol.md` §5 states "11 full-pitch LFs = 18
//! graphics LFs exactly; 11 half LFs = 9 graphics LFs" as a manual-verified
//! "rounding trap". Using the spec's own independently-verified numbers (a
//! text LF pitch of 1/6" = 12 y-units, and the fixed graphics LF of 7/72" = 7
//! y-units), `11 * 12 = 132` while `18 * 7 = 126` — not equal, and no integer
//! y-unit choice makes `11 * k = 126` work either (126 isn't divisible by
//! 11). Reconciling the manual's stated identity would require a physical
//! stepper-motor step-resolution fact that isn't in the spec document (the
//! most likely explanation: the identity is an artifact of discrete motor
//! step rounding across repeated feeds, not a statement about nominal inch
//! math) — that fact was not provided, so it is not fabricated here. The y-
//! unit arithmetic itself (7/72" graphics LF, 1/6"/1/8"/1/12" text LF
//! pitches) is implemented exactly per the individually-verified numbers;
//! see the `graphics_lf_vs_text_lf_rounding_trap_is_not_reproducible_from_
//! given_facts` test for the documented discrepancy.

use std::cell::RefCell;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::bitbanger::PrinterSink;
use crate::bitbanger::sink_serde::SinkState;
use crate::dmp105_font::Glyph;
use crate::printer::{Paper, PaperExtent, X_UNITS_PER_INCH, Y_UNITS_PER_INCH};

mod protocol;

/// Character cell width in dots at every pitch: 9 glyph + 3 gap
/// (`dmp105-protocol.md` §1, Appendix G p.59: "dots/char = 12").
const CELL_DOTS: u32 = 12;

/// Row offset (in dot rows) of the descender row below the 7-dot glyph body
/// (`dmp105-protocol.md` §1/§6).
const DESCENDER_ROW: u32 = 7;

/// Fixed graphics-mode line feed: 7/72" (`dmp105-protocol.md` §5).
const GRAPHICS_LF_UNITS: u32 = 7;

/// Right limit of the physical print zone in x-units: the head cannot move
/// past the 8.0" line (`dmp105-protocol.md` §1 — 960 dot columns at 10 CPI).
/// Marks past this are dropped, matching the physical platen limit; without
/// it, a stream that never sends CR (or an out-of-range `1B 10` position)
/// grows the paper model without bound.
const PRINT_WIDTH_X_UNITS: u32 = 8 * X_UNITS_PER_INCH;

/// Text-mode line-feed pitches, all exact whole numbers of
/// [`Y_UNITS_PER_INCH`] (`dmp105-protocol.md` §4 T9).
const LF_PITCH_1_6: u32 = Y_UNITS_PER_INCH / 6;
const LF_PITCH_1_8: u32 = Y_UNITS_PER_INCH / 8;
const LF_PITCH_1_12: u32 = Y_UNITS_PER_INCH / 12;

/// Non-ESC control codes (`dmp105-protocol.md` §3).
mod control {
    pub const NUL_IGNORED_0: u8 = 0x00;
    pub const NUL_IGNORED_1: u8 = 0x01;
    pub const LF: u8 = 0x0A;
    pub const CR: u8 = 0x0D;
    pub const END_UNDERLINE: u8 = 0x0E;
    pub const START_UNDERLINE: u8 = 0x0F;
    pub const SELECT_GRAPHICS: u8 = 0x12;
    /// Repeat introducer (`28 n c` / `1C n c`): distinct from the *escape*
    /// selector `1B 1C` ("LF pitch = 1/12") — same byte value, but only ever
    /// interpreted as this when it arrives outside an in-progress escape
    /// sequence.
    pub const REPEAT: u8 = 0x1C;
    pub const END_GRAPHICS: u8 = 0x1E;
    pub const ESC: u8 = 0x1B;
}

/// Escape-sequence selector bytes, i.e. the byte immediately after `ESC`
/// (`dmp105-protocol.md` §4).
mod esc {
    pub const ELONGATE_START: u8 = 0x0E;
    pub const ELONGATE_END: u8 = 0x0F;
    pub const POSITION: u8 = 0x10;
    pub const PITCH_NORMAL: u8 = 0x13;
    pub const PITCH_CONDENSED: u8 = 0x14;
    pub const NL_CR_ONLY: u8 = 0x15;
    pub const NL_CR_LF: u8 = 0x16;
    pub const PITCH_COMPRESSED: u8 = 0x17;
    /// `1B 1C`: LF pitch = 1/12" — same numeric value as [`super::control::REPEAT`]
    /// but only reached from `Pending::Esc`, never from a fresh byte.
    pub const LF_PITCH_1_12: u8 = 0x1C;
    pub const BOLD_START: u8 = 0x1F;
    pub const BOLD_END: u8 = 0x20;
    pub const LF_PITCH_1_6: u8 = 0x36;
    pub const LF_PITCH_1_8: u8 = 0x38;
    pub const DIRECTION: u8 = 0x55;
    pub const FEED_IMMEDIATE: u8 = 0x5A;
    pub const FEED_LATCH: u8 = 0x5B;
}

/// Character-Print vs Graphics mode (`dmp105-protocol.md` §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Mode {
    CharacterPrint,
    Graphics,
}

/// New-line mode selected by `1B 15`/`1B 16` (`dmp105-protocol.md` §4 T11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum NlMode {
    CrOnly,
    CrLf,
}

/// Carriage direction selected by `1B 55 00/01`
/// (`dmp105-protocol.md` §1/§4 T16). Stored only: no physical print head
/// exists to model direction against, so it never affects output — matching
/// the plan's "unidirectional/bidirectional affects nothing in emulation"
/// direction for this task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Direction {
    Bidirectional,
    Unidirectional,
}

/// Print pitch (`dmp105-protocol.md` §1 Appendix G p.59). Character cell
/// width is always [`CELL_DOTS`] dots regardless of pitch; pitch instead
/// changes how many dots (thus inches) that fixed-width cell spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Pitch {
    Normal,
    Compressed,
    Condensed,
}

impl Pitch {
    /// Dots per 8" line width, as given directly by Appendix G (p.59):
    /// Normal 960, Compressed (12 CPI) 1152, Condensed (16.7 CPI) 1600.
    const fn dots_per_line(self) -> u32 {
        match self {
            Pitch::Normal => 960,
            Pitch::Compressed => 1152,
            Pitch::Condensed => 1600,
        }
    }

    /// Dots per inch, derived (not directly stated) from
    /// `dots_per_line() / 8`: the 8" print-line width itself falls out of
    /// two independently-verified Appendix G facts (80 columns at 10 CPI =
    /// 8"; 960 dots / 80 columns = 12 dots/char, matching the separately
    /// verified "12 dots wide at every pitch" cell width) and is confirmed
    /// self-consistent against all three pitches (960/8=120 -> 12/120=1/10";
    /// 1152/8=144 -> 12/144=1/12"; 1600/8=200 -> 12/200=1/16.67", each
    /// exactly matching that pitch's named CPI).
    const fn dots_per_inch(self) -> u32 {
        self.dots_per_line() / 8
    }

    /// Physical spacing between two adjacent dots at this pitch, in
    /// [`X_UNITS_PER_INCH`] units — exact by construction (see that
    /// constant's doc comment).
    const fn dot_spacing(self) -> u32 {
        X_UNITS_PER_INCH / self.dots_per_inch()
    }
}

/// Assembly state for a not-yet-complete multi-byte escape or repeat
/// sequence (`dmp105-protocol.md` §4: "all sequences are 2-4 bytes, fixed
/// lengths").
#[derive(Debug, Clone, Serialize, Deserialize)]
enum Pending {
    None,
    /// Saw `ESC` ($1B), waiting for the selector byte.
    Esc,
    /// Saw `ESC` + a selector that takes operand bytes, collecting them.
    EscOperands {
        selector: u8,
        operands: Vec<u8>,
        need: usize,
    },
    /// Saw the repeat introducer ($1C), waiting for `n`.
    Repeat1,
    /// Saw repeat's `n`, waiting for `c`.
    Repeat2 {
        n: u8,
    },
}

/// The DMP-105 interpreter: control/escape-code state machine plus the
/// [`Paper`] it prints onto.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DMP105 {
    mode: Mode,
    /// Live pitch setting (`1B 13`/`1B 14`/`1B 17`), always up to date even
    /// while printing graphics (which uses `graphics_pitch` instead — see
    /// its doc comment).
    pitch: Pitch,
    /// Density latched at Graphics-mode entry (`dmp105-protocol.md` §5:
    /// "Pitch must be selected *before* entry (ignored inside)"): pitch
    /// changes received while in Graphics mode still update `pitch` (for
    /// whenever CP mode resumes) but never this field, so an in-progress
    /// graphics run's dot spacing never shifts underneath it.
    graphics_pitch: Pitch,
    /// Latched line-feed pitch in [`Y_UNITS_PER_INCH`] units, set by
    /// `1B 1C`/`1B 36`/`1B 38`/`1B 5B n` and consumed by plain `LF`/`CR`.
    lf_pitch_units: u32,
    nl_mode: NlMode,
    underline: bool,
    elongation: bool,
    bold: bool,
    direction: Direction,
    /// Absolute head position, in [`X_UNITS_PER_INCH`] units.
    x: u32,
    /// Absolute head position, in [`Y_UNITS_PER_INCH`] units.
    y: u32,
    pending: Pending,
    paper: Paper,
}

impl Default for DMP105 {
    /// Power-on defaults (`dmp105-protocol.md` §7): Normal 10 CPI; LF pitch
    /// 1/6"; NL mode CR+LF; underline/elongation/bold off; bidirectional;
    /// buffer cleared. Head position (0, 0) is this implementation's choice
    /// for "start of a fresh roll" — the manual doesn't give the resting
    /// head position a numeric value, but a blank roll has no other sensible
    /// origin.
    fn default() -> Self {
        Self {
            mode: Mode::CharacterPrint,
            pitch: Pitch::Normal,
            graphics_pitch: Pitch::Normal,
            lf_pitch_units: LF_PITCH_1_6,
            nl_mode: NlMode::CrLf,
            underline: false,
            elongation: false,
            bold: false,
            direction: Direction::Bidirectional,
            x: 0,
            y: 0,
            pending: Pending::None,
            paper: Paper::new(),
        }
    }
}

impl DMP105 {
    pub fn new() -> Self {
        Self::default()
    }

    /// Power-cycle reset (`dmp105-protocol.md` §7: "No software reset code
    /// exists" — the only reset entry point is a power cycle). Restores
    /// every register to its power-on default. Does **not** clear the
    /// paper: already-printed pages are physical and a power cycle doesn't
    /// erase them on real hardware either — see [`Paper::clear`] for the
    /// (separate, explicit) tear-off action that does.
    pub fn reset(&mut self) {
        let paper = std::mem::take(&mut self.paper);
        *self = Self::default();
        self.paper = paper;
    }

    pub fn paper(&self) -> &Paper {
        &self.paper
    }

    pub fn paper_mut(&mut self) -> &mut Paper {
        &mut self.paper
    }

    /// Render one glyph at the current head position, advance `x` by one
    /// character cell, and apply the active style bits
    /// (`docs/printer-plan.md` T4): bold is a second pass one dot column to
    /// the right; elongation doubles every glyph dot column (and thus the
    /// cell advance) horizontally; underline draws a full-cell-width rule on
    /// the descender row.
    fn print_glyph(&mut self, glyph: Glyph) {
        let dot = self.pitch.dot_spacing();
        let col_step = if self.elongation { dot * 2 } else { dot };
        for (col, &bits) in glyph.iter().enumerate() {
            let cx = self.x.saturating_add(col as u32 * col_step);
            self.plot_column(cx, bits);
            if self.bold {
                self.plot_column(cx.saturating_add(dot), bits);
            }
        }
        if self.underline {
            self.draw_underline_rule(dot, CELL_DOTS * col_step);
        }
        // Saturating, like every head-position advance in this module: a
        // long-enough stream without CR would otherwise overflow (a panic in
        // debug builds), and the interpreter must never panic on arbitrary
        // input. Marks past PRINT_WIDTH_X_UNITS are dropped in mark_dot.
        self.x = self.x.saturating_add(CELL_DOTS * col_step);
    }

    /// Mark one dot, dropping anything the physical head could never reach
    /// (past the 8" print zone — see [`PRINT_WIDTH_X_UNITS`]). All ink lands
    /// through here so the clamp is uniform.
    fn mark_dot(&mut self, x: u32, y: u32) {
        if x < PRINT_WIDTH_X_UNITS {
            self.paper.mark(x, y);
        }
    }

    /// Mark one glyph column's dots: bits 0-6 are the 7-row body (row `r` =
    /// bit `r`), bit 7 is the descender-row dot.
    fn plot_column(&mut self, cx: u32, bits: u8) {
        for row in 0..7u32 {
            if bits & (1 << row) != 0 {
                self.mark_dot(cx, self.y.saturating_add(row));
            }
        }
        if bits & 0x80 != 0 {
            self.mark_dot(cx, self.y.saturating_add(DESCENDER_ROW));
        }
    }

    /// A solid rule across `width` x-units starting at the current `x`,
    /// stepped at the base (un-elongated) dot spacing so the line stays
    /// solid even when printing elongated text.
    fn draw_underline_rule(&mut self, step: u32, width: u32) {
        let mut cx = self.x;
        let end = self.x.saturating_add(width).min(PRINT_WIDTH_X_UNITS);
        let row = self.y.saturating_add(DESCENDER_ROW);
        while cx < end {
            self.mark_dot(cx, row);
            cx += step;
        }
    }

    /// Graphics-mode data byte (`dmp105-protocol.md` §5): bit 0 = top dot
    /// (weight 1) ... bit 6 = bottom dot (weight 64), bit 7 is always the
    /// data marker (already tested by the caller), not an 8th pin.
    fn plot_graphics_byte(&mut self, b: u8) {
        let weights = b & 0x7F;
        for row in 0..7u32 {
            if weights & (1 << row) != 0 {
                self.mark_dot(self.x, self.y.saturating_add(row));
            }
        }
        self.x = self.x.saturating_add(self.graphics_pitch.dot_spacing());
    }
}

impl PrinterSink for DMP105 {
    fn write_byte(&mut self, b: u8) {
        self.feed(b);
    }
}

/// Shared handle to a live [`DMP105`]: the `CaptureSink` `Rc<RefCell<_>>`
/// pattern (`bitbanger.rs`), needed here because the frontend must be able
/// to read the accumulating paper while [`crate::bitbanger::BitBanger`] owns
/// the other half as its sink. Clone before handing one half to
/// [`crate::bitbanger::BitBanger::start_dmp105`]; keep the other to poll the
/// paper.
#[derive(Clone, Default)]
pub struct DMP105Handle(Rc<RefCell<DMP105>>);

impl DMP105Handle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Restore-path-only: rebuild a handle around an already-deserialized
    /// [`DMP105`] state (`sink_serde::SinkState::Dmp105` — see
    /// `bitbanger.rs`), wrapping it in a fresh `Rc<RefCell<_>>`
    /// (`docs/plan-save-states.md`). Unlike [`DMP105Handle::new`], this
    /// starts from real restored state rather than power-on defaults.
    pub(crate) fn from_state(state: DMP105) -> Self {
        Self(Rc::new(RefCell::new(state)))
    }

    /// How much paper has been printed on so far.
    pub fn paper_extent(&self) -> PaperExtent {
        self.0.borrow().paper.extent()
    }

    /// Every dot in the inclusive row range `y0..=y1` — see
    /// [`Paper::dots_in_range`].
    pub fn dots_in_range(&self, y0: u32, y1: u32) -> Vec<(u32, u32)> {
        self.0.borrow().paper.dots_in_range(y0, y1)
    }

    /// The row range touched since the last poll — see [`Paper::take_dirty`].
    pub fn take_dirty(&self) -> Option<(u32, u32)> {
        self.0.borrow_mut().paper.take_dirty()
    }

    /// Tear off: discard the printed paper (see [`Paper::clear`]'s doc
    /// comment on what this does and doesn't rebase).
    pub fn tear_off(&self) {
        self.0.borrow_mut().paper.clear();
    }

    /// Power-cycle the printer (see [`DMP105::reset`]): every register back
    /// to its power-on default, paper untouched.
    pub fn reset(&self) {
        self.0.borrow_mut().reset();
    }
}

impl PrinterSink for DMP105Handle {
    fn write_byte(&mut self, b: u8) {
        self.0.borrow_mut().feed(b);
    }

    /// The whole interpreter/paper state, cloned out of the shared
    /// `Rc<RefCell<_>>` — `Dmp105` is plain data (`Clone` derive), so this
    /// is a deep-but-cheap snapshot (`docs/plan-save-states.md`).
    fn snapshot(&self) -> SinkState {
        SinkState::Dmp105(self.0.borrow().clone())
    }

    fn as_dmp105(&self) -> Option<&DMP105Handle> {
        Some(self)
    }
}

#[cfg(test)]
#[path = "dmp105_test.rs"]
mod tests;
