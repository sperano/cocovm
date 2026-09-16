//! Known-ROM manifests for system firmware and CoCo cartridge images. System
//! ROM CRC32s come from MAME's ROM definitions (`src/mame/trs/coco3.cpp`,
//! `src/mame/trs/coco12.cpp`, `src/devices/bus/coco/coco_fdc.cpp`, master as
//! of 2026-07). [`KNOWN_CARTRIDGE_ROMS`] documents its separate provenance.
//!
//! Validation is advisory: an unrecognized or mismatching image still boots
//! (patched and homebrew ROMs are legitimate), but the loader can tell the
//! user exactly which known dump they have — or that they don't have one.

mod cartridges;

pub use cartridges::KNOWN_CARTRIDGE_ROMS;

use crate::config::MachineVariant;

/// One known-good dump from MAME's manifest.
#[derive(Debug, PartialEq, Eq)]
pub struct KnownROM {
    /// Canonical file name in the MAME romset.
    pub file: &'static str,
    /// Size in bytes; CRC32 matches are only trusted at the right size.
    pub size: usize,
    /// CRC32 (IEEE, as printed by MAME's `CRC(...)`).
    pub crc32: u32,
    pub desc: &'static str,
}

/// Cartridge implementation used to run a known ROM image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CartridgeHardware {
    /// A fixed ROM Pak image of up to 32 KiB.
    RomPak,
    /// A legacy `$FF40`-selected 16 KiB banked ROM Pak without GMC audio.
    BankedRomPak,
    /// A Games Master Cartridge with banked ROM and SN76489A sound.
    GamesMaster,
}

/// Which machine family a known cartridge image needs, per MAME's
/// `coco_cart.xml` compatibility tags cross-checked against XRoar's `rom.c`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CartridgeMachine {
    /// Runs on every CoCo, or detects the machine itself (Mind Roll, Tetris).
    Any,
    /// CoCo 1/2 build of a title that has a separate CoCo 3 dump.
    Coco12,
    /// Needs the CoCo 3 (GIME video, 128K).
    Coco3,
}

impl CartridgeMachine {
    /// Whether an image with this requirement runs on `variant`.
    pub fn supports(self, variant: MachineVariant) -> bool {
        match self {
            CartridgeMachine::Any => true,
            CartridgeMachine::Coco12 => variant != MachineVariant::Coco3,
            CartridgeMachine::Coco3 => variant == MachineVariant::Coco3,
        }
    }

    /// Short UI label ("CoCo 3", "CoCo 1/2"); none when any machine will do.
    pub fn label(self) -> Option<&'static str> {
        match self {
            CartridgeMachine::Any => None,
            CartridgeMachine::Coco12 => Some("CoCo 1/2"),
            CartridgeMachine::Coco3 => Some("CoCo 3"),
        }
    }
}

/// One known CoCo cartridge ROM image.
#[derive(Debug, PartialEq, Eq)]
pub struct KnownCartridgeROM {
    /// Size in bytes; CRC32 matches are only trusted at the right size.
    pub size: usize,
    /// CRC32 (IEEE, as printed by XRoar).
    pub crc32: u32,
    /// Title as printed on the cartridge label.
    pub name: &'static str,
    /// Release year, when known.
    pub year: Option<u16>,
    /// Publisher, when known.
    pub vendor: Option<&'static str>,
    /// Radio Shack catalog number (`26-xxxx`), for cartridges Tandy sold.
    pub catalog: Option<&'static str>,
    /// Dump-variant tag (XRoar's bracket suffix): "alt", "coco12", fix tags.
    pub variant: Option<&'static str>,
    /// Cartridge implementation that runs this image.
    pub hardware: CartridgeHardware,
    /// Machine family the image needs.
    pub machine: CartridgeMachine,
    /// File name under the asset bundle's `cartridges/` directory, when the bundle ships this image.
    pub bundled_file: Option<&'static str>,
}

impl KnownCartridgeROM {
    /// The XRoar-derived one-line description, e.g. "Atom (1983) (Tandy) (26-3149)".
    pub fn title(&self) -> String {
        use std::fmt::Write as _;
        let mut title = self.name.to_string();
        if let Some(year) = self.year {
            let _ = write!(title, " ({year})");
        }
        for field in [self.vendor, self.catalog].into_iter().flatten() {
            let _ = write!(title, " ({field})");
        }
        if let Some(variant) = self.variant {
            let _ = write!(title, " [{variant}]");
        }
        title
    }
}

/// Every system ROM the emulator knows how to use, per MAME.
pub const KNOWN_ROMS: &[KnownROM] = &[
    KnownROM {
        file: "coco3.rom",
        size: 0x8000,
        crc32: 0xb4c88d6c,
        desc: "Super Extended Color BASIC 2.0 (CoCo 3 NTSC)",
    },
    KnownROM {
        file: "coco3p.rom",
        size: 0x8000,
        crc32: 0xff050d80,
        desc: "Super Extended Color BASIC 2.0 (CoCo 3 PAL)",
    },
    KnownROM {
        file: "bas10.rom",
        size: 0x2000,
        crc32: 0x00b50aaa,
        desc: "Color BASIC 1.0 (CoCo 1/2)",
    },
    KnownROM {
        file: "bas11.rom",
        size: 0x2000,
        crc32: 0x6270955a,
        desc: "Color BASIC 1.1 (CoCo 1/2)",
    },
    KnownROM {
        file: "bas12.rom",
        size: 0x2000,
        crc32: 0x54368805,
        desc: "Color BASIC 1.2 (CoCo 1/2)",
    },
    KnownROM {
        file: "bas13.rom",
        size: 0x2000,
        crc32: 0xd8f4d15e,
        desc: "Color BASIC 1.3 (CoCo 2B)",
    },
    KnownROM {
        file: "extbas10.rom",
        size: 0x2000,
        crc32: 0x6111a086,
        desc: "Extended Color BASIC 1.0 (CoCo 1/2)",
    },
    KnownROM {
        file: "extbas11.rom",
        size: 0x2000,
        crc32: 0xa82a6254,
        desc: "Extended Color BASIC 1.1 (CoCo 1/2)",
    },
    KnownROM {
        file: "sp0256-al2.rom",
        size: 0x800,
        crc32: 0xb504ac15,
        desc: "SP0256-AL2 allophone ROM (Sound/Speech Cartridge)",
    },
    KnownROM {
        file: "ssc-tms7040.rom",
        size: 0x1000,
        crc32: 0xa8e2eb98,
        desc: "Sound/Speech Cartridge TMS7040 firmware (PIC-7040-510)",
    },
    KnownROM {
        file: "orch90.rom",
        size: 0x2000,
        crc32: 0x15fb39af,
        desc: "Orchestra-90/CC (26-3143)",
    },
    KnownROM {
        file: "disk10.rom",
        size: 0x2000,
        crc32: 0xb4f9968e,
        desc: "Disk Extended Color BASIC 1.0 (FD-502)",
    },
    KnownROM {
        file: "disk11.rom",
        size: 0x2000,
        crc32: 0x0b9c5415,
        desc: "Disk Extended Color BASIC 1.1 (FD-502)",
    },
];

/// What [`validate`] concluded about a ROM image.
#[derive(Debug, PartialEq, Eq)]
pub enum Validation {
    /// Byte-identical to a known dump (matched by size + CRC32).
    Verified(&'static KnownROM),
    /// The file name claims a known ROM, but the contents differ.
    Mismatch {
        expected: &'static KnownROM,
        actual_crc32: u32,
        actual_size: usize,
    },
    /// Not in the manifest — a homebrew, patched, or renamed image.
    Unknown,
}

/// CRC32 (IEEE 802.3, reflected) — the checksum MAME prints as `CRC(...)`.
pub fn crc32(bytes: &[u8]) -> u32 {
    const REFLECTED_POLY: u32 = 0xedb8_8320;
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..u8::BITS {
            let carry = crc & 1;
            crc >>= 1;
            if carry != 0 {
                crc ^= REFLECTED_POLY;
            }
        }
    }
    !crc
}

/// Look up an image by contents alone (size + CRC32), regardless of name.
pub fn identify(bytes: &[u8]) -> Option<&'static KnownROM> {
    let crc = crc32(bytes);
    KNOWN_ROMS
        .iter()
        .find(|r| r.size == bytes.len() && r.crc32 == crc)
}

/// Look up a cartridge image by contents alone (size + CRC32).
pub fn identify_cartridge(bytes: &[u8]) -> Option<&'static KnownCartridgeROM> {
    identify_cartridge_fingerprint(bytes.len(), crc32(bytes))
}

fn identify_cartridge_fingerprint(size: usize, crc32: u32) -> Option<&'static KnownCartridgeROM> {
    KNOWN_CARTRIDGE_ROMS
        .iter()
        .find(|rom| rom.size == size && rom.crc32 == crc32)
}

/// Validate an image against the manifest. `file_name` is the bare name
/// (no directory); matching is by contents first, then by claimed name.
pub fn validate(file_name: &str, bytes: &[u8]) -> Validation {
    if let Some(known) = identify(bytes) {
        return Validation::Verified(known);
    }
    match KNOWN_ROMS.iter().find(|r| r.file == file_name) {
        Some(expected) => Validation::Mismatch {
            expected,
            actual_crc32: crc32(bytes),
            actual_size: bytes.len(),
        },
        None => Validation::Unknown,
    }
}

#[cfg(test)]
#[path = "rom_db_test.rs"]
mod tests;
