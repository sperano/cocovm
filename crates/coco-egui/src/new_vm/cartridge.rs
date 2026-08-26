//! The Cartridge-row and MPI-slot choice types ([`CartridgeChoice`],
//! [`SlotChoice`]), their combo labels/file dialogs, and the `[peripherals]`
//! pack/seed pair ([`pack_peripherals`]/[`seed_peripherals`]) that is the one
//! place form picks and `machine_def::PeripheralsDTO` convert between each
//! other — split out of `new_vm.rs` once it grew past the project's
//! ~500-line ceiling.

use std::path::PathBuf;

use crate::machine_def::{self, CartridgeDTO, SlotDTO};

/// The form's Cartridge row. Not part of [`MachineConfig`] — the
/// cartridge port is populated after machine construction (the same way
/// `launch::mount_peripherals` and the Machine menu do it) — so it rides
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
    /// A program ROM Pak image plugged straight into the port; picked with
    /// a file dialog on selection.
    ROMPak(PathBuf),
    /// Disto RTC plugged straight into the port. No boot ROM — pairs with
    /// a VHD boot; for RTC + floppies use an MPI slot.
    RTC,
    /// Deluxe RS-232 Pak plugged straight into the port. Bare-port only —
    /// unlike the RTC/FD-502 there's no `mpi_insert_rs232`, so this choice
    /// isn't offered in the MPI's Slot combos.
    RS232,
    /// Games Master Cartridge (banked ROM + SN76489A) plugged straight into
    /// the port; picked with a file dialog on selection.
    GamesMaster(PathBuf),
    /// Orchestra-90/CC plugged straight into the port; picked with a file
    /// dialog on selection.
    Orch90(PathBuf),
    /// Sound/Speech Cartridge plugged straight into the port. No file to
    /// pick.
    SSC,
    /// MultiPak Interface; the form then shows its four Slot rows, and
    /// the Disk rows only once a slot holds the FD-502.
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
    /// A program ROM Pak image in this slot (see [`CartridgeChoice::ROMPak`]).
    ROMPak(PathBuf),
    /// Disto RTC in this slot (see [`CartridgeChoice::RTC`]).
    RTC,
    /// Games Master Cartridge in this slot (see [`CartridgeChoice::GamesMaster`]).
    GamesMaster(PathBuf),
    /// Orchestra-90/CC in this slot (see [`CartridgeChoice::Orch90`]).
    Orch90(PathBuf),
    /// Sound/Speech Cartridge in this slot (see [`CartridgeChoice::SSC`]).
    SSC,
}

pub(super) fn slot_label(slot: &SlotChoice) -> String {
    match slot {
        SlotChoice::Empty => "Empty".to_string(),
        SlotChoice::FD502 => "FD-502".to_string(),
        SlotChoice::ROMPak(path) => cart_file_name(path, "ROM Pak"),
        SlotChoice::RTC => "Disto RTC".to_string(),
        SlotChoice::GamesMaster(path) => cart_file_name(path, "Games Master"),
        SlotChoice::Orch90(path) => cart_file_name(path, "Orchestra-90"),
        SlotChoice::SSC => "Sound/Speech Cartridge".to_string(),
    }
}

pub(super) fn cartridge_label(cartridge: &CartridgeChoice) -> String {
    match cartridge {
        CartridgeChoice::None => "None".to_string(),
        CartridgeChoice::FD502 => "FD-502".to_string(),
        CartridgeChoice::ROMPak(path) => cart_file_name(path, "ROM Pak"),
        CartridgeChoice::RTC => "Disto RTC".to_string(),
        CartridgeChoice::RS232 => "RS-232 Pak".to_string(),
        CartridgeChoice::GamesMaster(path) => cart_file_name(path, "Games Master"),
        CartridgeChoice::Orch90(path) => cart_file_name(path, "Orchestra-90"),
        CartridgeChoice::SSC => "Sound/Speech Cartridge".to_string(),
        CartridgeChoice::MPI => "MultiPak Interface".to_string(),
    }
}

/// Combo text for a picked cartridge image: its file name, or `fallback` if
/// the path is unnamed (e.g. `/`).
fn cart_file_name(path: &std::path::Path, fallback: &str) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| fallback.to_string())
}

/// The extensions every cartridge-image file dialog (ROM Pak, Games Master,
/// Orchestra-90) accepts — the same set the Machine menu's own pickers use
/// (`chrome::menu_bar::machine`/`mpi`).
const ROM_EXTENSIONS: &[&str] = &["rom", "ccc", "bin"];

/// The same filter the Machine-menu "Insert Cartridge…" item uses.
pub(super) fn rom_pak_file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("ROM Pak", ROM_EXTENSIONS)
}

/// The same filter the Machine-menu "Insert Games Master…" item uses.
pub(super) fn gmc_file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Games Master ROM", ROM_EXTENSIONS)
}

/// The same filter the Machine-menu "Insert Orchestra-90…" item uses.
pub(super) fn orch90_file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Orchestra-90 ROM", ROM_EXTENSIONS)
}

impl From<&CartridgeChoice> for CartridgeDTO {
    fn from(choice: &CartridgeChoice) -> Self {
        match choice {
            CartridgeChoice::None => CartridgeDTO::None,
            CartridgeChoice::FD502 => CartridgeDTO::FD502,
            CartridgeChoice::ROMPak(path) => CartridgeDTO::ROMPak {
                path: path.display().to_string(),
            },
            CartridgeChoice::RTC => CartridgeDTO::RTC,
            CartridgeChoice::RS232 => CartridgeDTO::RS232,
            CartridgeChoice::GamesMaster(path) => CartridgeDTO::GamesMaster {
                path: path.display().to_string(),
            },
            CartridgeChoice::Orch90(path) => CartridgeDTO::Orch90 {
                path: path.display().to_string(),
            },
            CartridgeChoice::SSC => CartridgeDTO::SSC,
            CartridgeChoice::MPI => CartridgeDTO::MPI,
        }
    }
}

impl From<&CartridgeDTO> for CartridgeChoice {
    fn from(dto: &CartridgeDTO) -> Self {
        match dto {
            CartridgeDTO::None => CartridgeChoice::None,
            CartridgeDTO::FD502 => CartridgeChoice::FD502,
            CartridgeDTO::ROMPak { path } => CartridgeChoice::ROMPak(PathBuf::from(path)),
            CartridgeDTO::RTC => CartridgeChoice::RTC,
            CartridgeDTO::RS232 => CartridgeChoice::RS232,
            CartridgeDTO::GamesMaster { path } => CartridgeChoice::GamesMaster(PathBuf::from(path)),
            CartridgeDTO::Orch90 { path } => CartridgeChoice::Orch90(PathBuf::from(path)),
            CartridgeDTO::SSC => CartridgeChoice::SSC,
            CartridgeDTO::MPI => CartridgeChoice::MPI,
        }
    }
}

impl From<&SlotChoice> for SlotDTO {
    fn from(choice: &SlotChoice) -> Self {
        match choice {
            SlotChoice::Empty => SlotDTO::Empty,
            SlotChoice::FD502 => SlotDTO::FD502,
            SlotChoice::ROMPak(path) => SlotDTO::ROMPak {
                path: path.display().to_string(),
            },
            SlotChoice::RTC => SlotDTO::RTC,
            SlotChoice::GamesMaster(path) => SlotDTO::GamesMaster {
                path: path.display().to_string(),
            },
            SlotChoice::Orch90(path) => SlotDTO::Orch90 {
                path: path.display().to_string(),
            },
            SlotChoice::SSC => SlotDTO::SSC,
        }
    }
}

impl From<&SlotDTO> for SlotChoice {
    fn from(dto: &SlotDTO) -> Self {
        match dto {
            SlotDTO::Empty => SlotChoice::Empty,
            SlotDTO::FD502 => SlotChoice::FD502,
            SlotDTO::ROMPak { path } => SlotChoice::ROMPak(PathBuf::from(path)),
            SlotDTO::RTC => SlotChoice::RTC,
            SlotDTO::GamesMaster { path } => SlotChoice::GamesMaster(PathBuf::from(path)),
            SlotDTO::Orch90 { path } => SlotChoice::Orch90(PathBuf::from(path)),
            SlotDTO::SSC => SlotChoice::SSC,
        }
    }
}

/// Pack the form's Cartridge/Slot picks into `[peripherals]` — the inverse
/// of [`seed_peripherals`]. `slots` is only produced while the port holds an
/// MPI; otherwise every slot packs as [`SlotDTO::Empty`], which
/// `PeripheralsDTO::slots`'s `skip_serializing_if` then omits from the file.
///
/// [`PeripheralsDTO::slots`]: machine_def::PeripheralsDTO::slots
pub(crate) fn pack_peripherals(
    cartridge: &CartridgeChoice,
    mpi_slots: &[SlotChoice; crate::MPI_SLOT_COUNT],
) -> machine_def::PeripheralsDTO {
    let slots = if *cartridge == CartridgeChoice::MPI {
        std::array::from_fn(|i| (&mpi_slots[i]).into())
    } else {
        std::array::from_fn(|_| SlotDTO::Empty)
    };
    machine_def::PeripheralsDTO {
        cartridge: cartridge.into(),
        slots,
    }
}

/// Seed the form's Cartridge/Slot picks from `[peripherals]` — the inverse
/// of [`pack_peripherals`].
pub(crate) fn seed_peripherals(
    peripherals: &machine_def::PeripheralsDTO,
) -> (CartridgeChoice, [SlotChoice; crate::MPI_SLOT_COUNT]) {
    let cartridge = (&peripherals.cartridge).into();
    let slots = std::array::from_fn(|i| (&peripherals.slots[i]).into());
    (cartridge, slots)
}
