//! End-to-end regression: **HDB-DOS** (a Disk BASIC ROM replacement that
//! talks the DriveWire protocol over the Becker port, $FF41/$FF42, instead
//! of driving an FD-502) boots in the emulated CoCo 3 and lists/writes files
//! on a DriveWire-served `.dsk` through `DIR`/`SAVE`. Git-ignored assets:
//! `roms/coco3.rom`, `roms/hdbdw3bc3.rom` (HDB-DOS 1.1 DriveWire 3, Becker
//! build for CoCo 3), `disks/spetris.dsk` and `disks/blank02.dsk` (standard
//! flat 35-track/18-sector DECB images, 161,280 bytes = 630 × 256-byte
//! sectors, no header). Skips when any asset is absent.
//!
//! HDB-DOS is found by Color BASIC's cold-start "DK" signature probe at
//! `$C000`/`$C001` (`crates/coco-core/tests/cart.rs`'s
//! `disk_basic_pak_integrates_at_cold_start`, same mechanism as `disk11.rom`
//! Disk BASIC), not via the CART* FIRQ autostart line — so it's inserted as
//! a plain, non-autostart [`RomPak`], exactly like `disk11.rom` elsewhere in
//! this test suite. No FD-502/[`coco_core::fdc::DiskCart`] is involved: the
//! whole point of the Becker port is that no floppy hardware is present.
//!
//! `spetris.dsk` carries an `AUTOEXEC.BAS` that HDB-DOS auto-runs (real
//! DECB/HDB-DOS behaviour, observed directly: booting with the disk already
//! mounted lands on the Tetris clone's "CHOOSE DISPLAY MODE" menu, a
//! machine-code screen with no BASIC prompt to type `DIR` at). To keep the
//! `DIR` test hermetic, the disk is mounted only *after* the machine has
//! already reached its `OK` prompt with no disk present — HDB-DOS only scans
//! for `AUTOEXEC.BAS` during its cold-start init, not on every later disk
//! access, so this sidesteps the auto-run without touching any DriveWire
//! protocol code.

use std::path::{Path, PathBuf};

use coco_core::cart::RomPak;
use coco_core::drivewire::DwImage;
use coco_core::{Machine, MachineConfig};

fn asset(dir: &str, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir)
        .join(name)
}

/// Loads `roms/coco3.rom` and `roms/hdbdw3bc3.rom`, or `eprintln!`s a skip
/// notice tagged with `label` and returns `None` if either is absent.
fn load_roms(label: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let (Ok(coco), Ok(hdbdos)) = (
        std::fs::read(asset("roms", "coco3.rom")),
        std::fs::read(asset("roms", "hdbdw3bc3.rom")),
    ) else {
        eprintln!("skipping {label}: roms/coco3.rom or roms/hdbdw3bc3.rom not present");
        return None;
    };
    Some((coco, hdbdos))
}

/// Resolves `dir/name` under the repo root, or `eprintln!`s a skip notice
/// tagged with `label` and returns `None` if it isn't present.
fn require_disk_asset(dir: &str, name: &str, label: &str) -> Option<PathBuf> {
    let path = asset(dir, name);
    if !path.exists() {
        eprintln!("skipping {label}: {dir}/{name} not present");
        return None;
    }
    Some(path)
}

/// Copies `src` to `scratch_name` under the OS temp dir and opens the copy
/// read+write, so a test can drive writes through DriveWire without
/// perturbing the checked-in source asset. `copy_context` is the `expect`
/// message for the copy step.
fn scratch_copy(src: &Path, scratch_name: &str, copy_context: &str) -> (PathBuf, std::fs::File) {
    let scratch = std::env::temp_dir().join(scratch_name);
    std::fs::copy(src, &scratch).expect(copy_context);
    let file = std::fs::File::options()
        .read(true)
        .write(true)
        .open(&scratch)
        .expect("open scratch copy read+write");
    (scratch, file)
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
/// screen, panicking with the final screen after `max_fields`.
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

/// Boot a CoCo 3 with HDB-DOS inserted as a plain, non-autostart cartridge
/// (see module doc) and the Becker port enabled, but with no disk mounted
/// yet. Returns once the `OK` prompt shows. `hdbdos_mode` controls HDB-DOS's
/// own global lsn/630 flat-addressing remap (`DwState::set_hdbdos_mode`):
/// HDB-DOS itself needs it on, but a guest OS with its own DriveWire driver
/// (e.g. NitrOS-9's `rbdw`, which sends true per-drive LSNs) needs it off.
fn boot_to_hdbdos_prompt(coco: Vec<u8>, hdbdos: Vec<u8>, hdbdos_mode: bool) -> Machine {
    /// Upper bound while waiting for the `OK` prompt: observed boot lands
    /// well under 1 poll batch (300 fields) with no disk mounted.
    const MAX_BOOT_FIELDS: usize = 6_000;

    let mut m = Machine::new(MachineConfig::default(), coco.into_boxed_slice());
    m.insert_cartridge(RomPak::from_bytes(&hdbdos, false).unwrap());
    m.bus.enable_drivewire();
    let dw = m.bus.drivewire.as_mut().unwrap();
    dw.set_hdbdos_mode(hdbdos_mode);
    m.reset();

    wait_for_hdbdos_prompt(&mut m, MAX_BOOT_FIELDS);
    m
}

/// Like [`boot_to_hdbdos_prompt`], but mounts `disk_file` on drive 0 *before*
/// `reset()` — for disks (like `blank02.dsk`) with no `AUTOEXEC.BAS` to
/// trigger the auto-run complication documented on the module doc /
/// [`hdbdos_dir_lists_drivewire_disk`].
fn boot_to_hdbdos_prompt_with_disk_mounted(
    coco: Vec<u8>,
    hdbdos: Vec<u8>,
    hdbdos_mode: bool,
    disk_file: std::fs::File,
) -> Machine {
    const MAX_BOOT_FIELDS: usize = 6_000;

    let mut m = Machine::new(MachineConfig::default(), coco.into_boxed_slice());
    m.insert_cartridge(RomPak::from_bytes(&hdbdos, false).unwrap());
    m.bus.enable_drivewire();
    let dw = m.bus.drivewire.as_mut().unwrap();
    dw.set_hdbdos_mode(hdbdos_mode);
    dw.mount(0, DwImage::File(disk_file));
    m.reset();

    wait_for_hdbdos_prompt(&mut m, MAX_BOOT_FIELDS);
    m
}

/// Waits for the `OK` prompt and asserts the HDB-DOS banner is on screen —
/// the shared tail of both boot helpers above.
fn wait_for_hdbdos_prompt(m: &mut Machine, max_boot_fields: usize) {
    /// HDB-DOS's own banner, printed right after the standard Disk Extended
    /// Color BASIC copyright block (observed directly on the decoded text
    /// screen: "HDB-DOS 1.4 BECKER COCO 3").
    const HDBDOS_BANNER: &str = "HDB-DOS";

    let screen = wait_for(m, "OK", max_boot_fields);
    assert!(
        screen.contains(HDBDOS_BANNER),
        "expected the HDB-DOS banner on boot; screen:\n{screen}"
    );
}

#[test]
fn hdbdos_dir_lists_drivewire_disk() {
    /// Fields to let `DIR`'s DriveWire round trips (directory sectors read
    /// over the Becker port) settle before reading the screen.
    const DIR_FIELDS: usize = 600;

    let Some((coco, hdbdos)) = load_roms("DriveWire DIR test") else {
        return;
    };
    let Some(dsk_path) = require_disk_asset("disks", "spetris.dsk", "DriveWire DIR test") else {
        return;
    };
    // Read-only handle: DIR never writes, and this must not perturb the
    // checked-in asset.
    let dsk_file = std::fs::File::options()
        .read(true)
        .open(&dsk_path)
        .expect("open spetris.dsk read-only");

    let mut m = boot_to_hdbdos_prompt(coco, hdbdos, true);
    m.bus
        .drivewire
        .as_mut()
        .unwrap()
        .mount(0, DwImage::File(dsk_file));

    type_str(&mut m, "DIR");
    tap_char(&mut m, '\r');
    for _ in 0..DIR_FIELDS {
        m.run_field();
    }
    let screen = m.text_screen_lines().join("\n");

    assert!(
        !screen.contains("ERROR"),
        "DIR must not report a BASIC error; screen:\n{screen}"
    );
    // spetris.dsk's directory: track 17 (LSN 17*18 = 306); entries live in
    // sectors 3-11 of that track (LSN 308+), 32 bytes each, name in bytes
    // 0-7 and extension in bytes 8-10. The first live entry at LSN 308,
    // offset 0, is `AUTOEXEC` / `BAS` (verified directly against the
    // checked-in disks/spetris.dsk bytes).
    assert!(
        screen.contains("AUTOEXEC.BAS"),
        "expected AUTOEXEC.BAS in the DriveWire-served directory listing; screen:\n{screen}"
    );

    let dw = m.bus.drivewire.as_ref().unwrap();
    assert_eq!(
        dw.unknown_opcodes(),
        0,
        "HDB-DOS sent an opcode our DriveWire server doesn't recognize"
    );
}

#[test]
fn hdbdos_save_writes_through_drivewire() {
    /// Fields to let `SAVE`'s DriveWire round trips settle before reading
    /// the screen / checking the scratch file.
    const SAVE_FIELDS: usize = 600;

    let Some((coco, hdbdos)) = load_roms("DriveWire SAVE test") else {
        return;
    };
    let Some(blank_src) = require_disk_asset("disks", "blank02.dsk", "DriveWire SAVE test") else {
        return;
    };

    // Scratch copy under the OS temp dir: SAVE writes through DriveWire to
    // this image, and the checked-in blank02.dsk must stay pristine run to
    // run (mirrors vhd_boot.rs's VHD scratch-copy pattern).
    let (scratch, scratch_file) = scratch_copy(
        &blank_src,
        "coco-rs-test-drivewire-blank02.dsk",
        "copy blank02.dsk to scratch",
    );
    let original_bytes = std::fs::read(&scratch).expect("read scratch copy");

    // blank02.dsk carries no AUTOEXEC.BAS (its one file is SALUT.BAS), so
    // mounting before reset doesn't trigger the auto-run complication
    // documented on hdbdos_dir_lists_drivewire_disk / the module doc.
    let mut m = boot_to_hdbdos_prompt_with_disk_mounted(coco, hdbdos, true, scratch_file);

    type_str(&mut m, "10 REM X");
    tap_char(&mut m, '\r');
    type_str(&mut m, "SAVE\"T\"");
    tap_char(&mut m, '\r');
    for _ in 0..SAVE_FIELDS {
        m.run_field();
    }
    let screen = m.text_screen_lines().join("\n");

    assert!(
        !screen.contains("ERROR"),
        "SAVE must not report a BASIC error; screen:\n{screen}"
    );

    let dw = m.bus.drivewire.as_ref().unwrap();
    assert!(
        dw.sectors_written() > 0,
        "SAVE should have written at least one sector through DriveWire"
    );
    assert_eq!(
        dw.unknown_opcodes(),
        0,
        "HDB-DOS sent an opcode our DriveWire server doesn't recognize"
    );

    let new_bytes = std::fs::read(&scratch).expect("read scratch copy after SAVE");
    assert_ne!(
        new_bytes, original_bytes,
        "SAVE should have changed the on-disk image bytes"
    );

    let _ = std::fs::remove_file(&scratch);
}

/// End-to-end regression: **NitrOS-9 Level 2 3.3.0 for CoCo 3** boots all the
/// way to a working shell prompt over the Becker/DriveWire port. Unlike the
/// two tests above, HDB-DOS is only the bootstrap: once the boot track loads,
/// the guest OS's own `rbdw`/`dwio` drivers take over and drive DriveWire
/// directly with true per-drive LSNs, so HDB-DOS's global lsn/630 remap
/// (`set_hdbdos_mode`) must stay off — turning it on would double-remap
/// NitrOS-9's own LSNs. Git-ignored asset:
/// `disks/nos96809l2v030300coco3_becker.dsk`, a flat LSN (635,648 bytes =
/// 2483 x 256-byte sectors) NitrOS-9 3.3.0 Level 2 CoCo3 image whose boot
/// track and bootfile carry the Becker-transport drivers. Skips when any
/// asset is absent.
///
/// FORMERLY A KNOWN BLOCKER, now fixed: this stock disk's `STARTUP` launches
/// an `inetd`-style daemon (confirmed present on the image: `SYS/inetd.conf`
/// configured for `telnet ... login`, plus `scdwv`-family virtual-serial
/// descriptors `z1_scdwv`..`z7_scdwv`, `n1_scdwv`.. — DriveWire 4's
/// telnet-over-virtual-serial feature, a standard part of the official CoCo3
/// DW4 release). Its init sends `OP_SERREAD` (`'C'` = `$43`, from the
/// official NitrOS-9 source's `defs/drivewire.d`: `OP_SERREAD equ 'C`, part
/// of the `OP_SERINIT` ($45)/`OP_SERTERM`/`OP_SERREAD`/`OP_SERREADM`
/// (`'c'`)/`OP_SERWRITE` (`'C'+128`)/`OP_SERGETSTAT`(`'D'`)/`OP_SERSETSTAT`
/// (`'D'+128`) virtual-serial-port family. Previously
/// `crates/coco-core/src/drivewire.rs` didn't implement this family —
/// `handle_opcode`'s catch-all arm incremented `unknown_opcodes` and pushed
/// no reply, so the daemon's init blocked forever waiting for one and the
/// boot never reached a shell prompt (confirmed directly: the boot reliably
/// reached the full NitrOS-9 banner + auto-printed date, then froze
/// byte-for-byte with `sectors_read() == 306` unmoving and
/// `unknown_opcodes() == 1`). Now that `opcode::SERREAD`/`SERREADM`/
/// `SERWRITE`/`SERGETSTAT`/`SERSETSTAT`/`SERINIT`/`SERTERM`/`FASTWRITE_*`
/// are implemented (each a no-op consume, except `SERREAD` which always
/// replies "idle, no data"), the daemon's init unblocks and the boot
/// proceeds past the banner to Shell+'s prompt.
#[test]
fn nitros9_l2_boots_over_drivewire_to_shell_prompt() {
    /// A full OS-9 kernel-and-modules load over Becker is much heavier than
    /// HDB-DOS's own DIR/SAVE round trips (which settle in ~600 fields): this
    /// mirrors `vhd_boot.rs`'s EOU boot test, which uses the same order-of
    /// magnitude budget per wait stage for a whole-kernel-over-slow-transport
    /// boot.
    const MAX_BOOT_FIELDS: usize = 12_000;
    /// NitrOS-9's banner substrings (mixed case, observed directly on this
    /// same OS build's plain-FDC boot in `fdc.rs`'s
    /// `nitros9_l2_boot_reaches_shell_prompt`, and again here).
    const BANNER_NITROS9: &str = "NitrOS-9";
    const BANNER_LEVEL2: &str = "Level 2";
    // NOTE: unlike `fdc.rs`'s plain-FDC boot (no DriveWire) and
    // `vhd_boot.rs`'s EOU boot (FD-502 + VHD, no DriveWire either), this
    // boot's `clock2_dw` module fetches real time directly via the
    // already-implemented `opcode::TIME` ($23) round trip and the startup
    // script never shows an interactive "Time ?" prompt at all — confirmed
    // directly: the banner is immediately followed by an auto-printed
    // `"April 13, 2014  21:40:45"` line. Do not wait for "Time ?" here.
    //
    // Shell+'s prompt, observed directly on the decoded text screen once
    // the virtual-serial (`OP_SER*`) opcodes were implemented and the
    // `inetd`/`scdwv` startup no longer blocks: `{Term|02}/DD:` — the
    // `{Term|02}` window-name segment comes from Shell+'s `$prompt`
    // default and `/DD` is the current default data directory.
    const SHELL_PROMPT: &str = "{Term|02}/DD:";

    let Some((coco, hdbdos)) = load_roms("NitrOS-9/DriveWire boot test") else {
        return;
    };
    let Some(dsk_src) = require_disk_asset(
        "disks",
        "nos96809l2v030300coco3_becker.dsk",
        "NitrOS-9/DriveWire boot test",
    ) else {
        return;
    };

    // Scratch copy: NitrOS-9 writes to the boot disk in normal operation
    // (dirty bits etc.), and the checked-in asset must stay pristine run to
    // run (mirrors hdbdos_save_writes_through_drivewire's blank02.dsk
    // pattern).
    let (scratch, scratch_file) = scratch_copy(
        &dsk_src,
        "coco-rs-test-nos9-becker.dsk",
        "copy nos96809l2v030300coco3_becker.dsk to scratch",
    );

    // hdbdos_mode = false: NitrOS-9's own rbdw driver takes over after the
    // boot track loads and sends true per-drive LSNs (see doc comment).
    let mut m = boot_to_hdbdos_prompt(coco, hdbdos, false);
    m.bus
        .drivewire
        .as_mut()
        .unwrap()
        .mount(0, DwImage::File(scratch_file));

    type_str(&mut m, "DOS");
    tap_char(&mut m, '\r');

    let screen = wait_for(&mut m, BANNER_LEVEL2, MAX_BOOT_FIELDS);
    assert!(
        !screen.contains("FAILED"),
        "NitrOS-9 boot must not report FAILED; screen:\n{screen}"
    );
    assert!(
        screen.contains(BANNER_NITROS9) && screen.contains(BANNER_LEVEL2),
        "expected the NitrOS-9 Level 2 banner; screen:\n{screen}"
    );

    // No interactive "Time ?" prompt to answer here — see the NOTE above:
    // clock2_dw already got real time over DriveWire, so the startup script
    // proceeds straight past where fdc.rs/vhd_boot.rs's non-DriveWire boots
    // would stop and wait for one.
    wait_for(&mut m, SHELL_PROMPT, MAX_BOOT_FIELDS);

    // Prove the shell is actually live, not just sitting at a static prompt:
    // run a real command and check its output lands on screen.
    type_str(&mut m, "dir");
    tap_char(&mut m, '\r');
    // `dir`'s own output header, observed directly: "Directory of .  <date>
    // <time>" followed by the root directory's entry names.
    let screen = wait_for(&mut m, "Directory of", MAX_BOOT_FIELDS);
    assert!(
        screen.contains("CMDS") && screen.contains("SYS"),
        "expected `dir`'s root directory listing on screen; screen:\n{screen}"
    );

    let dw = m.bus.drivewire.as_ref().unwrap();
    eprintln!(
        "nitros9_l2_boots_over_drivewire_to_shell_prompt: sectors_read={} sectors_written={} unknown_opcodes={} vserial_ops={}",
        dw.sectors_read(),
        dw.sectors_written(),
        dw.unknown_opcodes(),
        dw.vserial_ops()
    );
    assert_eq!(
        dw.unknown_opcodes(),
        0,
        "NitrOS-9 sent an opcode our DriveWire server doesn't recognize"
    );
    assert!(
        dw.sectors_read() > 100,
        "a full kernel+modules boot should read well over 100 sectors; got {}",
        dw.sectors_read()
    );

    let _ = std::fs::remove_file(&scratch);
}
