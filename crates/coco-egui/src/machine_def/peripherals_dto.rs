//! The `[peripherals]` section's DTO types — split out of `dto.rs` once it
//! grew past the project's ~500-line ceiling. See that module's doc for the
//! overall DTO convention.

use serde::{Deserialize, Serialize};

/// Default for the image-backed ROM cartridge kinds' `autostart` fields and
/// their [`SlotDTO`] equivalents: tie CART* to Q so the pak runs at power-up,
/// like the runtime insert flow's old default checkbox state.
///
/// [`GamesMaster`]: CartridgeDTO::GamesMaster
fn default_autostart() -> bool {
    true
}

/// Default for [`CartridgeDTO::MPI`]'s `switch`: front-panel slot 4 (1-based),
/// the conventional disk-controller default — see
/// [`crate::DEFAULT_MPI_SWITCH_SLOT`]'s doc.
fn default_mpi_switch() -> usize {
    crate::DEFAULT_MPI_SWITCH_SLOT + 1
}

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
    /// (`resolve_media_path`). `autostart` ties CART* to Q so the pak runs at
    /// power-up.
    #[serde(rename = "rompak")]
    ROMPak {
        path: String,
        #[serde(default = "default_autostart")]
        autostart: bool,
    },
    /// A 16 KiB-window bank-switched ROM Pak with no sound hardware.
    #[serde(rename = "banked_rompak")]
    BankedROMPak {
        path: String,
        #[serde(default = "default_autostart")]
        autostart: bool,
    },
    /// Disto 4-N-1 RTC. No boot ROM — pairs with a VHD boot; for RTC +
    /// floppies use an MPI slot. A legacy `model` key is ignored.
    #[serde(rename = "rtc")]
    RTC,
    /// Deluxe RS-232 Pak. `endpoint` picks the host backend its serial line
    /// is wired to; can also appear nested in a [`SlotDTO`] (the pak decodes
    /// its ACIA off the full address bus itself, so it's reachable from any
    /// MPI slot — [`crate::MPISlot::DeluxeRS232`]'s doc).
    #[serde(rename = "rs232")]
    RS232 {
        #[serde(default)]
        endpoint: RS232EndpointDTO,
    },
    /// Games Master Cartridge (banked ROM + SN76489A). `autostart` like
    /// [`Self::ROMPak`]'s.
    #[serde(rename = "gmc")]
    GamesMaster {
        path: String,
        #[serde(default = "default_autostart")]
        autostart: bool,
    },
    /// Orchestra-90/CC. Fixed ROM at `roms/orch90.rom` — no `path` to pick.
    /// Always autostarts — its own CART* line ties to Q, so there's no
    /// `autostart` field to override it.
    #[serde(rename = "orch90")]
    Orch90,
    /// Sound/Speech Cartridge.
    #[serde(rename = "ssc")]
    SoundSpeech,
    /// MultiPak Interface; `slots` lists its 4 occupants (`SlotDTO::Empty`
    /// for an unused one), and is required — not `#[serde(default)]` — so a
    /// file that names the MPI without a loadout fails to load with serde's
    /// own "missing field `slots`" error instead of silently loading as an
    /// empty MultiPak and dropping whatever the file meant. `switch` is the
    /// 1-based front-panel slot number the power-on SCS/CTS decode selects
    /// (matching the UI's "Slot 1"–"Slot 4"); out-of-range values are
    /// rejected by `machine_def::io::check_mpi_slots`.
    #[serde(rename = "mpi")]
    MPI {
        slots: [SlotDTO; crate::MPI_SLOT_COUNT],
        #[serde(default = "default_mpi_switch")]
        switch: usize,
    },
}

/// One MultiPak slot's occupant ([`CartridgeDTO::MPI`]'s `slots`) —
/// [`CartridgeDTO`]'s sibling minus nested MPI (real MPIs can't nest). Maps
/// to [`crate::new_vm::SlotChoice`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind")]
pub enum SlotDTO {
    #[default]
    #[serde(rename = "empty")]
    Empty,
    #[serde(rename = "fd502")]
    FD502,
    #[serde(rename = "rompak")]
    ROMPak {
        path: String,
        #[serde(default = "default_autostart")]
        autostart: bool,
    },
    #[serde(rename = "banked_rompak")]
    BankedROMPak {
        path: String,
        #[serde(default = "default_autostart")]
        autostart: bool,
    },
    #[serde(rename = "rtc")]
    RTC,
    /// Deluxe RS-232 Pak in this slot — at most one across the whole
    /// machine, since two would fight over the ACIA at `$FF68`; a second one
    /// is rejected at load time.
    #[serde(rename = "rs232")]
    RS232 {
        #[serde(default)]
        endpoint: RS232EndpointDTO,
    },
    #[serde(rename = "gmc")]
    GamesMaster {
        path: String,
        #[serde(default = "default_autostart")]
        autostart: bool,
    },
    /// Orchestra-90/CC. Fixed ROM at `roms/orch90.rom` — no `path` to pick.
    #[serde(rename = "orch90")]
    Orch90,
    #[serde(rename = "ssc")]
    SoundSpeech,
}

/// [`CartridgeDTO::RS232`]'s `endpoint` — which host backend the Deluxe
/// RS-232 Pak's serial line is wired to. Maps to
/// [`crate::new_vm::RS232EndpointChoice`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind")]
pub enum RS232EndpointDTO {
    /// TX loops straight back to RX — the pak's inert power-on default.
    #[default]
    #[serde(rename = "loopback")]
    Loopback,
    /// TCP listener at `listen`; a host terminal connects with `nc`/`telnet`.
    #[serde(rename = "tcp")]
    TCP {
        #[serde(default = "default_rs232_tcp_listen")]
        listen: String,
    },
    /// Unix pseudo-terminal. Rejected at launch on non-Unix targets, where
    /// there is no PTY to open.
    #[serde(rename = "pty")]
    PTY,
}

/// Default for [`RS232EndpointDTO::TCP`]'s `listen`.
fn default_rs232_tcp_listen() -> String {
    crate::RS232_TCP_DEFAULT_ADDR.to_string()
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
