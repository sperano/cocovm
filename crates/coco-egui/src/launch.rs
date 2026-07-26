//! The manager's boot path: turning a saved machine definition into a running
//! [`CocoApp`]. The CLI's equivalent, starting from parsed flags instead, is
//! [`crate::boot`].

use std::path::PathBuf;

use crate::{
    CocoApp, DEFAULT_RTC_SLOT, KbMode, MPI_SLOT_COUNT, RomSource, UI_DRIVES, dev_roms_dir,
    load_rom_with_source, machine_def,
};

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
}

/// Build a running [`CocoApp`] from a saved machine definition
/// (`machine_def::MachineDef`): the same steps the CLI branch performs — load
/// the ROM (an explicit `[hardware].rom` if set, else the same `./roms`
/// resolution the CLI path uses), mount `[media]` (cart/disks/vhds/tape) and
/// `[peripherals]` (MPI/RTC) with the same `CocoApp` methods and ordering,
/// and enforce the same single-cartridge-port rule — but every failure is a
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
    };
    check_cartridge_port(&media, &peripherals)?;

    let mut app = new_app(config, rom, rom_source, &media, &peripherals);
    mount_peripherals(&mut app, media, &peripherals);

    // Every `insert_*`/`mpi_insert_*` helper above records its own failure in
    // `cart_error` rather than returning a `Result` (it's designed to run from
    // a live menu click, where the machine keeps running and a dialog reports
    // the problem). Promote that here into the launch `Result` instead of
    // returning a VM with a swallowed error nobody's watching for yet.
    if let Some(err) = app.cart_error.take() {
        return Err(err);
    }

    // The definition's [ui] preferences are the launched window's *starting*
    // state; F9 (aspect) and F12 (keyboard mode) keep working as live toggles
    // afterwards — the file controls where they begin, exactly like the
    // hardware section controls the machine's construction.
    app.aspect_correct = def.ui.aspect_correct;
    app.kb_mode = match def.ui.kb_mode {
        machine_def::KbModeDTO::Positional => KbMode::Positional,
        machine_def::KbModeDTO::Symbolic => KbMode::Symbolic,
    };
    Ok(app)
}

fn resolve_media(def: &machine_def::MachineDef, slug: &str) -> Media {
    let resolve = |p: Option<&str>| p.map(|p| machine_def::resolve_media_path(p, slug));
    Media {
        cart: resolve(def.media.cart.as_deref()),
        disks: [resolve(def.media.disk0.as_deref()), resolve(def.media.disk1.as_deref())],
        vhds: [resolve(def.media.vhd0.as_deref()), resolve(def.media.vhd1.as_deref())],
        tape: resolve(def.media.tape.as_deref()),
    }
}

/// The same rule the CLI branch enforces by hand (clap's declarative
/// `conflicts_with` can't express "only when --mpi is absent"): cart,
/// disk0/disk1 (which imply the FD-502), and rtc all want the single
/// cartridge port unless an MPI is installed.
fn check_cartridge_port(media: &Media, peripherals: &Peripherals) -> Result<(), String> {
    let claims = [media.cart.is_some(), peripherals.fd502, peripherals.rtc]
        .into_iter()
        .filter(|&claims| claims)
        .count();
    if !peripherals.mpi && claims > 1 {
        return Err("cart, disk0/disk1, and rtc all need the cartridge port; enable the MultiPak \
                    Interface peripheral to combine them"
            .to_string());
    }
    Ok(())
}

/// With an MPI installed the cart and disks target its slots instead of the
/// plain single-cartridge model, so the constructor gets neither and
/// [`mount_peripherals`] wires them up afterward.
fn new_app(
    config: coco_core::MachineConfig,
    rom: Box<[u8]>,
    rom_source: RomSource,
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
        // cart/fd502 (disk media is handled by the `CocoApp::new` call above,
        // same as the CLI's non-mpi branch) and rtc are mutually exclusive
        // here — `check_cartridge_port` already rejected any combination of
        // them without an MPI.
        app.insert_rtc();
    } else if peripherals.fd502 && let Err(e) = app.ensure_disk_controller() {
        // Empty-drive FD-502 from `[peripherals].fd502` alone; with disk media
        // set, `CocoApp::new` already inserted the controller and this is a
        // no-op Ok.
        app.cart_error = Some(e);
    }

    if let Some(path) = media.tape {
        app.insert_tape(path);
    }
}
