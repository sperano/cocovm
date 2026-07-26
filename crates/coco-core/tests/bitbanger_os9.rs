//! End-to-end regression for the bit-banger printer port under NitrOS-9
//! (`docs/printer-plan.md` T3): boot the real EOU 1.0.1 Level 2 disk images
//! to a shell (same asset pattern as `tests/vhd_boot.rs`), run `echo hello
//! >/p`, and assert the bytes reaching a [`CaptureSink`] match exactly what
//! the shell's `/p` redirection sent — proving the bit-banger decoder works
//! against a second, independently-written driver (NitrOS-9's own bit-bang
//! code), not just Color BASIC's. Skips gracefully if `roms/`/`disks/`
//! assets aren't present, matching `tests/vhd_boot.rs`.
//!
//! ## Bit rate: NitrOS-9 is not Color BASIC's 600-baud constant
//!
//! EOU boots the CoCo 3 GIME straight into high-speed mode (`$FFD9`,
//! `GIME::cpu_fast == true` — confirmed live, not assumed) and its `/p`
//! driver does **not** behave like Color BASIC's, which busy-waits a fixed
//! cycle count that the speed poke exactly doubles the effective baud of
//! (`docs/bitbanger-spec.md` "Baud timing"). Direct instrumentation of
//! `BitBanger::tick`'s raw PA1 edge intervals during a live boot (decoding a
//! 619-byte `dir /dd >/p` listing byte-for-byte against the known directory
//! contents) found the bit-cell quantum is **twice**
//! [`bitbanger::DEFAULT_BIT_PERIOD`] (1486 cycles): NitrOS-9's driver holds
//! true wall-clock baud at 600 regardless of `cpu_fast` by doubling its own
//! delay-loop cycle count to compensate for the doubled clock, the opposite
//! of BASIC's speed-oblivious driver. See `docs/bitbanger-spec.md`'s
//! "NitrOS-9 `/p` driver (T3 finding, empirical, not ROM-disassembled)" for
//! the full derivation — this is a measured fact from unmodified EOU code,
//! not a disassembly of the `/p` driver's source.

use std::path::PathBuf;

use coco_core::bitbanger::{self, CaptureSink};
use coco_core::fdc::{DiskCart, JvcDisk};
use coco_core::vhd::VhdImage;
use coco_core::{Machine, MachineConfig};

/// NitrOS-9's `/p` driver holds true 600 baud at the CoCo 3's doubled
/// (`$FFD9`) clock by running its delay loop for twice the cycles Color
/// BASIC's speed-oblivious driver would (see module doc comment and
/// `docs/bitbanger-spec.md`).
const OS9_PRINTER_BIT_PERIOD: u32 = 2 * bitbanger::DEFAULT_BIT_PERIOD;

fn asset(dir: &str, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir)
        .join(name)
}

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
fn shell_prompt_count(m: &mut Machine) -> usize {
    m.text_screen_lines()
        .iter()
        .filter(|line| line.contains("}/DD:"))
        .count()
}

fn wait_for_new_shell_prompt(m: &mut Machine, baseline: usize, max_fields: usize) -> String {
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
/// pristine run to run. Returns the machine and the scratch VHD path (for
/// the caller to remove when done), or `None` (test should skip) if any
/// asset is missing.
fn boot_eou_shell() -> Option<(Machine, PathBuf)> {
    /// Fields of BASIC settling before `DOS` is typed (matches
    /// `tests/vhd_boot.rs`).
    const BASIC_FIELDS: usize = 300;
    /// Upper bound on each boot-stage wait (matches `tests/vhd_boot.rs`).
    const MAX_BOOT_FIELDS: usize = 12_000;

    let (Ok(coco), Ok(disk_rom), Ok(dsk)) = (
        std::fs::read(asset("roms", "coco3.rom")),
        std::fs::read(asset("roms", "disk11.rom")),
        std::fs::read(asset("disks", "68EMU.dsk")),
    ) else {
        eprintln!("skipping NitrOS-9 /p boot test: roms/ or disks/ assets not present");
        return None;
    };
    let vhd_src = asset("disks", "68SDC.VHD");
    if !vhd_src.exists() {
        eprintln!("skipping NitrOS-9 /p boot test: disks/68SDC.VHD not present");
        return None;
    }
    // Scratch copy, as `tests/vhd_boot.rs`: EOU's startup writes to its
    // system disk and the pristine image must stay reproducible run to run.
    let vhd_copy = std::env::temp_dir().join("cocovm-test-68SDC-bitbanger-os9.VHD");
    std::fs::copy(&vhd_src, &vhd_copy).expect("copy VHD to scratch");
    let vhd_file = std::fs::File::options()
        .read(true)
        .write(true)
        .open(&vhd_copy)
        .expect("open scratch VHD");

    let mut m = Machine::new(MachineConfig::default(), coco.into_boxed_slice());
    let mut cart = DiskCart::new(disk_rom.into_boxed_slice());
    cart.insert_disk(0, JvcDisk::from_bytes(dsk).unwrap());
    m.insert_cartridge(cart);
    m.bus.vhd.insert(0, VhdImage::File(vhd_file));
    m.reset();

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

/// Boot NitrOS-9 EOU to a shell, `echo hello >/p`, and check the bit-banger
/// decoder (retuned to [`OS9_PRINTER_BIT_PERIOD`]) captured exactly what the
/// shell sent, with no framing errors.
#[test]
fn os9_echo_redirected_to_printer_is_captured() {
    /// Generous upper bound on `echo hello >/p` finishing and the shell
    /// prompt returning: a handful of bytes at 600 baud (14,860 cycles/byte
    /// at [`OS9_PRINTER_BIT_PERIOD`]-per-bit) is nowhere near this budget
    /// even accounting for OS-9 scheduling overhead.
    const MAX_PRINT_FIELDS: usize = 6_000;

    let Some((mut m, vhd_copy)) = boot_eou_shell() else {
        return;
    };

    let capture = CaptureSink::new();
    m.bus.bitbanger.set_sink(Box::new(capture.clone()));
    m.bus.bitbanger.set_bit_period(OS9_PRINTER_BIT_PERIOD);
    let baseline_prompt = shell_prompt_count(&mut m);

    type_str(&mut m, "echo hello >/p");
    tap_char(&mut m, '\r');

    let screen = wait_for_new_shell_prompt(&mut m, baseline_prompt, MAX_PRINT_FIELDS);
    assert_eq!(
        m.bus.bitbanger.framing_errors(),
        0,
        "bit-banger decoder saw framing errors decoding OS-9's /p output; screen:\n{screen}"
    );
    assert_eq!(
        capture.bytes(),
        b"hello \r",
        "captured /p bytes did not match `echo hello`'s expected output; got {:?}",
        String::from_utf8_lossy(&capture.bytes())
    );

    let _ = std::fs::remove_file(&vhd_copy);
}
