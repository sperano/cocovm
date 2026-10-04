//! BASIC's text cursor, read out of the ROM's RAM variables
//! ([`crate::basic_vars`]) for the screen currently on display.

use crate::basic_vars::{self, hrwidth};
use crate::config::MachineVariant;
use crate::{gime_video, video};

use super::{Machine, VideoMode};

/// A text cursor position, 0-based, in the rows/columns of
/// [`Machine::text_screen_lines`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextCursor {
    pub row: usize,
    pub col: usize,
}

impl Machine {
    /// Where BASIC's next character lands on the text screen being
    /// displayed. `None` in graphics modes and whenever BASIC's cursor
    /// variables don't describe the displayed screen (another program owns
    /// it, or the variables are out of range). Reads with no side effects.
    pub fn basic_text_cursor(&self) -> Option<TextCursor> {
        match self.video_mode() {
            VideoMode::CocoText => self.vdg_basic_cursor(),
            VideoMode::GIMEText => self.hires_basic_cursor(),
            VideoMode::CocoGraphics | VideoMode::GIMEGraphics => None,
        }
    }

    /// The 32-column cursor from CURPOS, if BASIC's VDG screen is the one
    /// shown (and, on a CoCo 3, `WIDTH 32` is in effect).
    fn vdg_basic_cursor(&self) -> Option<TextCursor> {
        if self.config.variant == MachineVariant::Coco3
            && self.bus.peek(basic_vars::HRWIDTH) != hrwidth::VDG_32
        {
            return None;
        }
        if self.legacy_display_base() != basic_vars::VDG_SCREEN_BASE {
            return None;
        }
        let pos = u16::from_be_bytes([
            self.bus.peek(basic_vars::CURPOS),
            self.bus.peek(basic_vars::CURPOS.wrapping_add(1)),
        ]);
        if !(basic_vars::VDG_SCREEN_BASE..=basic_vars::VDG_SCREEN_LAST).contains(&pos) {
            return None;
        }
        let offset = usize::from(pos - basic_vars::VDG_SCREEN_BASE);
        Some(TextCursor {
            row: offset / video::COLS,
            col: offset % video::COLS,
        })
    }

    /// The `WIDTH 40`/`80` cursor from H.CURSX/H.CURSY, if `WIDTH 40` or
    /// `80` is in effect and BASIC's screen size matches the GIME's.
    fn hires_basic_cursor(&self) -> Option<TextCursor> {
        let width = self.bus.peek(basic_vars::HRWIDTH);
        if width != hrwidth::HIRES_40 && width != hrwidth::HIRES_80 {
            return None;
        }
        let mode = gime_video::decode_text(&self.bus.gime);
        let shown_rows = mode.lines.checked_div(mode.lines_per_row).unwrap_or(0);
        let cols = usize::from(self.bus.peek(basic_vars::H_COLUMN));
        let rows = usize::from(self.bus.peek(basic_vars::H_ROW));
        if cols != mode.cols || rows > shown_rows {
            return None;
        }
        let col = usize::from(self.bus.peek(basic_vars::H_CURSX));
        let row = usize::from(self.bus.peek(basic_vars::H_CURSY));
        (col < cols && row < rows).then_some(TextCursor { row, col })
    }
}
