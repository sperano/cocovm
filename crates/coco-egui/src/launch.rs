//! The manager's boot path, and the only one in the app: turning a saved
//! machine definition into a running [`CocoApp`].

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::MachineVariant;

use crate::machine_def::{CartridgeDTO, SlotDTO};
use crate::rom_load::{load_default_rom, load_explicit_rom};
use crate::{
    AppParams, CocoApp, KbMode, MPI_SLOT_COUNT, ROMSource, UI_DRIVES, installed_roms_dir,
    machine_def,
};

/// File name [`launch_machine`] captures `[ports].serial = "file"` to,
/// resolved against the machine's artifact directory — the same auto-named
/// target the runtime Machine menu's "Capture to File…" item suggests
/// (`chrome::menu_bar::machine`'s file dialog).
const PRINTOUT_FILE: &str = "printout.txt";

/// Where a definition's `[media]` paths land once resolved against the
/// machine's own directory (`machine_def::resolve_media_path`).
struct Media {
    disks: [Option<PathBuf>; UI_DRIVES],
    vhds: [Option<PathBuf>; UI_DRIVES],
    tape: Option<PathBuf>,
}

/// The definition's cartridge-port occupant, with any embedded path already
/// resolved against the machine's artifact directory like `[media]`'s
/// (`resolve_cartridge`).
enum Cartridge {
    None,
    FD502,
    ROMPak(PathBuf),
    RTC,
    /// Bare-port only — no `mpi_insert_rs232`, so this can't appear nested
    /// in a [`Slot`].
    RS232,
    GamesMaster(PathBuf),
    Orch90(PathBuf),
    SoundSpeech,
    MPI([Slot; MPI_SLOT_COUNT]),
}

/// One MultiPak slot's occupant — [`Cartridge`]'s sibling minus the
/// bare-port-only kinds (nested MPI, RS-232).
enum Slot {
    Empty,
    FD502,
    ROMPak(PathBuf),
    RTC,
    GamesMaster(PathBuf),
    Orch90(PathBuf),
    SoundSpeech,
}

/// Build a running [`CocoApp`] from a saved machine definition: load the ROM,
/// mount `[media]`/`[peripherals]`/`[ports]`. Any failure returns `Err`
/// instead of a partial VM.
pub(crate) fn launch_machine(def: &machine_def::MachineDef, slug: &str) -> Result<CocoApp, String> {
    let config = def.to_machine_config()?;
    let explicit_rom = def.hardware.rom.as_ref().map(PathBuf::from);
    let (rom, rom_source) = load_rom(explicit_rom.as_deref(), config.variant)?;

    let media = resolve_media(def, slug);
    let cartridge = resolve_cartridge(def, slug);
    validate_disk_media(&media, &cartridge)?;

    let mut app = new_app(config, rom, rom_source, &media, &cartridge);
    mount_peripherals(&mut app, media, cartridge);
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

/// Resolve `[peripherals].cartridge` into [`Cartridge`], resolving any
/// embedded path like [`resolve_media`] does for `[media]`.
fn resolve_cartridge(def: &machine_def::MachineDef, slug: &str) -> Cartridge {
    let path = |p: &str| machine_def::resolve_media_path(p, slug);
    match &def.peripherals.cartridge {
        CartridgeDTO::None => Cartridge::None,
        CartridgeDTO::FD502 => Cartridge::FD502,
        CartridgeDTO::ROMPak { path: p } => Cartridge::ROMPak(path(p)),
        CartridgeDTO::RTC => Cartridge::RTC,
        CartridgeDTO::RS232 => Cartridge::RS232,
        CartridgeDTO::GamesMaster { path: p } => Cartridge::GamesMaster(path(p)),
        CartridgeDTO::Orch90 { path: p } => Cartridge::Orch90(path(p)),
        CartridgeDTO::SoundSpeech => Cartridge::SoundSpeech,
        CartridgeDTO::MPI { slots } => {
            Cartridge::MPI(std::array::from_fn(|i| resolve_slot(&slots[i], slug)))
        }
    }
}

fn resolve_slot(slot: &SlotDTO, slug: &str) -> Slot {
    let path = |p: &str| machine_def::resolve_media_path(p, slug);
    match slot {
        SlotDTO::Empty => Slot::Empty,
        SlotDTO::FD502 => Slot::FD502,
        SlotDTO::ROMPak { path: p } => Slot::ROMPak(path(p)),
        SlotDTO::RTC => Slot::RTC,
        SlotDTO::GamesMaster { path: p } => Slot::GamesMaster(path(p)),
        SlotDTO::Orch90 { path: p } => Slot::Orch90(path(p)),
        SlotDTO::SoundSpeech => Slot::SoundSpeech,
    }
}

/// Whether a disk controller is reachable: the bare FD-502, or one in an MPI slot.
fn cartridge_has_fd502(cartridge: &Cartridge) -> bool {
    match cartridge {
        Cartridge::FD502 => true,
        Cartridge::MPI(slots) => slots.iter().any(|s| matches!(s, Slot::FD502)),
        Cartridge::None
        | Cartridge::ROMPak(_)
        | Cartridge::RTC
        | Cartridge::RS232
        | Cartridge::GamesMaster(_)
        | Cartridge::Orch90(_)
        | Cartridge::SoundSpeech => false,
    }
}

/// `disk0`/`disk1` need a disk controller to mean anything — unlike schema
/// 1, nothing implies one anymore, so listing disk media with none reachable
/// is a fatal error naming the fix, not a silent auto-insert.
fn validate_disk_media(media: &Media, cartridge: &Cartridge) -> Result<(), String> {
    if media.disks.iter().any(Option::is_some) && !cartridge_has_fd502(cartridge) {
        return Err(
            "disk0/disk1 need a disk controller: put the FD-502 in the cartridge port, or in \
             an MPI slot"
                .to_string(),
        );
    }
    Ok(())
}

/// With an MPI installed, or any cartridge kind beyond a bare ROM Pak, the
/// port is populated afterward by [`mount_peripherals`] instead — the
/// constructor only ever loads a ROM Pak directly (`CocoApp::insert_cartridge`).
fn new_app(
    config: coco_core::MachineConfig,
    rom: Box<[u8]>,
    rom_source: ROMSource,
    media: &Media,
    cartridge: &Cartridge,
) -> CocoApp {
    let cart_path = match cartridge {
        Cartridge::ROMPak(path) => Some(path.clone()),
        _ => None,
    };
    let disks = match cartridge {
        Cartridge::MPI(_) => [None, None],
        _ => media.disks.clone(),
    };
    CocoApp::new(
        config,
        rom,
        rom_source,
        AppParams {
            cart_path,
            disk_paths: disks,
            vhd_paths: media.vhds.clone(),
            // DriveWire and tape-wav stay at their defaults: no definition
            // field drives them yet (see the `AppParams` field docs).
            ..AppParams::default()
        },
    )
}

fn mount_peripherals(app: &mut CocoApp, media: Media, cartridge: Cartridge) {
    match cartridge {
        Cartridge::None | Cartridge::ROMPak(_) => {}
        Cartridge::FD502 => {
            // Empty-drive FD-502: a no-op if `CocoApp::new` already inserted the controller for
            // disk media passed through `AppParams`.
            if let Err(e) = app.ensure_disk_controller() {
                app.cart_error = Some(e);
            }
        }
        Cartridge::RTC => app.insert_rtc(),
        Cartridge::RS232 => app.insert_rs232(),
        Cartridge::GamesMaster(path) => app.insert_gmc(path),
        Cartridge::Orch90(path) => app.insert_orch90(path),
        Cartridge::SoundSpeech => app.insert_ssc(),
        Cartridge::MPI(slots) => {
            app.insert_multipak();
            for (slot, occupant) in slots.into_iter().enumerate() {
                match occupant {
                    Slot::Empty => {}
                    Slot::FD502 => app.mpi_insert_fd502(slot),
                    Slot::ROMPak(path) => app.mpi_insert_rompak(slot, path),
                    Slot::RTC => app.mpi_insert_rtc(slot),
                    Slot::GamesMaster(path) => app.mpi_insert_gmc(slot, path),
                    Slot::Orch90(path) => app.mpi_insert_orch90(slot, path),
                    Slot::SoundSpeech => app.mpi_insert_ssc(slot),
                }
            }
            for (drive, path) in media.disks.into_iter().enumerate() {
                if let Some(path) = path {
                    app.insert_disk(drive, path);
                }
            }
        }
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
