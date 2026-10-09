//! The manager's boot path, and the only one in the app: turning a saved
//! machine definition into a running [`CocoApp`].

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::MachineVariant;

use crate::app::DriveWireLaunch;
use crate::machine_def::{CartridgeDTO, DosRom, RS232EndpointDTO, SlotDTO};
use crate::rom_load::{load_default_rom, load_explicit_rom};
use crate::{
    AppParams, CocoApp, KbMode, MPI_SLOT_COUNT, ROMSource, RS232EndpointKind, UI_DRIVES,
    installed_roms_dir, machine_def,
};

/// File name [`launch_machine`] captures `[ports].serial = "file"` to,
/// resolved against the machine's artifact directory — the same auto-named
/// target the runtime Printer menu's "Start Print Capture…" item suggests
/// (`chrome::menus::print_capture`'s file dialog).
const PRINTOUT_FILE: &str = "printout.txt";

/// Where a definition's `[media]` paths land once resolved against the
/// machine's own directory (`machine_def::resolve_media_path`).
struct Media {
    disks: [Option<PathBuf>; UI_DRIVES],
    vhds: [Option<PathBuf>; UI_DRIVES],
    tape: Option<PathBuf>,
    drivewire: [Option<PathBuf>; coco_core::drivewire::DRIVE_COUNT],
}

/// The definition's cartridge-port occupant, with any embedded path already
/// resolved against the machine's artifact directory like `[media]`'s
/// (`resolve_cartridge`).
enum Cartridge {
    None,
    FD502 {
        dos_rom: DosRom,
    },
    ROMPak {
        path: PathBuf,
    },
    BankedROMPak {
        path: PathBuf,
    },
    RTC,
    RS232 {
        endpoint: RS232EndpointDTO,
    },
    GamesMaster {
        path: PathBuf,
    },
    Orch90,
    SoundSpeech,
    CoCoMax,
    MPI {
        slots: [Slot; MPI_SLOT_COUNT],
        /// 1-based front-panel slot ([`CartridgeDTO::MPI`]'s `switch`);
        /// converted to the app's 0-based convention in
        /// [`mount_peripherals`].
        switch: usize,
    },
}

/// One MultiPak slot's occupant — [`Cartridge`]'s sibling minus nested MPI
/// (real MPIs can't nest).
enum Slot {
    Empty,
    FD502 { dos_rom: DosRom },
    ROMPak { path: PathBuf },
    BankedROMPak { path: PathBuf },
    RTC,
    RS232 { endpoint: RS232EndpointDTO },
    GamesMaster { path: PathBuf },
    Orch90,
    SoundSpeech,
    CoCoMax,
}

/// Builds a running [`CocoApp`] from a saved machine definition, loads the ROM,
/// and mounts `[media]`, `[peripherals]`, and `[ports]`. Any failure returns `Err`
/// instead of a partial VM.
pub(crate) fn launch_machine_with_gamepad(
    def: &machine_def::MachineDef,
    slug: &str,
    gamepad: crate::joy::SharedGamepad,
) -> Result<CocoApp, String> {
    let config = def.to_machine_config()?;
    let explicit_rom = def.hardware.rom.as_ref().map(PathBuf::from);
    let (rom, rom_source) = load_rom(explicit_rom.as_deref(), config.variant)?;

    let media = resolve_media(def, slug);
    let cartridge = resolve_cartridge(def, slug);
    validate_disk_media(&media, &cartridge)?;
    validate_drivewire_media(def, &media)?;

    let drivewire = drivewire_settings(def, slug);
    let mut app = new_app(
        config, rom, rom_source, &media, &cartridge, drivewire, gamepad,
    );
    mount_peripherals(&mut app, media, cartridge);
    mount_serial(&mut app, def.ports.serial, slug);

    // Promote any `cart_error` the insert_*/mpi_insert_* helpers recorded into this launch
    // `Result`.
    if let Some(err) = app.cart_error.take() {
        return Err(err);
    }

    // [ui] preferences are the window's starting state only; F12/joysticks stay live toggles
    // afterward.
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
    app.machine.bus.joysticks.set_hires(
        coco_core::joystick::RIGHT,
        crate::joy::HiResChoice::from(def.ui.hires_right).into(),
    );
    app.machine.bus.joysticks.set_hires(
        coco_core::joystick::LEFT,
        crate::joy::HiResChoice::from(def.ui.hires_left).into(),
    );
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
        drivewire: resolve_drivewire_paths(def, slug),
    }
}

/// `[drivewire]` with its paths resolved; `None` when DriveWire is disabled.
pub(crate) fn drivewire_settings(
    def: &machine_def::MachineDef,
    slug: &str,
) -> Option<DriveWireLaunch> {
    def.drivewire.enabled.then(|| DriveWireLaunch {
        hdbdos_mode: def.drivewire.hdbdos_mode,
        disk_paths: resolve_drivewire_paths(def, slug),
    })
}

fn resolve_drivewire_paths(
    def: &machine_def::MachineDef,
    slug: &str,
) -> [Option<PathBuf>; coco_core::drivewire::DRIVE_COUNT] {
    def.drivewire
        .disk_paths()
        .map(|path| path.map(|p| machine_def::resolve_media_path(p, slug)))
}

fn validate_drivewire_media(def: &machine_def::MachineDef, media: &Media) -> Result<(), String> {
    if !def.drivewire.enabled {
        return Ok(());
    }
    for path in media.drivewire.iter().flatten() {
        let metadata = fs::metadata(path)
            .map_err(|e| format!("could not open DriveWire image {}: {e}", path.display()))?;
        if !metadata.is_file() {
            return Err(format!("DriveWire image {} is not a file", path.display()));
        }
    }
    Ok(())
}

/// Resolve `[peripherals].cartridge` into [`Cartridge`], resolving any
/// embedded path like [`resolve_media`] does for `[media]`.
fn resolve_cartridge(def: &machine_def::MachineDef, slug: &str) -> Cartridge {
    let path = |p: &str| machine_def::resolve_media_path(p, slug);
    match &def.peripherals.cartridge {
        CartridgeDTO::None => Cartridge::None,
        CartridgeDTO::FD502 { dos_rom } => Cartridge::FD502 { dos_rom: *dos_rom },
        CartridgeDTO::ROMPak { path: p } => Cartridge::ROMPak { path: path(p) },
        CartridgeDTO::BankedROMPak { path: p } => Cartridge::BankedROMPak { path: path(p) },
        CartridgeDTO::RTC => Cartridge::RTC,
        CartridgeDTO::RS232 { endpoint } => Cartridge::RS232 {
            endpoint: endpoint.clone(),
        },
        CartridgeDTO::GamesMaster { path: p } => Cartridge::GamesMaster { path: path(p) },
        CartridgeDTO::Orch90 => Cartridge::Orch90,
        CartridgeDTO::SoundSpeech => Cartridge::SoundSpeech,
        CartridgeDTO::CoCoMax => Cartridge::CoCoMax,
        CartridgeDTO::MPI { slots, switch } => Cartridge::MPI {
            slots: std::array::from_fn(|i| resolve_slot(&slots[i], slug)),
            switch: *switch,
        },
    }
}

fn resolve_slot(slot: &SlotDTO, slug: &str) -> Slot {
    let path = |p: &str| machine_def::resolve_media_path(p, slug);
    match slot {
        SlotDTO::Empty => Slot::Empty,
        SlotDTO::FD502 { dos_rom } => Slot::FD502 { dos_rom: *dos_rom },
        SlotDTO::ROMPak { path: p } => Slot::ROMPak { path: path(p) },
        SlotDTO::BankedROMPak { path: p } => Slot::BankedROMPak { path: path(p) },
        SlotDTO::RTC => Slot::RTC,
        SlotDTO::RS232 { endpoint } => Slot::RS232 {
            endpoint: endpoint.clone(),
        },
        SlotDTO::GamesMaster { path: p } => Slot::GamesMaster { path: path(p) },
        SlotDTO::Orch90 => Slot::Orch90,
        SlotDTO::SoundSpeech => Slot::SoundSpeech,
        SlotDTO::CoCoMax => Slot::CoCoMax,
    }
}

/// Whether a disk controller is reachable: the bare FD-502, or one in an MPI slot.
fn cartridge_has_fd502(cartridge: &Cartridge) -> bool {
    match cartridge {
        Cartridge::FD502 { .. } => true,
        Cartridge::MPI { slots, .. } => slots.iter().any(|s| matches!(s, Slot::FD502 { .. })),
        Cartridge::None
        | Cartridge::ROMPak { .. }
        | Cartridge::BankedROMPak { .. }
        | Cartridge::RTC
        | Cartridge::RS232 { .. }
        | Cartridge::GamesMaster { .. }
        | Cartridge::Orch90
        | Cartridge::SoundSpeech
        | Cartridge::CoCoMax => false,
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

/// The constructor only ever loads a ROM Pak directly (`CocoApp::insert_cartridge`);
/// every other cartridge kind, and the disk media that needs an FD-502 to exist
/// first, is mounted afterward by [`mount_peripherals`].
fn new_app(
    config: coco_core::MachineConfig,
    rom: Box<[u8]>,
    rom_source: ROMSource,
    media: &Media,
    cartridge: &Cartridge,
    drivewire: Option<DriveWireLaunch>,
    gamepad: crate::joy::SharedGamepad,
) -> CocoApp {
    let cart_path = match cartridge {
        Cartridge::ROMPak { path } => Some(path.clone()),
        _ => None,
    };
    CocoApp::new(
        config,
        rom,
        rom_source,
        AppParams {
            cart_path,
            vhd_paths: media.vhds.clone(),
            drivewire,
            ..AppParams::default()
        },
        gamepad,
    )
}

/// Test-only convenience for launch behavior that isn't concerned with manager
/// ownership. Production launches always receive the manager's shared backend.
#[cfg(test)]
pub(crate) fn launch_machine(def: &machine_def::MachineDef, slug: &str) -> Result<CocoApp, String> {
    launch_machine_with_gamepad(def, slug, crate::joy::SharedGamepad::without_backend())
}

/// Installs whichever peripheral claims the cartridge port (or an MPI slot), then
/// mounts disk media once its FD-502 exists — `validate_disk_media` already
/// guaranteed one is reachable, so only a failed insert leaves the drives empty.
fn mount_peripherals(app: &mut CocoApp, media: Media, cartridge: Cartridge) {
    match cartridge {
        Cartridge::None | Cartridge::ROMPak { .. } => {}
        Cartridge::BankedROMPak { path } => app.insert_banked_rompak(path),
        Cartridge::FD502 { dos_rom } => {
            if let Err(e) = app.insert_disk_controller(dos_rom) {
                app.cart_error = Some(e);
            }
        }
        Cartridge::RTC => app.insert_rtc(),
        Cartridge::RS232 { endpoint } => mount_rs232(app, endpoint),
        Cartridge::GamesMaster { path } => app.insert_gmc(path),
        Cartridge::Orch90 => app.insert_orch90(),
        Cartridge::SoundSpeech => app.insert_ssc(),
        Cartridge::CoCoMax => app.insert_cocomax(),
        Cartridge::MPI { slots, switch } => {
            app.insert_multipak();
            for (slot, occupant) in slots.into_iter().enumerate() {
                match occupant {
                    Slot::Empty => {}
                    Slot::FD502 { dos_rom } => app.mpi_insert_fd502(slot, dos_rom),
                    Slot::ROMPak { path } => app.mpi_insert_rompak(slot, path),
                    Slot::BankedROMPak { path } => app.mpi_insert_banked_rompak(slot, path),
                    Slot::RTC => app.mpi_insert_rtc(slot),
                    Slot::RS232 { endpoint } => {
                        app.mpi_insert_rs232(slot);
                        apply_rs232_endpoint(app, endpoint);
                    }
                    Slot::GamesMaster { path } => app.mpi_insert_gmc(slot, path),
                    Slot::Orch90 => app.mpi_insert_orch90(slot),
                    Slot::SoundSpeech => app.mpi_insert_ssc(slot),
                    Slot::CoCoMax => app.mpi_insert_cocomax(slot),
                }
            }
            // `switch` is 1-based (matching the UI's "Slot 1"); `mpi_set_switch` is 0-based.
            app.mpi_set_switch(switch - 1);
        }
    }

    // Query the machine, not the definition: a failed controller insert (bare or slotted)
    // already left its own cart_error, which a doomed insert_disk would overwrite.
    if app.machine.bus.cart.as_disk_cart().is_some() {
        for (drive, path) in media.disks.into_iter().enumerate() {
            if let Some(path) = path {
                app.insert_disk(drive, path);
            }
        }
    }

    if let Some(path) = media.tape {
        app.insert_tape(path);
    }
}

/// Insert the Deluxe RS-232 Pak into the bare cartridge port and wire its serial
/// line to `endpoint` — see [`apply_rs232_endpoint`]'s doc for the endpoint half,
/// shared with the MPI-slot case ([`mount_peripherals`]'s `Slot::RS232` arm).
fn mount_rs232(app: &mut CocoApp, endpoint: RS232EndpointDTO) {
    app.insert_rs232();
    apply_rs232_endpoint(app, endpoint);
}

/// Wire `endpoint` onto whichever Deluxe RS-232 Pak was inserted — bare port
/// ([`mount_rs232`]) or an MPI slot ([`mount_peripherals`]'s `Slot::RS232` arm);
/// loopback is [`CocoApp::insert_rs232`]/[`CocoApp::mpi_insert_rs232`]'s own
/// default, so it needs no follow-up call. Records `app.rs232_configured` for a
/// non-loopback pick so [`CocoApp::rebuild_cart_mirrors`] can rebind it after a
/// Load State drops the core's live endpoint.
///
/// A TCP endpoint that fails to bind falls back to loopback (the pak is still usable
/// on its inert default) and reports through the non-fatal status-bar toast instead of
/// failing the whole launch — every machine definition otherwise defaults to the same
/// listen address ([`crate::RS232_TCP_DEFAULT_ADDR`]), so a second RS-232 machine would always
/// refuse to launch.
fn apply_rs232_endpoint(app: &mut CocoApp, endpoint: RS232EndpointDTO) {
    match endpoint {
        RS232EndpointDTO::Loopback => {}
        RS232EndpointDTO::TCP { listen } => {
            // Only a bind failure from this call is demoted to a toast; an earlier
            // slot's fatal cart_error (for example, a missing Disk BASIC ROM) must survive.
            let prior = app.cart_error.take();
            app.rs232_tcp_addr = listen;
            app.rs232_configured = Some(RS232EndpointKind::TCP);
            app.rs232_set_endpoint(RS232EndpointKind::TCP);
            if let Some(err) = app.cart_error.take() {
                app.toast = Some((
                    format!(
                        "{err} -- edit this definition's Listen address field; \
                         falling back to loopback"
                    ),
                    std::time::Instant::now(),
                ));
            }
            app.cart_error = prior;
        }
        RS232EndpointDTO::PTY => mount_rs232_pty(app),
    }
}

#[cfg(unix)]
fn mount_rs232_pty(app: &mut CocoApp) {
    app.rs232_configured = Some(RS232EndpointKind::PTY);
    app.rs232_set_endpoint(RS232EndpointKind::PTY);
}

#[cfg(not(unix))]
fn mount_rs232_pty(app: &mut CocoApp) {
    app.cart_error = Some("PTY endpoints require a Unix host".to_string());
}

/// Wire `[ports].serial` — the built-in bit-banger serial port's host sink,
/// distinct from the cartridge-port RS-232 Pak. Runs before
/// `launch_machine`'s `cart_error` promotion so failures surface the same way.
fn mount_serial(app: &mut CocoApp, serial: Option<machine_def::SerialDTO>, slug: &str) {
    match serial {
        // Attached with the paper window closed; output accumulates and the status bar's
        // printer menu ("View Papers") shows it.
        Some(machine_def::SerialDTO::Printer) => {
            app.attach_printer(coco_core::dmp::DmpModel::Dmp105);
        }
        Some(machine_def::SerialDTO::Dmp130) => {
            app.attach_printer(coco_core::dmp::DmpModel::Dmp130);
        }
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

#[cfg(test)]
#[path = "launch_drivewire_test.rs"]
mod drivewire_tests;
