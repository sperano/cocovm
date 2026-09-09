//! IBM emulation grammar (Operation Manual pp. 67–77, 93–99).
use super::{
    codes::{control as c, ibm as i},
    *,
};
impl DMP130 {
    pub(super) fn ibm_byte(&mut self, byte: u8) {
        let control = byte & !c::HIGH_BIT;
        match control {
            c::BELL => {}
            c::BS => {
                self.flush();
                self.x = self
                    .x
                    .saturating_sub(self.character_width(b' '))
                    .max(self.left);
            }
            c::HT => self.horizontal_tab(),
            c::LF | c::VT => self.line_feed(),
            c::FF => self.form_feed(),
            c::CR => self.carriage_return(),
            c::SO => self.style.transient_wide = true,
            c::SI => {
                self.flush();
                self.style.condensed = true;
            }
            c::DC2 => {
                self.flush();
                self.style.condensed = false;
            }
            c::DC4 => self.style.transient_wide = false,
            c::CAN => {
                self.buffered.clear();
                self.x = self.left;
            }
            _ if byte >= c::ASCII_FIRST => self.print_character(byte),
            _ => {}
        }
    }
    pub(super) fn ibm_escape(&mut self, selector: u8, operands: &[u8]) {
        let value = operands.first().copied().unwrap_or(0);
        match selector {
            i::SWITCH => self.switch_grammar(),
            i::FEED_EIGHTH => self.feed = (Y_UNITS_PER_INCH / 8) as i32,
            i::FEED_GRAPHICS => self.feed = GRAPHICS_FEED,
            i::ACTIVATE_FEED => self.feed = self.staged_feed,
            i::FEED_N_216 => self.feed = i32::from(value) * (Y_UNITS_PER_INCH / 216) as i32,
            i::STAGE_FEED if value <= MAX_STAGED_FEED => {
                self.staged_feed = i32::from(value) * (Y_UNITS_PER_INCH / 72) as i32
            }
            i::FEED_IMMEDIATE => {
                self.move_paper(i32::from(value) * (Y_UNITS_PER_INCH / 216) as i32)
            }
            i::REVERSE => self.move_paper(-FULL_FEED),
            i::TOP_OF_FORM => self.form_top = self.y,
            i::FORM => self.ibm_form(value),
            i::CR_LF if value <= 1 => self.cr_lf = value == 1,
            i::CHARSET_TWO => self.charset = Charset::Ibm2,
            i::CHARSET_ONE => self.charset = Charset::Ibm1,
            i::PAPER_OUT_OFF => self.paper_out = false,
            i::PAPER_OUT_ON => self.paper_out = true,
            i::RESET_TABS => self.reset_tabs(),
            i::SKIP if (1..=MAX_FORM_LINES).contains(&value) => {
                self.set_skip(u32::from(value) * self.feed.max(0) as u32)
            }
            i::SKIP_OFF => self.skip = 0,
            i::MARGINS => self.ibm_margins(operands[0], operands[1]),
            i::HOME => {
                self.flush();
                self.x = self.left;
                self.home_next_line = true;
            }
            i::FORWARD | i::BACKWARD => self.relative_position(
                u16::from_le_bytes([operands[0], operands[1]]),
                selector == i::BACKWARD,
            ),
            i::GRAPHICS_60 | i::GRAPHICS_120_SLOW | i::GRAPHICS_120 | i::GRAPHICS_240 => {
                self.begin_ibm_graphics(selector, operands)
            }
            i::LITERAL => self.print_character(if matches!(value, 3..=6 | 19..=21) {
                value
            } else {
                b' '
            }),
            _ => self.ibm_style(selector, value),
        }
    }
    pub(super) fn ibm_style(&mut self, selector: u8, value: u8) {
        let control = selector & !c::HIGH_BIT;
        if matches!(
            control,
            c::BELL
                | c::BS
                | c::HT
                | c::LF
                | c::VT
                | c::FF
                | c::CR
                | c::SO
                | c::SI
                | c::DC2
                | c::DC4
                | c::CAN
        ) {
            self.ibm_byte(control);
            return;
        }
        if !matches!(
            selector,
            i::UNDERLINE
                | i::ELITE
                | i::PICA
                | i::BOLD
                | i::BOLD_OFF
                | i::DOUBLE_STRIKE
                | i::DOUBLE_STRIKE_OFF
                | i::QUALITY
                | i::PROPORTIONAL
                | i::SCRIPT
                | i::END_SCRIPT
                | i::DIRECTION
                | i::WIDE
        ) {
            return;
        }
        self.flush();
        match selector {
            i::UNDERLINE if value <= 1 => self.style.underline = value == 1,
            i::ELITE => self.style.pitch = Pitch::Elite,
            i::PICA => self.style.pitch = Pitch::Pica,
            i::BOLD => self.style.bold = true,
            i::BOLD_OFF => self.style.bold = false,
            i::DOUBLE_STRIKE => self.style.double_strike = true,
            i::DOUBLE_STRIKE_OFF => self.style.double_strike = false,
            i::QUALITY if (1..=3).contains(&value) => self.style.nlq = value != 1,
            i::PROPORTIONAL if value <= 1 => self.style.proportional = value == 1,
            i::SCRIPT if value <= 1 => {
                self.style.script = Some(if value == 0 {
                    Script::Super
                } else {
                    Script::Sub
                })
            }
            i::END_SCRIPT => self.style.script = None,
            i::DIRECTION if value <= 1 => self.bidirectional = value == 0,
            i::WIDE if value <= 1 => self.style.wide = value == 1,
            _ => {}
        }
    }
    pub(super) fn ibm_form(&mut self, lines: u8) {
        if lines == 0 {
            self.pending = Pending::FormInches;
        } else if lines <= MAX_FORM_LINES {
            self.set_form(u32::from(lines) * self.feed.max(0) as u32);
        }
    }
    pub(super) fn begin_ibm_graphics(&mut self, selector: u8, operands: &[u8]) {
        self.flush();
        let remaining = u16::from_le_bytes([operands[0], operands[1]]);
        let dpi = match selector {
            i::GRAPHICS_60 => GRAPHICS_DPI,
            i::GRAPHICS_240 => IBM_QUADRUPLE_DPI,
            _ => IBM_DOUBLE_DPI,
        };
        if remaining > 0 {
            self.pending = Pending::Graphics {
                remaining,
                step: X_INCH / dpi,
            };
        }
    }
}
