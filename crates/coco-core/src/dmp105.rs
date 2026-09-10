//! Tandy DMP-105 dot-matrix printer interpreter: byte stream in (as decoded
//! by [`crate::bitbanger::BitBanger`]), abstract dot-raster paper out
//! (`crate::printer::Paper`). Every hardware fact cited here is sourced from
//! `docs/dmp105-protocol.md`; only entries that document marks VERIFIED are
//! implemented. See that document's own INFERRED/UNVERIFIABLE flags for what
//! remains out of scope, such as exact BUSY assertion granularity and the
//! European character set's per-code glyph mapping.
//!
//! This module provides the DMP-family paper model and the DMP-105-specific
//! control-code dialect. A DMP-130/Epson dialect (V3) can share the paper
//! without inheriting DMP-105-only parsing.
//!
//! # Position accounting
//!
//! `x` is in [`crate::printer::X_UNITS_PER_INCH`] units, `y` in
//! [`crate::printer::Y_UNITS_PER_INCH`] units — both exact fixed-point
//! integers, never floats (see `printer.rs`'s doc comment for why those
//! particular denominators were chosen).
//!
//! Graphics line feed follows the explicit 7/72-inch command definition
//! on manual pp.25 and 39. Appendix D p.51 gives an incompatible repeated-
//! feed ratio; no available evidence establishes the inferred 22/216-inch
//! alternative. See `docs/dmp105-protocol.md` for the source conflict.

use serde::{Deserialize, Serialize};

use crate::bitbanger::PrinterSink;
use crate::dmp_charset::{BLOCK_SIZE, BlockGlyph};
use crate::dmp105_font::Glyph;
use crate::printer::{Paper, X_UNITS_PER_INCH, Y_UNITS_PER_INCH};

mod protocol;

/// Character cell width in dots at every pitch: 9 glyph + 3 gap
/// (`dmp105-protocol.md` §1, Appendix G p.59: "dots/char = 12").
const CELL_DOTS: u32 = 12;

/// Block graphics spread their 6 dot columns over the whole 12-dot cell so
/// adjoining cells join (`dmp105-protocol.md` §6); each column fills both
/// positions so areas print solid, as bold text already does.
const BLOCK_DOT_STEP: usize = CELL_DOTS as usize / BLOCK_SIZE;

/// Row offset (in dot rows) of the descender row below the 7-dot glyph body
/// (`dmp105-protocol.md` §1/§6).
const DOT_ROW_UNITS: u32 = Y_UNITS_PER_INCH / 72;
const DESCENDER_ROW: u32 = 7 * DOT_ROW_UNITS;

/// Fixed graphics-mode line feed: 7/72" (`dmp105-protocol.md` §5).
const GRAPHICS_LF_UNITS: u32 = 7 * DOT_ROW_UNITS;

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
    pub const LF_HIGH: u8 = 0x8A;
    pub const CR_HIGH: u8 = 0x8D;
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

/// Escape-sequence selector bytes, that is, the byte immediately after `ESC`
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
/// exists to model direction against, so it never affects output. This matches
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
    /// `dots_per_line() / 8`; self-consistent with each pitch's named CPI.
    const fn dots_per_inch(self) -> u32 {
        self.dots_per_line() / 8
    }

    /// Physical spacing between two adjacent dots at this pitch, in
    /// [`X_UNITS_PER_INCH`] units — exact by construction (see that
    /// constant's doc comment).
    const fn dot_spacing(self) -> u32 {
        X_UNITS_PER_INCH / self.dots_per_inch()
    }

    /// Manual pp.30/33: host positioning and graphics address every other
    /// text dot (480/576/800 columns across the 8-inch print area).
    const fn addressable_spacing(self) -> u32 {
        2 * self.dot_spacing()
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
    /// Character pitch; ignored pitch commands in graphics do not change it.
    pitch: Pitch,
    /// Density selected before graphics entry (manual p.33).
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
    /// Power-on defaults (`dmp105-protocol.md` §7): Normal 10 CPI, LF pitch
    /// 1/6", and NL mode CR+LF. Head position (0, 0) is this implementation's
    /// choice because the manual gives no numeric value.
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

    /// Power-cycle reset (`dmp105-protocol.md` §7: the only reset entry
    /// point). Restores registers to power-on defaults; does **not** clear
    /// the paper — see [`Paper::clear`].
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

    /// Render one text glyph at the current head position.
    fn print_glyph(&mut self, glyph: Glyph) {
        self.print_cell(&glyph);
    }

    /// Render one block-graphic cell at the current head position.
    fn print_block(&mut self, block: BlockGlyph) {
        let mut cell = [0u8; CELL_DOTS as usize];
        for (positions, &bits) in cell.chunks_mut(BLOCK_DOT_STEP).zip(block.iter()) {
            positions.fill(bits);
        }
        self.print_cell(&cell);
    }

    /// Plot `columns` (one byte per dot position) from the current head
    /// position, advance `x` by one cell, and apply bold/elongation/underline.
    fn print_cell(&mut self, columns: &[u8]) {
        let dot = self.pitch.dot_spacing();
        let col_step = if self.elongation { dot * 2 } else { dot };
        for (col, &bits) in columns.iter().enumerate() {
            let cx = self.x.saturating_add(col as u32 * col_step);
            self.plot_column(cx, bits);
            if self.bold {
                self.plot_column(cx.saturating_add(dot), bits);
            }
        }
        if self.underline {
            self.draw_underline_rule(dot, CELL_DOTS * col_step);
        }
        // Saturating: the interpreter must never panic on a CR-less stream of arbitrary length.
        self.x = self.x.saturating_add(CELL_DOTS * col_step);
    }

    /// Mark one dot, dropping anything past the physical print zone (see
    /// [`PRINT_WIDTH_X_UNITS`]) — all ink lands through here so the clamp is uniform.
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
                self.mark_dot(cx, self.y.saturating_add(row * DOT_ROW_UNITS));
            }
        }
        if bits & 0x80 != 0 {
            self.mark_dot(cx, self.y.saturating_add(DESCENDER_ROW));
        }
    }

    /// A solid rule across `width` x-units from the current `x`, stepped at
    /// the base (un-elongated) dot spacing so it stays solid under elongated text.
    fn draw_underline_rule(&mut self, step: u32, width: u32) {
        let mut cx = self.x;
        let end = self.x.saturating_add(width).min(PRINT_WIDTH_X_UNITS);
        let row = self.y.saturating_add(DESCENDER_ROW);
        while cx < end {
            self.mark_dot(cx, row);
            cx += step;
        }
    }

    /// Graphics-mode data byte (`dmp105-protocol.md` §5): bits 0-6 are dot
    /// rows top-to-bottom; bit 7 is the data marker, not an 8th pin.
    fn plot_graphics_byte(&mut self, b: u8) {
        let weights = b & 0x7F;
        for row in 0..7u32 {
            if weights & (1 << row) != 0 {
                self.mark_dot(self.x, self.y.saturating_add(row * DOT_ROW_UNITS));
            }
        }
        self.x = self
            .x
            .saturating_add(self.graphics_pitch.addressable_spacing());
    }
}

impl PrinterSink for DMP105 {
    fn write_byte(&mut self, b: u8) {
        self.feed(b);
    }
}

/// Default DMP-105 handle; shared paper access also supports the DMP-130.
pub type DMP105Handle = crate::dmp::DmpHandle;

#[cfg(test)]
#[path = "dmp105_test.rs"]
mod tests;

#[cfg(test)]
#[path = "dmp105_graphics_test.rs"]
mod graphics_tests;
