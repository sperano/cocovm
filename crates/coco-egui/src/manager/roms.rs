//! The detail pane's read-only "ROMs" group: every ROM image the selected
//! definition loads at its next cold start, resolved the way `launch.rs`
//! does, with a presence/validation status so a missing or doubtful dump
//! shows before Start instead of as a boot failure.

use std::path::{Path, PathBuf};

use coco_core::rom_db::{self, Validation};
use eframe::egui;

use crate::machine_def::{self, CartridgeDTO, MachineDef, SlotDTO};
use crate::{new_vm, rom_load};

/// One ROM image the definition will load.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ROMRow {
    /// What the image is for ("System ROM", "Slot 2: Disk BASIC").
    pub(super) role: String,
    /// Where it is read from.
    pub(super) path: PathBuf,
    pub(super) status: ROMStatus,
}

/// What was found at a [`ROMRow`]'s path.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ROMStatus {
    /// Byte-identical to a known dump: its manifest description.
    Verified(&'static str),
    /// Present but not in the manifest (homebrew, patched, renamed): its size.
    Unrecognized(usize),
    /// Named like a known dump but with different contents.
    Mismatch(&'static str),
    Missing,
    /// An optional dump that isn't installed; the hardware works without it.
    OptionalAbsent,
}

/// Whether a role's image is required to boot.
#[derive(Clone, Copy)]
enum Need {
    Required,
    Optional,
}

/// The ROM-bearing cartridge kinds, shared by the bare port and MPI slots.
enum Occupant<'a> {
    None,
    FD502,
    /// A user-supplied image: its role name and the definition's path.
    Image {
        kind: &'static str,
        path: &'a str,
    },
    RS232,
    Orch90,
    SoundSpeech,
}

/// The rows for `def`, in load order: the system ROM, then the cartridge
/// port's occupant or each MPI slot's. `roms_dir` is the installed ROM
/// directory; `None` (no home directory) leaves every stock image missing.
pub(super) fn rom_rows(def: &MachineDef, slug: &str, roms_dir: Option<&Path>) -> Vec<ROMRow> {
    let mut rows = Vec::new();
    system_rows(def, roms_dir, &mut rows);
    match &def.peripherals.cartridge {
        CartridgeDTO::MPI { slots, .. } => {
            for (i, slot) in slots.iter().enumerate() {
                let prefix = format!("Slot {}: ", i + 1);
                occupant_rows(slot_occupant(slot), &prefix, slug, roms_dir, &mut rows);
            }
        }
        cartridge => occupant_rows(cartridge_occupant(cartridge), "", slug, roms_dir, &mut rows),
    }
    rows
}

/// The system ROM: an explicit `[hardware].rom`, else the CoCo 3 image, else
/// the CoCo 1/2 Color BASIC dump plus its optional Extended BASIC one.
fn system_rows(def: &MachineDef, roms_dir: Option<&Path>, rows: &mut Vec<ROMRow>) {
    if let Some(explicit) = &def.hardware.rom {
        rows.push(row("System ROM", PathBuf::from(explicit), Need::Required));
        return;
    }
    match def.hardware.variant.into() {
        coco_core::MachineVariant::Coco3 => rows.push(stock(
            "System ROM",
            &[rom_load::COCO3_ROM_FILE],
            roms_dir,
            Need::Required,
        )),
        coco_core::MachineVariant::Coco1 | coco_core::MachineVariant::Coco2 => {
            rows.push(stock(
                "Color BASIC",
                rom_load::COCO_BASIC_CANDIDATES,
                roms_dir,
                Need::Required,
            ));
            rows.push(stock(
                "Extended Color BASIC",
                rom_load::EXTENDED_BASIC_CANDIDATES,
                roms_dir,
                Need::Optional,
            ));
        }
    }
}

fn cartridge_occupant(cartridge: &CartridgeDTO) -> Occupant<'_> {
    match cartridge {
        CartridgeDTO::None
        | CartridgeDTO::RTC
        | CartridgeDTO::CoCoMax
        | CartridgeDTO::MPI { .. } => Occupant::None,
        CartridgeDTO::FD502 => Occupant::FD502,
        CartridgeDTO::ROMPak { path, .. } => Occupant::Image {
            kind: "ROM Pak",
            path,
        },
        CartridgeDTO::BankedROMPak { path, .. } => Occupant::Image {
            kind: "Banked ROM Pak",
            path,
        },
        CartridgeDTO::RS232 { .. } => Occupant::RS232,
        CartridgeDTO::GamesMaster { path, .. } => Occupant::Image {
            kind: "Games Master ROM",
            path,
        },
        CartridgeDTO::Orch90 => Occupant::Orch90,
        CartridgeDTO::SoundSpeech => Occupant::SoundSpeech,
    }
}

fn slot_occupant(slot: &SlotDTO) -> Occupant<'_> {
    match slot {
        SlotDTO::Empty | SlotDTO::RTC | SlotDTO::CoCoMax => Occupant::None,
        SlotDTO::FD502 => Occupant::FD502,
        SlotDTO::ROMPak { path, .. } => Occupant::Image {
            kind: "ROM Pak",
            path,
        },
        SlotDTO::BankedROMPak { path, .. } => Occupant::Image {
            kind: "Banked ROM Pak",
            path,
        },
        SlotDTO::RS232 { .. } => Occupant::RS232,
        SlotDTO::GamesMaster { path, .. } => Occupant::Image {
            kind: "Games Master ROM",
            path,
        },
        SlotDTO::Orch90 => Occupant::Orch90,
        SlotDTO::SoundSpeech => Occupant::SoundSpeech,
    }
}

/// The rows one cartridge-port occupant needs, roles prefixed by `prefix`
/// ("Slot 2: " inside an MPI, empty on the bare port).
fn occupant_rows(
    occupant: Occupant<'_>,
    prefix: &str,
    slug: &str,
    roms_dir: Option<&Path>,
    rows: &mut Vec<ROMRow>,
) {
    let role = |name: &str| format!("{prefix}{name}");
    match occupant {
        Occupant::None => {}
        Occupant::FD502 => rows.push(stock(
            &role("Disk BASIC"),
            &[rom_load::DISK_BASIC_ROM],
            roms_dir,
            Need::Required,
        )),
        Occupant::Image { kind, path } => rows.push(row(
            &role(kind),
            machine_def::resolve_media_path(path, slug),
            Need::Required,
        )),
        Occupant::RS232 => rows.push(stock(
            &role("RS-232 Pak EPROM"),
            &[rom_load::RS232_EPROM_ROM],
            roms_dir,
            Need::Optional,
        )),
        Occupant::SoundSpeech => {
            rows.push(stock(
                &role("Speech/Sound allophones"),
                &[rom_load::SP0256_ROM],
                roms_dir,
                Need::Required,
            ));
            rows.push(stock(
                &role("Speech/Sound firmware"),
                &[rom_load::SSC_FIRMWARE_ROM],
                roms_dir,
                Need::Required,
            ));
        }
        Occupant::Orch90 => rows.push(stock(
            &role("Orchestra-90 ROM"),
            &[rom_load::ORCH90_ROM],
            roms_dir,
            Need::Required,
        )),
    }
}

/// A stock image under `roms_dir`: the first of `candidates` readable
/// (`rom_load::find_rom`, the probe launch uses), else the preferred
/// (first) name reported absent.
fn stock(role: &str, candidates: &[&str], roms_dir: Option<&Path>, need: Need) -> ROMRow {
    let preferred = candidates[0];
    let found = roms_dir.and_then(|dir| rom_load::find_rom(dir, candidates));
    let (path, status) = match found {
        Some((path, bytes)) => {
            let status = validate(&path, &bytes);
            (path, status)
        }
        None => (
            roms_dir.map_or_else(|| PathBuf::from(preferred), |dir| dir.join(preferred)),
            absent(need),
        ),
    };
    ROMRow {
        role: role.to_string(),
        path,
        status,
    }
}

/// A row for `path`, read and checked against the known-ROM manifest.
fn row(role: &str, path: PathBuf, need: Need) -> ROMRow {
    let status = match std::fs::read(&path) {
        Ok(bytes) => validate(&path, &bytes),
        Err(_) => absent(need),
    };
    ROMRow {
        role: role.to_string(),
        path,
        status,
    }
}

fn absent(need: Need) -> ROMStatus {
    match need {
        Need::Required => ROMStatus::Missing,
        Need::Optional => ROMStatus::OptionalAbsent,
    }
}

fn validate(path: &Path, bytes: &[u8]) -> ROMStatus {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    match rom_db::validate(name, bytes) {
        Validation::Verified(known) => ROMStatus::Verified(known.desc),
        Validation::Mismatch { expected, .. } => ROMStatus::Mismatch(expected.desc),
        Validation::Unknown => ROMStatus::Unrecognized(bytes.len()),
    }
}

/// The "ROMs" group: one row per image — role, file name (full path as
/// hover text), and status.
pub(super) fn draw_roms(ui: &mut egui::Ui, slug: &str, rows: &[ROMRow]) {
    crate::titled_group(ui, "ROMs", |ui| {
        egui::Grid::new(("detail_form_roms", slug))
            .num_columns(3)
            .spacing(new_vm::FORM_GRID_SPACING)
            .min_col_width(new_vm::FORM_LABEL_MIN_WIDTH)
            .show(ui, |ui| {
                for entry in rows {
                    ui.label(&entry.role);
                    ui.label(file_name(&entry.path))
                        .on_hover_text(entry.path.display().to_string());
                    let (text, color) = status_label(ui, &entry.status);
                    ui.colored_label(color, text);
                    ui.end_row();
                }
            });
    });
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The status column's text and color: errors in the error color, doubts
/// in the warning color, the rest plain or dimmed.
fn status_label(ui: &egui::Ui, status: &ROMStatus) -> (String, egui::Color32) {
    let visuals = ui.visuals();
    match status {
        ROMStatus::Verified(desc) => ((*desc).to_string(), visuals.text_color()),
        ROMStatus::Unrecognized(size) => (
            format!("unrecognized dump, {size} bytes"),
            visuals.weak_text_color(),
        ),
        ROMStatus::Mismatch(desc) => (format!("differs from {desc}"), visuals.warn_fg_color),
        ROMStatus::Missing => ("missing".to_string(), visuals.error_fg_color),
        ROMStatus::OptionalAbsent => (
            "not installed (optional)".to_string(),
            visuals.weak_text_color(),
        ),
    }
}

#[cfg(test)]
#[path = "roms_test.rs"]
mod tests;
