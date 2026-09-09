//! Run EOU's original DMP-105 VEF picture printer through the serial port.
//! Requires the optional real ROM/EOU assets and Toolshed's `os9` executable.

use std::path::Path;
use std::process::Command;

use coco_core::printer::{X_UNITS_PER_INCH, Y_UNITS_PER_INCH};

mod printer_boot;
use printer_boot::*;

const IMAGE_WIDTH: usize = 640;
const IMAGE_HEIGHT: usize = 200;
const PALETTE_SIZE: usize = 16;
const VEF_HEADER_SIZE: usize = 2 + PALETTE_SIZE;
const VEF_MONOCHROME_TYPE: u8 = 4;
const WHITE: u8 = 0x3F;
const MAX_COMMAND_FIELDS: usize = 6_000;
const MAX_PRINT_FIELDS: usize = 120_000;

/// Two separated black rectangles on white, authored for this test.
fn test_picture() -> Vec<u8> {
    let mut bytes = vec![0; VEF_HEADER_SIZE + IMAGE_WIDTH * IMAGE_HEIGHT / 8];
    bytes[1] = VEF_MONOCHROME_TYPE;
    bytes[2] = WHITE;
    for y in 0..IMAGE_HEIGHT {
        for x in 0..IMAGE_WIDTH {
            if ((64..128).contains(&x) && (16..48).contains(&y))
                || ((320..448).contains(&x) && (96..160).contains(&y))
            {
                bytes[VEF_HEADER_SIZE + y * IMAGE_WIDTH / 8 + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    bytes
}

fn install_picture(vhd: &Path) {
    let picture = vhd.with_extension("vef");
    std::fs::write(&picture, test_picture()).unwrap();
    copy_file(
        picture.to_str().unwrap(),
        &format!("{},printer-test.vef", vhd.display()),
    );
    std::fs::remove_file(picture).unwrap();
    prepare_legacy_printer_environment(vhd);
}

fn copy_file(source: &str, destination: &str) {
    let result = Command::new("os9")
        .args(["copy", "-r", source, destination])
        .output()
        .expect("run Toolshed os9 copy");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn prepare_legacy_printer_environment(vhd: &Path) {
    // This old PrtDmp misparses the comments in EOU's extended env.file and
    // silently opens a fragment of a comment as its printer path. Putting
    // PRPORT first lets it stop parsing, while preserving every boot setting.
    let temporary = vhd.with_extension("env");
    let target = format!("{},SYS/env.file", vhd.display());
    copy_file(&target, temporary.to_str().unwrap());
    let mut contents = b"PRPORT=/p\r".to_vec();
    contents.extend(std::fs::read(&temporary).unwrap());
    std::fs::write(&temporary, contents).unwrap();
    copy_file(temporary.to_str().unwrap(), &target);
    std::fs::remove_file(temporary).unwrap();
}

#[test]
fn eou_dmp105_picture_printer_preserves_two_rectangle_geometry() {
    if Command::new("os9").arg("-h").output().is_err() {
        eprintln!("skipping EOU picture-print test: Toolshed os9 is unavailable");
        return;
    }
    let Some((mut machine, scratch)) = boot_eou_shell("dmp105-graphics", install_picture) else {
        return;
    };
    let paper = machine.bus.bitbanger.start_dmp105();
    machine.bus.bitbanger.set_bit_period(OS9_PRINTER_BIT_PERIOD);
    let baseline = shell_prompt_count(&mut machine);
    type_str(&mut machine, "load /dd/prtdmps/prtdmp.dmp105\r");
    wait_for_new_shell_prompt(&mut machine, baseline, MAX_COMMAND_FIELDS);
    let baseline = shell_prompt_count(&mut machine);
    type_str(&mut machine, "prtdmp </dd/printer-test.vef\r");
    let screen = wait_for_new_shell_prompt(&mut machine, baseline, MAX_PRINT_FIELDS);
    assert_eq!(machine.bus.bitbanger.framing_errors(), 0, "{screen}");
    let dots = paper.dots_in_range(0, u32::MAX);
    const EXPECTED_PRINTER_BYTES: u64 = 46_464;
    assert_eq!(machine.bus.bitbanger.bytes_out(), EXPECTED_PRINTER_BYTES);
    assert_picture_geometry(&dots);
    drop(machine);
    std::fs::remove_file(scratch).unwrap();
}

fn assert_picture_geometry(dots: &[(u32, u32)]) {
    // PrtDmp expands 640 source pixels to 800 condensed columns and doubles
    // the 200 source rows; physical output is 100 columns/inch and72rows/inch.
    const COLUMN_UNITS: u32 = X_UNITS_PER_INCH / 100;
    const ROW_UNITS: u32 = Y_UNITS_PER_INCH / 72;
    assert!(!dots.is_empty(), "picture printer produced no ink");
    let mut first = 0;
    let mut second = 0;
    for &(x, y) in dots {
        assert_eq!(x % COLUMN_UNITS, 0);
        assert_eq!(y % ROW_UNITS, 0);
        let column = x / COLUMN_UNITS;
        let row = y / ROW_UNITS;
        if (80..160).contains(&column) && (32..96).contains(&row) {
            first += 1;
        } else if (400..560).contains(&column) && (192..320).contains(&row) {
            second += 1;
        } else {
            panic!("unexpected ink at column{column}, row{row}");
        }
    }
    assert_eq!(first, 80 * 64);
    assert_eq!(second, 160 * 128);
}
