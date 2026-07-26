//! Control/escape-code state machine: [`Dmp105::feed`] is the entry point
//! every decoded byte flows through, dispatching per-mode
//! ([`Mode::CharacterPrint`](super::Mode)/[`Mode::Graphics`](super::Mode))
//! and assembling multi-byte escape/repeat sequences
//! (`dmp105-protocol.md` §3/§4). Glyph/dot rendering lives back in the
//! `dmp105` module itself.

use crate::dmp105_font;

use super::{
    Dmp105, Direction, GRAPHICS_LF_UNITS, LF_PITCH_1_6, LF_PITCH_1_8, LF_PITCH_1_12, Mode,
    NlMode, Pending, Pitch, control, esc,
};

impl Dmp105 {
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

    /// Character-Print mode dispatch (`dmp105-protocol.md` §3 and §6).
    fn dispatch_cp(&mut self, b: u8) {
        match b {
            control::NUL_IGNORED_0 | control::NUL_IGNORED_1 => {}
            control::LF => self.y = self.y.saturating_add(self.lf_pitch_units),
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
            control::LF => self.y = self.y.saturating_add(GRAPHICS_LF_UNITS),
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
            self.y = self.y.saturating_add(self.lf_pitch_units);
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
            esc::FEED_IMMEDIATE => self.y = self.y.saturating_add(u32::from(ops[0])),
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

    /// Execute `28 n c` / `1C n c`: repeat `c` `n` times
    /// (`dmp105-protocol.md` §3). In Graphics mode, only honored if `c` has
    /// its MSB set (i.e. is valid graphics data) — the spec table's explicit
    /// restriction for that mode.
    ///
    /// The repeated byte is expanded through the per-mode dispatchers, NOT
    /// through [`Dmp105::feed`]: inside a repeat, `c` is the datum being
    /// repeated, never a new ESC/repeat sequence introducer (the spec gives
    /// no semantics for repeating an introducer, and `dispatch_cp` already
    /// handles an out-of-band `1C`/`1B` as an undefined code printing `X`).
    /// Recursing through `feed` here would let the stream `1C 1C 1C` rebuild
    /// its own spawning state unboundedly — a stack-overflow crash on
    /// three bytes of arbitrary printer traffic.
    fn execute_repeat(&mut self, n: u8, c: u8) {
        if self.mode == Mode::Graphics && c & 0x80 == 0 {
            return;
        }
        for _ in 0..n {
            match self.mode {
                Mode::CharacterPrint => self.dispatch_cp(c),
                Mode::Graphics => self.dispatch_graphics(c),
            }
        }
    }
}
