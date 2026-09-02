use std::path::{Path, PathBuf};

use coco_core::MachineVariant;

/// Read an explicit system ROM image as-is — a CoCo 3 image, or a pre-composed
/// CoCo 1/2 flat layout (extbas at offset 0, Color BASIC at offset $2000).
pub(crate) fn load_explicit_rom(path: &Path) -> Result<Box<[u8]>, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            report_rom_validation(path, &bytes);
            Ok(bytes.into_boxed_slice())
        }
        Err(e) => Err(format!("could not load {}: {e}", path.display())),
    }
}

/// The CoCo 3's single system ROM image under a roms dir — the file
/// [`load_default_rom`] reads and records as its [`ROMSource::File`].
pub(crate) const COCO3_ROM_FILE: &str = "coco3.rom";

/// Load the default boot ROM set for `variant` from `roms_dir`: the CoCo 3
/// system ROM, or a composed CoCo 1/2 image. Errors are returned, not fatal —
/// the manager reports them inline.
pub(crate) fn load_default_rom(
    variant: MachineVariant,
    roms_dir: &Path,
) -> Result<(Box<[u8]>, ROMSource), String> {
    match variant {
        MachineVariant::Coco3 => {
            let path = roms_dir.join(COCO3_ROM_FILE);
            match std::fs::read(&path) {
                Ok(bytes) => {
                    report_rom_validation(&path, &bytes);
                    Ok((bytes.into_boxed_slice(), ROMSource::File(path)))
                }
                Err(e) => Err(format!("could not load {}: {e}", path.display())),
            }
        }
        MachineVariant::Coco1 | MachineVariant::Coco2 => match compose_coco12_rom(roms_dir) {
            Coco12ROMResult::Composed { image, bas, extbas } => {
                report_rom_validation(&bas.0, &bas.1);
                match extbas {
                    Some((ext_path, ext_bytes)) => report_rom_validation(&ext_path, &ext_bytes),
                    None => tracing::info!(
                        "no Extended Color BASIC ROM found ({}); booting Color BASIC only",
                        EXTENDED_BASIC_CANDIDATES.join(", ")
                    ),
                }
                Ok((image, ROMSource::ComposedCoco12))
            }
            Coco12ROMResult::NoColorBasic => Err(format!(
                "no Color BASIC ROM found: place one of {} in {}",
                COCO_BASIC_CANDIDATES.join(", "),
                roms_dir.display()
            )),
        },
    }
}

/// Plain-SAM ROM composition (CoCo 1/2 only): the flat image that `bus.rs`'s
/// primary-SAM path expects has Extended Color BASIC at offset 0 (8K) and
/// Color BASIC at offset [`COCO12_BAS_OFFSET`] (8K). See
/// `bus.rs::SAM_BAS_ROM_OFFSET`.
pub(crate) const COCO12_BAS_OFFSET: usize = 8 * 1024;

/// Color BASIC dumps accepted for the CoCo 1/2 machine kinds (any one is
/// enough to boot), newest-preferred among the versions these machines
/// actually shipped with: 1.2 first, down to 1.0. `bas13.rom` (the CoCo 2B's
/// Color BASIC, shipped with the MC6847T1 boards) boots fine too but is the
/// far rarer dump, so it stays a last resort rather than the preferred one —
/// even though the CoCo 2 now defaults to `VdgVariant::Mc6847T1`, 1.2 runs
/// identically on a T1 machine (lowercase is unused).
pub(crate) const COCO_BASIC_CANDIDATES: &[&str] =
    &["bas12.rom", "bas11.rom", "bas10.rom", "bas13.rom"];

/// Newest-preferred Extended Color BASIC dumps; optional. A Color-BASIC-only
/// machine still boots (see `"ROM files"`).
pub(crate) const EXTENDED_BASIC_CANDIDATES: &[&str] = &["extbas11.rom", "extbas10.rom"];

/// Fill byte for the Extended Color BASIC half of the flat image when no
/// Extended BASIC dump is present — the conventional open-bus value used
/// elsewhere in the emulator.
pub(crate) const OPEN_BUS_FILLER: u8 = 0xFF;

/// Find the first of `candidates` that exists under `roms_dir`, returning its
/// path and contents.
pub(crate) fn find_rom(roms_dir: &Path, candidates: &[&str]) -> Option<(PathBuf, Vec<u8>)> {
    candidates.iter().find_map(|name| {
        let path = roms_dir.join(name);
        std::fs::read(&path).ok().map(|bytes| (path, bytes))
    })
}

/// What [`compose_coco12_rom`] found (or didn't) while composing the flat
/// image, so the caller ([`load_default_rom`]) can report it and the pure
/// composition logic stays unit-testable without touching
/// `std::process::exit`.
pub(crate) enum Coco12ROMResult {
    Composed {
        image: Box<[u8]>,
        bas: (PathBuf, Vec<u8>),
        extbas: Option<(PathBuf, Vec<u8>)>,
    },
    /// No Color BASIC dump found under `roms_dir` — nothing to boot.
    NoColorBasic,
}

/// Search `roms_dir` for the newest-present Color BASIC (required) and
/// Extended Color BASIC (optional) dumps, laid out for `bus.rs`'s plain-SAM
/// decode. Missing Extended BASIC leaves that half at [`OPEN_BUS_FILLER`].
pub(crate) fn compose_coco12_rom(roms_dir: &Path) -> Coco12ROMResult {
    let Some((bas_path, bas_bytes)) = find_rom(roms_dir, COCO_BASIC_CANDIDATES) else {
        return Coco12ROMResult::NoColorBasic;
    };

    let mut image = vec![OPEN_BUS_FILLER; COCO12_BAS_OFFSET];
    let extbas = find_rom(roms_dir, EXTENDED_BASIC_CANDIDATES);
    if let Some((_, ext_bytes)) = &extbas {
        let n = ext_bytes.len().min(COCO12_BAS_OFFSET);
        image[..n].copy_from_slice(&ext_bytes[..n]);
    }
    image.extend_from_slice(&bas_bytes);
    Coco12ROMResult::Composed {
        image: image.into_boxed_slice(),
        bas: (bas_path, bas_bytes),
        extbas,
    }
}

/// Logs one advisory line per loaded ROM against the MAME-derived manifest
/// ([`coco_core::rom_db`]). Never fatal — patched/homebrew images are legitimate.
pub(crate) fn report_rom_validation(path: &Path, bytes: &[u8]) {
    use coco_core::rom_db::{self, Validation};
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match rom_db::validate(&name, bytes) {
        Validation::Verified(known) => {
            tracing::info!(
                "{name}: verified {} [crc32 {:08x}]",
                known.desc,
                known.crc32
            );
        }
        Validation::Mismatch {
            expected,
            actual_crc32,
            actual_size,
        } => {
            tracing::warn!(
                "{name} does not match the known dump of {}: \
                 expected {} bytes crc32 {:08x}, got {} bytes crc32 {actual_crc32:08x} \
                 (patched image, or a bad dump)",
                expected.desc,
                expected.size,
                expected.crc32,
                actual_size,
            );
        }
        Validation::Unknown => {
            tracing::info!(
                "{name} is not in the known-ROM manifest ({} bytes, crc32 {:08x})",
                bytes.len(),
                rom_db::crc32(bytes),
            );
        }
    }
}

/// The per-user installed ROM directory, populated by
/// [`crate::startup::ensure_assets`]. Panics only if no home directory can be
/// determined; `ensure_assets` already exits the process before that can happen.
pub(crate) fn installed_roms_dir() -> PathBuf {
    crate::paths::roms_dir().expect("no home directory (checked at startup by ensure_assets)")
}

/// Where the FD-502 disk controller and save-state hashing read the Disk
/// BASIC ROM from.
pub(crate) fn disk_basic_rom_path() -> PathBuf {
    installed_roms_dir().join("disk11.rom")
}

/// Where [`crate::CocoApp::insert_rs232`] reads the Deluxe RS-232 pak's optional
/// EPROM dump from, if present.
pub(crate) fn rs232_eprom_default_path() -> PathBuf {
    installed_roms_dir().join("rs232.rom")
}

/// Where the Sound/Speech Cartridge reads its SP0256-AL2 allophone ROM
/// from, if present (MAME's `sp0256-al2.bin`, renamed). Without it the
/// cartridge's speech half stays silent.
pub(crate) fn sp0256_rom_path() -> PathBuf {
    installed_roms_dir().join("sp0256-al2.rom")
}

/// Where the currently-loaded system ROM image came from — tracked so
/// `CocoApp::save_state_to`/`CocoApp::load_state_from` (`save_state.rs`) can
/// record and re-resolve it without a second copy of the boot-time ROM logic
/// ([`load_default_rom`]/[`load_explicit_rom`]/[`compose_coco12_rom`]).
pub(crate) enum ROMSource {
    /// Loaded verbatim from a real file: `roms/coco3.rom`, or an explicit
    /// `[hardware].rom` in a machine definition (which, for CoCo 1/2, must
    /// already be the composed flat layout: extbas at offset 0, Color BASIC
    /// at offset $2000). Hashed and re-read by path directly.
    File(PathBuf),
    /// A CoCo 1/2 flat image composed at boot from separate Color/Extended
    /// Color BASIC dumps under [`installed_roms_dir`]
    /// ([`compose_coco12_rom`]) — no single backing file. The snapshot
    /// records a pseudo-path ([`rom_db_pseudo_path`]) instead of a real
    /// one; restore recomposes from [`installed_roms_dir`] (the only roms
    /// dir every construction site uses — no per-instance value to carry
    /// here) and hash-compares against the snapshot's recorded hash.
    ComposedCoco12,
}

/// Prefix marking a [`coco_core::snapshot::MediaRef::path`] as one of
/// [`ROMSource::ComposedCoco12`]'s pseudo-paths rather than a real
/// filesystem path — [`coco_core::snapshot::MediaRef`]'s own doc says it
/// "never resolves or interprets it, only carries it"; this module is the
/// only reader.
pub(crate) const ROM_DB_PSEUDO_PATH_PREFIX: &str = "rom-db:";

/// Build [`ROMSource::ComposedCoco12`]'s pseudo-path for `variant` (CoCo 3
/// never produces one — its system ROM is always [`ROMSource::File`]).
pub(crate) fn rom_db_pseudo_path(variant: MachineVariant) -> PathBuf {
    let label = match variant {
        MachineVariant::Coco1 => "coco1",
        MachineVariant::Coco2 => "coco2",
        MachineVariant::Coco3 => "coco3",
    };
    PathBuf::from(format!("{ROM_DB_PSEUDO_PATH_PREFIX}{label}"))
}

#[cfg(test)]
#[path = "rom_load_test.rs"]
mod tests;
