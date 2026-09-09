//! Paper motion and physical margins (Operation Manual pp. 41–57, 68–72).
use super::*;
impl DMP130 {
    pub(super) fn line_pitch(&self) -> i32 {
        match self.mode {
            Mode::Graphics => GRAPHICS_FEED,
            Mode::WordProcessing => {
                if self.style.micro {
                    FULL_FEED / 2
                } else {
                    FULL_FEED
                }
            }
            Mode::DataProcessing => {
                if self.style.micro {
                    self.feed / 2
                } else {
                    self.feed
                }
            }
        }
    }
    pub(super) fn move_paper(&mut self, amount: i32) {
        self.flush();
        self.y = self.y.saturating_add_signed(amount);
        if amount > 0 && self.skip > 0 && self.form_length > 0 {
            let offset = self.y.saturating_sub(self.form_top) % self.form_length;
            if offset >= self.form_length - self.skip {
                self.y = self.y.saturating_add(self.form_length - offset);
            }
        }
    }
    pub(super) fn line_feed(&mut self) {
        self.move_paper(self.line_pitch());
        if self.grammar == Grammar::Ibm {
            self.style.transient_wide = false;
        }
    }
    pub(super) fn carriage_return(&mut self) {
        self.flush();
        self.x = if self.mode == Mode::Graphics {
            0
        } else {
            self.left
        };
        self.home_next_line = false;
        if self.cr_lf {
            self.line_feed();
        }
    }
    pub(super) fn wrap_line(&mut self) {
        if self.mode == Mode::Graphics {
            self.line_feed();
            self.x = 0;
        } else {
            self.carriage_return();
        }
    }
    pub(super) fn form_feed(&mut self) {
        self.flush();
        let offset = self.y.saturating_sub(self.form_top) % self.form_length;
        self.y = self.y.saturating_add(self.form_length - offset);
    }
    pub(super) fn set_form(&mut self, length: u32) {
        if length == 0 {
            return;
        }
        self.form_length = length;
        self.form_top = self.y;
        self.skip = 0;
    }
    pub(super) fn set_skip(&mut self, skip: u32) {
        if skip < self.form_length {
            self.skip = skip;
        }
    }
    pub(super) fn reset_tabs(&mut self) {
        let cell = self.cell_width();
        self.tabs = (TAB_INTERVAL..=MAX_COLUMNS)
            .step_by(TAB_INTERVAL as usize)
            .map(|column| u64::from(column) * cell)
            .collect();
    }
    pub(super) fn horizontal_tab(&mut self) {
        if let Some(next) = self
            .tabs
            .iter()
            .copied()
            .find(|&x| x > self.x && x < self.right)
        {
            if self.style.underline {
                self.underline_to(next);
            }
            self.x = next;
        }
    }
    pub(super) fn tandy_margin(&mut self, left: bool, column: u8) {
        let cell = self.cell_width();
        let position = u64::from(column) * cell;
        if left && position + 2 * cell <= self.right {
            self.left = position;
            self.x = self.x.max(self.left);
        } else if !left && position >= self.left + 2 * cell && position <= PRINT_WIDTH {
            self.right = position;
        }
    }
    pub(super) fn ibm_margins(&mut self, left: u8, right: u8) {
        if left == 0 || left >= right {
            return;
        }
        let cell = self.cell_width();
        let start = u64::from(left - 1) * cell;
        let end = u64::from(right) * cell;
        const MIN_WIDTH: u64 = X_INCH / 5;
        if end > PRINT_WIDTH || end - start < MIN_WIDTH {
            return;
        }
        self.flush();
        self.left = start;
        self.right = end;
        self.x = start;
    }
    pub(super) fn relative_position(&mut self, distance: u16, backward: bool) {
        self.flush();
        const STEP: u64 = X_INCH / 120;
        let delta = u64::from(distance) * STEP;
        let target = if backward {
            self.x.checked_sub(delta)
        } else {
            self.x.checked_add(delta)
        };
        // Chapter p. 72 says return home on overflow; Appendix p. 99 differs.
        self.x = target
            .filter(|&x| x >= self.left && x <= self.right)
            .unwrap_or(self.left);
    }
}
