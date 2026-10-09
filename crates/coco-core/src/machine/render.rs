//! Framebuffer rendering onto the canonical 640×240 raster (Option B): the
//! CoCo 3 per-scanline path and the CoCo 1/2 whole-field snapshot path.

use mc6809::Bus;

use crate::config::{MachineVariant, VDGVariant, VideoStandard};
use crate::gime::{self, MonitorType, vmode};
use crate::{gime_video, raster, video};

use super::{BYTES_PER_PIXEL, Machine, TEXT_BORDER_COLOR};

/// The active (non-border) picture rectangle within the framebuffer, in
/// framebuffer pixels — [`Machine::active_rect`]'s return type. `u32` to
/// match [`Machine::fb_width`]/[`Machine::fb_height`], the dimensions a
/// consumer pairs it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Machine {
    /// Current scanline within the field (`0..lines_per_field`); rows ≥ 240
    /// are vertical blanking. Exposed for scanline-timed tests and debug UI.
    pub fn scanline(&self) -> u32 {
        self.line
    }

    /// The active (non-border) picture rectangle in framebuffer pixel
    /// coordinates, for the frontend's pointer→joystick mapping. Whether the
    /// field is legacy comes from the latched `field_scan` when available,
    /// rather than the live INIT0 COCO bit.
    pub fn active_rect(&self) -> ActiveRect {
        if self.config.variant != MachineVariant::Coco3 {
            return ActiveRect {
                x: raster::NON_WIDE_BORDER_X as u32,
                y: video::VDG_ACTIVE_TOP as u32,
                width: raster::NON_WIDE_ACTIVE_W as u32,
                height: video::ACTIVE_H as u32,
            };
        }

        let legacy = self.field_scan.as_ref().map_or_else(
            || self.bus.gime.init0 & gime::init0::COCO != 0,
            |s| s.legacy,
        );
        let (x, w) = if legacy {
            (raster::NON_WIDE_BORDER_X, raster::NON_WIDE_ACTIVE_W)
        } else {
            gime_video::active_span(&self.bus.gime)
        };
        let (top, body) = gime_video::active_rows(&self.bus.gime);

        ActiveRect {
            x: x as u32,
            y: top as u32,
            width: w as u32,
            height: body as u32,
        }
    }

    /// Paint the current scanline of the canonical raster, called every line
    /// so mid-frame register writes take effect on the next line. Only
    /// GIME-native fields paint here; legacy fields use [`Machine::render_field`] instead.
    pub(super) fn render_scanline(&mut self) {
        if self.config.variant != MachineVariant::Coco3 {
            return;
        }
        if self.line == 0 {
            let legacy = self.bus.gime.init0 & gime::init0::COCO != 0;
            self.field_scan = Some(gime_video::FieldScan::latch(&self.bus.gime, legacy));
            self.framebuffer
                .resize(raster::CANVAS_W * raster::CANVAS_H * BYTES_PER_PIXEL, 0);
            self.fb_width = raster::CANVAS_W as u32;
            self.fb_height = raster::CANVAS_H as u32;
        }
        let row = self.line as usize;
        let Some(scan) = self.field_scan.as_ref() else {
            return;
        };
        if row >= raster::CANVAS_H {
            return; // blanking lines 240..262
        }
        if scan.legacy {
            self.paint_legacy_scanline(row);
            return;
        }
        // Blink phase is toggled by the GIME interval timer.
        let blink_on = self.bus.gime.blink_state;
        let scan = self.field_scan.as_mut().expect("checked Some above");
        gime_video::paint_scanline(
            &self.bus.gime,
            &self.bus.ram,
            scan,
            blink_on,
            row,
            &mut self.framebuffer,
        );
    }

    /// Paint one canvas row of a CoCo 3 legacy (VDG-compatible) field, reading
    /// mode bits, palette, and border live each line. The border is NOT
    /// fixed black — it follows [`video::legacy_border_value`].
    fn paint_legacy_scanline(&mut self, row: usize) {
        let ff22 = self.bus.pia1.b.output;
        let border = self.bus.gime.color(video::legacy_border_value(ff22));
        let row_px = &mut self.framebuffer[row * raster::CANVAS_W * BYTES_PER_PIXEL..]
            [..raster::CANVAS_W * BYTES_PER_PIXEL];

        // Vertical placement from the live LPF bits — GIME applies LPF even in legacy modes.
        let (top, body) = gime_video::active_rows(&self.bus.gime);
        if row < top || row >= top + body {
            for px in row_px.as_chunks_mut::<BYTES_PER_PIXEL>().0 {
                px.copy_from_slice(&border);
            }
            return;
        }

        // Side borders around the 512 px active span (legacy is always non-wide).
        for px in row_px[..raster::NON_WIDE_BORDER_X * BYTES_PER_PIXEL]
            .as_chunks_mut::<BYTES_PER_PIXEL>()
            .0
        {
            px.copy_from_slice(&border);
        }
        for px in row_px
            [(raster::NON_WIDE_BORDER_X + raster::NON_WIDE_ACTIVE_W) * BYTES_PER_PIXEL..]
            .as_chunks_mut::<BYTES_PER_PIXEL>()
            .0
        {
            px.copy_from_slice(&border);
        }

        // Live per-line mode decode: bytes to fetch and this mode's LPR.
        let ag = ff22 & video::VDG_AG != 0;
        let css = ff22 & video::VDG_CSS != 0;
        let sam_video = self.bus.gime.sam_video;
        let (row_bytes, lines_per_row) = if ag {
            let mode = video::decode_vdg_graphics(ff22);
            let lpr = video::LEGACY_GFX_LINES_PER_ROW[(sam_video & 0x07) as usize];
            (mode.bytes_per_row, lpr)
        } else {
            (video::COLS, video::CELL_H)
        };

        // Fetch the current data row through the bus (MMU-honouring, 16-bit wrap).
        let (base, line_in_row) = {
            let scan = self.field_scan.as_ref().expect("legacy field latched");
            (scan.row_base as u16, scan.line_in_row)
        };
        let mut buf = [0u8; video::COLS];
        for (i, byte) in buf.iter_mut().take(row_bytes).enumerate() {
            *byte = self.bus.read(base.wrapping_add(i as u16));
        }

        let palette = self.legacy_palette(css);
        let artifact_phase = if ag {
            let mode = video::decode_vdg_graphics(ff22);
            self.coco3_rg6_artifact_phase(&mode)
        } else {
            None
        };
        let active = &mut self.framebuffer
            [(row * raster::CANVAS_W + raster::NON_WIDE_BORDER_X) * BYTES_PER_PIXEL..]
            [..raster::NON_WIDE_ACTIVE_W * BYTES_PER_PIXEL];
        if ag {
            let mode = video::decode_vdg_graphics(ff22);
            let indices = video::vdg_palette_indices(mode.bpp, usize::from(css));
            let mut colors = [[0u8; 4]; video::MAX_VDG_COLORS];
            for (slot, &reg) in colors.iter_mut().zip(indices) {
                *slot = palette[reg];
            }
            let xscale = raster::NON_WIDE_ACTIVE_W / mode.logical_w;
            if let Some(phase) = artifact_phase {
                let decoder = video::RG6ArtifactDecoder::new([colors[0], colors[1]], phase);
                let edges = video::RG6ArtifactEdges::solid(border == colors[1]);
                decoder.decode_scanline(&buf, edges, xscale, active);
            } else {
                video::paint_legacy_graphics_line(
                    &buf[..row_bytes],
                    &mode,
                    &colors[..indices.len()],
                    xscale,
                    active,
                );
            }
        } else {
            let generator = video::AlphaGenerator::GIME;
            let xscale = raster::NON_WIDE_ACTIVE_W / (video::COLS * video::CELL_W);
            video::paint_legacy_text_line(
                &buf[..row_bytes],
                &palette,
                generator,
                ff22,
                line_in_row,
                xscale,
                active,
            );
        }

        // Advance the shared vertical counter (MAME `record_full_body_scanline`).
        let scan = self.field_scan.as_mut().expect("legacy field latched");
        scan.line_in_row += 1;
        if scan.line_in_row >= lines_per_row {
            scan.line_in_row = 0;
            scan.row_base += row_bytes;
        }
    }

    /// The live CoCo 3 artifact phase for this scanline, when its output path
    /// carries NTSC composite RG6. Compatibility mode is already field-latched
    /// by the caller; BPI and the monitor cable remain live per scanline.
    fn coco3_rg6_artifact_phase(
        &self,
        mode: &video::VDGGraphicsMode,
    ) -> Option<video::RG6ArtifactPhase> {
        if self.config.variant != MachineVariant::Coco3
            || self.config.video != VideoStandard::NTSC
            || self.bus.gime.monitor != MonitorType::Composite
            || !mode.is_rg6()
        {
            return None;
        }
        Some(if self.bus.gime.vmode & vmode::BPI == 0 {
            video::RG6ArtifactPhase::Standard
        } else {
            video::RG6ArtifactPhase::Reverse
        })
    }

    /// The reset-selected CoCo 1/2 phase when the verified MC1372 burst
    /// circuit is active. The external burst injector requires both RG6's GM0
    /// and CSS lines high, so CSS=0 retains the fixed two-color output.
    fn coco12_rg6_artifact_phase(
        &self,
        mode: &video::VDGGraphicsMode,
        css: bool,
    ) -> Option<video::RG6ArtifactPhase> {
        if !matches!(
            self.config.variant,
            MachineVariant::Coco1 | MachineVariant::Coco2
        ) || self.config.video != VideoStandard::NTSC
            || !mode.is_rg6()
            || !css
        {
            return None;
        }
        Some(self.ntsc_rg6_artifact_phase())
    }

    /// Render one video field into `framebuffer` at field end. Only CoCo 1/2
    /// renders here as a whole-frame snapshot; CoCo 3 fields are already
    /// painted line by line by [`Machine::render_scanline`].
    pub(super) fn render_field(&mut self) {
        if self.config.variant == MachineVariant::Coco3 {
            return;
        }
        if self.bus.pia1.b.output & video::VDG_AG != 0 {
            self.render_coco_graphics();
        } else {
            self.render_coco_text();
        }
    }

    /// Render the legacy CoCo-compatible 32×16 text screen (`DESIGN.md` §6).
    fn render_coco_text(&mut self) {
        // Snapshot the text screen through the bus from the display-base register.
        // TODO: per-scanline scanout straight from RAM (`DESIGN.md` §2b/§6).
        let base = self.legacy_display_base();
        let mut screen = [0u8; video::SCREEN_LEN];
        for (i, cell) in screen.iter_mut().enumerate() {
            *cell = self.bus.read(base.wrapping_add(i as u16));
        }
        let ff22 = self.bus.pia1.b.output;
        let css = ff22 & video::VDG_CSS != 0;
        let palette = self.legacy_palette(css);
        // The legacy CoCo-compatible text border is fixed black on both variants.
        let border = match self.config.variant {
            MachineVariant::Coco3 => self.bus.gime.color(TEXT_BORDER_COLOR),
            MachineVariant::Coco1 | MachineVariant::Coco2 => {
                video::VDG_FIXED_PALETTE[video::TEXT_BORDER_INDEX]
            }
        };
        // CoCo 3 has no VDG chip: text mode uses the GIME's own compat-text
        // generator, not `self.config.vdg`.
        let generator = match self.config.variant {
            MachineVariant::Coco3 => video::AlphaGenerator::GIME,
            MachineVariant::Coco1 | MachineVariant::Coco2 => match self.config.vdg {
                Some(VDGVariant::MC6847T1) => video::AlphaGenerator::MC6847T1,
                // `None` is rejected for CoCo 1/2 elsewhere; fall back rather than panic.
                Some(VDGVariant::MC6847) | None => video::AlphaGenerator::MC6847,
            },
        };
        video::render_text(
            &screen,
            &palette,
            border,
            generator,
            ff22,
            &mut self.framebuffer,
        );
    }

    /// Render a VDG bitmap graphics (PMODE) field through the discrete MC6883.
    fn render_coco_graphics(&mut self) {
        let ff22 = self.bus.pia1.b.output;
        let mode = video::decode_vdg_graphics(ff22);
        let css_bit = ff22 & video::VDG_CSS != 0;
        let css = usize::from(css_bit);
        let indices = video::vdg_palette_indices(mode.bpp, css);
        let palette = self.legacy_palette(css_bit);
        let mut colors = [[0u8; 4]; video::MAX_VDG_COLORS];
        for (slot, &reg) in colors.iter_mut().zip(indices) {
            *slot = palette[reg];
        }

        self.sample_coco_graphics(&mode);

        // The legacy graphics border is not black: green (CSS=0) or buff
        // (CSS=1); CoCo 3 stays fixed black.
        let border = match self.config.variant {
            MachineVariant::Coco3 => self.bus.gime.color(TEXT_BORDER_COLOR),
            MachineVariant::Coco1 | MachineVariant::Coco2 => {
                video::VDG_FIXED_PALETTE[video::vdg_graphics_border_index(css_bit)]
            }
        };
        let colors = &colors[..indices.len()];
        video::render_sampled_graphics(
            &self.graphics_scratch,
            &mode,
            colors,
            border,
            &mut self.framebuffer,
        );
        if let Some(phase) = self.coco12_rg6_artifact_phase(&mode, css_bit) {
            self.artifact_coco12_rg6_field(colors, border, phase);
        }
    }

    fn artifact_coco12_rg6_field(
        &mut self,
        colors: &[[u8; 4]],
        border: [u8; 4],
        phase: video::RG6ArtifactPhase,
    ) {
        let decoder = video::RG6ArtifactDecoder::new([colors[0], colors[1]], phase);
        let edges = video::RG6ArtifactEdges::solid(border == colors[1]);
        for y in 0..video::ACTIVE_H {
            let data_start = y * video::RG6_BYTES_PER_LINE;
            let row_data = self.graphics_scratch[data_start..][..video::RG6_BYTES_PER_LINE]
                .try_into()
                .expect("RG6 row has fixed width");
            let output = &mut self.framebuffer[video::active_row_range(y)];
            decoder.decode_scanline(row_data, edges, video::VDG_XSCALE, output);
        }
    }

    /// Resolve each MC6847 request to the physical RAM address emitted by the
    /// discrete MC6883. CoCo 3 never reaches this field-end rendering path.
    fn sample_coco_graphics(&mut self, mode: &video::VDGGraphicsMode) {
        let video_mode = self.bus.sam.v_bits();
        let mut stream =
            crate::sam::SAMVideoAddressStream::new(self.bus.sam.display_base() as u16, video_mode);
        self.graphics_scratch
            .resize(video::ACTIVE_H * mode.bytes_per_row, 0);

        let lines_per_row = video::ACTIVE_H / mode.rows;
        for line in 0..video::ACTIVE_H {
            let logical_base = line / lines_per_row * mode.bytes_per_row;
            let data_base = line * mode.bytes_per_row;
            for sample in 0..mode.bytes_per_row {
                let address = stream.sample(logical_base + sample);
                self.graphics_scratch[data_base + sample] = self.bus.sam_video_read(address);
            }
            stream.horizontal_sync();
        }
    }
}

#[cfg(test)]
#[path = "render_test.rs"]
mod tests;
