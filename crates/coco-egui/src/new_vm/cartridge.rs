//! The Cartridge-row and MPI-slot choice types ([`CartridgeChoice`],
//! [`SlotChoice`], [`RS232EndpointChoice`]), their combo labels/file
//! dialogs, and the `[peripherals]` pack/seed pair
//! ([`pack_peripherals`]/[`seed_peripherals`]) that is the one place form
//! picks and `machine_def::PeripheralsDTO` convert between each
//! other — split out of `new_vm.rs` once it grew past the project's
//! ~500-line ceiling.

use std::path::{Path, PathBuf};

use coco_core::rom_db::{self, CartridgeHardware, KnownCartridgeROM};
use owo_colors::colors::xterm;
use owo_colors::{OwoColorize, Stream};

use crate::machine_def::{self, CartridgeDTO, RS232EndpointDTO, SlotDTO};

/// The form's Cartridge row. Not part of [`MachineConfig`] — the
/// cartridge port is populated after machine construction (the same way
/// `launch::mount_peripherals` does it) — so it rides
/// alongside the config in [`super::MachineForm`] and is packed into the
/// definition's `[peripherals]` by [`pack_peripherals`].
///
/// [`MachineConfig`]: coco_core::MachineConfig
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CartridgeChoice {
    #[default]
    None,
    /// FD-502 disk controller (Disk BASIC ROM + WD1773, empty drives).
    FD502,
    /// A cartridge ROM image plugged straight into the port. Its hardware
    /// implementation comes from content detection or the unknown-ROM
    /// fallback in [`CartridgeImageChoice`].
    Image(CartridgeImageChoice),
    /// Disto RTC plugged straight into the port. No boot ROM — pairs with
    /// a VHD boot; for RTC + floppies use an MPI slot.
    RTC,
    /// Deluxe RS-232 Pak plugged straight into the port. The endpoint pick
    /// itself lives in [`super::MachineForm::rs232_endpoint`], a sibling
    /// field — the same relationship [`Self::MPI`] has with `mpi_slots`.
    /// Can also be picked per-slot ([`SlotChoice::RS232`]) while the MPI is
    /// selected.
    RS232,
    /// Orchestra-90/CC plugged straight into the port; picked with a file
    /// dialog on selection. Always autostarts — no `autostart` field.
    Orch90(PathBuf),
    /// Sound/Speech Cartridge plugged straight into the port. No file to
    /// pick.
    SoundSpeech,
    /// MultiPak Interface; the form then shows its four Slot rows, and
    /// the Disk rows only once a slot holds the FD-502. The switch pick
    /// itself lives in [`super::MachineForm::mpi_switch`], a sibling field,
    /// like `mpi_slots`.
    MPI,
}

/// One MultiPak slot's pick in the form's Slot rows (shown while the
/// cartridge is the MPI). At most one slot holds the FD-502 (a second
/// disk controller would fight the first for the SCS decode) and at most
/// one the Disto RTC (two would shadow each other at `$FF50`). ROM Paks,
/// the Games Master, Orchestra-90, and the Sound/Speech Cartridge carry no
/// such conflict: any number of slots may hold one of each
/// ([`crate::CocoApp::mpi_insert_ssc`]'s doc).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SlotChoice {
    #[default]
    Empty,
    FD502,
    /// A cartridge ROM image in this slot (see [`CartridgeChoice::Image`]).
    Image(CartridgeImageChoice),
    /// Disto RTC in this slot (see [`CartridgeChoice::RTC`]).
    RTC,
    /// Deluxe RS-232 Pak in this slot (see [`CartridgeChoice::RS232`]); at
    /// most one across the whole loadout — two would fight over the shared
    /// ACIA at `$FF68`, reachable from any slot regardless of switch/`$FF7F`
    /// selection (the pak decodes the full address bus itself).
    RS232(RS232EndpointChoice),
    /// Orchestra-90/CC in this slot (see [`CartridgeChoice::Orch90`]).
    Orch90(PathBuf),
    /// Sound/Speech Cartridge in this slot (see
    /// [`CartridgeChoice::SoundSpeech`]).
    SoundSpeech,
}

/// One cartridge ROM selection and the hardware used to run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CartridgeImageChoice {
    pub path: PathBuf,
    pub autostart: bool,
    pub hardware: CartridgeHardware,
    /// `true` when the ROM database identified `hardware`; `false` when the
    /// user can change the fallback for an unknown or unreadable image.
    pub hardware_detected: bool,
}

/// [`super::MachineForm::rs232_endpoint`]'s pick — which host backend the
/// Deluxe RS-232 Pak's serial line is wired to, shown only while
/// [`CartridgeChoice::RS232`] is the Cartridge pick. Maps to
/// [`machine_def::RS232EndpointDTO`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RS232EndpointChoice {
    /// TX loops straight back to RX — the pak's inert power-on default.
    #[default]
    Loopback,
    /// TCP listener at `listen`; a host terminal connects with `nc`/`telnet`.
    TCP { listen: String },
    /// Unix pseudo-terminal. Offered only on Unix — there is no PTY to open
    /// on Windows.
    #[cfg(unix)]
    PTY,
}

pub(super) fn slot_label(slot: &SlotChoice) -> String {
    match slot {
        SlotChoice::Empty => "Empty".to_string(),
        SlotChoice::FD502 => "FD-502".to_string(),
        SlotChoice::Image(image) => cart_file_name(&image.path, "Cartridge ROM"),
        SlotChoice::RTC => RTC_LABEL.to_string(),
        SlotChoice::RS232(_) => "RS-232 Pak".to_string(),
        SlotChoice::Orch90(path) => cart_file_name(path, "Orchestra-90"),
        SlotChoice::SoundSpeech => "Sound/Speech Cartridge".to_string(),
    }
}

pub(super) fn cartridge_label(cartridge: &CartridgeChoice) -> String {
    match cartridge {
        CartridgeChoice::None => "None".to_string(),
        CartridgeChoice::FD502 => "FD-502".to_string(),
        CartridgeChoice::Image(image) => cart_file_name(&image.path, "Cartridge ROM"),
        CartridgeChoice::RTC => RTC_LABEL.to_string(),
        CartridgeChoice::RS232 => "RS-232 Pak".to_string(),
        CartridgeChoice::Orch90(path) => cart_file_name(path, "Orchestra-90"),
        CartridgeChoice::SoundSpeech => "Sound/Speech Cartridge".to_string(),
        CartridgeChoice::MPI => "MultiPak Interface".to_string(),
    }
}

/// Combo text for a picked cartridge image: its file name, or `fallback` if
/// the path is unnamed (for example, `/`).
fn cart_file_name(path: &std::path::Path, fallback: &str) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| fallback.to_string())
}

/// Combo text for the Disto RTC pick; names the card so the user knows which
/// NitrOS-9 clock driver (`clock2_disto4`) it answers to.
pub(super) const RTC_LABEL: &str = "Disto RTC (4-N-1)";

/// The extensions every cartridge-image file dialog accepts.
const ROM_EXTENSIONS: &[&str] = &["rom", "ccc", "bin"];

/// The unified cartridge ROM combo entry's file dialog.
pub(super) fn cartridge_rom_file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Cartridge ROM", ROM_EXTENSIONS)
}

/// The Orchestra-90 combo entry's file dialog.
pub(super) fn orch90_file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Orchestra-90 ROM", ROM_EXTENSIONS)
}

/// Image-backed ROM cartridges' autostart default when a combo pick first
/// creates one — checked, matching `CartridgeDTO`'s own field default.
pub(super) const DEFAULT_AUTOSTART: bool = true;

fn identify_image(path: &Path) -> Option<&'static KnownCartridgeROM> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| rom_db::identify_cartridge(&bytes))
}

fn image_choice(
    path: PathBuf,
    autostart: bool,
    fallback: CartridgeHardware,
) -> CartridgeImageChoice {
    let detected = identify_image(&path);
    CartridgeImageChoice {
        path,
        autostart,
        hardware: detected.map_or(fallback, |known| known.hardware),
        hardware_detected: detected.is_some(),
    }
}

fn hardware_name(hardware: CartridgeHardware) -> &'static str {
    match hardware {
        CartridgeHardware::RomPak => "ROM Pak",
        CartridgeHardware::BankedRomPak => "Banked ROM Pak",
        CartridgeHardware::GamesMaster => "Games Master Cartridge",
    }
}

fn announce_image_detection(known: Option<&KnownCartridgeROM>) {
    match known {
        Some(known) => println!(
            " {} {} {} {}",
            "Detected".if_supports_color(Stream::Stdout, |v| v.fg::<xterm::PersianGreen>()),
            known
                .title()
                .if_supports_color(Stream::Stdout, |v| v.cyan()),
            "→".if_supports_color(Stream::Stdout, |v| v.dimmed()),
            hardware_name(known.hardware).if_supports_color(Stream::Stdout, |v| v.white()),
        ),
        None => println!(
            " {} {} {} {}",
            "Unrecognized"
                .if_supports_color(Stream::Stdout, |v| { v.fg::<xterm::BittersweetOrange>() }),
            "cartridge ROM".if_supports_color(Stream::Stdout, |v| v.cyan()),
            "→".if_supports_color(Stream::Stdout, |v| v.dimmed()),
            "using ROM Pak fallback".if_supports_color(Stream::Stdout, |v| v.white()),
        ),
    }
}

fn selected_image_choice(path: PathBuf) -> CartridgeImageChoice {
    let known = identify_image(&path);
    announce_image_detection(known);
    CartridgeImageChoice {
        path,
        autostart: DEFAULT_AUTOSTART,
        hardware: known.map_or(CartridgeHardware::RomPak, |rom| rom.hardware),
        hardware_detected: known.is_some(),
    }
}

fn persisted_image_choice(
    path: &str,
    autostart: bool,
    hardware: CartridgeHardware,
) -> CartridgeImageChoice {
    let mut image = image_choice(PathBuf::from(path), autostart, hardware);
    image.hardware_detected &= image.hardware == hardware;
    image.hardware = hardware;
    image
}

/// A content-detected cartridge image, with ROM Pak as the unknown fallback.
pub(super) fn cartridge_rom(path: PathBuf) -> CartridgeChoice {
    CartridgeChoice::Image(selected_image_choice(path))
}

/// [`cartridge_rom`]'s MPI-slot sibling.
pub(super) fn slot_cartridge_rom(path: PathBuf) -> SlotChoice {
    SlotChoice::Image(selected_image_choice(path))
}

fn slot_dto_for_image(image: &CartridgeImageChoice) -> SlotDTO {
    let path = image.path.display().to_string();
    match image.hardware {
        CartridgeHardware::RomPak => SlotDTO::ROMPak {
            path,
            autostart: image.autostart,
        },
        CartridgeHardware::BankedRomPak => SlotDTO::BankedROMPak {
            path,
            autostart: image.autostart,
        },
        CartridgeHardware::GamesMaster => SlotDTO::GamesMaster {
            path,
            autostart: image.autostart,
        },
    }
}

fn cartridge_dto_for_image(image: &CartridgeImageChoice) -> CartridgeDTO {
    let path = image.path.display().to_string();
    match image.hardware {
        CartridgeHardware::RomPak => CartridgeDTO::ROMPak {
            path,
            autostart: image.autostart,
        },
        CartridgeHardware::BankedRomPak => CartridgeDTO::BankedROMPak {
            path,
            autostart: image.autostart,
        },
        CartridgeHardware::GamesMaster => CartridgeDTO::GamesMaster {
            path,
            autostart: image.autostart,
        },
    }
}

impl From<&CartridgeDTO> for CartridgeChoice {
    /// `CartridgeDTO::RS232`'s `endpoint` and `MPI`'s `switch` aren't
    /// representable here — [`seed_peripherals`] reads those directly off
    /// the DTO into the form's sibling fields instead of through this impl.
    fn from(dto: &CartridgeDTO) -> Self {
        match dto {
            CartridgeDTO::None => CartridgeChoice::None,
            CartridgeDTO::FD502 => CartridgeChoice::FD502,
            CartridgeDTO::ROMPak { path, autostart } => CartridgeChoice::Image(
                persisted_image_choice(path, *autostart, CartridgeHardware::RomPak),
            ),
            CartridgeDTO::BankedROMPak { path, autostart } => CartridgeChoice::Image(
                persisted_image_choice(path, *autostart, CartridgeHardware::BankedRomPak),
            ),
            CartridgeDTO::RTC => CartridgeChoice::RTC,
            CartridgeDTO::RS232 { .. } => CartridgeChoice::RS232,
            CartridgeDTO::GamesMaster { path, autostart } => CartridgeChoice::Image(
                persisted_image_choice(path, *autostart, CartridgeHardware::GamesMaster),
            ),
            CartridgeDTO::Orch90 { path } => CartridgeChoice::Orch90(PathBuf::from(path)),
            CartridgeDTO::SoundSpeech => CartridgeChoice::SoundSpeech,
            CartridgeDTO::MPI { .. } => CartridgeChoice::MPI,
        }
    }
}

impl From<&SlotChoice> for SlotDTO {
    fn from(choice: &SlotChoice) -> Self {
        match choice {
            SlotChoice::Empty => SlotDTO::Empty,
            SlotChoice::FD502 => SlotDTO::FD502,
            SlotChoice::Image(image) => slot_dto_for_image(image),
            SlotChoice::RTC => SlotDTO::RTC,
            SlotChoice::RS232(endpoint) => SlotDTO::RS232 {
                endpoint: endpoint.into(),
            },
            SlotChoice::Orch90(path) => SlotDTO::Orch90 {
                path: path.display().to_string(),
            },
            SlotChoice::SoundSpeech => SlotDTO::SoundSpeech,
        }
    }
}

impl From<&SlotDTO> for SlotChoice {
    fn from(dto: &SlotDTO) -> Self {
        match dto {
            SlotDTO::Empty => SlotChoice::Empty,
            SlotDTO::FD502 => SlotChoice::FD502,
            SlotDTO::ROMPak { path, autostart } => SlotChoice::Image(persisted_image_choice(
                path,
                *autostart,
                CartridgeHardware::RomPak,
            )),
            SlotDTO::BankedROMPak { path, autostart } => SlotChoice::Image(persisted_image_choice(
                path,
                *autostart,
                CartridgeHardware::BankedRomPak,
            )),
            SlotDTO::RTC => SlotChoice::RTC,
            SlotDTO::RS232 { endpoint } => SlotChoice::RS232(endpoint.into()),
            SlotDTO::GamesMaster { path, autostart } => SlotChoice::Image(persisted_image_choice(
                path,
                *autostart,
                CartridgeHardware::GamesMaster,
            )),
            SlotDTO::Orch90 { path } => SlotChoice::Orch90(PathBuf::from(path)),
            SlotDTO::SoundSpeech => SlotChoice::SoundSpeech,
        }
    }
}

impl From<&RS232EndpointChoice> for RS232EndpointDTO {
    fn from(choice: &RS232EndpointChoice) -> Self {
        match choice {
            RS232EndpointChoice::Loopback => RS232EndpointDTO::Loopback,
            RS232EndpointChoice::TCP { listen } => RS232EndpointDTO::TCP {
                listen: listen.clone(),
            },
            #[cfg(unix)]
            RS232EndpointChoice::PTY => RS232EndpointDTO::PTY,
        }
    }
}

impl From<&RS232EndpointDTO> for RS232EndpointChoice {
    /// A [`RS232EndpointDTO::PTY`] loaded on a non-Unix host seeds the form
    /// back to [`Self::Loopback`] — there is no PTY combo entry to hold it
    /// there, and `pack_peripherals` would otherwise silently drop it on the
    /// next save. `launch::mount_rs232_pty` is what actually rejects it at
    /// launch time.
    fn from(dto: &RS232EndpointDTO) -> Self {
        match dto {
            RS232EndpointDTO::Loopback => RS232EndpointChoice::Loopback,
            RS232EndpointDTO::TCP { listen } => RS232EndpointChoice::TCP {
                listen: listen.clone(),
            },
            RS232EndpointDTO::PTY => {
                #[cfg(unix)]
                {
                    RS232EndpointChoice::PTY
                }
                #[cfg(not(unix))]
                {
                    RS232EndpointChoice::Loopback
                }
            }
        }
    }
}

/// Pack the form's Cartridge/Slot/MPI-switch/RS-232-endpoint picks into
/// `[peripherals]` — the inverse of [`seed_peripherals`]. `CartridgeDTO::MPI`'s
/// `slots`/`switch` and `RS232`'s `endpoint` need form state beyond the
/// cartridge choice itself, so this builds them directly rather than through
/// a `From<&CartridgeChoice>` impl.
pub(crate) fn pack_peripherals(
    cartridge: &CartridgeChoice,
    mpi_slots: &[SlotChoice; crate::MPI_SLOT_COUNT],
    mpi_switch: usize,
    rs232_endpoint: &RS232EndpointChoice,
) -> machine_def::PeripheralsDTO {
    let cartridge = match cartridge {
        CartridgeChoice::None => CartridgeDTO::None,
        CartridgeChoice::FD502 => CartridgeDTO::FD502,
        CartridgeChoice::Image(image) => cartridge_dto_for_image(image),
        CartridgeChoice::RTC => CartridgeDTO::RTC,
        CartridgeChoice::RS232 => CartridgeDTO::RS232 {
            endpoint: rs232_endpoint.into(),
        },
        CartridgeChoice::Orch90(path) => CartridgeDTO::Orch90 {
            path: path.display().to_string(),
        },
        CartridgeChoice::SoundSpeech => CartridgeDTO::SoundSpeech,
        CartridgeChoice::MPI => CartridgeDTO::MPI {
            slots: std::array::from_fn(|i| (&mpi_slots[i]).into()),
            // The form keeps the app's 0-based switch convention; the DTO is 1-based (the UI's
            // "Slot 1").
            switch: mpi_switch + 1,
        },
    };
    machine_def::PeripheralsDTO { cartridge }
}

/// Seed the form's Cartridge/Slot/MPI-switch/RS-232-endpoint picks from
/// `[peripherals]` — the inverse of [`pack_peripherals`]. A non-MPI cartridge
/// seeds every slot back to [`SlotChoice::Empty`] and the switch back to
/// [`crate::DEFAULT_MPI_SWITCH_SLOT`], since only [`CartridgeDTO::MPI`]
/// carries a loadout/switch; likewise a non-RS232 cartridge seeds the
/// endpoint back to [`RS232EndpointChoice::Loopback`].
pub(crate) fn seed_peripherals(
    peripherals: &machine_def::PeripheralsDTO,
) -> (
    CartridgeChoice,
    [SlotChoice; crate::MPI_SLOT_COUNT],
    usize,
    RS232EndpointChoice,
) {
    match &peripherals.cartridge {
        CartridgeDTO::MPI { slots, switch } => (
            CartridgeChoice::MPI,
            std::array::from_fn(|i| (&slots[i]).into()),
            switch - 1,
            RS232EndpointChoice::default(),
        ),
        CartridgeDTO::RS232 { endpoint } => (
            CartridgeChoice::RS232,
            std::array::from_fn(|_| SlotChoice::default()),
            crate::DEFAULT_MPI_SWITCH_SLOT,
            endpoint.into(),
        ),
        other => (
            other.into(),
            std::array::from_fn(|_| SlotChoice::default()),
            crate::DEFAULT_MPI_SWITCH_SLOT,
            RS232EndpointChoice::default(),
        ),
    }
}

#[cfg(test)]
#[path = "cartridge_test.rs"]
mod tests;
