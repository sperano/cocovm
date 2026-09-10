//! Font impressions are an explicit artistic approximation, not ROM glyphs.
//! Metrics follow manual pp. 31–44, 53, 59–63; see the protocol document.
use super::*;
use crate::dmp_charset::{BLOCK_SIZE, BlockGlyph};
use crate::dmp105_font::Glyph;
const NORMAL_DOTS: u32 = 960;
const ELITE_DOTS: u32 = 1152;
const CONDENSED_DOTS: u32 = 1918;
const NORMAL_CELL: u32 = 12;
const CONDENSED_CELL: u32 = 14;
const PIN_PITCH: u32 = Y_UNITS_PER_INCH / 72;
const GLYPH_COLUMNS: u32 = 9;
const CONDENSED_COLUMNS: u32 = 11;
const TANDY_PINS: u32 = 7;
const IBM_PINS: u32 = 8;
const GLYPH_ROWS: u32 = 8;
const BODY_ROWS: u32 = 9;
const NLQ_ROWS: u32 = 18;
const NLQ_COLUMNS: u32 = 19;
/// Dot positions per block-graphic column (12 positions over 6 columns).
const BLOCK_STRIKES: u64 = 2;
impl DMP130 {
    pub(super) fn active_pitch(&self) -> Pitch {
        if self.grammar == Grammar::Tandy {
            self.style.pitch
        } else if self.style.pitch == Pitch::Elite {
            Pitch::Elite
        } else if self.style.proportional || self.style.bold {
            Pitch::Pica
        } else if self.style.condensed {
            Pitch::Condensed
        } else {
            self.style.pitch
        }
    }
    pub(super) fn nlq_density(&self) -> bool {
        (self.style.nlq || self.style.proportional) && self.active_pitch() != Pitch::Condensed
    }
    pub(super) fn nlq_active(&self) -> bool {
        self.nlq_density() && self.style.script.is_none()
    }
    pub(super) fn dots_per_line(&self) -> u32 {
        let dots = match self.active_pitch() {
            Pitch::Pica => NORMAL_DOTS,
            Pitch::Elite => ELITE_DOTS,
            Pitch::Condensed => CONDENSED_DOTS,
        };
        if self.nlq_density() { dots * 2 } else { dots }
    }
    pub(super) fn dot_step(&self) -> u64 {
        PRINT_WIDTH / u64::from(self.dots_per_line())
    }
    pub(super) fn width_multiplier(&self) -> u64 {
        if self.style.wide || self.style.transient_wide {
            DOUBLE_WIDTH
        } else {
            1
        }
    }
    pub(super) fn cell_width(&self) -> u64 {
        let dots = if self.active_pitch() == Pitch::Condensed {
            CONDENSED_CELL
        } else if self.nlq_density() {
            NORMAL_CELL * 2
        } else {
            NORMAL_CELL
        };
        self.dot_step() * u64::from(dots) * self.width_multiplier()
    }
    pub(super) fn character_width(&self, byte: u8) -> u64 {
        if !self.style.proportional || self.active_pitch() == Pitch::Elite {
            return self.cell_width();
        }
        const FIRST_ASCII: u8 = 0x20;
        const LAST_ASCII: u8 = 0x7e;
        let columns = if (FIRST_ASCII..=LAST_ASCII).contains(&byte) {
            let widths = if self.nlq_density() {
                &widths::CORRESPONDENCE_ASCII_WIDTHS
            } else {
                &widths::STANDARD_ASCII_WIDTHS
            };
            u64::from(widths[usize::from(byte - FIRST_ASCII)])
        } else {
            return self.cell_width();
        };
        columns * self.dot_step() * self.width_multiplier()
    }
    pub(super) fn glyph(&self, byte: u8) -> Glyph {
        font::glyph(self.charset, self.country, byte)
    }
    pub(super) fn print_character(&mut self, byte: u8) {
        let width = self.character_width(byte);
        if self.x.saturating_add(width) > self.right {
            self.wrap_line();
        }
        match font::block(self.charset, byte) {
            Some(block) => self.block_dots(block, width),
            None => self.text_dots(self.glyph(byte)),
        }
        if self.style.underline {
            self.underline_to(self.x.saturating_add(width));
        }
        self.x = self.x.saturating_add(width);
        if self.x >= self.right {
            self.carriage_return();
        }
    }
    /// Six dot columns spread evenly over the whole cell so neighbouring
    /// blocks join at every pitch, each struck twice so areas print solid
    /// (p. 34: 6-dot-high graphics, half line feed for diagrams).
    fn block_dots(&mut self, block: BlockGlyph, width: u64) {
        let positions = BLOCK_SIZE as u64 * BLOCK_STRIKES;
        for (column, bits) in block.iter().enumerate() {
            for row in 0..BLOCK_SIZE {
                if bits & (1 << row) != 0 {
                    let y = self.y.saturating_add(row as u32 * PIN_PITCH);
                    for strike in 0..BLOCK_STRIKES {
                        let position = column as u64 * BLOCK_STRIKES + strike;
                        let x = self.x.saturating_add(width * position / positions);
                        self.styled_dot(x, y, false);
                    }
                }
            }
        }
    }
    fn text_dots(&mut self, glyph: Glyph) {
        let columns = if self.nlq_density() {
            NLQ_COLUMNS
        } else if self.active_pitch() == Pitch::Condensed {
            CONDENSED_COLUMNS
        } else {
            GLYPH_COLUMNS
        };
        let rows = if self.nlq_active() {
            NLQ_ROWS
        } else {
            BODY_ROWS
        };
        for column in 0..columns {
            let bits = glyph[(column * GLYPH_COLUMNS / columns) as usize];
            for row in 0..rows {
                if bits & (1 << (row * GLYPH_ROWS / rows)) != 0 {
                    self.text_dot(column, row, rows);
                }
            }
        }
    }
    pub(super) fn text_dot(&mut self, column: u32, row: u32, rows: u32) {
        let step = self.dot_step();
        let half = self.style.script.is_some();
        let row_pitch = if rows == NLQ_ROWS {
            PIN_PITCH / 2
        } else {
            PIN_PITCH
        };
        let mut vertical = row * row_pitch;
        if half {
            vertical /= 2;
        }
        if self.style.script == Some(Script::Sub) {
            vertical += BODY_ROWS * PIN_PITCH / 2;
        }
        let italic = if self.style.italic && !self.style.micro {
            (rows - row - 1) / 3
        } else {
            0
        };
        let x = self
            .x
            .saturating_add(u64::from(column + italic) * step * self.width_multiplier());
        let y = self.y.saturating_add(vertical);
        self.styled_dot(x, y, half);
    }
    /// One dot plus its bold and double-strike companions; `half` marks
    /// half-height script text, which gets neither.
    fn styled_dot(&mut self, x: u64, y: u32, half: bool) {
        self.buffer_dot(x, y);
        if self.style.bold && self.active_pitch() != Pitch::Condensed && !half {
            self.buffer_dot(x.saturating_add(self.dot_step()), y);
        }
        if self.style.double_strike && !self.nlq_active() && !half {
            self.buffer_dot(x, y.saturating_add(PIN_PITCH / 2));
        }
    }
    pub(super) fn buffer_dot(&mut self, x: u64, y: u32) {
        if x < self.right && x < PRINT_WIDTH {
            self.buffered.push(((x / X_FRACTION) as u32, y));
        }
    }
    pub(super) fn underline_to(&mut self, end: u64) {
        let mut x = self.x;
        let end = end.min(self.right);
        let y = self.y.saturating_add(BODY_ROWS * PIN_PITCH);
        while x < end {
            self.buffer_dot(x, y);
            x = x.saturating_add(self.dot_step());
        }
    }
    pub(super) fn tandy_graphics(&mut self, byte: u8) {
        let width = if self.graphics_wide { DOUBLE_WIDTH } else { 1 };
        let step = PRINT_WIDTH / u64::from(GRAPHICS_COLUMNS) * width;
        if self.x >= PRINT_WIDTH {
            self.wrap_line();
        }
        for row in 0..TANDY_PINS {
            if byte & (1 << row) != 0 {
                self.paper.mark(
                    (self.x / X_FRACTION) as u32,
                    self.y.saturating_add(row * PIN_PITCH),
                );
            }
        }
        self.x = self.x.saturating_add(step);
    }
    pub(super) fn ibm_graphics(&mut self, byte: u8, step: u64) {
        // Count-delimited payload remains data even beyond the printable zone.
        if self.x < self.right {
            for row in 0..IBM_PINS {
                if byte & (codes::control::HIGH_BIT >> row) != 0 {
                    self.paper.mark(
                        (self.x / X_FRACTION) as u32,
                        self.y.saturating_add(row * PIN_PITCH),
                    );
                }
            }
        }
        self.x = self.x.saturating_add(step);
    }
}
