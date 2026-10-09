use super::*;
use crate::config::{MachineConfig, MemorySize, VDGVariant};
use crate::gime::init0;
use crate::keyboard;
use crate::video::{VDG_AG, VDG_CSS};
use mc6809::Bus;

const ROM_SIZE: usize = 32 * 1024;
const RG6_MODE_BITS: u8 = 7 << 4;
const RG3_MODE_BITS: u8 = 5 << 4;
const RG6_CSS1: u8 = VDG_AG | VDG_CSS | RG6_MODE_BITS;
const RG3_CSS1: u8 = VDG_AG | VDG_CSS | RG3_MODE_BITS;
const RG6_C0_INDEX: usize = 10;
const RG6_C1_INDEX: usize = 11;
const BLACK6: u8 = 0x00;
const WHITE6: u8 = 0x3f;
const FIRST_PATTERN: u8 = 0x55;
const SECOND_PATTERN: u8 = 0xaa;
const COCO12_ROM_SIZE: usize = 16 * 1024;
const COCO12_SCREEN_BASE: u16 = 0x0400;
const SAM_F1_SET: u16 = 0xffc9;
const SAM_V1_SET: u16 = 0xffc3;
const SAM_V2_SET: u16 = 0xffc5;
const PHASE_SEARCH_RESETS: usize = 16;

fn config(video_standard: VideoStandard, monitor: MonitorType) -> MachineConfig {
    MachineConfig {
        variant: MachineVariant::Coco3,
        video: video_standard,
        memory: MemorySize::K512,
        monitor: Some(monitor),
        vdg: None,
    }
}

fn parked_coco3(video_standard: VideoStandard, monitor: MonitorType, ff22: u8) -> Machine {
    let mut machine = Machine::new(
        config(video_standard, monitor),
        vec![0; ROM_SIZE].into_boxed_slice(),
    );
    machine.bus.gime.write_init0(init0::COCO);
    machine.bus.pia1.b.output = ff22;
    machine.bus.gime.palette[RG6_C0_INDEX] = BLACK6;
    machine.bus.gime.palette[RG6_C1_INDEX] = WHITE6;
    machine.bus.write(0x0000, 0x20);
    machine.bus.write(0x0001, 0xfe);
    machine
}

fn seed_pattern(machine: &mut Machine) -> usize {
    machine.line = 0;
    machine.render_scanline();
    let row_base = machine
        .field_scan
        .as_ref()
        .expect("line zero latches field scan")
        .row_base;
    let half = video::RG6_BYTES_PER_LINE / 2;
    for offset in 0..video::RG6_BYTES_PER_LINE {
        let value = if offset < half {
            FIRST_PATTERN
        } else {
            SECOND_PATTERN
        };
        machine.bus.write((row_base + offset) as u16, value);
    }
    assert_eq!(machine.bus.read(row_base as u16), FIRST_PATTERN);
    gime_video::active_rows(&machine.bus.gime).0
}

fn render_pattern_line(machine: &mut Machine, row: usize) -> Vec<u8> {
    machine.line = row as u32;
    machine.render_scanline();
    let start = (row * raster::CANVAS_W + raster::NON_WIDE_BORDER_X) * BYTES_PER_PIXEL;
    let len = raster::NON_WIDE_ACTIVE_W * BYTES_PER_PIXEL;
    machine.framebuffer[start..start + len].to_vec()
}

fn logical_pixel(line: &[u8], x: usize) -> [u8; BYTES_PER_PIXEL] {
    let scaled_x = x * 2;
    line[scaled_x * BYTES_PER_PIXEL..][..BYTES_PER_PIXEL]
        .try_into()
        .expect("RGBA pixel")
}

fn is_grey(color: [u8; BYTES_PER_PIXEL]) -> bool {
    color[0] == color[1] && color[1] == color[2]
}

fn coco12_config(variant: MachineVariant, vdg: VDGVariant) -> MachineConfig {
    MachineConfig {
        variant,
        video: VideoStandard::NTSC,
        memory: match variant {
            MachineVariant::Coco1 => MemorySize::K32,
            MachineVariant::Coco2 => MemorySize::K64,
            MachineVariant::Coco3 => unreachable!("CoCo 3 has no VDG"),
        },
        monitor: None,
        vdg: Some(vdg),
    }
}

fn parked_coco12(variant: MachineVariant, vdg: VDGVariant, seed: u64) -> Machine {
    let mut machine = Machine::new_with_artifact_seed(
        coco12_config(variant, vdg),
        vec![0; COCO12_ROM_SIZE].into_boxed_slice(),
        seed,
    );
    machine.bus.pia1.b.output = RG6_CSS1;
    machine.bus.write(SAM_F1_SET, 0);
    machine.bus.write(SAM_V1_SET, 0);
    machine.bus.write(SAM_V2_SET, 0);
    machine
}

fn fill_coco12_pattern(machine: &mut Machine, first_half: u8, second_half: u8) {
    let bytes_per_row = video::RG6_BYTES_PER_LINE;
    for y in 0..video::ACTIVE_H {
        for x in 0..bytes_per_row {
            let value = if x < bytes_per_row / 2 {
                first_half
            } else {
                second_half
            };
            machine
                .bus
                .write(COCO12_SCREEN_BASE + (y * bytes_per_row + x) as u16, value);
        }
    }
}

fn coco12_pixel(machine: &Machine, x: usize, y: usize) -> [u8; BYTES_PER_PIXEL] {
    let offset = video::active_row_range(y).start + x * video::VDG_XSCALE * BYTES_PER_PIXEL;
    machine.framebuffer[offset..offset + BYTES_PER_PIXEL]
        .try_into()
        .expect("RGBA pixel")
}

#[test]
fn coco3_composite_rg6_uses_live_bpi_phase_per_scanline() {
    let mut machine = parked_coco3(VideoStandard::NTSC, MonitorType::Composite, RG6_CSS1);
    let top = seed_pattern(&mut machine);
    let standard = render_pattern_line(&mut machine, top);
    machine.bus.write(0xff98, vmode::BPI);
    let reverse = render_pattern_line(&mut machine, top + 1);
    let first_x = video::RG6_PIXELS_PER_LINE / 4;
    let second_x = first_x + video::RG6_PIXELS_PER_LINE / 2;

    let standard_first = logical_pixel(&standard, first_x);
    let standard_second = logical_pixel(&standard, second_x);
    assert!(
        !is_grey(standard_first),
        "first alternating region rendered {standard_first:?}"
    );
    assert!(
        !is_grey(standard_second),
        "second alternating region rendered {standard_second:?}"
    );
    assert_ne!(standard_first, standard_second);
    assert_eq!(standard_first, logical_pixel(&reverse, second_x));
    assert_eq!(standard_second, logical_pixel(&reverse, first_x));
}

#[test]
fn coco1_and_coco2_vdgs_render_rg6_artifact_colors() {
    let variants = [
        (MachineVariant::Coco1, VDGVariant::MC6847),
        (MachineVariant::Coco2, VDGVariant::MC6847),
        (MachineVariant::Coco2, VDGVariant::MC6847T1),
    ];
    for (variant, vdg) in variants {
        let mut machine = parked_coco12(variant, vdg, 0);
        fill_coco12_pattern(&mut machine, SECOND_PATTERN, FIRST_PATTERN);
        machine.render_field();
        let first = coco12_pixel(&machine, video::RG6_PIXELS_PER_LINE / 4, 0);
        let second = coco12_pixel(&machine, video::RG6_PIXELS_PER_LINE * 3 / 4, 0);
        let base_colors = [
            video::VDG_FIXED_PALETTE[RG6_C0_INDEX],
            video::VDG_FIXED_PALETTE[RG6_C1_INDEX],
        ];

        assert!(
            !base_colors.contains(&first),
            "{variant:?} {vdg:?} first region"
        );
        assert!(
            !base_colors.contains(&second),
            "{variant:?} {vdg:?} second region"
        );
        assert_ne!(first, second, "{variant:?} {vdg:?} complementary regions");

        fill_coco12_pattern(&mut machine, 0xff, 0x00);
        machine.render_field();
        assert_eq!(
            coco12_pixel(
                &machine,
                video::RG6_PIXELS_PER_LINE / 4,
                video::ACTIVE_H - 1
            ),
            base_colors[1]
        );
        assert_eq!(
            coco12_pixel(
                &machine,
                video::RG6_PIXELS_PER_LINE * 3 / 4,
                video::ACTIVE_H - 1
            ),
            base_colors[0]
        );
    }
}

#[test]
fn coco2_reset_can_swap_rg6_artifact_colors() {
    let mut machine = parked_coco12(MachineVariant::Coco2, VDGVariant::MC6847, 0);
    fill_coco12_pattern(&mut machine, SECOND_PATTERN, FIRST_PATTERN);
    machine.render_field();
    let sample_x = video::RG6_PIXELS_PER_LINE / 4;
    let before = coco12_pixel(&machine, sample_x, 0);
    let initial_phase = machine.ntsc_rg6_artifact_phase();
    for _ in 0..PHASE_SEARCH_RESETS {
        machine.reset();
        if machine.ntsc_rg6_artifact_phase() != initial_phase {
            break;
        }
    }
    assert_ne!(machine.ntsc_rg6_artifact_phase(), initial_phase);
    machine.render_field();
    let after = coco12_pixel(&machine, sample_x, 0);

    assert_ne!(before, after);
}

#[test]
fn coco2_rg6_css_zero_keeps_verified_nonburst_colors() {
    const RG6_CSS0: u8 = VDG_AG | RG6_MODE_BITS;
    let mut machine = parked_coco12(MachineVariant::Coco2, VDGVariant::MC6847, 0);
    machine.bus.pia1.b.output = RG6_CSS0;
    fill_coco12_pattern(&mut machine, SECOND_PATTERN, FIRST_PATTERN);
    machine.render_field();
    let base_colors = [video::VDG_FIXED_PALETTE[8], video::VDG_FIXED_PALETTE[9]];

    for x in 0..video::RG6_PIXELS_PER_LINE {
        assert!(base_colors.contains(&coco12_pixel(&machine, x, 0)));
    }
}

#[test]
fn coco3_rgb_and_pal_rg6_remain_unartifacted() {
    for (video_standard, monitor) in [
        (VideoStandard::NTSC, MonitorType::RGB),
        (VideoStandard::PAL, MonitorType::Composite),
    ] {
        let mut machine = parked_coco3(video_standard, monitor, RG6_CSS1);
        let top = seed_pattern(&mut machine);
        let line = render_pattern_line(&mut machine, top);
        assert!(
            line.as_chunks::<BYTES_PER_PIXEL>()
                .0
                .iter()
                .all(|pixel| is_grey(*pixel)),
            "{video_standard:?} {monitor:?} must retain the base RG6 colors"
        );
    }
}

#[test]
fn coco3_non_rg6_legacy_graphics_remain_unartifacted() {
    let mut machine = parked_coco3(VideoStandard::NTSC, MonitorType::Composite, RG3_CSS1);
    let top = seed_pattern(&mut machine);
    let line = render_pattern_line(&mut machine, top);

    assert!(
        line.as_chunks::<BYTES_PER_PIXEL>()
            .0
            .iter()
            .all(|pixel| is_grey(*pixel))
    );
}

#[test]
fn coco3_rgb_rg6_is_byte_identical_across_bpi_changes() {
    let mut machine = parked_coco3(VideoStandard::NTSC, MonitorType::RGB, RG6_CSS1);
    let top = seed_pattern(&mut machine);
    let standard = render_pattern_line(&mut machine, top);
    machine.bus.write(0xff98, vmode::BPI);
    let reverse = render_pattern_line(&mut machine, top + 1);

    assert_eq!(standard, reverse);
}

#[test]
fn coco3_gime_native_modes_remain_unartifacted() {
    for native_mode in [vmode::BPI, vmode::BPI | vmode::BP] {
        let mut machine = parked_coco3(VideoStandard::NTSC, MonitorType::Composite, RG6_CSS1);
        machine.bus.gime.write_init0(0);
        machine.bus.gime.vmode = native_mode;
        machine.bus.gime.palette[0] = BLACK6;
        machine.bus.gime.palette[1] = WHITE6;
        machine.line = 0;
        machine.render_scanline();
        machine.bus.ram[..video::RG6_BYTES_PER_LINE].fill(FIRST_PATTERN);
        let top = gime_video::active_rows(&machine.bus.gime).0;
        machine.line = top as u32;
        machine.render_scanline();
        let (active_x, active_w) = gime_video::active_span(&machine.bus.gime);
        let start = (top * raster::CANVAS_W + active_x) * BYTES_PER_PIXEL;
        let line = &machine.framebuffer[start..start + active_w * BYTES_PER_PIXEL];

        assert!(
            line.as_chunks::<BYTES_PER_PIXEL>()
                .0
                .iter()
                .all(|pixel| is_grey(*pixel))
        );
    }
}

#[test]
fn coco3_rom_f1_reset_selects_reverse_bpi() {
    const BOOT_FIELDS: usize = 300;
    let path = test_assets::rom(test_assets::rom::COCO3);
    let Ok(rom) = std::fs::read(&path) else {
        eprintln!(
            "skipping F1 artifact-phase boot test: {} is unavailable",
            path.display()
        );
        return;
    };
    let mut machine = Machine::new(
        config(VideoStandard::NTSC, MonitorType::Composite),
        rom.into_boxed_slice(),
    );
    machine.bus.keyboard.set(keyboard::F1, true);
    machine.reset();
    for _ in 0..BOOT_FIELDS {
        machine.run_field();
    }
    machine.bus.keyboard.set(keyboard::F1, false);

    assert_ne!(machine.bus.gime.vmode & vmode::BPI, 0);
}
