//! Incremental, snapshot-safe command parser (manual pp. 89–99).
use super::{
    codes::{control as c, ibm as i, tandy as t},
    *,
};
impl DMP130 {
    pub(super) fn feed_byte(&mut self, byte: u8) {
        match std::mem::take(&mut self.pending) {
            Pending::None => self.fresh_byte(byte),
            Pending::Escape => self.start_escape(byte),
            Pending::Operands {
                selector,
                mut bytes,
                need,
            } => {
                bytes.push(byte);
                if bytes.len() == need {
                    self.escape(selector, &bytes);
                } else {
                    self.pending = Pending::Operands {
                        selector,
                        bytes,
                        need,
                    };
                }
            }
            Pending::RepeatCount => self.pending = Pending::RepeatData(byte),
            Pending::RepeatData(count) => self.repeat(count, byte),
            Pending::Backspace => {
                self.x = self
                    .x
                    .saturating_sub(u64::from(byte) * self.dot_step())
                    .max(self.left)
            }
            Pending::Tabs { stops, valid } => self.collect_tab(byte, stops, valid),
            Pending::FormInches => {
                if (1..=MAX_FORM_INCHES).contains(&byte) {
                    self.set_form(u32::from(byte) * Y_UNITS_PER_INCH);
                }
            }
            Pending::Graphics { remaining, step } => {
                self.ibm_graphics(byte, step);
                if remaining > 1 {
                    self.pending = Pending::Graphics {
                        remaining: remaining - 1,
                        step,
                    };
                }
            }
        }
    }
    pub(super) fn fresh_byte(&mut self, byte: u8) {
        if byte == c::ESC {
            self.pending = Pending::Escape;
        } else if self.grammar == Grammar::Ibm {
            self.ibm_byte(byte);
        } else {
            self.tandy_byte(byte);
        }
    }
    pub(super) fn start_escape(&mut self, selector: u8) {
        if self.grammar == Grammar::Ibm && selector == i::TABS {
            self.pending = Pending::Tabs {
                stops: Vec::new(),
                valid: true,
            };
            return;
        }
        let need = if self.grammar == Grammar::Tandy {
            match selector {
                t::POSITION => 2,
                t::FORM
                | t::FEED_N
                | t::ITALIC
                | t::SKIP
                | t::LEFT
                | t::RIGHT
                | t::SCRIPT
                | t::DIRECTION
                | t::COUNTRY => 1,
                _ => 0,
            }
        } else {
            match selector {
                i::GRAPHICS_60
                | i::GRAPHICS_120_SLOW
                | i::GRAPHICS_120
                | i::GRAPHICS_240
                | i::MARGINS
                | i::FORWARD
                | i::BACKWARD => 2,
                i::UNDERLINE
                | i::FEED_N_216
                | i::CR_LF
                | i::STAGE_FEED
                | i::FORM
                | i::QUALITY
                | i::FEED_IMMEDIATE
                | i::SKIP
                | i::PROPORTIONAL
                | i::SCRIPT
                | i::DIRECTION
                | i::WIDE
                | i::LITERAL => 1,
                _ => 0,
            }
        };
        if need == 0 {
            self.escape(selector, &[]);
        } else {
            self.pending = Pending::Operands {
                selector,
                bytes: Vec::new(),
                need,
            };
        }
    }
    pub(super) fn escape(&mut self, selector: u8, bytes: &[u8]) {
        if self.grammar == Grammar::Tandy {
            self.tandy_escape(selector, bytes);
        } else {
            self.ibm_escape(selector, bytes);
        }
    }
    pub(super) fn repeat(&mut self, count: u8, byte: u8) {
        if self.mode == Mode::Graphics {
            if byte & c::HIGH_BIT != 0 {
                for _ in 0..count {
                    self.tandy_graphics(byte);
                }
            }
        } else if byte >= c::ASCII_FIRST
            && !matches!(byte, c::DELETE | c::HIGH_LF | c::HIGH_CR | u8::MAX)
        {
            for _ in 0..count {
                self.print_character(byte);
            }
        }
    }
    pub(super) fn collect_tab(&mut self, byte: u8, mut stops: Vec<u8>, mut valid: bool) {
        if byte == 0 {
            self.tabs = stops
                .into_iter()
                .map(|column| u64::from(column) * self.cell_width())
                .collect();
            return;
        }
        valid &= usize::from(byte) <= MAX_COLUMNS as usize
            && stops.len() < MAX_TABS
            && stops.last().is_none_or(|&last| byte > last);
        if valid {
            stops.push(byte);
        }
        self.pending = Pending::Tabs { stops, valid };
    }
}
