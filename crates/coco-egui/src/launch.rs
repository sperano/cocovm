//! The manager's boot path, and the only one in the app: turning a saved
//! machine definition into a running [`CocoApp`].

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::MachineVariant;

use crate::rom_load::{load_default_rom, load_explicit_rom};
use crate::{
    AppParams, CocoApp, DEFAULT_RTC_SLOT, KbMode, MPI_SLOT_COUNT, ROMSource, UI_DRIVES,
    installed_roms_dir, machine_def,
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

/// Build a running [`CocoApp`] from a saved machine definition: load the ROM,
/// mount `[media]`/`[peripherals]`/`[ports]` enforcing the single-cartridge-
/// port rule below. Any failure returns `Err` instead of a partial VM.
pub(crate) fn launch_machine(def: &machine_def::MachineDef, slug: &str) -> Result<CocoApp, String> {
    let config = def.to_machine_config()?;
    let explicit_rom = def.hardware.rom.as_ref().map(PathBuf::from);
    let (rom, rom_source) = load_rom(explicit_rom.as_deref(), config.variant)?;

    let media = resolve_media(def, slug);
    let peripherals = Peripherals {
        mpi: def.peripherals.mpi,
        rtc: def.peripherals.rtc,
        // Disk media implies the controller even when the flag itself is off.
        fd502: def.peripherals.fd502 || media.disks.iter().any(|p| p.is_some()),
        rs232: def.peripherals.rs232,
    };
    check_cartridge_port(&media, &peripherals)?;

    let mut app = new_app(config, rom, rom_source, &media, &peripherals);
    mount_peripherals(&mut app, media, &peripherals);
    mount_serial(&mut app, def.ports.serial, slug);

    // Promote any `cart_error` the insert_*/mpi_insert_* helpers recorded into this launch
    // `Result`.
    if let Some(err) = app.cart_error.take() {
        return Err(err);
    }

    // [ui] preferences are the window's starting state only; F9/F12/joysticks stay live toggles
    // afterward.
    app.aspect_correct = def.ui.aspect_correct;
    // Seeds the status bar's cumulative Runtime readout with whatever this machine already accrued.
    app.total_runtime = std::time::Duration::from_secs(def.stats.runtime_secs);
    // `CocoApp::new` derived a display from the signal path alone, which can't tell a CoCo 3 TV
    // from composite.
    app.display = def.display();
    app.tv = crate::display::TVSettings {
        scanline_pct: def.ui.tv_scanline,
        noise_pct: def.ui.tv_noise,
        overscan_pct: def.ui.tv_overscan,
    }
    .clamped();
    app.kb_mode = match def.ui.kb_mode {
        machine_def::KbModeDTO::Positional => KbMode::Positional,
        machine_def::KbModeDTO::Symbolic => KbMode::Symbolic,
    };
    app.joysticks.sources[coco_core::joystick::RIGHT] = def.ui.joy_right.into();
    app.joysticks.sources[coco_core::joystick::LEFT] = def.ui.joy_left.into();
    Ok(app)
}

/// The definition's system ROM: an explicit `[hardware].rom`
/// ([`load_explicit_rom`]) or the default resolution ([`load_default_rom`]),
/// paired with the [`ROMSource`] a snapshot needs to re-resolve it.
fn load_rom(
    explicit: Option<&Path>,
    variant: MachineVariant,
) -> Result<(Box<[u8]>, ROMSource), String> {
    match explicit {
        Some(path) => Ok((
            load_explicit_rom(path)?,
            ROMSource::File(path.to_path_buf()),
        )),
        None => load_default_rom(variant, &installed_roms_dir()),
    }
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

/// Enforced by hand: cart, disk0/disk1 (implying FD-502), rtc, and rs232 all
/// want the single cartridge port unless an MPI is installed; rs232 has no
/// MPI-slot support yet, so `mpi && rs232` is rejected too.
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

/// With an MPI installed, cart and disks target its slots instead of the
/// single-cartridge model, so the constructor gets neither —
/// [`mount_peripherals`] wires them up afterward.
fn new_app(
    config: coco_core::MachineConfig,
    rom: Box<[u8]>,
    rom_source: ROMSource,
    media: &Media,
    peripherals: &Peripherals,
) -> CocoApp {
    let (cart, disks) = if peripherals.mpi {
        (None, [None, None])
    } else {
        (media.cart.clone(), media.disks.clone())
    };
    CocoApp::new(
        config,
        rom,
        rom_source,
        AppParams {
            cart_path: cart,
            disk_paths: disks,
            vhd_paths: media.vhds.clone(),
            // DriveWire and tape-wav stay at their defaults: no definition
            // field drives them yet (see the `AppParams` field docs).
            ..AppParams::default()
        },
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
        // cart/fd502/rs232 and rtc are mutually exclusive here — `check_cartridge_port`
        // rejected other combos.
        app.insert_rtc();
    } else if peripherals.rs232 {
        // Starts on the inert Loopback endpoint; TCP/PTY stay a runtime-menu-only setting.
        app.insert_rs232();
    } else if peripherals.fd502
        && let Err(e) = app.ensure_disk_controller()
    {
        // Empty-drive FD-502 only; with disk media set, `CocoApp::new` already inserted the
        // controller.
        app.cart_error = Some(e);
    }

    if let Some(path) = media.tape {
        app.insert_tape(path);
    }
}

/// Wire `[ports].serial` — the built-in bit-banger serial port's host sink,
/// distinct from the cartridge-port RS-232 Pak. Runs before
/// `launch_machine`'s `cart_error` promotion so failures surface the same way.
fn mount_serial(app: &mut CocoApp, serial: Option<machine_def::SerialDTO>, slug: &str) {
    match serial {
        // Attached with the paper window closed; output accumulates and View ▸ Printer Paper
        // shows it.
        Some(machine_def::SerialDTO::Printer) => app.attach_dmp105(),
        Some(machine_def::SerialDTO::File) => {
            let path = machine_def::resolve_media_path(PRINTOUT_FILE, slug);
            if let Some(parent) = path.parent() {
                // The artifact directory may not exist yet for a `[ports]`-only definition.
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
