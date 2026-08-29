//! End-to-end regression: NitrOS-9 EOU (Ease of Use, 6809 Level 2) boots from
//! the emudsk VHD interface all the way to a usable shell. The ignored
//! assets are the EOU 1.0.1 emulator pair: `disks/68EMU.dsk` (boot floppy
//! whose OS9Boot carries the EmuDsk driver and `/h0` descriptors) and
//! `disks/68SDC.VHD` (the 128MB system image the startup script runs from).
//! Skips when any asset is absent.

use std::path::PathBuf;

use coco_core::cart::MultiPak;
use coco_core::fdc::{DiskCart, JVCDisk};
use coco_core::vhd::VHDImage;
use coco_core::{Machine, MachineConfig};
use test_assets::{
    disk::{EOU_BOOT, EOU_SYSTEM_VHD},
    rom::{COCO3, DISK11},
};

fn tap(m: &mut Machine, pos: (u8, u8)) {
    for _ in 0..3 {
        m.bus.keyboard.set(pos, true);
        m.run_field();
    }
    m.bus.keyboard.set(pos, false);
    for _ in 0..3 {
        m.run_field();
    }
}

fn tap_char(m: &mut Machine, c: char) {
    let (pos, shift) =
        coco_core::keyboard::char_key(c).unwrap_or_else(|| panic!("no key for {c:?}"));
    if shift {
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, true);
    }
    tap(m, pos);
    if shift {
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, false);
    }
}

fn type_str(m: &mut Machine, s: &str) {
    for c in s.chars() {
        tap_char(m, c);
    }
}

#[test]
fn nitros9_eou_boots_from_vhd_to_shell() {
    boot_eou_to_shell(false);
}

/// Same boot, but with the FD-502 nested in MultiPak slot 4 (the real-world
/// configuration and MAME's default): proves the MPI forwards the CTS ROM
/// window, the SCS window, and the wire-OR'd HALT*/NMI handshake
/// transparently enough for a full OS boot off floppy + VHD.
#[test]
fn nitros9_eou_boots_from_vhd_with_fd502_in_multipak_slot4() {
    boot_eou_to_shell(true);
}

/// Loads the real ROM/disk/VHD assets, wires up a machine with the FD-502
/// either bare or nested in MultiPak slot 4 per `through_mpi`, and boots it
/// against a scratch copy of the VHD (EOU's startup writes to its system
/// disk, and the pristine image must stay reproducible across runs — one
/// scratch file per variant, since both boot tests run concurrently in this
/// binary). Returns the reset machine and the scratch VHD path (for the
/// caller to remove when done), or `None` (test should skip) if any asset is
/// missing.
fn setup_machine(through_mpi: bool) -> Option<(Machine, PathBuf)> {
    /// The conventional disk-controller slot (0-indexed slot 4).
    const MPI_FDC_SLOT: usize = 3;

    let (Ok(coco), Ok(disk_rom), Ok(dsk)) = (
        std::fs::read(test_assets::rom(COCO3)),
        std::fs::read(test_assets::rom(DISK11)),
        std::fs::read(test_assets::disk(EOU_BOOT)),
    ) else {
        eprintln!("skipping EOU VHD boot test: roms/ or disks/ assets not present");
        return None;
    };
    let vhd_src = test_assets::disk(EOU_SYSTEM_VHD);
    if !vhd_src.exists() {
        eprintln!("skipping EOU VHD boot test: disks/68SDC.VHD not present");
        return None;
    }
    let scratch_name = if through_mpi {
        "cocovm-test-68SDC-mpi.VHD"
    } else {
        "cocovm-test-68SDC.VHD"
    };
    let vhd_copy = std::env::temp_dir().join(scratch_name);
    std::fs::copy(&vhd_src, &vhd_copy).expect("copy VHD to scratch");
    let vhd_file = std::fs::File::options()
        .read(true)
        .write(true)
        .open(&vhd_copy)
        .expect("open scratch VHD");

    let mut m = Machine::new(MachineConfig::default(), coco.into_boxed_slice());
    let mut cart = DiskCart::new(disk_rom.into_boxed_slice());
    cart.insert_disk(0, JVCDisk::from_bytes(dsk).unwrap());
    if through_mpi {
        let mut mpi = MultiPak::new(MPI_FDC_SLOT);
        mpi.insert(MPI_FDC_SLOT, cart);
        m.insert_cartridge(mpi);
    } else {
        m.insert_cartridge(cart);
    }
    m.bus.vhd.insert(0, VHDImage::File(vhd_file));
    m.reset();

    Some((m, vhd_copy))
}

fn boot_eou_to_shell(through_mpi: bool) {
    /// Fields of BASIC settling before `DOS` is typed.
    const BASIC_FIELDS: usize = 300;
    /// Upper bound on each boot-stage wait; [`wait_for`] bails out early as
    /// soon as its marker shows.
    const MAX_BOOT_FIELDS: usize = 12_000;

    let Some((mut m, vhd_copy)) = setup_machine(through_mpi) else {
        return;
    };

    for _ in 0..BASIC_FIELDS {
        m.run_field();
    }
    type_str(&mut m, "DOS");
    tap_char(&mut m, '\r');

    let screen = wait_for(&mut m, "Time ?", MAX_BOOT_FIELDS);
    assert!(
        !screen.contains("FAILED"),
        "EOU boot must not report FAILED; screen:\n{screen}"
    );

    // Skip the time prompt and wait for the rest of the startup script
    // ("Starting Windows...", font loading — all VHD reads) to land at the
    // shell prompt, then exercise a live read of the VHD's root directory.
    tap_char(&mut m, '\r');
    wait_for(&mut m, "}/DD:", MAX_BOOT_FIELDS);
    type_str(&mut m, "dir /dd");
    tap_char(&mut m, '\r');
    let screen = wait_for(&mut m, "CMDS", MAX_BOOT_FIELDS);
    assert!(
        screen.contains("OS9Boot") && screen.contains("SYS"),
        "expected the EOU system disk's root directory listing; screen:\n{screen}"
    );

    let _ = std::fs::remove_file(&vhd_copy);
}

/// Run fields in `POLL_FIELDS`-sized batches until `marker` shows on the
/// text screen, panicking with the final screen after `max_fields`.
fn wait_for(m: &mut Machine, marker: &str, max_fields: usize) -> String {
    const POLL_FIELDS: usize = 300;
    let mut fields = 0;
    loop {
        for _ in 0..POLL_FIELDS {
            m.run_field();
        }
        fields += POLL_FIELDS;
        let screen = m.text_screen_lines().join("\n");
        if screen.contains(marker) {
            return screen;
        }
        assert!(
            fields < max_fields,
            "never saw {marker:?} after {fields} fields; screen:\n{screen}"
        );
    }
}
