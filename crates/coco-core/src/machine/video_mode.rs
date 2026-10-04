//! Video mode classification and the legacy (VDG-compatible) display-base/
//! palette resolution shared by the CoCo 1/2 and CoCo 3 text/graphics
//! renderers.

use mc6809::Bus;

use crate::config::MachineVariant;
use crate::{gime, gime_video, video};

use super::{Machine, VideoMode};

use super::TextCursor;

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
    pub(super) fn video_mode(&self) -> VideoMode {
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

    /// Decode the current text screen and BASIC's validated cursor position.
    /// The cursor cell is normalized to a space because BASIC modifies that
    /// byte while blinking it. Graphics modes have no text buffer or cursor.
    pub fn text_screen(&mut self) -> TextScreen {
        match self.video_mode() {
            VideoMode::CocoText => {
                let base = self.legacy_display_base();
                let mut lines = self.legacy_text_lines(base);
                let cursor = self.basic_text_cursor();
                if let Some(cursor) = cursor {
                    lines[cursor.row].replace_range(cursor.col..cursor.col + 1, " ");
                }
                TextScreen { lines, cursor }
            }
            VideoMode::CocoGraphics => TextScreen {
                lines: vec!["<no text buffer: VDG graphics mode (PMODE, $FF22 A/G=1)>".to_string()],
                cursor: None,
            },
            VideoMode::GIMEText => {
                let lines = gime_video::text_lines(&self.bus.gime, &self.bus.ram);
                let cursor = self.basic_text_cursor();
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
