//! The cartridge-port half of the boot path: resolving a definition's
//! `[peripherals].cartridge` and installing it, either at launch
//! ([`super::launch_machine_with_gamepad`]) or into a running VM whose port
//! (or MultiPak slot) was just emptied ([`mount_cartridge`], [`mount_slot`]).

use std::path::PathBuf;

use crate::machine_def::{self, CartridgeDTO, DosRom, RS232EndpointDTO, SlotDTO};
use crate::{CocoApp, MPI_SLOT_COUNT, RS232EndpointKind};

/// The definition's cartridge-port occupant, with any embedded path already
/// resolved against the machine's artifact directory like `[media]`'s
/// ([`resolve`]).
pub(super) enum Cartridge {
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
        /// converted to the app's 0-based convention in [`install`].
        switch: usize,
    },
}

/// One MultiPak slot's occupant — [`Cartridge`]'s sibling minus nested MPI
/// (real MPIs can't nest).
pub(super) enum Slot {
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

/// Install `cartridge` into `app`'s empty cartridge port, the way launching a
/// definition naming it would. Failures land in `app.cart_error`, like the
/// `insert_*` helpers it calls; embedded paths resolve against `slug`'s
/// artifact directory.
pub(crate) fn mount_cartridge(app: &mut CocoApp, cartridge: &CartridgeDTO, slug: &str) {
    match resolve(cartridge, slug) {
        Cartridge::ROMPak { path } => app.insert_cartridge(path),
        other => install(app, other),
    }
}

/// Install `occupant` into the inserted MultiPak's empty `slot` (0-based),
/// the way launching a definition naming it would. Failures land in
/// `app.cart_error`.
pub(crate) fn mount_slot(app: &mut CocoApp, slot: usize, occupant: &SlotDTO, slug: &str) {
    install_slot(app, slot, resolve_slot(occupant, slug));
}

/// Resolve `[peripherals].cartridge` into [`Cartridge`], resolving any
/// embedded path like `launch::resolve_media` does for `[media]`.
pub(super) fn resolve(cartridge: &CartridgeDTO, slug: &str) -> Cartridge {
    let path = |p: &str| machine_def::resolve_media_path(p, slug);
    match cartridge {
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
pub(super) fn has_fd502(cartridge: &Cartridge) -> bool {
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

/// Install whichever peripheral claims the cartridge port (or an MPI slot).
/// A bare ROM Pak is left to the caller: at launch, `CocoApp::new` has already
/// loaded it.
pub(super) fn install(app: &mut CocoApp, cartridge: Cartridge) {
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
                install_slot(app, slot, occupant);
            }
            // `switch` is 1-based (matching the UI's "Slot 1"); `mpi_set_switch` is 0-based.
            app.mpi_set_switch(switch - 1);
        }
    }
}

fn install_slot(app: &mut CocoApp, slot: usize, occupant: Slot) {
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

/// Insert the Deluxe RS-232 Pak into the bare cartridge port and wire its serial
/// line to `endpoint` — see [`apply_rs232_endpoint`]'s doc for the endpoint half,
/// shared with the MPI-slot case ([`install_slot`]'s `Slot::RS232` arm).
fn mount_rs232(app: &mut CocoApp, endpoint: RS232EndpointDTO) {
    app.insert_rs232();
    apply_rs232_endpoint(app, endpoint);
}

/// Wire `endpoint` onto whichever Deluxe RS-232 Pak was inserted — bare port
/// ([`mount_rs232`]) or an MPI slot ([`install_slot`]'s `Slot::RS232` arm);
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
pub(super) fn apply_rs232_endpoint(app: &mut CocoApp, endpoint: RS232EndpointDTO) {
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
