//! Control/escape-code state machine: [`DMP105::feed`] is the entry point
//! every decoded byte flows through, dispatching per-mode
//! ([`Mode::CharacterPrint`](super::Mode)/[`Mode::Graphics`](super::Mode))
//! and assembling multi-byte escape/repeat sequences
//! (wiki `cocovm/dmp105-protocol` §3/§4). Glyph and dot rendering lives in the parent
//! `dmp105` module.

use crate::dmp105_font::{self, Glyph};
use crate::{dmp_charset, dmp_symbols};

use super::{
    DMP105, DOT_ROW_UNITS, Direction, GRAPHICS_LF_UNITS, LF_PITCH_1_6, LF_PITCH_1_8, LF_PITCH_1_12,
    Mode, NlMode, Pending, Pitch, control, esc,
};

/// Glyph for a CP-mode byte that is not a block graphic (wiki `cocovm/dmp105-protocol`
/// §6): ASCII, then the European table, else the undefined-code `X`.
fn text_glyph(byte: u8) -> Glyph {
    dmp105_font::ascii_glyph(byte)
        .or_else(|| dmp_charset::european_symbol(byte).map(dmp_symbols::symbol_glyph))
        .unwrap_or_else(dmp105_font::undefined_glyph)
}

impl DMP105 {
    /// Feed one decoded byte through the interpreter.
    pub(super) fn feed(&mut self, b: u8) {
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

    /// Character-Print mode dispatch (wiki `cocovm/dmp105-protocol` §3 and §6).
    fn dispatch_cp(&mut self, b: u8) {
        match b {
            control::NUL_IGNORED_0 | control::NUL_IGNORED_1 => {}
            control::LF | control::LF_HIGH => self.y = self.y.saturating_add(self.lf_pitch_units),
            control::CR | control::CR_HIGH => self.control_cr(),
            control::END_UNDERLINE => self.underline = false,
            control::START_UNDERLINE => self.underline = true,
            control::SELECT_GRAPHICS => {
                self.graphics_pitch = self.pitch;
                self.mode = Mode::Graphics;
            }
            control::END_GRAPHICS => {} // already CP mode: ignored
            _ => match dmp_charset::block_glyph(b) {
                Some(block) => self.print_block(block),
                None => self.print_glyph(text_glyph(b)),
            },
        }
    }

    /// Graphics mode dispatch (wiki `cocovm/dmp105-protocol` §5): bit 7 set is always
    /// data; bit 7 clear is a recognized control code or else ignored (never
    /// printed — no `X`-glyph fallback in Graphics mode).
    fn dispatch_graphics(&mut self, b: u8) {
        if b & 0x80 != 0 {
            self.plot_graphics_byte(b);
            return;
        }
        match b {
            control::LF => self.y = self.y.saturating_add(GRAPHICS_LF_UNITS),
            control::CR => self.control_cr(),
            control::END_GRAPHICS => self.mode = Mode::CharacterPrint,
            _ => {} // undefined / not applicable inside Graphics: ignored
        }
    }

    /// Manual p.39: CR uses the fixed graphics feed in graphics mode;
    /// otherwise it uses the latched text pitch.
    fn control_cr(&mut self) {
        self.x = 0;
        if self.nl_mode == NlMode::CrLf {
            let feed = match self.mode {
                Mode::CharacterPrint => self.lf_pitch_units,
                Mode::Graphics => GRAPHICS_LF_UNITS,
            };
            self.y = self.y.saturating_add(feed);
        }
    }

    /// The pitch governing dot spacing right now: the live setting in CP
    /// mode, or the density latched at Graphics-mode entry.
    fn active_pitch(&self) -> Pitch {
        match self.mode {
            Mode::CharacterPrint => self.pitch,
            Mode::Graphics => self.graphics_pitch,
        }
    }

    /// Second byte of an escape sequence: figure out how many operand bytes
    /// it takes and either execute immediately or start collecting them.
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

    /// Execute a fully-assembled escape sequence. An unrecognized selector is
    /// silently ignored rather than inventing undocumented behavior.
    fn execute_esc(&mut self, selector: u8, ops: &[u8]) {
        // Appendix A pp.39–40: these are the only escapes active in graphics.
        if self.mode == Mode::Graphics
            && !matches!(
                selector,
                esc::ELONGATE_START | esc::ELONGATE_END | esc::POSITION | esc::FEED_IMMEDIATE
            )
        {
            return;
        }
        match selector {
            esc::ELONGATE_START => self.elongation = true,
            esc::ELONGATE_END => self.elongation = false,
            esc::POSITION => {
                self.position_head(u16::from_be_bytes([ops[0], ops[1]]));
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
            // Immediate feed applies in both modes, unlike the latched-only 5B case that follows.
            esc::FEED_IMMEDIATE => {
                self.y = self.y.saturating_add(u32::from(ops[0]) * DOT_ROW_UNITS)
            }
            // Latched feed is CP-mode only; in Graphics mode it falls to the
            // catch-all, consumed but inert.
            esc::FEED_LATCH if self.mode == Mode::CharacterPrint => {
                self.lf_pitch_units = u32::from(ops[0]) * DOT_ROW_UNITS;
            }
            _ => {}
        }
    }

    fn position_head(&mut self, column: u16) {
        let spacing = self.active_pitch().addressable_spacing();
        let position = u32::from(column) * spacing;
        if position < super::PRINT_WIDTH_X_UNITS {
            self.x = position;
        } else if self.mode == Mode::Graphics && position == super::PRINT_WIDTH_X_UNITS {
            // Manual p.34: POS 800 at condensed pitch wraps to the next line.
            self.x = 0;
            self.y = self.y.saturating_add(GRAPHICS_LF_UNITS);
        }
        // Other out-of-range positions are ignored (manual p.27).
    }

    /// Execute `28 n c` / `1C n c`: repeat `c` `n` times (in Graphics mode,
    /// only if `c`'s MSB is set). Dispatches `c` through the per-mode
    /// handlers directly, never through [`DMP105::feed`] — recursing there
    /// would let `1C 1C 1C` rebuild its own spawning state unboundedly.
    fn execute_repeat(&mut self, n: u8, c: u8) {
        if self.mode == Mode::Graphics && c & 0x80 == 0 {
            return;
        }
        for _ in 0..n {
            match self.mode {
                Mode::CharacterPrint if c < 0x20 => {
                    self.print_glyph(dmp105_font::undefined_glyph());
                }
                Mode::CharacterPrint => self.dispatch_cp(c),
                Mode::Graphics => self.dispatch_graphics(c),
            }
        }
    }
}
