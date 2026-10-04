//! Video mode classification and the legacy (VDG-compatible) display-base/
//! palette resolution shared by the CoCo 1/2 and CoCo 3 text/graphics
//! renderers.

use mc6809::Bus;

use crate::config::MachineVariant;
use crate::{gime, gime_video, video};

use super::{Machine, VideoMode};

/// Logical base of the flat system-ROM image in [`crate::SystemBus::rom`].
const SYSTEM_ROM_BASE: u16 = 0x8000;
/// Stock BASIC's pointer to the active CoCo-compatible cursor cell.
///
/// `bas12.rom` and `coco3.rom` at $A1A1 load the pointer from $0088, then
/// $A1A3-$A1A9 temporarily transform the pointed-to byte for cursor blink.
const LEGACY_CURSOR_POINTER: u16 = 0x0088;
/// Stock Color BASIC cursor-blink routine at $A1A1.
///
/// `bas12.rom` and `coco3.rom` contain `LDX <$88; LDA ,X; ADDA #$10;
/// ORA #$8F; STA ,X` at this address. Checking these immutable ROM bytes
/// distinguishes the workspace convention from an arbitrary program that
/// happens to leave an in-range value at $0088.
const LEGACY_CURSOR_ROUTINE_OFFSET: usize = 0xA1A1 - SYSTEM_ROM_BASE as usize;
const LEGACY_CURSOR_ROUTINE: &[u8] = &[0x9E, 0x88, 0xA6, 0x84, 0x8B, 0x10, 0x8A, 0x8F, 0xA7, 0x84];
/// Super Extended BASIC's zero-based GIME text cursor column.
///
/// `coco3.rom` $F7E2-$F852 uses $FE02/$FE03 for the cursor coordinates and
/// $FE04/$FE05 for the configured text dimensions.
const GIME_CURSOR_COLUMN: u16 = 0xFE02;
const GIME_CURSOR_ROW: u16 = 0xFE03;
const GIME_TEXT_COLUMNS: u16 = 0xFE04;
const GIME_TEXT_ROWS: u16 = 0xFE05;
const BASIC_GIME_TEXT_COLUMNS: [usize; 2] = [40, 80];
/// Super Extended BASIC's coordinate-update routine at $F7F2.
///
/// `coco3.rom` contains `LDD $FE02; DECA; BPL; DECB; STB $FE03;
/// LDA $FE04; DECA; STA $FE02` here, establishing the $FE02-$FE04
/// workspace convention used by the surrounding $F7E2-$F852 cursor code.
const GIME_CURSOR_ROUTINE_OFFSET: usize = 0xF7F2 - SYSTEM_ROM_BASE as usize;
const GIME_CURSOR_ROUTINE: &[u8] = &[
    0xFC, 0xFE, 0x02, 0x4A, 0x2A, 0x08, 0x5A, 0xF7, 0xFE, 0x03, 0xB6, 0xFE, 0x04, 0x4A, 0xB7, 0xFE,
    0x02,
];

/// A zero-based insertion position in a decoded text screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextCursor {
    /// Character row from the top of the decoded screen.
    pub row: usize,
    /// Character column from the left edge of the decoded screen.
    pub column: usize,
}

/// Decoded screen text and a cursor when the active ROM convention validates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextScreen {
    /// Fixed-width decoded character rows.
    pub lines: Vec<String>,
    /// Validated ROM insertion position, or `None` outside known conventions.
    pub cursor: Option<TextCursor>,
}

impl Machine {
    /// Classify the current video mode. CoCo 1/2 (no GIME) always runs the
    /// VDG-native path (PIA1 $FF22 bit 7 selects graphics vs text); CoCo 3
    /// also checks INIT0 COCO and $FF98 BP.
    fn video_mode(&self) -> VideoMode {
        match self.config.variant {
            MachineVariant::Coco1 | MachineVariant::Coco2 => {
                if self.bus.pia1.b.output & video::VDG_AG != 0 {
                    VideoMode::CocoGraphics
                } else {
                    VideoMode::CocoText
                }
            }
            MachineVariant::Coco3 => {
                let g = &self.bus.gime;
                if g.init0 & gime::init0::COCO != 0 {
                    // PIA1 $FF22 bit 7 selects VDG graphics (PMODE) vs alphanumerics/semigraphics.
                    if self.bus.pia1.b.output & video::VDG_AG != 0 {
                        VideoMode::CocoGraphics
                    } else {
                        VideoMode::CocoText
                    }
                } else if g.vmode & gime::vmode::BP != 0 {
                    VideoMode::GIMEGraphics
                } else {
                    VideoMode::GIMEText
                }
            }
        }
    }

    /// CoCo-compatible video/text base address, per variant: the GIME's own
    /// SAM-compat page register (CoCo 3) or the primary SAM's F-bits (CoCo 1/2).
    pub(super) fn legacy_display_base(&self) -> u16 {
        match self.config.variant {
            MachineVariant::Coco3 => self.bus.gime.sam_display_base(),
            MachineVariant::Coco1 | MachineVariant::Coco2 => self.bus.sam.display_base() as u16,
        }
    }

    /// Resolve the 16-entry colour table the CoCo-compatible text/graphics
    /// renderers read from: GIME palette registers on CoCo 3, the fixed VDG
    /// RGB table on CoCo 1/2 (`css` only matters there).
    pub(super) fn legacy_palette(&self, css: bool) -> [[u8; 4]; video::PALETTE_LEN] {
        match self.config.variant {
            MachineVariant::Coco3 => {
                let mut resolved = [[0u8; 4]; video::PALETTE_LEN];
                for (i, entry) in resolved.iter_mut().enumerate() {
                    *entry = self.bus.gime.color(self.bus.gime.palette[i]);
                }
                video::ColorSource::GIMEPalette(&resolved).resolve(css)
            }
            MachineVariant::Coco1 | MachineVariant::Coco2 => {
                video::ColorSource::VDGFixed.resolve(css)
            }
        }
    }

    /// Decode the current text screen and its validated ROM cursor convention.
    /// The compatibility cursor cell is normalized to a space because BASIC
    /// modifies that byte while blinking it. Graphics modes have no cursor.
    pub fn text_screen(&mut self) -> TextScreen {
        match self.video_mode() {
            VideoMode::CocoText => {
                let base = self.legacy_display_base();
                let mut lines = self.legacy_text_lines(base);
                let cursor = self.legacy_text_cursor(base);
                if let Some(cursor) = cursor {
                    lines[cursor.row].replace_range(cursor.column..cursor.column + 1, " ");
                }
                TextScreen { lines, cursor }
            }
            VideoMode::CocoGraphics => {
                let base = self.legacy_display_base();
                TextScreen {
                    lines: self.legacy_text_lines(base),
                    cursor: None,
                }
            }
            VideoMode::GIMEText => {
                let lines = gime_video::text_lines(&self.bus.gime, &self.bus.ram);
                let cursor = self.gime_text_cursor(&lines);
                TextScreen { lines, cursor }
            }
            VideoMode::GIMEGraphics => TextScreen {
                lines: vec![
                    "<no text buffer: GIME graphics mode (HSCREEN, $FF98 BP=1)>".to_string(),
                ],
                cursor: None,
            },
        }
    }

    /// Decode only the text lines. Callers that expose screen state should
    /// prefer [`Self::text_screen`] so they also report cursor position.
    pub fn text_screen_lines(&mut self) -> Vec<String> {
        self.text_screen().lines
    }

    fn legacy_text_lines(&mut self, base: u16) -> Vec<String> {
        (0..video::ROWS as u16)
            .map(|row| {
                (0..video::COLS as u16)
                    .map(|col| {
                        let addr = base.wrapping_add(row * video::COLS as u16 + col);
                        video::decode_alpha_char(self.bus.read(addr))
                    })
                    .collect()
            })
            .collect()
    }

    fn legacy_text_cursor(&self, base: u16) -> Option<TextCursor> {
        if !self.rom_has_signature(LEGACY_CURSOR_ROUTINE_OFFSET, LEGACY_CURSOR_ROUTINE) {
            return None;
        }
        let high = u16::from(self.bus.peek(LEGACY_CURSOR_POINTER));
        let low = u16::from(self.bus.peek(LEGACY_CURSOR_POINTER + 1));
        let address = (high << 8) | low;
        let offset = address.checked_sub(base).map(usize::from)?;
        if offset >= video::SCREEN_LEN {
            return None;
        }
        Some(TextCursor {
            row: offset / video::COLS,
            column: offset % video::COLS,
        })
    }

    fn gime_text_cursor(&self, lines: &[String]) -> Option<TextCursor> {
        if !self.rom_has_signature(GIME_CURSOR_ROUTINE_OFFSET, GIME_CURSOR_ROUTINE) {
            return None;
        }
        let columns = usize::from(self.bus.peek(GIME_TEXT_COLUMNS));
        let rows = usize::from(self.bus.peek(GIME_TEXT_ROWS));
        let decoded_columns = lines.first()?.chars().count();
        if !BASIC_GIME_TEXT_COLUMNS.contains(&columns)
            || rows != lines.len()
            || columns != decoded_columns
            || lines.iter().any(|line| line.chars().count() != columns)
        {
            return None;
        }
        let cursor = TextCursor {
            row: usize::from(self.bus.peek(GIME_CURSOR_ROW)),
            column: usize::from(self.bus.peek(GIME_CURSOR_COLUMN)),
        };
        (cursor.row < rows && cursor.column < columns).then_some(cursor)
    }

    fn rom_has_signature(&self, offset: usize, signature: &[u8]) -> bool {
        self.bus
            .rom
            .get(offset..offset + signature.len())
            .is_some_and(|bytes| bytes == signature)
    }

    /// One-line diagnostic summary of the current video mode and its text/video
    /// base address. Pairs with [`Machine::text_screen_lines`] to explain a blank or garbled dump.
    pub fn video_mode_summary(&self) -> String {
        match self.video_mode() {
            VideoMode::CocoText => {
                format!(
                    "video mode: CoCo-compatible text, base=${:04X}",
                    self.legacy_display_base()
                )
            }
            VideoMode::CocoGraphics => {
                format!(
                    "video mode: CoCo-compatible graphics (PMODE), base=${:04X}",
                    self.legacy_display_base()
                )
            }
            VideoMode::GIMEText => {
                format!(
                    "video mode: GIME hi-res text, base=${:06X}",
                    self.bus.gime.video_base()
                )
            }
            VideoMode::GIMEGraphics => {
                format!(
                    "video mode: GIME graphics (HSCREEN), base=${:06X}",
                    self.bus.gime.video_base()
                )
            }
        }
    }
}

#[cfg(test)]
#[path = "video_mode_test.rs"]
mod tests;
