//! Boot Disk Extended Color BASIC with a disk image mounted, type a command,
//! and dump the text screen — a quick smoke test for the FD-502 against real
//! disk images (companion to `cart_boot_probe`).
//!
//!     cargo run -p coco-core --example disk_boot_probe <disk.dsk> [--fields N] [command...]
//!
//! Each further argument is typed as its own line (LOAD, notably, discards the
//! rest of its command line, so `LOAD"X":LIST` never LISTs). Defaults to `DIR`.
//! `--fields N` overrides how many fields are run after each typed command
//! before the next one (and before the final dump) — bump it for boots that
//! take longer to settle than stock Disk BASIC, e.g. an OS-9 disk still
//! loading modules long after the command line is typed. Defaults to the
//! same field count this probe has always used per command.
//!
//! The screen dump works in either video mode the GIME can be driving when the
//! dump is taken (`Machine::text_screen_lines`): the legacy CoCo-compatible
//! VDG text screen, or — e.g. once OS-9 switches over — the GIME-native
//! hi-res text screen. A one-line mode summary follows the dump so a blank or
//! garbled screen can be told apart from "this is actually a graphics-mode
//! screen with no text buffer".

use coco_core::fdc::{DiskCart, JVCDisk};
use coco_core::keyboard::char_key;
use coco_core::{Machine, MachineConfig};
use test_assets::rom;

const BOOT_FIELDS: u32 = 300;
const FIELDS_PER_KEY: u32 = 4;
/// Fields run after each typed command before the next (and before the final
/// dump) unless overridden by `--fields`.
const DEFAULT_COMMAND_FIELDS: u32 = 600;

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

/// Parse `<disk.dsk> [--fields N] [command...]` (`--fields` may appear
/// anywhere after the disk path). Returns (disk path, commands, per-command
/// settle fields).
fn parse_args() -> (String, Vec<String>, u32) {
    let mut args = std::env::args().skip(1);
    let disk_path = args.next().expect("disk image path");
    let mut commands = Vec::new();
    let mut command_fields = DEFAULT_COMMAND_FIELDS;
    while let Some(arg) = args.next() {
        if arg == "--fields" {
            let value = args.next().expect("--fields requires a value");
            command_fields = value
                .parse()
                .unwrap_or_else(|e| panic!("--fields value {value:?}: {e}"));
        } else {
            commands.push(arg);
        }
    }
    if commands.is_empty() {
        commands.push("DIR".into());
    }
    (disk_path, commands, command_fields)
}

fn main() {
    let (disk_path, commands, command_fields) = parse_args();

    let rom = std::fs::read(test_assets::rom(rom::COCO3))
        .unwrap()
        .into_boxed_slice();
    let disk_rom = std::fs::read(test_assets::rom(rom::DISK11))
        .unwrap()
        .into_boxed_slice();
    let disk = JVCDisk::from_bytes(std::fs::read(&disk_path).unwrap()).unwrap();
    println!(
        "mounted {disk_path}: {} tracks, {} sectors/track, {} side(s)",
        disk.track_count(),
        disk.sectors_per_track(),
        disk.sides()
    );

    let mut m = Machine::new(MachineConfig::default(), rom);
    let mut cart = DiskCart::new(disk_rom);
    cart.insert_disk(0, disk);
    m.insert_cartridge(cart);
    m.reset();

    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }
    for command in &commands {
        type_line(&mut m, command);
        for _ in 0..command_fields {
            m.run_field();
        }
    }

    for line in m.text_screen_lines() {
        println!("|{line}|");
    }
    println!("{}", m.video_mode_summary());
}
