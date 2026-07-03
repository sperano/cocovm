//! Boot Disk Extended Color BASIC with a disk image mounted, type a command,
//! and dump the text screen — a quick smoke test for the FD-502 against real
//! disk images (companion to `cart_boot_probe`).
//!
//!     cargo run -p coco-core --example disk_boot_probe <disk.dsk> [command...]
//!
//! Each further argument is typed as its own line (LOAD, notably, discards the
//! rest of its command line, so `LOAD"X":LIST` never LISTs). Defaults to `DIR`.

use coco_core::fdc::{DiskCart, JvcDisk};
use coco_core::keyboard::char_key;
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

const BOOT_FIELDS: u32 = 300;
const FIELDS_PER_KEY: u32 = 4;
const COMMAND_FIELDS: u32 = 600;
const SCREEN_BASE: u16 = 0x0400;
const SCREEN_ROWS: u16 = 16;
const SCREEN_COLS: u16 = 32;

fn screen_row(m: &mut Machine, row: u16) -> String {
    (0..SCREEN_COLS)
        .map(|c| {
            let code = m.bus.read(SCREEN_BASE + row * SCREEN_COLS + c) & 0x3F;
            if code < 0x20 {
                (b'@' + code) as char
            } else {
                (b' ' + (code - 0x20)) as char
            }
        })
        .collect()
}

fn type_line(m: &mut Machine, text: &str) {
    for ch in text.chars().chain(std::iter::once('\r')) {
        let Some((pos, shifted)) = char_key(ch) else {
            panic!("no CoCo key for {ch:?}");
        };
        if shifted {
            m.bus.keyboard.set(coco_core::keyboard::SHIFT, true);
        }
        m.bus.keyboard.set(pos, true);
        for _ in 0..FIELDS_PER_KEY {
            m.run_field();
        }
        m.bus.keyboard.set(pos, false);
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, false);
        for _ in 0..FIELDS_PER_KEY {
            m.run_field();
        }
    }
}

fn main() {
    let disk_path = std::env::args().nth(1).expect("disk image path");
    let mut commands: Vec<String> = std::env::args().skip(2).collect();
    if commands.is_empty() {
        commands.push("DIR".into());
    }

    let rom = std::fs::read("roms/coco3.rom").unwrap().into_boxed_slice();
    let disk_rom = std::fs::read("roms/disk11.rom").unwrap().into_boxed_slice();
    let disk = JvcDisk::from_bytes(std::fs::read(&disk_path).unwrap()).unwrap();
    println!(
        "mounted {disk_path}: {} tracks, {} sectors/track, {} side(s)",
        disk.track_count(),
        disk.sectors_per_track(),
        disk.sides()
    );

    let mut m = Machine::new(MachineConfig::default(), rom);
    let mut cart = DiskCart::new(disk_rom);
    cart.insert_disk(0, disk);
    m.insert_cartridge(Box::new(cart));
    m.reset();

    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }
    for command in &commands {
        type_line(&mut m, command);
        for _ in 0..COMMAND_FIELDS {
            m.run_field();
        }
    }

    for row in 0..SCREEN_ROWS {
        println!("|{}|", screen_row(&mut m, row));
    }
}
