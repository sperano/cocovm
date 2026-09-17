//! Coverage for `Machine::active_rect` — the active (non-border) picture
//! rectangle over which the frontend maps pointer positions for mouse-as-joystick.
//! Mirrors the geometry the renderers themselves paint: the fixed non-wide
//! 192-line window on CoCo 1/2 (`video::VDG_ACTIVE_TOP`/`ACTIVE_H`), and on
//! CoCo 3 `gime_video::active_span`'s wide/non-wide split plus
//! `active_rows`'s LPF placement — the same helpers that the renderers use. The corresponding
//! renderer tests are in `render_gime.rs` and `render.rs`.
//!
//! CPU execution is not needed because `active_rect` only reads live
//! `MachineConfig` and `GIME` state. These `Machine`s therefore run a zeroed
//! synthetic ROM, as `render.rs`'s `text_renderer_follows_sam_page_register`
//! test does.

use coco_core::gime::init0;
use coco_core::{
    ActiveRect, Machine, MachineConfig, MachineVariant, MemorySize, VDGVariant, VideoStandard,
};

/// $FF99 HRES=%100 (64-column text): non-wide, 512 px active span with 64 px
/// side borders (same encoding `render_gime.rs`'s `VRES_TEXT64` uses).
const VRES_NON_WIDE: u8 = 0x10;
/// $FF99 HRES=%101 (80-column text): wide, full 640 px active span, no
/// side borders (same encoding `render_gime.rs`'s `VRES_TEXT80_ATTR` uses,
/// minus its attributes bit — irrelevant to geometry).
const VRES_WIDE: u8 = 0x14;

/// $FF99 LPF=%00: 192-line body, 25 top border rows.
const LPF_192: u8 = 0x00;
/// $FF99 LPF=%10: the glitched line count real silicon mangles; this crate
/// approximates it as a centered 210-line body (`raster::vertical_window`),
/// so 15 top border rows.
const LPF_GLITCHED_210: u8 = 0x40;
/// $FF99 LPF=%11: 225-line body, 8 top border rows.
const LPF_225: u8 = 0x60;

/// The non-wide span: 512 px at x0=64 (`raster::NON_WIDE_BORDER_X`/
/// `NON_WIDE_ACTIVE_W`), with the 192-line LPF window — the CoCo 3's
/// legacy placement and the CoCo 1/2's only one.
const NON_WIDE_LPF_192_RECT: ActiveRect = ActiveRect {
    x: 64,
    y: 25,
    width: 512,
    height: 192,
};

fn coco3_machine() -> Machine {
    Machine::new(
        MachineConfig::default(), // Coco3/NTSC/K512/RGB — see `config.rs`'s `Default` impl.
        vec![0u8; 32 * 1024].into_boxed_slice(),
    )
}

#[test]
fn coco2_shares_the_coco3_legacy_active_rect() {
    let config = MachineConfig {
        variant: MachineVariant::Coco2,
        video: VideoStandard::NTSC,
        memory: MemorySize::K64,
        monitor: None,
        vdg: Some(VDGVariant::MC6847),
    };
    config.validate().expect("Coco2/Ntsc/K64 is valid");
    let m = Machine::new(config, vec![0u8; 16 * 1024].into_boxed_slice());
    assert_eq!(m.active_rect(), NON_WIDE_LPF_192_RECT);
}

#[test]
fn coco3_legacy_field_is_non_wide_even_with_the_wide_bit_set() {
    let mut m = coco3_machine();
    // INIT0 COCO=1 selects a legacy VDG-compatible field. The wide HRES bit is
    // set deliberately, so this verifies that legacy mode ignores it instead
    // of taking the non-wide branch only because HRES is zero. LPF still
    // applies in legacy modes, as described by `paint_legacy_scanline`.
    m.bus.gime.write_init0(init0::COCO);
    m.bus.gime.vres = VRES_WIDE | LPF_192;
    assert_eq!(m.active_rect(), NON_WIDE_LPF_192_RECT);
}

#[test]
fn coco3_gime_native_wide_fills_the_full_canvas_width() {
    let mut m = coco3_machine();
    m.bus.gime.write_init0(0); // INIT0 COCO=0: GIME-native.
    m.bus.gime.vres = VRES_WIDE | LPF_192;
    assert_eq!(
        m.active_rect(),
        ActiveRect {
            x: 0,
            y: 25,
            width: 640,
            height: 192,
        }
    );
}

#[test]
fn coco3_gime_native_non_wide_leaves_side_borders() {
    let mut m = coco3_machine();
    m.bus.gime.write_init0(0);
    m.bus.gime.vres = VRES_NON_WIDE | LPF_192;
    assert_eq!(m.active_rect(), NON_WIDE_LPF_192_RECT);
}

#[test]
fn coco3_lpf_225_moves_the_vertical_window() {
    let mut m = coco3_machine();
    m.bus.gime.write_init0(0);
    m.bus.gime.vres = VRES_NON_WIDE | LPF_225;
    assert_eq!(
        m.active_rect(),
        ActiveRect {
            x: 64,
            y: 8,
            width: 512,
            height: 225,
        }
    );
}

#[test]
fn coco3_glitched_lpf_uses_the_centered_210_line_approximation() {
    let mut m = coco3_machine();
    m.bus.gime.write_init0(0);
    m.bus.gime.vres = VRES_NON_WIDE | LPF_GLITCHED_210;
    assert_eq!(
        m.active_rect(),
        ActiveRect {
            x: 64,
            y: 15,
            width: 512,
            height: 210,
        }
    );
}
