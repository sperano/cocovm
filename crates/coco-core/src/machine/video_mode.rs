//! Video mode classification and the legacy (VDG-compatible) display-base/
//! palette resolution shared by the CoCo 1/2 and CoCo 3 text/graphics
//! renderers.

use mc6809::Bus;

use crate::config::MachineVariant;
use crate::{gime, gime_video, video};

use super::{Machine, VideoMode};

impl Machine {
    /// Classify the current video mode.
    ///
    /// CoCo 1/2 (no GIME) never has INIT0/$FF98 to consult: they always run the
    /// VDG-native path, chosen purely by PIA1 $FF22 bit 7 (A/G) —
    /// `docs/coco12-plan.md` Phase 3. CoCo 3 keeps its existing INIT0 COCO /
    /// $FF98 BP dispatch, unchanged.
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
    /// SAM-compat page register (CoCo 3, unchanged) or the primary SAM's F-bits
    /// (CoCo 1/2 — `docs/coco12-plan.md` Phase 3).
    pub(super) fn legacy_display_base(&self) -> u16 {
        match self.config.variant {
            MachineVariant::Coco3 => self.bus.gime.sam_display_base(),
            MachineVariant::Coco1 | MachineVariant::Coco2 => self.bus.sam.display_base() as u16,
        }
    }

    /// Resolve the 16-entry colour table the CoCo-compatible text/graphics
    /// renderers read from, per variant (`docs/coco12-plan.md` Phase 3):
    /// CoCo 3 snapshots the GIME palette registers (existing behaviour,
    /// unchanged); CoCo 1/2 has none, so it resolves the fixed VDG RGB table.
    /// `css` (PIA1 $FF22 bit 3) only matters for the fixed-VDG path — see
    /// [`video::ColorSource::resolve`].
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

    /// Decode the current text screen to ASCII lines, whichever video mode is
    /// active — a debug/probe helper, not a renderer (`gime_video::text_lines`
    /// and `video::decode_alpha_char` do the actual decoding, shared with the
    /// real renderers so this can't drift from what's actually on screen):
    ///
    /// - CoCo-compatible mode (INIT0 COCO=1): the legacy VDG alphanumeric
    ///   screen at the SAM page base, decoded the same way `render_coco_text`
    ///   reads it (through the bus, honouring the MMU).
    /// - GIME hi-res text (INIT0 COCO=0, $FF98 BP=0): the GIME-native text
    ///   buffer at the vertical-offset registers' physical address.
    ///
    /// The two graphics modes (VDG PMODE, GIME HSCREEN) have no text buffer to
    /// decode; each returns one placeholder line naming the mode. Pair with
    /// [`Machine::video_mode_summary`] to tell a "genuinely blank screen" apart
    /// from "this is a graphics-mode screen with nothing to decode".
    pub fn text_screen_lines(&mut self) -> Vec<String> {
        match self.video_mode() {
            VideoMode::CocoText | VideoMode::CocoGraphics => {
                let base = self.legacy_display_base();
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
            VideoMode::GIMEText => gime_video::text_lines(&self.bus.gime, &self.bus.ram),
            VideoMode::GIMEGraphics => {
                vec!["<no text buffer: GIME graphics mode (HSCREEN, $FF98 BP=1)>".to_string()]
            }
        }
    }

    /// One-line diagnostic summary of the current video mode and its text/video
    /// base address. Pairs with [`Machine::text_screen_lines`] to explain an
    /// unexpectedly blank or garbled dump — most commonly, the machine has
    /// switched to a graphics mode, which has no text buffer.
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
