//! Integration: boot Disk Extended Color BASIC and read a synthesized RS-DOS
//! directory via DIR.

use coco_core::fdc::{DiskCart, JVCDisk};
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

use super::common::{
    boot_machine, load_rom, screen_row, synthesized_ml_disk, synthesized_rsdos_disk, tap_char,
    try_load_disk, try_load_rom, type_str, SECTOR_SIZE,
};

#[test]
fn boots_to_disk_basic_and_dir_lists_the_synthesized_file() {
    const FIELDS: usize = 400;
    let mut m = boot_machine();
    let mut cart = DiskCart::new(load_rom("disk11.rom"));
    cart.insert_disk(0, synthesized_rsdos_disk("HELLO", "BAS"));
    m.insert_cartridge(cart);
    m.reset();
    for _ in 0..FIELDS {
        m.run_field();
    }
    let banner = (0..16).any(|r| screen_row(&mut m, r).contains("DISK EXTENDED COLOR BASIC"));
    assert!(banner, "expected the Disk BASIC banner; row0 = {:?}", screen_row(&mut m, 0));

    type_str(&mut m, "DIR");
    tap_char(&mut m, '\r');
    for _ in 0..FIELDS {
        m.run_field();
    }

    let found = (0..16).any(|r| screen_row(&mut m, r).contains("HELLO"));
    assert!(
        found,
        "expected DIR to list HELLO.BAS; screen:\n{}",
        (0..16).map(|r| screen_row(&mut m, r)).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn boots_to_disk_basic_without_a_disk_inserted() {
    const FIELDS: usize = 400;
    let mut m = boot_machine();
    m.insert_cartridge(DiskCart::new(load_rom("disk11.rom")));
    m.reset();
    for _ in 0..FIELDS {
        m.run_field();
    }
    let banner = (0..16).any(|r| screen_row(&mut m, r).contains("DISK EXTENDED COLOR BASIC"));
    assert!(banner, "expected the Disk BASIC banner even with no disk mounted (no spurious HALT)");
}

/// Hot-plugging the controller into a machine already sitting at the BASIC
/// prompt must power-cycle to take effect: the DK probe that links Disk
/// BASIC only runs on the ROM's cold-start path, and a warm reset skips it
/// because BASIC's warm-start magic is still in RAM (the frontend's
/// Insert/New Disk menu path relies on this).
#[test]
fn controller_added_mid_session_boots_disk_basic_after_power_cycle() {
    const FIELDS: usize = 400;
    let mut m = boot_machine();
    m.reset();
    for _ in 0..FIELDS {
        m.run_field();
    }
    let plain = (0..16).any(|r| screen_row(&mut m, r).contains("COLOR BASIC"))
        && !(0..16).any(|r| screen_row(&mut m, r).contains("DISK"));
    assert!(plain, "precondition: booted to non-disk BASIC");

    m.insert_cartridge(DiskCart::new(load_rom("disk11.rom")));
    m.power_cycle();
    for _ in 0..FIELDS {
        m.run_field();
    }
    let banner = (0..16).any(|r| screen_row(&mut m, r).contains("DISK EXTENDED COLOR BASIC"));
    assert!(banner, "expected Disk BASIC after mid-session insert + power cycle");
}

/// Regression: the FD-502 halt/NMI handshake must not drop a sector's final
/// byte. Before the fix, the sector-completion NMI preempted DSKCON's
/// `STB ,X+` for the last byte of every 256-byte sector — the MC6809
/// recognizes interrupts only at instruction-end boundaries, so the
/// instruction that HALT* released must run before the NMI is acknowledged.
/// One byte per sector was loading as $00. The payload here fills three
/// sectors with a non-zero marker; every byte, including each sector's 256th,
/// must survive LOADM.
#[test]
fn loadm_preserves_every_sector_byte_across_the_halt_nmi_handshake() {
    const LOAD_ADDR: u16 = 0x3F00;
    const DATA_LEN: usize = 600; // spans 3 sectors -> 2 sector-boundary bytes in payload
    const FILL: u8 = 0xE5;
    const FIELDS: usize = 400;

    let (Some(coco), Some(disk_rom)) = (try_load_rom("coco3.rom"), try_load_rom("disk11.rom"))
    else {
        eprintln!("skipping loadm_preserves_every_sector_byte: roms/ assets not present");
        return;
    };

    let data = vec![FILL; DATA_LEN];
    let mut m = Machine::new(MachineConfig::default(), coco);
    let mut cart = DiskCart::new(disk_rom);
    cart.insert_disk(0, synthesized_ml_disk("TESTML", LOAD_ADDR, &data));
    m.insert_cartridge(cart);
    m.reset();
    for _ in 0..FIELDS {
        m.run_field();
    }
    assert!(
        (0..16).any(|r| screen_row(&mut m, r).contains("DISK EXTENDED COLOR BASIC")),
        "expected the Disk BASIC banner before LOADM"
    );

    type_str(&mut m, "LOADM\"TESTML\"");
    tap_char(&mut m, '\r');
    for _ in 0..FIELDS {
        m.run_field();
    }

    let loaded: Vec<u8> = (0..DATA_LEN as u16)
        .map(|i| m.bus.read(LOAD_ADDR + i))
        .collect();
    if let Some(i) = loaded.iter().position(|&b| b != FILL) {
        panic!(
            "byte at {:#06X} loaded as {:#04X}, expected {:#04X} — a sector's final byte was dropped",
            LOAD_ADDR + i as u16,
            loaded[i],
            FILL
        );
    }
}

/// End-to-end: boot Disk BASIC with a completely blank (0-byte) image
/// mounted, run `DSKINI0` to format it in-emulator (exercising the WD1773's
/// new Write Track MFM parser through the real ROM, not a synthetic stream),
/// then confirm the image came out as a full 35-track RS-DOS disk filled with
/// DSKINI's $FF fill byte, and that a subsequent `DIR` doesn't choke on it.
#[test]
fn dskini_formats_a_blank_disk_and_dir_reports_no_io_error() {
    /// DSKINI formats all 35 tracks (18 sectors each); this needs far more
    /// DRQ-paced field budget than the ~400-field boot/DIR tests above.
    const FORMAT_FIELDS: usize = 4000;
    const BOOT_FIELDS: usize = 400;
    const DIR_FIELDS: usize = 400;

    let (Some(coco), Some(disk_rom)) = (try_load_rom("coco3.rom"), try_load_rom("disk11.rom"))
    else {
        eprintln!("skipping dskini_formats_a_blank_disk_and_dir_reports_no_io_error: roms/ assets not present");
        return;
    };

    let mut m = Machine::new(MachineConfig::default(), coco);
    let mut cart = DiskCart::new(disk_rom);
    cart.insert_disk(0, JVCDisk::from_bytes(Vec::new()).unwrap()); // blank, 0 tracks
    m.insert_cartridge(cart);
    m.reset();
    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }
    assert!(
        (0..16).any(|r| screen_row(&mut m, r).contains("DISK EXTENDED COLOR BASIC")),
        "expected the Disk BASIC banner before DSKINI"
    );

    type_str(&mut m, "DSKINI0");
    tap_char(&mut m, '\r');
    for _ in 0..FORMAT_FIELDS {
        m.run_field();
    }

    const EXPECTED_LEN: usize = 35 * 18 * SECTOR_SIZE;
    {
        let cart = m.bus.cart.as_disk_cart().expect("disk controller still inserted");
        let disk = cart.disk(0).expect("drive 0 still mounted");
        assert_eq!(disk.bytes().len(), EXPECTED_LEN, "formatted image must be a full 35-track disk");

        // Sample track 5 sector 1's data region: DSKINI fills every sector
        // with $FF.
        let off = disk.sector_offset(5, 0, 1).expect("track 5 sector 1 must exist after formatting");
        let sample = disk.read_bytes(off, SECTOR_SIZE);
        assert!(sample.iter().all(|&b| b == 0xFF), "DSKINI must fill every sector with $FF");
    }

    type_str(&mut m, "DIR");
    tap_char(&mut m, '\r');
    for _ in 0..DIR_FIELDS {
        m.run_field();
    }
    let screen: Vec<String> = (0..16).map(|r| screen_row(&mut m, r)).collect();
    assert!(
        !screen.iter().any(|row| row.contains("?IO ERROR")),
        "DIR must not report an IO error after DSKINI; screen:\n{}",
        screen.join("\n")
    );
}

/// End-to-end regression that NitrOS-9 Level 2 boots all the way to its shell
/// prompt on the real 40-track two-sided image, booted via Disk BASIC's `DOS`
/// command. Two FD-502 bugs each stalled this boot:
///
/// - Read Sector first-byte latency: `boot_1773` arms its HALT/NMI collection
///   loop only after a short post-command `Delay2`; a first byte paced at one
///   DRQ interval landed inside that delay, tripped LOST DATA, and printed
///   FAILED after the "NITROS9 BOOT" banner.
/// - Read Sector side-at-dispatch: the RBF driver flips DSKREG to the next side
///   *after* writing the Read Sector command, so sampling the side at dispatch
///   read the wrong physical side across every side boundary and corrupted
///   `rb1773`'s body — the console (`/Term`) attach then wedged the boot at the
///   "NITROS9 BOOT" banner forever (see
///   `read_sector_samples_side_when_the_data_field_streams_not_at_dispatch`).
///
/// Reaching the shell's `Time ?` startup prompt proves the whole boot module
/// set loaded and linked and the console attach completed. Skips when the
/// git-ignored ROM or disk assets aren't present.
#[test]
fn nitros9_l2_boot_reaches_shell_prompt() {
    const BOOT_FIELDS: usize = 300;
    const SETTLE_FIELDS: usize = 1500;
    const DISK: &str = "NOS9_6809_L2_v030300_coco3_40d_1.dsk";

    let (Some(coco), Some(disk_rom), Some(dsk)) = (
        try_load_rom("coco3.rom"),
        try_load_rom("disk11.rom"),
        try_load_disk(DISK),
    ) else {
        eprintln!("skipping nitros9_l2_boot_reaches_shell_prompt: roms/ or disks/ assets not present");
        return;
    };

    let mut m = Machine::new(MachineConfig::default(), coco);
    let mut cart = DiskCart::new(disk_rom);
    cart.insert_disk(0, JVCDisk::from_bytes(dsk).unwrap());
    m.insert_cartridge(cart);
    m.reset();
    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }
    type_str(&mut m, "DOS");
    tap_char(&mut m, '\r');
    for _ in 0..SETTLE_FIELDS {
        m.run_field();
    }

    // OS-9 switches to the GIME hi-res text screen; text_screen_lines() reads it.
    let screen = m.text_screen_lines().join("\n");
    assert!(
        !screen.contains("FAILED"),
        "NitrOS-9 boot must not report FAILED; screen:\n{screen}"
    );
    assert!(
        screen.contains("NitrOS-9") && screen.contains("Level 2"),
        "expected the NitrOS-9 Level 2 banner (console attach completed); screen:\n{screen}"
    );
    assert!(
        screen.contains("Time ?"),
        "expected the shell startup script's 'Time ?' prompt (full boot to shell); screen:\n{screen}"
    );
}
