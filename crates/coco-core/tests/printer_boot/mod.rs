//! Shared real-ROM EOU printer integration harness.

use std::path::PathBuf;

use coco_core::bitbanger;
use coco_core::fdc::{DiskCart, JVCDisk};
use coco_core::vhd::VHDImage;
use coco_core::{Machine, MachineConfig};
use test_assets::{
    disk::{EOU_BOOT, EOU_SYSTEM_VHD},
    rom::{COCO3, DISK11},
};

/// NitrOS-9's `/p` driver holds true 600 baud at the CoCo 3's doubled
/// (`$FFD9`) clock by running its delay loop for twice the cycles Color
/// BASIC's speed-oblivious driver would (see module doc comment and
/// wiki `cocovm/bitbanger-spec`).
pub const OS9_PRINTER_BIT_PERIOD: u32 = 2 * bitbanger::DEFAULT_BIT_PERIOD;

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

pub fn tap_char(m: &mut Machine, c: char) {
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

pub fn type_str(m: &mut Machine, s: &str) {
    for c in s.chars() {
        tap_char(m, c);
    }
}

/// Run fields in `POLL_FIELDS`-sized batches until `marker` shows on the text
/// screen, panicking with the final screen after `max_fields` (same shape as
/// `tests/vhd_boot.rs`'s `wait_for`).
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

/// Count of on-screen rows that are the shell prompt (`}/DD:` is always the
/// tail of it — same substring `tests/vhd_boot.rs` waits for) so a new
/// prompt appearing after a command can be told apart from the one already
/// on screen before it ran (`tests/bitbanger_boot.rs`'s `ok_prompt_count`
/// pattern, adapted to OS-9's prompt).
pub fn shell_prompt_count(m: &mut Machine) -> usize {
    m.text_screen_lines()
        .iter()
        .filter(|line| line.contains("}/DD:"))
        .count()
}

pub fn wait_for_new_shell_prompt(m: &mut Machine, baseline: usize, max_fields: usize) -> String {
    const POLL_FIELDS: usize = 300;
    let mut fields = 0;
    loop {
        for _ in 0..POLL_FIELDS {
            m.run_field();
        }
        fields += POLL_FIELDS;
        if shell_prompt_count(m) > baseline {
            return m.text_screen_lines().join("\n");
        }
        assert!(
            fields < max_fields,
            "never saw a new shell prompt after {fields} fields; screen:\n{}",
            m.text_screen_lines().join("\n")
        );
    }
}

/// Boots NitrOS-9 EOU (real ROM/disk/VHD assets) all the way to the shell
/// prompt, on a scratch copy of the VHD so the checked-in image stays
/// pristine across runs. Returns the machine and the scratch VHD path (for
/// the caller to remove when done), or `None` (test should skip) if any
/// asset is missing.
pub fn boot_eou_shell(
    name: &str,
    prepare: impl FnOnce(&std::path::Path),
) -> Option<(Machine, PathBuf)> {
    /// Fields of BASIC settling before `DOS` is typed (matches
    /// `tests/vhd_boot.rs`).
    const BASIC_FIELDS: usize = 300;
    /// Upper bound on each boot-stage wait (matches `tests/vhd_boot.rs`).
    const MAX_BOOT_FIELDS: usize = 12_000;

    let (Ok(coco), Ok(disk_rom), Ok(dsk)) = (
        std::fs::read(test_assets::rom(COCO3)),
        std::fs::read(test_assets::rom(DISK11)),
        std::fs::read(test_assets::disk(EOU_BOOT)),
    ) else {
        eprintln!("skipping NitrOS-9 /p boot test: roms/ or tests/ assets not present");
        return None;
    };
    let vhd_src = test_assets::disk(EOU_SYSTEM_VHD);
    if !vhd_src.exists() {
        eprintln!("skipping NitrOS-9 /p boot test: tests/68SDC.VHD not present");
        return None;
    }
    let (mut m, vhd_copy) = make_machine(name, prepare, coco, disk_rom, dsk, &vhd_src);

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
    tap_char(&mut m, '\r');
    wait_for(&mut m, "}/DD:", MAX_BOOT_FIELDS);

    // NitrOS-9 EOU is already running the GIME at double speed by the time
    // it reaches the shell (module doc comment) — confirm that's still true
    // rather than silently relying on a stale assumption, since
    // OS9_PRINTER_BIT_PERIOD's derivation depends on it.
    assert!(
        m.bus.gime.cpu_fast,
        "expected NitrOS-9 EOU to have switched the GIME to high-speed mode \
         ($FFD9) by the shell prompt; OS9_PRINTER_BIT_PERIOD's derivation \
         assumes this"
    );

    Some((m, vhd_copy))
}

fn make_machine(
    name: &str,
    prepare: impl FnOnce(&std::path::Path),
    coco: Vec<u8>,
    disk_rom: Vec<u8>,
    dsk: Vec<u8>,
    vhd_src: &std::path::Path,
) -> (Machine, PathBuf) {
    // Scratch copy, as `tests/vhd_boot.rs`: EOU's startup writes to its
    // system disk and the pristine image must stay reproducible across runs.
    let vhd_copy = std::env::temp_dir().join(format!("cocovm-{name}-{}.VHD", std::process::id()));
    std::fs::copy(vhd_src, &vhd_copy).expect("copy VHD to scratch");
    prepare(&vhd_copy);
    let vhd_file = std::fs::File::options()
        .read(true)
        .write(true)
        .open(&vhd_copy)
        .expect("open scratch VHD");

    let mut m = Machine::new(MachineConfig::default(), coco.into_boxed_slice());
    let mut cart = DiskCart::new(disk_rom.into_boxed_slice());
    cart.insert_disk(0, JVCDisk::from_bytes(dsk).unwrap());
    m.insert_cartridge(cart);
    m.bus.vhd.insert(0, VHDImage::File(vhd_file));
    m.reset();

    (m, vhd_copy)
}
