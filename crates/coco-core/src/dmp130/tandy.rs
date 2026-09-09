//! Tandy DP, WP, and graphics grammar (Operation Manual pp. 27–64).
use super::{
    codes::{control as c, tandy as t},
    *,
};
impl DMP130 {
    pub(super) fn set_wide(&mut self, enabled: bool) {
        if self.mode == Mode::Graphics {
            self.graphics_wide = enabled;
        } else {
            self.style.wide = enabled;
        }
    }

    pub(super) fn tandy_byte(&mut self, byte: u8) {
        if self.mode == Mode::Graphics && byte & c::HIGH_BIT != 0 {
            self.tandy_graphics(byte);
            return;
        }
        match byte {
            0 | 1 | c::BELL | c::DELETE | u8::MAX => {}
            c::REPEAT => self.pending = Pending::RepeatCount,
            c::LF | c::HIGH_LF => self.line_feed(),
            c::CR | c::HIGH_CR => self.carriage_return(),
            c::FF => self.form_feed(),
            c::END_GRAPHICS if self.mode == Mode::Graphics => {
                self.flush();
                self.mode = self.text_mode;
            }
            _ if self.mode == Mode::Graphics => {}
            c::BS => self.pending = Pending::Backspace,
            c::SO => self.style.underline = false,
            c::SI => self.style.underline = true,
            c::DC2 => {
                self.flush();
                self.text_mode = self.mode;
                self.graphics_wide = self.style.wide;
                self.mode = Mode::Graphics;
            }
            c::DP => {
                self.flush();
                self.mode = Mode::DataProcessing;
            }
            c::DC4 => {
                self.flush();
                self.mode = Mode::WordProcessing;
            }
            c::END_GRAPHICS => {}
            _ => self.print_character(byte),
        }
    }
    pub(super) fn tandy_escape(&mut self, selector: u8, operands: &[u8]) {
        let value = operands.first().copied().unwrap_or(0);
        match selector {
            t::WIDE => self.set_wide(true),
            t::NARROW => self.set_wide(false),
            t::POSITION => self.tandy_position(u16::from_be_bytes([operands[0], operands[1]])),
            t::CR_ONLY => self.cr_lf = false,
            t::CR_LF => self.cr_lf = true,
            t::SWITCH => self.switch_grammar(),
            t::FEED_48 => self.move_paper((Y_UNITS_PER_INCH / 48) as i32),
            t::FEED_72 => self.move_paper((Y_UNITS_PER_INCH / 72) as i32),
            t::FEED_216 => self.move_paper((Y_UNITS_PER_INCH / 216) as i32),
            t::FEED_144 => self.move_paper((Y_UNITS_PER_INCH / 144) as i32),
            t::FEED_N => self.move_paper(i32::from(value) * (Y_UNITS_PER_INCH / 144) as i32),
            t::FORM => {
                self.set_form(u32::from(value.max(MIN_TANDY_FORM_LINES)) * Y_UNITS_PER_INCH / 6)
            }
            t::SKIP => self.set_skip(u32::from(value) * FULL_FEED as u32),
            _ if self.mode == Mode::Graphics => {}
            _ => self.tandy_text_escape(selector, value),
        }
    }
    pub(super) fn tandy_text_escape(&mut self, selector: u8, value: u8) {
        match selector {
            1..=9 => self.insert_dot_spaces(selector),
            t::REVERSE => self.tandy_feed(-FULL_FEED),
            t::HALF => self.tandy_feed(FULL_FEED / 2),
            t::HALF_REVERSE => self.tandy_feed(-FULL_FEED / 2),
            t::FULL => self.tandy_feed(FULL_FEED),
            t::THREE_QUARTERS => self.tandy_feed(FULL_FEED * 3 / 4),
            t::IBM_CHARSET => self.charset = Charset::Ibm2,
            t::TANDY_CHARSET => self.charset = Charset::Tandy,
            t::LEFT => self.tandy_margin(true, value),
            t::RIGHT => self.tandy_margin(false, value),
            t::DIRECTION if value <= 1 => self.bidirectional = value == 1,
            t::COUNTRY if (USA..=LAST_COUNTRY).contains(&value) => self.country = value,
            _ => self.tandy_style(selector, value),
        }
    }
    pub(super) fn insert_dot_spaces(&mut self, count: u8) {
        let width = u64::from(count) * self.dot_step();
        if self.x.saturating_add(width) > self.right {
            self.wrap_line();
        }
        self.x = self.x.saturating_add(width);
        if self.x >= self.right {
            self.carriage_return();
        }
    }
    pub(super) fn tandy_feed(&mut self, pitch: i32) {
        if self.mode == Mode::DataProcessing {
            self.feed = pitch;
        } else {
            self.move_paper(pitch);
        }
    }
    pub(super) fn tandy_style(&mut self, selector: u8, value: u8) {
        if !matches!(
            selector,
            t::PROPORTIONAL
                | t::PICA
                | t::ELITE
                | t::CONDENSED
                | t::NLQ_PICA
                | t::NLQ_ELITE
                | t::BOLD
                | t::BOLD_OFF
                | t::ITALIC
                | t::MICRO
                | t::SCRIPT
                | t::END_SCRIPT
        ) {
            return;
        }
        self.flush();
        match selector {
            t::PROPORTIONAL => {
                self.style.proportional = true;
                self.style.nlq = true;
                self.style.pitch = Pitch::Pica;
            }
            t::PICA | t::ELITE | t::CONDENSED | t::NLQ_PICA | t::NLQ_ELITE => {
                self.style.pitch = match selector {
                    t::ELITE | t::NLQ_ELITE => Pitch::Elite,
                    t::CONDENSED => Pitch::Condensed,
                    _ => Pitch::Pica,
                };
                self.style.nlq = matches!(selector, t::NLQ_PICA | t::NLQ_ELITE);
                self.style.proportional = false;
            }
            t::BOLD => self.style.bold = true,
            t::BOLD_OFF => self.style.bold = false,
            t::ITALIC if value <= 1 => self.style.italic = value == 1,
            t::MICRO => {
                self.style.micro = true;
                self.style.script = Some(Script::Super);
            }
            t::SCRIPT if value <= 1 => {
                self.style.micro = false;
                self.style.script = Some(if value == 0 {
                    Script::Super
                } else {
                    Script::Sub
                });
            }
            t::END_SCRIPT => {
                self.style.script = None;
                self.style.micro = false;
            }
            _ => {}
        }
    }
    pub(super) fn tandy_position(&mut self, column: u16) {
        self.flush();
        let (columns, step) = if self.mode == Mode::Graphics {
            (GRAPHICS_COLUMNS, PRINT_WIDTH / u64::from(GRAPHICS_COLUMNS))
        } else {
            (self.dots_per_line() / 2, self.dot_step() * 2)
        };
        if u32::from(column) < columns {
            self.x = u64::from(column) * step;
        } else if self.mode == Mode::Graphics && u32::from(column) == columns {
            self.wrap_line();
        }
    }
}
