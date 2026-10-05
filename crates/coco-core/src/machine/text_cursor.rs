//! BASIC's text cursor, read out of the ROM's RAM variables
//! ([`crate::basic_vars`]) for the screen currently on display.

use crate::basic_vars::{self, hrwidth};
use crate::config::MachineVariant;
use crate::{gime_video, video};

use super::{Machine, VideoMode};

/// Logical base of the flat system-ROM image in [`crate::SystemBus::rom`].
const SYSTEM_ROM_BASE: u16 = 0x8000;
/// Stock Color BASIC cursor-blink routine at $A1A1.
///
/// `bas12.rom` and `coco3.rom` contain `LDX <$88; LDA ,X; ADDA #$10;
/// ORA #$8F; STA ,X` at this address. Checking these immutable ROM bytes
/// distinguishes the workspace convention from an arbitrary program that
/// happens to leave an in-range value at CURPOS.
pub(super) const VDG_CURSOR_ROUTINE_OFFSET: usize = 0xA1A1 - SYSTEM_ROM_BASE as usize;
pub(super) const VDG_CURSOR_ROUTINE: &[u8] =
    &[0x9E, 0x88, 0xA6, 0x84, 0x8B, 0x10, 0x8A, 0x8F, 0xA7, 0x84];
/// Super Extended BASIC's coordinate-update routine at $F7F4.
///
/// `coco3.rom` contains `LDD $FE02; DECA; BPL; DECB; STB $FE03;
/// LDA $FE04; DECA; STA $FE02` here, establishing the $FE02-$FE04
/// workspace convention used by the surrounding $F7E2-$F852 cursor code.
pub(super) const GIME_CURSOR_ROUTINE_OFFSET: usize = 0xF7F4 - SYSTEM_ROM_BASE as usize;
pub(super) const GIME_CURSOR_ROUTINE: &[u8] = &[
    0xFC, 0xFE, 0x02, 0x4A, 0x2A, 0x08, 0x5A, 0xF7, 0xFE, 0x03, 0xB6, 0xFE, 0x04, 0x4A, 0xB7, 0xFE,
    0x02,
];

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
        if !self.rom_has_signature(VDG_CURSOR_ROUTINE_OFFSET, VDG_CURSOR_ROUTINE) {
            return None;
        }
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
            self.bus.peek(basic_vars::CURPOS + 1),
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
        if !self.rom_has_signature(GIME_CURSOR_ROUTINE_OFFSET, GIME_CURSOR_ROUTINE) {
            return None;
        }
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

    fn rom_has_signature(&self, offset: usize, signature: &[u8]) -> bool {
        self.bus
            .rom
            .get(offset..offset + signature.len())
            .is_some_and(|bytes| bytes == signature)
    }
}
