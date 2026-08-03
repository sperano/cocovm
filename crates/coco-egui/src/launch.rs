//! The manager's boot path: turning a saved machine definition into a running
//! [`CocoApp`]. The CLI's equivalent, starting from parsed flags instead, is
//! [`crate::boot`].

use std::fs;
use std::path::PathBuf;

use crate::{
    CocoApp, DEFAULT_RTC_SLOT, KbMode, MPI_SLOT_COUNT, ROMSource, UI_DRIVES, dev_roms_dir,
    load_rom_with_source, machine_def,
};

/// File name [`launch_machine`] captures `[ports].serial = "file"` to,
/// resolved against the machine's artifact directory — the same auto-named
/// target the runtime Machine menu's "Capture to File…" item suggests
/// (`chrome::menu_bar::machine`'s file dialog).
const PRINTOUT_FILE: &str = "printout.txt";

/// Where a definition's `[media]` paths land once resolved against the
/// machine's own directory (`machine_def::resolve_media_path`).
struct Media {
    cart: Option<PathBuf>,
    disks: [Option<PathBuf>; UI_DRIVES],
    vhds: [Option<PathBuf>; UI_DRIVES],
    tape: Option<PathBuf>,
}

/// What the definition's `[peripherals]` section asks for in the cartridge
/// port, after folding in the media that implies a controller.
struct Peripherals {
    mpi: bool,
    rtc: bool,
    fd502: bool,
    /// Deluxe RS-232 Pak. Bare-port only — no MPI-slot support yet
    /// (`check_cartridge_port` rejects `mpi && rs232`).
    rs232: bool,
}

/// Build a running [`CocoApp`] from a saved machine definition
/// (`machine_def::MachineDef`): the same steps the CLI branch performs — load
/// the ROM (an explicit `[hardware].rom` if set, else the same `./roms`
/// resolution the CLI path uses), mount `[media]` (cart/disks/vhds/tape),
/// `[peripherals]` (MPI/RTC/RS-232), and `[ports]` (the built-in serial
/// port's host sink) with the same `CocoApp` methods and ordering, and
/// enforce the same single-cartridge-port rule — but every failure is a
/// returned `Err` here instead of a process exit, since the caller (the
/// manager's Start button, `manager.rs`) must show it in the detail pane
/// rather than crash the whole app (`docs/plan-machine-persistence.md`
/// step 5). On any mount-time failure (a bad disk/VHD/cassette image, or a
/// disk-BASIC ROM read failure inside `mpi_insert_fd502` — not just a missing
/// path, caught early below) the partially-built VM is discarded rather than
/// returned: callers get either a fully-mounted machine or a precise error,
/// never a half-broken one.
pub(crate) fn launch_machine(def: &machine_def::MachineDef, slug: &str) -> Result<CocoApp, String> {
    let config = def.to_machine_config()?;
    let explicit_rom = def.hardware.rom.as_ref().map(PathBuf::from);
    let (rom, rom_source) =
        load_rom_with_source(explicit_rom.as_deref(), config.variant, &dev_roms_dir())?;

    let media = resolve_media(def, slug);
    let peripherals = Peripherals {
        mpi: def.peripherals.mpi,
        rtc: def.peripherals.rtc,
        // Disk media implies the controller even when the flag is off (older
        // definition files predate `[peripherals].fd502`).
        fd502: def.peripherals.fd502 || media.disks.iter().any(|p| p.is_some()),
        rs232: def.peripherals.rs232,
    };
    check_cartridge_port(&media, &peripherals)?;

    let mut app = new_app(config, rom, rom_source, &media, &peripherals);
    mount_peripherals(&mut app, media, &peripherals);
    mount_serial(&mut app, def.ports.serial, slug);

    // Every `insert_*`/`mpi_insert_*` helper above records its own failure in
    // `cart_error` rather than returning a `Result` (it's designed to run from
    // a live menu click, where the machine keeps running and a dialog reports
    // the problem). Promote that here into the launch `Result` instead of
    // returning a VM with a swallowed error nobody's watching for yet.
    if let Some(err) = app.cart_error.take() {
        return Err(err);
    }

    // Built by the manager, not a direct CLI boot — gates the VM window's
    // own Suspend tile (`CocoApp::managed`'s doc).
    app.managed = true;

    // The definition's [ui] preferences are the launched window's *starting*
    // state; F9 (aspect), F12 (keyboard mode), and the Joysticks menu keep
    // working as live toggles afterwards — the file controls where they
    // begin, exactly like the hardware section controls the machine's
    // construction.
    app.aspect_correct = def.ui.aspect_correct;
    // `CocoApp::new` derived a display from the config's signal path, which
    // can't tell a CoCo 3 TV from a composite monitor — overwrite it with
    // the definition's actual `[hardware].display` choice.
    app.display = def.display();
    app.tv.scanline_pct = def.ui.tv_scanline.min(100);
    app.kb_mode = match def.ui.kb_mode {
        machine_def::KbModeDTO::Positional => KbMode::Positional,
        machine_def::KbModeDTO::Symbolic => KbMode::Symbolic,
    };
    app.joysticks.sources[coco_core::joystick::RIGHT] = def.ui.joy_right.into();
    app.joysticks.sources[coco_core::joystick::LEFT] = def.ui.joy_left.into();
    Ok(app)
}

fn resolve_media(def: &machine_def::MachineDef, slug: &str) -> Media {
    let resolve = |p: Option<&str>| p.map(|p| machine_def::resolve_media_path(p, slug));
    Media {
        cart: resolve(def.media.cart.as_deref()),
        disks: [
            resolve(def.media.disk0.as_deref()),
            resolve(def.media.disk1.as_deref()),
        ],
        vhds: [
            resolve(def.media.vhd0.as_deref()),
            resolve(def.media.vhd1.as_deref()),
        ],
        tape: resolve(def.media.tape.as_deref()),
    }
}

/// The same rule the CLI branch enforces by hand (clap's declarative
/// `conflicts_with` can't express "only when --mpi is absent"): cart,
/// disk0/disk1 (which imply the FD-502), rtc, and rs232 all want the single
/// cartridge port unless an MPI is installed. rs232 additionally has no
/// MPI-slot support at all yet (no `mpi_insert_rs232`), so `mpi && rs232` is
/// rejected even though an MPI would otherwise lift the one-peripheral
/// limit.
fn check_cartridge_port(media: &Media, peripherals: &Peripherals) -> Result<(), String> {
    if peripherals.mpi && peripherals.rs232 {
        return Err(
            "the Deluxe RS-232 Pak has no MultiPak slot support yet; disable the MultiPak \
             Interface peripheral to use it"
                .to_string(),
        );
    }
    let claims = [
        media.cart.is_some(),
        peripherals.fd502,
        peripherals.rtc,
        peripherals.rs232,
    ]
    .into_iter()
    .filter(|&claims| claims)
    .count();
    if !peripherals.mpi && claims > 1 {
        return Err(
            "cart, disk0/disk1, rtc, and rs232 all need the cartridge port; enable the \
             MultiPak Interface peripheral to combine them"
                .to_string(),
        );
    }
    Ok(())
}

/// With an MPI installed the cart and disks target its slots instead of the
/// plain single-cartridge model, so the constructor gets neither and
/// [`mount_peripherals`] wires them up afterward.
fn new_app(
    config: coco_core::MachineConfig,
    rom: Box<[u8]>,
    rom_source: ROMSource,
    media: &Media,
    peripherals: &Peripherals,
) -> CocoApp {
    // No definition field for these UI preferences yet (`machine_def.rs`'s
    // schema doc); matches the CLI defaults — `--tape-wav` off, no DriveWire
    // disks, Becker port disabled, HDB-DOS off.
    const SAVE_TAPE_WAV: bool = false;
    const BECKER_ENABLED: bool = false;
    const HDBDOS_MODE: bool = false;
    let (cart, disks) = if peripherals.mpi {
        (None, [None, None])
    } else {
        (media.cart.clone(), media.disks.clone())
    };
    CocoApp::new(
        config,
        rom,
        rom_source,
        cart,
        disks,
        media.vhds.clone(),
        std::array::from_fn(|_| None),
        BECKER_ENABLED,
        HDBDOS_MODE,
        SAVE_TAPE_WAV,
    )
}

fn mount_peripherals(app: &mut CocoApp, media: Media, peripherals: &Peripherals) {
    if peripherals.mpi {
        app.insert_multipak();
        if let Some(path) = media.cart {
            app.mpi_insert_rompak(0, path);
        }
        if peripherals.fd502 {
            app.mpi_insert_fd502(MPI_SLOT_COUNT - 1);
        }
        if peripherals.rtc {
            app.mpi_insert_rtc(DEFAULT_RTC_SLOT);
        }
        for (drive, path) in media.disks.into_iter().enumerate() {
            if let Some(path) = path {
                app.insert_disk(drive, path);
            }
        }
    } else if peripherals.rtc {
        // cart/fd502/rs232 (disk media is handled by the `CocoApp::new` call
        // above, same as the CLI's non-mpi branch) and rtc are mutually
        // exclusive here — `check_cartridge_port` already rejected any
        // combination of them without an MPI.
        app.insert_rtc();
    } else if peripherals.rs232 {
        // Starts on the inert Loopback endpoint; TCP/PTY stay a
        // runtime-menu-only setting (`chrome::menu_bar::rs232`).
        app.insert_rs232();
    } else if peripherals.fd502
        && let Err(e) = app.ensure_disk_controller()
    {
        // Empty-drive FD-502 from `[peripherals].fd502` alone; with disk media
        // set, `CocoApp::new` already inserted the controller and this is a
        // no-op Ok.
        app.cart_error = Some(e);
    }

    if let Some(path) = media.tape {
        app.insert_tape(path);
    }
}

/// Wire `[ports].serial` — the built-in bit-banger serial port's host sink,
/// distinct from the cartridge-port Deluxe RS-232 Pak the peripherals above
/// mount. Runs after [`mount_peripherals`] and before `launch_machine`'s
/// `cart_error` promotion, so a failure here (an unwritable artifact
/// directory, or whatever [`CocoApp::start_print_capture`] itself rejects)
/// surfaces as a launch error the same way an `insert_*` failure does.
fn mount_serial(app: &mut CocoApp, serial: Option<machine_def::SerialDTO>, slug: &str) {
    match serial {
        // Attached with the paper window closed; output accumulates and
        // View ▸ Printer Paper shows it (`paper_view`'s module doc — the
        // window just displays whatever handle it's given).
        Some(machine_def::SerialDTO::Printer) => app.attach_dmp105(),
        Some(machine_def::SerialDTO::File) => {
            let path = machine_def::resolve_media_path(PRINTOUT_FILE, slug);
            if let Some(parent) = path.parent() {
                // The artifact directory may not exist yet — nothing else
                // creates it ahead of a `[ports]`-only definition.
                if let Err(e) = fs::create_dir_all(parent) {
                    app.cart_error = Some(format!("{}: {e}", parent.display()));
                    return;
                }
            }
            app.start_print_capture(path);
        }
        None => {}
    }
}

#[cfg(test)]
#[path = "launch_test.rs"]
mod tests;
