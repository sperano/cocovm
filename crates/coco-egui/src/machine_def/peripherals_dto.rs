//! The `[peripherals]` section's DTO types — split out of `dto.rs` once it
//! grew past the project's ~500-line ceiling. See that module's doc for the
//! overall DTO convention.

use serde::{Deserialize, Serialize};

/// `[peripherals].cartridge` — what's plugged into the cartridge port. Maps
/// to [`crate::new_vm::CartridgeChoice`]; the DTO→Choice `From` impl lives
/// there, like [`super::SerialDTO`]'s pair in `new_vm.rs` (its doc explains
/// why) — the reverse direction is folded into `new_vm::pack_peripherals`
/// instead, since building [`CartridgeDTO::MPI`] needs the form's slot picks
/// too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind")]
pub enum CartridgeDTO {
    #[default]
    #[serde(rename = "none")]
    None,
    /// FD-502 disk controller (Disk BASIC ROM + WD1773, empty drives).
    #[serde(rename = "fd502")]
    FD502,
    /// A program ROM Pak image, `path` resolved like `[media]`'s paths
    /// (`resolve_media_path`).
    #[serde(rename = "rompak")]
    ROMPak { path: String },
    /// Disto RTC. No boot ROM — pairs with a VHD boot; for RTC + floppies
    /// use an MPI slot.
    #[serde(rename = "rtc")]
    RTC,
    /// Deluxe RS-232 Pak. Bare-port only: there's no `mpi_insert_rs232`, so
    /// this kind never appears in a [`SlotDTO`].
    #[serde(rename = "rs232")]
    RS232,
    /// Games Master Cartridge (banked ROM + SN76489A).
    #[serde(rename = "gmc")]
    GamesMaster { path: String },
    /// Orchestra-90/CC.
    #[serde(rename = "orch90")]
    Orch90 { path: String },
    /// Sound/Speech Cartridge.
    #[serde(rename = "ssc")]
    SoundSpeech,
    /// MultiPak Interface; `slots` lists its 4 occupants (`SlotDTO::Empty`
    /// for an unused one), and is required — not `#[serde(default)]` — so a
    /// file that names the MPI without a loadout fails to load with serde's
    /// own "missing field `slots`" error instead of silently loading as an
    /// empty MultiPak and dropping whatever the file meant.
    #[serde(rename = "mpi")]
    MPI {
        slots: [SlotDTO; crate::MPI_SLOT_COUNT],
    },
}

/// One MultiPak slot's occupant ([`CartridgeDTO::MPI`]'s `slots`) —
/// [`CartridgeDTO`]'s sibling minus the bare-port-only kinds (nested MPI,
/// RS-232). Maps to [`crate::new_vm::SlotChoice`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind")]
pub enum SlotDTO {
    #[default]
    #[serde(rename = "empty")]
    Empty,
    #[serde(rename = "fd502")]
    FD502,
    #[serde(rename = "rompak")]
    ROMPak { path: String },
    #[serde(rename = "rtc")]
    RTC,
    #[serde(rename = "gmc")]
    GamesMaster { path: String },
    #[serde(rename = "orch90")]
    Orch90 { path: String },
    #[serde(rename = "ssc")]
    SoundSpeech,
}

/// `[peripherals]` section — section itself optional (a definition with no
/// `[peripherals]` table at all has an empty port), but `cartridge` is
/// required *within* the section once it's present: no `#[serde(default)]`,
/// so a schema-1 file's leftover `[peripherals]` (`mpi`/`rtc`/`fd502`/`rs232`
/// booleans, no `cartridge` key) fails with a "missing field" error instead
/// of silently loading as an empty port and dropping whatever those flags
/// meant (`io::load_one` has no schema-1 migration by design).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PeripheralsDTO {
    pub cartridge: CartridgeDTO,
}

#[cfg(test)]
#[path = "peripherals_dto_test.rs"]
mod tests;
