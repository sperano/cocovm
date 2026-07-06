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

use crate::bitbanger::PrinterSink;
use crate::dmp105_font::{self, Glyph};
use crate::printer::{Paper, PaperExtent, X_UNITS_PER_INCH, Y_UNITS_PER_INCH};

/// Character cell width in dots at every pitch: 9 glyph + 3 gap
/// (`dmp105-protocol.md` §1, Appendix G p.59: "dots/char = 12").
const CELL_DOTS: u32 = 12;

/// Row offset (in dot rows) of the descender row below the 7-dot glyph body
/// (`dmp105-protocol.md` §1/§6).
const DESCENDER_ROW: u32 = 7;

/// Fixed graphics-mode line feed: 7/72" (`dmp105-protocol.md` §5).
const GRAPHICS_LF_UNITS: u32 = 7;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    CharacterPrint,
    Graphics,
}

/// New-line mode selected by `1B 15`/`1B 16` (`dmp105-protocol.md` §4 T11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NlMode {
    CrOnly,
    CrLf,
}

/// Carriage direction selected by `1B 55 00/01`
/// (`dmp105-protocol.md` §1/§4 T16). Stored only: no physical print head
/// exists to model direction against, so it never affects output — matching
/// the plan's "unidirectional/bidirectional affects nothing in emulation"
/// direction for this task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Bidirectional,
    Unidirectional,
}

/// Print pitch (`dmp105-protocol.md` §1 Appendix G p.59). Character cell
/// width is always [`CELL_DOTS`] dots regardless of pitch; pitch instead
/// changes how many dots (thus inches) that fixed-width cell spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone)]
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
#[derive(Debug)]
pub struct Dmp105 {
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

impl Default for Dmp105 {
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

impl Dmp105 {
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

    /// Feed one decoded byte through the interpreter.
    fn feed(&mut self, b: u8) {
        match std::mem::replace(&mut self.pending, Pending::None) {
            Pending::None => self.dispatch_fresh(b),
            Pending::Esc => self.begin_esc_operands(b),
            Pending::EscOperands {
                selector,
                mut operands,
                need,
            } => {
                operands.push(b);
                if operands.len() == need {
                    self.execute_esc(selector, &operands);
                } else {
                    self.pending = Pending::EscOperands {
                        selector,
                        operands,
                        need,
                    };
                }
            }
            Pending::Repeat1 => self.pending = Pending::Repeat2 { n: b },
            Pending::Repeat2 { n } => self.execute_repeat(n, b),
        }
    }

    /// A byte arriving with no escape/repeat sequence in progress: either
    /// starts one, or dispatches immediately per the current mode.
    fn dispatch_fresh(&mut self, b: u8) {
        match b {
            control::ESC => self.pending = Pending::Esc,
            control::REPEAT => self.pending = Pending::Repeat1,
            _ => match self.mode {
                Mode::CharacterPrint => self.dispatch_cp(b),
                Mode::Graphics => self.dispatch_graphics(b),
            },
        }
    }

    /// Character-Print mode dispatch (`dmp105-protocol.md` §3 and §6).
    fn dispatch_cp(&mut self, b: u8) {
        match b {
            control::NUL_IGNORED_0 | control::NUL_IGNORED_1 => {}
            control::LF => self.y += self.lf_pitch_units,
            control::CR => self.control_cr(),
            control::END_UNDERLINE => self.underline = false,
            control::START_UNDERLINE => self.underline = true,
            control::SELECT_GRAPHICS => {
                self.graphics_pitch = self.pitch;
                self.mode = Mode::Graphics;
            }
            control::END_GRAPHICS => {} // already CP mode: ignored
            0x20..=0x7E => self.print_glyph(dmp105_font::ascii_glyph(b).expect("in range")),
            0xA0..=0xBF => self
                .print_glyph(dmp105_font::european_glyph(b).expect("in range, TODO placeholder")),
            0xE0..=0xFE => self.print_glyph(
                dmp105_font::block_glyph(b).unwrap_or_else(dmp105_font::undefined_glyph),
            ),
            _ => self.print_glyph(dmp105_font::undefined_glyph()),
        }
    }

    /// Graphics mode dispatch (`dmp105-protocol.md` §5): bit 7 set is always
    /// data (the marker bit), never a control code; bit 7 clear is a control
    /// code if recognized, otherwise ignored (never printed — Graphics mode
    /// has no `X`-glyph fallback, per §3's "ignored in Graphics").
    fn dispatch_graphics(&mut self, b: u8) {
        if b & 0x80 != 0 {
            self.plot_graphics_byte(b);
            return;
        }
        match b {
            control::LF => self.y += GRAPHICS_LF_UNITS,
            control::CR => self.control_cr(),
            control::END_GRAPHICS => self.mode = Mode::CharacterPrint,
            _ => {} // undefined / not applicable inside Graphics: ignored
        }
    }

    /// `CR` behavior is identical in both modes (`dmp105-protocol.md` §3:
    /// "0D same"): return to column 0, and additionally feed a line at the
    /// latched (text) LF pitch if NL mode is CR+LF.
    fn control_cr(&mut self) {
        self.x = 0;
        if self.nl_mode == NlMode::CrLf {
            self.y += self.lf_pitch_units;
        }
    }

    /// The pitch that governs dot spacing for whatever is being addressed
    /// right now: the live setting in CP mode, or the density latched at
    /// Graphics-mode entry while inside a graphics run.
    fn active_pitch(&self) -> Pitch {
        match self.mode {
            Mode::CharacterPrint => self.pitch,
            Mode::Graphics => self.graphics_pitch,
        }
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
            let cx = self.x + col as u32 * col_step;
            self.plot_column(cx, bits);
            if self.bold {
                self.plot_column(cx + dot, bits);
            }
        }
        if self.underline {
            self.draw_underline_rule(dot, CELL_DOTS * col_step);
        }
        self.x += CELL_DOTS * col_step;
    }

    /// Mark one glyph column's dots: bits 0-6 are the 7-row body (row `r` =
    /// bit `r`), bit 7 is the descender-row dot.
    fn plot_column(&mut self, cx: u32, bits: u8) {
        for row in 0..7u32 {
            if bits & (1 << row) != 0 {
                self.paper.mark(cx, self.y + row);
            }
        }
        if bits & 0x80 != 0 {
            self.paper.mark(cx, self.y + DESCENDER_ROW);
        }
    }

    /// A solid rule across `width` x-units starting at the current `x`,
    /// stepped at the base (un-elongated) dot spacing so the line stays
    /// solid even when printing elongated text.
    fn draw_underline_rule(&mut self, step: u32, width: u32) {
        let mut cx = self.x;
        let end = self.x + width;
        while cx < end {
            self.paper.mark(cx, self.y + DESCENDER_ROW);
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
                self.paper.mark(self.x, self.y + row);
            }
        }
        self.x += self.graphics_pitch.dot_spacing();
    }

    /// Second byte of an escape sequence just arrived: figure out how many
    /// operand bytes it takes (`dmp105-protocol.md` §4's table — 0, 1, or 2
    /// more bytes) and either execute immediately or start collecting them.
    fn begin_esc_operands(&mut self, selector: u8) {
        let need = match selector {
            esc::POSITION => 2,
            esc::DIRECTION | esc::FEED_IMMEDIATE | esc::FEED_LATCH => 1,
            _ => 0,
        };
        if need == 0 {
            self.execute_esc(selector, &[]);
        } else {
            self.pending = Pending::EscOperands {
                selector,
                operands: Vec::new(),
                need,
            };
        }
    }

    /// Execute a fully-assembled escape sequence. An unrecognized selector
    /// (the manual states "No ... other sequences" exist) is silently
    /// ignored: behavior for a truly out-of-spec byte here isn't documented,
    /// and a no-op is the safe choice rather than inventing one.
    fn execute_esc(&mut self, selector: u8, ops: &[u8]) {
        match selector {
            esc::ELONGATE_START => self.elongation = true,
            esc::ELONGATE_END => self.elongation = false,
            esc::POSITION => {
                let column = u32::from(ops[0]) * 256 + u32::from(ops[1]);
                self.x = column * self.active_pitch().dot_spacing();
            }
            esc::PITCH_NORMAL => self.pitch = Pitch::Normal,
            esc::PITCH_CONDENSED => self.pitch = Pitch::Condensed,
            esc::NL_CR_ONLY => self.nl_mode = NlMode::CrOnly,
            esc::NL_CR_LF => self.nl_mode = NlMode::CrLf,
            esc::PITCH_COMPRESSED => self.pitch = Pitch::Compressed,
            esc::LF_PITCH_1_12 => self.lf_pitch_units = LF_PITCH_1_12,
            esc::BOLD_START => self.bold = true,
            esc::BOLD_END => self.bold = false,
            esc::LF_PITCH_1_6 => self.lf_pitch_units = LF_PITCH_1_6,
            esc::LF_PITCH_1_8 => self.lf_pitch_units = LF_PITCH_1_8,
            esc::DIRECTION => {
                self.direction = if ops[0] == 1 {
                    Direction::Unidirectional
                } else {
                    Direction::Bidirectional
                };
            }
            // Immediate feed: applies in both modes (`dmp105-protocol.md`
            // §4's "1B 5A n" row), unlike the latched-only 5B below.
            esc::FEED_IMMEDIATE => self.y += u32::from(ops[0]),
            // Latched-only feed: CP mode only per the spec table; while in
            // Graphics mode the byte is still consumed (escape parsing is
            // mode-independent) but has no effect, matching how pitch
            // selection is likewise inert during an active graphics run.
            esc::FEED_LATCH => {
                if self.mode == Mode::CharacterPrint {
                    self.lf_pitch_units = u32::from(ops[0]);
                }
            }
            _ => {}
        }
    }

    /// Execute `28 n c` / `1C n c`: feed `c` through the interpreter `n`
    /// times, as if it had arrived `n` separate times
    /// (`dmp105-protocol.md` §3). In Graphics mode, only honored if `c` has
    /// its MSB set (i.e. is valid graphics data) — the spec table's explicit
    /// restriction for that mode.
    fn execute_repeat(&mut self, n: u8, c: u8) {
        if self.mode == Mode::Graphics && c & 0x80 == 0 {
            return;
        }
        for _ in 0..n {
            self.feed(c);
        }
    }
}

impl PrinterSink for Dmp105 {
    fn write_byte(&mut self, b: u8) {
        self.feed(b);
    }
}

/// Shared handle to a live [`Dmp105`]: the `CaptureSink` `Rc<RefCell<_>>`
/// pattern (`bitbanger.rs`), needed here because the frontend must be able
/// to read the accumulating paper while [`crate::bitbanger::BitBanger`] owns
/// the other half as its sink. Clone before handing one half to
/// [`crate::bitbanger::BitBanger::start_dmp105`]; keep the other to poll the
/// paper.
#[derive(Clone, Default)]
pub struct Dmp105Handle(Rc<RefCell<Dmp105>>);

impl Dmp105Handle {
    pub fn new() -> Self {
        Self::default()
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

    /// Power-cycle the printer (see [`Dmp105::reset`]): every register back
    /// to its power-on default, paper untouched.
    pub fn reset(&self) {
        self.0.borrow_mut().reset();
    }
}

impl PrinterSink for Dmp105Handle {
    fn write_byte(&mut self, b: u8) {
        self.0.borrow_mut().feed(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render `paper`'s dots in `[y0, y1)` as a compact debug string: one
    /// line per row, `#`/`.` per dot column between `x0` and `x1` — a small
    /// helper for debugging test failures, not a fixture format tests
    /// assert against (per `docs/printer-plan.md` T4's "compact expected-
    /// pattern representations ... not giant ASCII-art fixtures").
    #[allow(dead_code)]
    fn ascii_art(paper: &Paper, x0: u32, x1: u32, y0: u32, y1: u32, step: u32) -> String {
        let mut out = String::new();
        for y in (y0..y1).step_by(step as usize) {
            let dots = paper.dots_in_range(y, y);
            let mut x = x0;
            while x < x1 {
                out.push(if dots.iter().any(|&(dx, _)| dx == x) {
                    '#'
                } else {
                    '.'
                });
                x += step;
            }
            out.push('\n');
        }
        out
    }

    fn feed_str(dmp: &mut Dmp105, bytes: &[u8]) {
        for &b in bytes {
            dmp.feed(b);
        }
    }

    /// A blank glyph advances `x` (the 12-dot cell) without marking any
    /// dots, so column position between characters can be checked purely
    /// from `dot_spacing() * CELL_DOTS`.
    fn normal_cell_width() -> u32 {
        CELL_DOTS * Pitch::Normal.dot_spacing()
    }

    #[test]
    fn hello_cr_at_normal_pitch_produces_expected_glyph_columns_and_row() {
        let mut dmp = Dmp105::new();
        feed_str(&mut dmp, b"HELLO\r");

        // 'H' is the widened uppercase H: a solid vertical stroke at column 1
        // (all 7 body rows, since the crossbar's row also coincides with the
        // already-solid stroke) — check it lands exactly at x=0 (first cell)
        // and y=0 (default head position).
        let cell = normal_cell_width();
        let col1_dots = dmp.paper.dots_in_range(0, DESCENDER_ROW);
        let h_col1: Vec<u32> = col1_dots
            .iter()
            .filter(|&&(x, _)| x == Pitch::Normal.dot_spacing())
            .map(|&(_, y)| y)
            .collect();
        assert_eq!(h_col1, vec![0, 1, 2, 3, 4, 5, 6], "H's left stroke column");

        // Five glyph cells wide (H,E,L,L,O), so the next cell after 'O'
        // starts at x = 5 * cell width — spot check that some dot exists in
        // O's cell (columns 4*cell..5*cell) and none at exactly 5*cell.
        let fifth_cell_dots = dmp.paper.dots_in_range(0, DESCENDER_ROW);
        assert!(
            fifth_cell_dots
                .iter()
                .any(|&(x, _)| (4 * cell..5 * cell).contains(&x)),
            "expected some ink within O's cell"
        );
        assert!(
            !fifth_cell_dots.iter().any(|&(x, _)| x == 5 * cell + 1),
            "no ink expected one dot past the fifth cell"
        );

        // CR (default NL mode CR+LF) returns x to 0 and feeds one line at
        // the default 1/6" pitch (12 y-units).
        assert_eq!(dmp.x, 0);
        assert_eq!(dmp.y, LF_PITCH_1_6);
    }

    #[test]
    fn cr_only_mode_does_not_advance_y() {
        let mut dmp = Dmp105::new();
        feed_str(&mut dmp, b"\x1B\x15"); // ESC 15: CR = CR only
        feed_str(&mut dmp, b"A\r");
        assert_eq!(dmp.x, 0);
        assert_eq!(dmp.y, 0, "CR-only mode must not feed a line");
    }

    #[test]
    fn cr_lf_mode_advances_y_by_latched_pitch() {
        let mut dmp = Dmp105::new();
        feed_str(&mut dmp, b"\x1B\x16"); // ESC 16: CR = CR+LF (also the default)
        feed_str(&mut dmp, b"A\r");
        assert_eq!(dmp.y, LF_PITCH_1_6);
    }

    #[test]
    fn plain_lf_advances_y_without_touching_x() {
        let mut dmp = Dmp105::new();
        feed_str(&mut dmp, b"AB\n");
        assert_eq!(dmp.y, LF_PITCH_1_6);
        assert_eq!(dmp.x, 2 * normal_cell_width(), "LF alone must not reset x");
    }

    #[test]
    fn pitch_change_mid_line_changes_subsequent_cell_width() {
        let mut dmp = Dmp105::new();
        feed_str(&mut dmp, b"A"); // at Normal pitch
        let after_a = dmp.x;
        assert_eq!(after_a, normal_cell_width());

        feed_str(&mut dmp, b"\x1B\x14"); // ESC 14: Condensed 16.7 CPI
        feed_str(&mut dmp, b"B");
        let condensed_width = CELL_DOTS * Pitch::Condensed.dot_spacing();
        assert_eq!(dmp.x, after_a + condensed_width);
        assert_ne!(
            condensed_width,
            normal_cell_width(),
            "condensed cell must differ from normal cell for this test to mean anything"
        );
    }

    #[test]
    fn elongation_doubles_dot_spacing_and_cell_advance() {
        let mut normal = Dmp105::new();
        feed_str(&mut normal, b"A");
        let normal_advance = normal.x;

        let mut elongated = Dmp105::new();
        feed_str(&mut elongated, b"\x1B\x0E"); // ESC 0E: start elongation
        feed_str(&mut elongated, b"A");
        assert_eq!(elongated.x, normal_advance * 2);

        // Every glyph column of 'A' also lands twice as far apart. Find the
        // first column with a row-0 dot from the font data itself (rather
        // than hand-guessing which column that is) and check it landed at
        // twice the normal dot spacing.
        let dot = Pitch::Normal.dot_spacing();
        let glyph = dmp105_font::ascii_glyph(b'A').unwrap();
        let first_row0_col = glyph
            .iter()
            .position(|&bits| bits & 1 != 0)
            .expect("'A' must have at least one row-0 dot") as u32;
        let dots = elongated.paper.dots_in_range(0, 0);
        assert!(
            dots.iter().any(|&(x, _)| x == first_row0_col * dot * 2),
            "expected 'A's row-0 dot at twice the normal dot spacing when elongated"
        );
    }

    #[test]
    fn underline_marks_full_cell_width_on_descender_row() {
        let mut dmp = Dmp105::new();
        feed_str(&mut dmp, b"\x0F"); // start underline
        feed_str(&mut dmp, b"A");
        let dot = Pitch::Normal.dot_spacing();
        let width = CELL_DOTS * dot;
        let rule_dots: Vec<u32> = dmp
            .paper
            .dots_in_range(DESCENDER_ROW, DESCENDER_ROW)
            .into_iter()
            .map(|(x, _)| x)
            .collect();
        // A solid rule stepped at `dot` across the whole cell width means
        // x=0 and the last in-range multiple of `dot` before `width` must
        // both be present.
        assert!(rule_dots.contains(&0));
        let last_step = ((width - 1) / dot) * dot;
        assert!(rule_dots.contains(&last_step));
        feed_str(&mut dmp, b"\x0E"); // end underline
        feed_str(&mut dmp, b"B");
        // 'B' cell (second cell) must NOT get a descender-row rule now that
        // underline is off.
        let second_cell_start = CELL_DOTS * dot;
        let second_cell_end = second_cell_start + CELL_DOTS * dot;
        let rule_in_b_cell = dmp
            .paper
            .dots_in_range(DESCENDER_ROW, DESCENDER_ROW)
            .into_iter()
            .filter(|&(x, _)| (second_cell_start..second_cell_end).contains(&x))
            .count();
        assert_eq!(rule_in_b_cell, 0);
    }

    #[test]
    fn bold_strikes_every_column_twice_one_dot_over() {
        let mut plain = Dmp105::new();
        feed_str(&mut plain, b"H");
        let plain_dots = plain.paper.dots_in_range(0, DESCENDER_ROW).len();

        let mut bold = Dmp105::new();
        feed_str(&mut bold, b"\x1B\x1F"); // ESC 1F: start bold
        feed_str(&mut bold, b"H");
        let bold_dots = bold.paper.dots_in_range(0, DESCENDER_ROW).len();

        assert_eq!(
            bold_dots,
            plain_dots * 2,
            "bold must double every dot via its one-column-over second pass"
        );

        // Cell advance itself is unaffected by bold (only elongation
        // changes cell width).
        assert_eq!(bold.x, plain.x);
    }

    #[test]
    fn repeat_code_repeats_a_character_n_times_in_cp_mode() {
        let mut dmp = Dmp105::new();
        feed_str(&mut dmp, b"\x1C\x03A"); // repeat 'A' 3 times
        assert_eq!(dmp.x, 3 * normal_cell_width());
    }

    #[test]
    fn repeat_code_in_graphics_mode_requires_msb_set_on_c() {
        let mut dmp = Dmp105::new();
        dmp.feed(control::SELECT_GRAPHICS);

        // c without MSB set: per spec, not honored at all in Graphics mode.
        feed_str(&mut dmp, b"\x1C\x05\x41"); // n=5, c=0x41 (no MSB)
        assert_eq!(
            dmp.x, 0,
            "repeat with MSB-clear c must be a no-op in Graphics mode"
        );

        // c with MSB set: valid graphics data, repeated n times.
        feed_str(&mut dmp, b"\x1C\x03\xFF"); // n=3, c=0xFF (all 7 dots)
        let spacing = Pitch::Normal.dot_spacing();
        assert_eq!(dmp.x, 3 * spacing);
        assert_eq!(dmp.paper.dots_in_range(0, 6).len(), 3 * 7);
    }

    #[test]
    fn graphics_mode_enter_data_lf_and_exit_round_trip() {
        let mut dmp = Dmp105::new();
        feed_str(&mut dmp, b"A"); // one CP char first, to prove entry doesn't reset x
        let x_before_graphics = dmp.x;

        dmp.feed(control::SELECT_GRAPHICS);
        assert_eq!(dmp.mode, Mode::Graphics);
        assert_eq!(
            dmp.x, x_before_graphics,
            "entering Graphics must not reset x"
        );

        dmp.feed(0xFF); // all 7 dots
        let spacing = Pitch::Normal.dot_spacing();
        assert_eq!(dmp.x, x_before_graphics + spacing);
        let dots = dmp.paper.dots_in_range(0, 6);
        assert_eq!(
            dots.iter()
                .filter(|&&(x, _)| x == x_before_graphics)
                .count(),
            7,
            "0xFF must plot all 7 body rows"
        );

        let y_before_lf = dmp.y;
        dmp.feed(control::LF); // fixed 7/72" feed, does not touch x
        assert_eq!(dmp.y, y_before_lf + GRAPHICS_LF_UNITS);
        assert_eq!(
            dmp.x,
            x_before_graphics + spacing,
            "graphics LF must not touch x"
        );

        dmp.feed(control::END_GRAPHICS);
        assert_eq!(dmp.mode, Mode::CharacterPrint);
    }

    #[test]
    fn graphics_lf_vs_text_lf_rounding_trap_is_not_reproducible_from_given_facts() {
        // See the module doc comment: `dmp105-protocol.md` states "11
        // full-pitch LFs = 18 graphics LFs exactly", but the individually-
        // verified numbers for those two facts (12 y-units per default text
        // LF, 7 y-units per graphics LF) do not actually satisfy that
        // equality, and no other integer y-unit choice can (126 is not
        // divisible by 11). This test documents the arithmetic rather than
        // asserting a fabricated resolution.
        let full_pitch_total = 11 * LF_PITCH_1_6;
        let graphics_total = 18 * GRAPHICS_LF_UNITS;
        assert_eq!(full_pitch_total, 132);
        assert_eq!(graphics_total, 126);
        assert_ne!(
            full_pitch_total, graphics_total,
            "if this ever holds, the manual's identity has become reproducible \
             from verified facts alone -- update the module doc comment"
        );
    }

    #[test]
    fn head_positioning_sets_absolute_column_including_explicit_zero_band() {
        let mut dmp = Dmp105::new();
        // 1B 10 n1 n2, n1=0 n2=0: the worked example's "mandatory CHR$(0)"
        // case -- must be accepted as a real 2-byte operand pair, not
        // skipped because the value is zero.
        feed_str(&mut dmp, b"\x1B\x10\x00\x00");
        assert_eq!(dmp.x, 0);

        // n1=1, n2=44 -> column 300, at Normal pitch (30 x-units/dot).
        feed_str(&mut dmp, b"\x1B\x10\x01\x2C");
        let expected = (256 + 44) * Pitch::Normal.dot_spacing();
        assert_eq!(dmp.x, expected);
    }

    #[test]
    fn undefined_codes_print_the_x_glyph() {
        let mut undefined = Dmp105::new();
        undefined.feed(0x02); // undefined low control code
        let mut x_glyph = Dmp105::new();
        feed_str(&mut x_glyph, b"X");
        assert_eq!(
            undefined.paper.dots_in_range(0, DESCENDER_ROW),
            x_glyph.paper.dots_in_range(0, DESCENDER_ROW)
        );

        let mut undefined_high = Dmp105::new();
        undefined_high.feed(0x85); // undefined in $80-$9F
        assert_eq!(
            undefined_high.paper.dots_in_range(0, DESCENDER_ROW),
            x_glyph.paper.dots_in_range(0, DESCENDER_ROW)
        );
    }

    #[test]
    fn esc_5a_feeds_immediately_esc_5b_only_latches() {
        let mut immediate = Dmp105::new();
        feed_str(&mut immediate, b"\x1B\x5A\x0A"); // ESC 5A 10: feed 10/72" now
        assert_eq!(immediate.y, 10);
        // Latched pitch is untouched by the immediate feed: a later plain
        // LF still uses the default 1/6" pitch.
        immediate.feed(control::LF);
        assert_eq!(immediate.y, 10 + LF_PITCH_1_6);

        let mut latched = Dmp105::new();
        feed_str(&mut latched, b"\x1B\x5B\x0A"); // ESC 5B 10: latch only, no feed
        assert_eq!(latched.y, 0, "5B must not feed immediately");
        latched.feed(control::LF);
        assert_eq!(
            latched.y, 10,
            "a later plain LF must use the newly latched pitch"
        );
    }

    #[test]
    fn reset_restores_power_on_defaults_but_leaves_paper_alone() {
        let mut dmp = Dmp105::new();
        feed_str(&mut dmp, b"\x1B\x0EHELLO"); // elongated, some ink on the paper
        assert!(dmp.x > 0);
        assert!(!dmp.paper.dots_in_range(0, DESCENDER_ROW).is_empty());

        dmp.reset();
        assert_eq!(dmp.x, 0);
        assert_eq!(dmp.y, 0);
        assert_eq!(dmp.pitch, Pitch::Normal);
        assert_eq!(dmp.lf_pitch_units, LF_PITCH_1_6);
        assert!(!dmp.elongation);
        assert!(
            !dmp.paper.dots_in_range(0, DESCENDER_ROW).is_empty(),
            "reset must not erase already-printed paper"
        );
    }

    #[test]
    fn handle_exposes_extent_dirty_range_and_tear_off() {
        let handle = Dmp105Handle::new();
        let mut sink: Box<dyn PrinterSink> = Box::new(handle.clone());
        for &b in b"HI\r" {
            sink.write_byte(b);
        }
        let extent = handle.paper_extent();
        assert!(extent.dot_count > 0);
        assert!(handle.take_dirty().is_some());
        assert!(handle.take_dirty().is_none());

        handle.tear_off();
        assert_eq!(handle.paper_extent().dot_count, 0);
    }
}
