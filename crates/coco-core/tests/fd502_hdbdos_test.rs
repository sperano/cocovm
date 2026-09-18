//! HDB-DOS on an FD-502 retains both its Becker and floppy paths. The installed
//! `hdbdw3bc3.rom` routes `DRIVE ON` through DriveWire and `DRIVE OFF` through
//! the original Disk BASIC floppy routine (HDB-DOS DSKCON2 at $DA0F, original
//! DSKCON at $D763). Exercise both paths with distinct directory contents.

use coco_core::drivewire::DWImage;
use coco_core::fdc::{DiskCart, JVCDisk};
use coco_core::{Machine, MachineConfig};
use test_assets::{
    disk::{BLANK02, SPETRIS},
    rom::{COCO3, HDBDW3BC3},
};

const DRIVE: usize = 0;
const KEY_FIELDS: usize = 3;
const BOOT_POLL_FIELDS: usize = 300;
const MAX_BOOT_FIELDS: usize = 6_000;
const DIR_FIELDS: usize = 600;
const DRIVEWIRE_FILE: &str = "AUTOEXEC.BAS";
const FLOPPY_FILE: &str = "SALUT   .BAS";

struct Assets {
    coco: Vec<u8>,
    hdbdos: Vec<u8>,
    drivewire_disk: Vec<u8>,
    floppy_disk: Vec<u8>,
}

fn load_assets() -> Option<Assets> {
    let paths = [
        test_assets::rom(COCO3),
        test_assets::rom(HDBDW3BC3),
        test_assets::disk(SPETRIS),
        test_assets::disk(BLANK02),
    ];
    for path in &paths {
        if !path.exists() {
            eprintln!(
                "skipping FD-502 HDB-DOS test: {} not present",
                path.display()
            );
            return None;
        }
    }
    let [coco, hdbdos, drivewire_disk, floppy_disk] =
        paths.map(|path| std::fs::read(path).expect("read installed test asset"));
    Some(Assets {
        coco,
        hdbdos,
        drivewire_disk,
        floppy_disk,
    })
}

fn run_fields(machine: &mut Machine, fields: usize) {
    for _ in 0..fields {
        machine.run_field();
    }
}

fn type_command(machine: &mut Machine, command: &str) {
    for character in command.chars().chain(std::iter::once('\r')) {
        let (key, shift) = coco_core::keyboard::char_key(character).expect("BASIC command key");
        machine.bus.keyboard.set(coco_core::keyboard::SHIFT, shift);
        machine.bus.keyboard.set(key, true);
        run_fields(machine, KEY_FIELDS);
        machine.bus.keyboard.set(key, false);
        machine.bus.keyboard.set(coco_core::keyboard::SHIFT, false);
        run_fields(machine, KEY_FIELDS);
    }
}

fn boot(coco: Vec<u8>, hdbdos: Vec<u8>) -> Machine {
    let mut machine = Machine::new(MachineConfig::default(), coco.into_boxed_slice());
    machine.insert_cartridge(DiskCart::new(hdbdos.into_boxed_slice()));
    machine.bus.enable_drivewire();
    machine
        .bus
        .drivewire
        .as_mut()
        .unwrap()
        .set_hdbdos_mode(true);
    machine.reset();
    for _ in (0..MAX_BOOT_FIELDS).step_by(BOOT_POLL_FIELDS) {
        run_fields(&mut machine, BOOT_POLL_FIELDS);
        let screen = machine.text_screen_lines().join("\n");
        if screen.contains("OK") {
            assert!(
                screen.contains("HDB-DOS"),
                "missing HDB-DOS banner:\n{screen}"
            );
            return machine;
        }
    }
    panic!(
        "HDB-DOS did not boot:\n{}",
        machine.text_screen_lines().join("\n")
    );
}

fn directory(machine: &mut Machine, command: &str, expected: &str, unexpected: &str) {
    // CLS prevents a previous directory listing from satisfying either assertion.
    type_command(machine, command);
    run_fields(machine, DIR_FIELDS);
    let screen = machine.text_screen_lines().join("\n");
    assert!(!screen.contains("ERROR"), "{command} failed:\n{screen}");
    assert!(screen.contains(expected), "missing {expected}:\n{screen}");
    assert!(
        !screen.contains(unexpected),
        "unexpected {unexpected}:\n{screen}"
    );
}

#[test]
fn hdbdos_on_fd502_switches_between_drivewire_and_floppy_directories() {
    let Some(assets) = load_assets() else {
        return;
    };
    let mut machine = boot(assets.coco, assets.hdbdos);
    // Mount after boot so SPETRIS's AUTOEXEC.BAS cannot start its game.
    machine
        .bus
        .drivewire
        .as_mut()
        .unwrap()
        .mount(DRIVE, DWImage::Memory(assets.drivewire_disk));
    machine
        .bus
        .cart
        .as_disk_cart()
        .expect("FD-502 remains inserted")
        .insert_disk(DRIVE, JVCDisk::from_bytes(assets.floppy_disk).unwrap());

    let before = machine.bus.drivewire.as_ref().unwrap().sectors_read();
    directory(
        &mut machine,
        "CLS:DRIVE ON:DIR",
        DRIVEWIRE_FILE,
        FLOPPY_FILE,
    );
    let after = machine.bus.drivewire.as_ref().unwrap().sectors_read();
    assert!(
        after > before,
        "DRIVE ON must read sectors through DriveWire"
    );

    directory(
        &mut machine,
        "CLS:DRIVE OFF:DIR",
        FLOPPY_FILE,
        DRIVEWIRE_FILE,
    );
    assert_eq!(
        machine.bus.drivewire.as_ref().unwrap().sectors_read(),
        after,
        "DRIVE OFF must read the floppy without issuing DriveWire reads"
    );
}
