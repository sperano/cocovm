use super::*;
use crate::config::{MachineConfig, MemorySize};
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
fn coco3_rgb_and_pal_rg6_remain_unartifacted() {
    for (video_standard, monitor) in [
        (VideoStandard::NTSC, MonitorType::RGB),
        (VideoStandard::PAL, MonitorType::Composite),
    ] {
        let mut machine = parked_coco3(video_standard, monitor, RG6_CSS1);
        let top = seed_pattern(&mut machine);
        let line = render_pattern_line(&mut machine, top);
        assert!(
            line.chunks_exact(BYTES_PER_PIXEL)
                .all(|pixel| is_grey(pixel.try_into().expect("RGBA pixel"))),
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
        line.chunks_exact(BYTES_PER_PIXEL)
            .all(|pixel| is_grey(pixel.try_into().expect("RGBA pixel")))
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
            line.chunks_exact(BYTES_PER_PIXEL)
                .all(|pixel| is_grey(pixel.try_into().expect("RGBA pixel")))
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
