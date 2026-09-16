// Cartridge metadata adapted from XRoar `src/rom.c`.
// Copyright 2026 Ciaran Anscomb.
//
// XRoar is free software; you can redistribute it and/or modify it under the
// terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version. See XRoar's COPYING.GPL for redistribution conditions.
//
// Adapted for CocoVM on 2026-09-11. This table omits Dragon-only cartridges
// and non-cartridge system, DOS, and IDE ROMs, and adds CyD GMC metadata.
// XRoar's `gmc` type also drives legacy `$FF40`-banked carts; those entries
// use `BankedRomPak` here so they don't claim the GMC sound port.
//
// The machine column follows XRoar's `.machine` where MAME's `coco_cart.xml`
// agrees. Departures: TypeMate is CoCo 3 per MAME (XRoar: any); Dungeons of
// Daggorath is any (XRoar: CoCo 3, but it predates the machine); the
// `coco12` dump variants are CoCo 1/2 (XRoar marks Silpheed's as CoCo 3).
//
// This table holds games and applications only. Hardware paks (RS-232,
// Orchestra-90, Disk BASIC) ship as `roms/*.rom` and are omitted here, so
// XRoar's Deluxe RS-232 Program Pak row is dropped.

use super::{CartridgeHardware, KnownCartridgeROM, MachineFamily};

// Arguments: crc32, size, hardware, machine, name, year, vendor, catalog
// (Radio Shack 26-xxxx number), dump-variant tag, file name in the asset
// bundle's `cartridges/` directory.
macro_rules! rom {
    ($crc32:expr, $size:expr, $hardware:ident, $machine:ident, $name:expr, $year:expr,
     $vendor:expr, $catalog:expr, $variant:expr, $bundled:expr) => {
        KnownCartridgeROM {
            size: $size,
            crc32: $crc32,
            name: $name,
            year: $year,
            vendor: $vendor,
            catalog: $catalog,
            variant: $variant,
            hardware: CartridgeHardware::$hardware,
            machine: MachineFamily::$machine,
            bundled_file: $bundled,
        }
    };
}

/// Known CoCo cartridge ROMs adapted from XRoar, plus CocoVM additions.
#[rustfmt::skip]
pub const KNOWN_CARTRIDGE_ROMS: &[KnownCartridgeROM] = &[
    rom!(0x7d1cac0e, 0x2000, RomPak, Any, "Androne", Some(1983), Some("Tandy"), Some("26-3096"), None, Some("Androne (1983) (26-3096) (Tandy).ccc")),
    rom!(0x2fdf5b58, 0x1000, RomPak, Any, "Art Gallery", Some(1981), Some("Robert G. Kilgus"), Some("26-3061"), None, Some("Art Gallery (1981) (26-3061) (Tandy).ccc")),
    rom!(0xf76f6fbe, 0x4000, RomPak, Any, "Atom", Some(1983), Some("Tandy"), Some("26-3149"), None, Some("Atom (1983) (26-3149) (Tandy).ccc")),
    rom!(0x16d2d946, 0x0800, RomPak, Any, "Audio Spectrum Analyzer", Some(1981), Some("Tandy"), Some("26-3156"), None, Some("Audio Spectrum Analyzer (1981) (26-3156) (Tandy) (Coco 1-2).ccc")),
    rom!(0x0d964862, 0x0800, RomPak, Any, "Audio Spectrum Analyzer v2", Some(1983), Some("Tandy"), Some("26-3156"), None, None),
    rom!(0xa3b8ba85, 0x1000, RomPak, Any, "Backgammon", Some(1980), Some("Tandy"), Some("26-3059"), None, Some("Backgammon (1980) (26-3059) (Tandy).ccc")),
    rom!(0x3be0cf60, 0x1000, RomPak, Any, "Bingo Math", Some(1980), Some("Tandy"), Some("26-3150"), None, Some("Bingo Math (1980) (26-3150) (Tandy).ccc")),
    rom!(0xd6b940f1, 0x2000, RomPak, Any, "Bridge Tutor I", Some(1982), Some("Philidor Software"), Some("26-3158"), None, Some("Bridge Tutor I (1982) (26-3158) (Tandy).ccc")),
    rom!(0xc7ed9d30, 0x1000, RomPak, Any, "Bustout", Some(1981), Some("Tandy"), Some("26-3056"), None, Some("Bustout (1981) (26-3056) (Tandy).ccc")),
    rom!(0x41dd5a1a, 0x2000, RomPak, Any, "Canyon Climber", Some(1982), Some("Tandy"), Some("26-3089"), None, Some("Canyon Climber (1982) (26-3089) (Tandy).ccc")),
    rom!(0x8869eddc, 0x1000, RomPak, Any, "Castle Guard", Some(1981), Some("The Image Producers"), Some("26-3079"), None, Some("Castle Guard (1981) (26-3079) (Tandy).ccc")),
    rom!(0x05dc5ef3, 0x1000, RomPak, Any, "Checker King", Some(1980), Some("Personal Software"), Some("26-3055"), None, Some("Checker King (1980) (26-3055) (Tandy).ccc")),
    rom!(0xfe4c93e4, 0x2000, RomPak, Any, "Clowns & Balloons", Some(1982), Some("Tandy"), Some("26-3087"), None, Some("Clowns & Balloons (1982) (26-3087) (Tandy).ccc")),
    rom!(0xad1937c1, 0x2000, RomPak, Any, "Color Baseball", Some(1980), Some("Dale A. Lear"), Some("26-3095"), None, Some("Color Baseball (1980) (26-3095) (Tandy).ccc")),
    rom!(0xf4be90bc, 0x1000, RomPak, Any, "Color Cubes", Some(1981), Some("Robert G. Kilgus"), Some("26-3075"), None, Some("Color Cubes (1981) (26-3075) (Tandy).ccc")),
    rom!(0xd78be10a, 0x1000, RomPak, Any, "Color File", Some(1981), Some("Tandy"), Some("26-3103"), None, Some("Color File (1981) (26-3103) (Tandy).ccc")),
    rom!(0x8acd7ea2, 0x2800, RomPak, Any, "Color Forth", Some(1981), Some("Microworks"), None, None, Some("Color Forth (1981) (Microworks).ccc")),
    rom!(0x9fb1e7d9, 0x2000, RomPak, Any, "Color Logo", Some(1983), Some("Larry Kheriaty & George Gerhold"), Some("26-2722"), None, Some("Color Logo (1983) (26-2722) (Tandy).ccc")),
    rom!(0x3aca199b, 0x2000, RomPak, Any, "Color Scripsit", Some(1981), Some("Tandy"), Some("26-3105"), None, Some("Color Scripsit (1981) (26-3105) (Tandy).ccc")),
    rom!(0x7bffd03a, 0x4000, RomPak, Any, "Color Scripsit II", Some(1986), Some("Tandy"), Some("26-3109"), None, Some("Color Scripsit II (1986) (26-3109) (Tandy) (6847T1).ccc")),
    rom!(0x06075f2a, 0x1000, RomPak, Any, "Crosswords", Some(1981), None, Some("26-3082"), None, Some("Crosswords (1981) (26-3082) (Tandy).ccc")),
    rom!(0xbfa3585d, 0x4000, RomPak, Any, "Cyrus World Class Chess", Some(1983), Some("Tandy"), Some("26-3064"), None, Some("Cyrus World Class Chess (1983) (26-3064) (Tandy).ccc")),
    rom!(0x1199d27f, 0x2000, RomPak, Any, "Demolition Derby", Some(1984), Some("Tandy"), Some("26-3044"), None, Some("Demolition Derby (1984) (26-3044) (Tandy).ccc")),
    rom!(0xb7a1aa3e, 0x3f00, RomPak, Any, "Demon Attack", Some(1984), Some("Tandy"), Some("26-3099"), None, Some("Demon Attack (1984) (26-3099) (Tandy).ccc")),
    rom!(0xd5257b50, 0x0800, RomPak, Any, "Diagnostics", Some(1980), Some("Tandy"), Some("26-3019"), None, Some("Diagnostics (1980) (26-3019) (Tandy).ccc")),
    rom!(0x8cd56308, 0x0800, RomPak, Any, "Diagnostics v2.0", Some(1982), Some("Tandy"), Some("26-3019"), None, Some("Diagnostics v2.0 (1982) (26-3019) (Tandy).ccc")),
    rom!(0xba1d9e81, 0x2000, RomPak, Any, "Dino Wars", Some(1981), Some("Tandy"), Some("26-3057"), None, Some("Dino Wars (1981) (26-3057) (Tandy).ccc")),
    rom!(0x667bc55d, 0x2000, RomPak, Any, "Direct Connect Modem Pak", Some(1985), Some("Tandy"), Some("26-2228"), None, Some("Direct Connect Modem Pak (1985) (26-2228) (Tandy).ccc")),
    rom!(0x819fc9fb, 0x2000, RomPak, Any, "Don Pan", Some(1985), Some("Tandy"), Some("26-3097"), None, Some("DON-PAN (1984) (26-3097) (Tandy).ccc")),
    rom!(0xb86830a9, 0x3f00, RomPak, Any, "Doodle Bug 1", Some(1982), Some("Computerware"), None, None, Some("Doodle Bug (1982) (Computerware).ccc")),
    rom!(0xc3d41232, 0x2000, RomPak, Any, "Doodle Bug 2", Some(1982), Some("Computerware"), None, None, Some("Doodle Bug (1982) (Computerware) (Buff).ccc")),
    rom!(0xea820c39, 0x2000, RomPak, Any, "Doodle Bug 3", Some(1982), Some("Computerware"), None, None, Some("Doodle Bug (1982) (Computerware) (Green).ccc")),
    rom!(0x4cc44337, 0x1000, RomPak, Any, "Doubleback", Some(1982), Some("Tandy"), Some("26-3091"), None, Some("Doubleback (1982) (26-3091) (Tandy).ccc")),
    rom!(0xa923f5a2, 0x2000, RomPak, Any, "Downland v1.0", Some(1983), Some("Tandy"), Some("26-3046"), None, Some("Downland v1.0 (1983) (26-3046) (Tandy).ccc")),
    rom!(0x0229c319, 0x2000, RomPak, Any, "Downland v1.1", Some(1983), Some("Tandy"), Some("26-3046"), None, Some("Downland v1.1 (1983) (26-3046) (Tandy).ccc")),
    rom!(0x6f1e913a, 0x4000, RomPak, Any, "Dragonfire", Some(1984), Some("Tandy"), Some("26-3098"), None, Some("Dragon Fire (1984) (26-3098) (Tandy).ccc")),
    rom!(0xf4374a55, 0x4000, RomPak, Any, "EDTASM+", Some(1982), Some("Tandy"), Some("26-3250"), None, Some("EDTASM+ (1982) (26-3250) (Tandy).ccc")),
    rom!(0x7a1c2f2d, 0x2000, RomPak, Any, "Facemaker", Some(1984), Some("Tandy"), Some("26-3166"), None, Some("Facemaker (1984) (26-3166) (Spinnaker).ccc")),
    rom!(0xf17b5b38, 0x2000, RomPak, Any, "Football", Some(1980), Some("Tandy"), Some("26-3053"), None, Some("Football (1980) (26-3053) (Tandy).ccc")),
    rom!(0x93153b15, 0x2000, RomPak, Any, "Fraction Fever", Some(1984), Some("Spinnaker"), Some("26-3169"), None, Some("Fraction Fever (1984) (26-3169) (Spinnaker).ccc")),
    rom!(0x984ee0d9, 0x1000, RomPak, Any, "Galactic Attack", Some(1982), Some("Tandy"), Some("26-3066"), None, Some("Galactic Attack (1982) (26-3066) (Tandy).ccc")),
    rom!(0xfd84fc5c, 0x2000, RomPak, Any, "Gomoku-Renju", Some(1983), Some("Tandy"), Some("26-3069"), None, Some("Gomoku-Renju (1983) (26-3069) (Tandy).ccc")),
    rom!(0xb2e463b6, 0x2000, RomPak, Any, "Kids on Keys", Some(1984), Some("Spinnaker"), Some("26-3167"), None, Some("Kids on Keys (1984) (26-3167) (Spinnaker).ccc")),
    rom!(0x36eecac4, 0x2000, RomPak, Any, "Kindercomp", Some(1984), Some("Tandy"), Some("26-3168"), None, Some("Kindercomp (1984) (26-3168) (Spinnaker).ccc")),
    rom!(0x4cee1c54, 0x2000, RomPak, Any, "Mega-Bug", Some(1982), Some("Tandy"), Some("26-3076"), None, Some("Mega-Bug (1982) (26-3076) (Tandy).ccc")),
    rom!(0x66130daa, 0x2000, RomPak, Any, "Micro Chess V2.0", Some(1980), Some("Personal Software"), Some("26-3050"), None, Some("Micro Chess v2.0 (1980) (26-3050) (Tandy).ccc")),
    rom!(0xd4320300, 0x1000, RomPak, Any, "Micro Painter", Some(1982), Some("Datasoft"), Some("26-3077"), None, Some("Micro Painter (1982) (26-3077) (Tandy).ccc")),
    rom!(0x24772d4f, 0x1000, RomPak, Any, "Microbes", Some(1981), Some("Tandy"), Some("26-3085"), None, Some("Microbes (1981) (26-3085) (Tandy).ccc")),
    rom!(0xf7653118, 0x1000, RomPak, Any, "Monster Maze", Some(1983), Some("Tandy"), Some("26-3081"), None, Some("Monster Maze (1983) (26-3081) (Tandy).ccc")),
    rom!(0x90e48836, 0x1000, RomPak, Any, "Music", Some(1980), Some("Tandy"), Some("26-3151"), None, Some("Music (1980) (26-3151) (Tandy).ccc")),
    rom!(0x54094dea, 0x2000, RomPak, Any, "Panic Button", Some(1983), Some("Tandy"), Some("26-3147"), None, Some("Panic Button (1983) (26-3147) (Tandy).ccc")),
    rom!(0xd577436a, 0x2000, RomPak, Any, "Personal Finance", Some(1980), Some("Tandy"), Some("26-3101"), None, Some("Personal Finance (1980) (26-3101) (Tandy).ccc")),
    rom!(0x73c41fde, 0x2000, RomPak, Any, "Personal Finance II", Some(1983), Some("Tandy"), Some("26-3106"), None, Some("Personal Finance II (1983) (26-3106) (Tandy).ccc")),
    rom!(0xd5baf3ad, 0x1000, RomPak, Any, "Pinball", Some(1980), Some("Tandy"), Some("26-3052"), None, Some("Pinball (1980) (26-3052) (Tandy).ccc")),
    rom!(0xf7dcc3bb, 0x1000, RomPak, Any, "Polaris", Some(1981), Some("Tandy"), Some("26-3065"), None, Some("Polaris (1981) (26-3065) (Tandy).ccc")),
    rom!(0x770e5f3a, 0x2000, RomPak, Any, "Poltergeist", Some(1982), Some("Tandy"), Some("26-3073"), None, Some("Poltergeist (1982) (26-3073) (Tandy).ccc")),
    rom!(0x7f507089, 0x0800, RomPak, Any, "Popcorn", Some(1981), Some("Tandy"), Some("26-3090"), None, Some("Popcorn (1981) (26-3090) (Steve Bjork).ccc")),
    rom!(0x4123fb50, 0x2000, RomPak, Any, "Project Nebula", Some(1981), Some("Tandy"), Some("26-3063"), None, Some("Project Nebula (1981) (26-3063) (Tandy).ccc")),
    rom!(0x3218e0f5, 0x1000, RomPak, Any, "Quasar Commander", Some(1980), Some("Tandy"), Some("26-3051"), None, Some("Quasar Commander (1980) (26-3051) (Tandy).ccc")),
    rom!(0xfad4c7e3, 0x1000, RomPak, Any, "Reactoid", Some(1983), Some("Tandy"), Some("26-3092"), None, Some("Reactoid (1983) (26-3092) (Tandy).ccc")),
    rom!(0x3bf566bb, 0x2000, RomPak, Any, "Roman Checkers", Some(1981), Some("Tandy"), Some("26-3071"), None, Some("Roman Checkers (1981) (26-3071) (Tandy).ccc")),
    rom!(0x1f1ba95d, 0x2000, RomPak, Any, "Shooting Gallery", Some(1982), Some("Tandy"), Some("26-3088"), None, Some("Shooting Gallery (1982) (26-3088) (Tandy).ccc")),
    rom!(0x1a05a395, 0x2000, RomPak, Any, "Skiing", Some(1981), Some("Tandy"), Some("26-3058"), None, Some("Skiing (1981) (26-3058) (Tandy).ccc")),
    rom!(0xb98d5eaf, 0x2000, RomPak, Any, "Slay the Nereis", Some(1983), Some("Tandy"), Some("26-3086"), None, Some("Slay the Nereis (1983) (26-3086) (Tandy).ccc")),
    rom!(0x44390e55, 0x4000, RomPak, Any, "Soko-Ban", Some(1988), Some("Tandy"), Some("26-3161"), None, Some("Soko-Ban (1988) (26-3161) (Tandy).ccc")),
    rom!(0xf4700120, 0x1000, RomPak, Any, "Space Assault", Some(1981), Some("Tandy"), Some("26-3060"), None, Some("Space Assault (1981) (26-3060) (Tandy).ccc")),
    rom!(0x80d7ac8b, 0x3e00, RomPak, Any, "Spectaculator", Some(1983), Some("Tandy"), Some("26-3104"), None, None),
    rom!(0xec9d0199, 0x1000, RomPak, Any, "Spidercide", Some(1983), Some("Tandy"), Some("26-3049"), None, Some("Spidercide (1983) (26-3049) (Tandy).ccc")),
    rom!(0x36f31eb7, 0x2000, RomPak, Any, "Starblaze", Some(1983), Some("Greg Zumwalt"), Some("26-3094"), None, Some("Starblaze (1983) (26-3094) (Tandy).ccc")),
    rom!(0xaeb38e39, 0x2000, RomPak, Any, "Stellar Lifeline", Some(1983), Some("Tandy"), Some("26-3047"), None, Some("Stellar Lifeline (1983) (26-3047) (Tandy).ccc")),
    rom!(0xf4f2b0a0, 0x2000, RomPak, Any, "Temple of Rom", Some(1984), Some("Tandy"), Some("26-3045"), None, Some("Temple of ROM (1984) (26-3045) (Tandy).ccc")),
    rom!(0x99de1a03, 0x1000, RomPak, Any, "Tennis", Some(1981), Some("Tandy"), Some("26-3080"), None, Some("Tennis (1981) (26-3080) (Tandy).ccc")),
    rom!(0x54f38ab8, 0x3ff0, RomPak, Coco3, "TypeMate", Some(1988), Some("ZCT Systems"), Some("26-3155"), None, Some("TypeMate (1988) (26-3155) (Tandy).ccc")),
    rom!(0xcebd3196, 0x1000, RomPak, Any, "Typing Tutor", Some(1980), Some("Leah R. O'Connor"), Some("26-3152"), None, Some("Typing Tutor (1980) (26-3152) (Tandy).ccc")),
    rom!(0x8987b1e3, 0x0800, RomPak, Any, "Videotex v1.1", Some(1981), Some("Tandy"), Some("26-2222"), None, Some("Videotex v1.1 (1981) (26-2222) (Tandy).ccc")),
    rom!(0x1cd04106, 0x0800, RomPak, Any, "Videotex v1.2", Some(1981), Some("Tandy"), Some("26-2222"), None, Some("Videotex v1.2 (1981) (26-2222) (Tandy).ccc")),
    rom!(0xab96914a, 0x1000, RomPak, Any, "Wildcatting", Some(1982), Some("Tandy"), Some("26-3067"), None, Some("Wildcatting (1982) (26-3067) (Tandy).ccc")),
    rom!(0x498fde21, 0x4000, RomPak, Coco12, "Arkanoid", Some(1987), Some("Taito"), None, Some("coco12"), Some("Arkanoid (1987) (26-3043) (Tandy) (Coco 1-2 version).ccc")),
    rom!(0xbc930185, 0x2000, RomPak, Any, "Color Scripsit", Some(1981), Some("Tandy"), Some("26-3105"), Some("alt"), Some("Color Scripsit (1981) (26-3105) (Tandy) (alt).ccc")),
    rom!(0x7abb161b, 0x2000, RomPak, Any, "Micro Chess V2.0", Some(1980), Some("Personal Software"), Some("26-3050"), Some("alt"), Some("Micro Chess v2.0 (1980) (26-3050) (Tandy) (alt).ccc")),
    rom!(0xbed7dcde, 0x4000, RomPak, Coco12, "Silpheed", Some(1988), Some("Sierra"), None, Some("coco12"), None),
    rom!(0xd4bbe731, 0x4000, RomPak, Coco3, "A Mazing World of Malcom Mortar", Some(1987), Some("Tandy"), Some("26-3160"), None, Some("A Mazing World of Malcom Mortar (1987) (26-3160) (Tandy) (Coco 3).ccc")),
    rom!(0x2fab4955, 0x8000, RomPak, Coco3, "Arkanoid", Some(1987), Some("Taito"), None, None, Some("Arkanoid (1987) (26-3043) (Taito).ccc")),
    rom!(0x82929650, 0x4000, RomPak, Coco3, "Castle of Tharoggad", Some(1988), Some("Tandy"), Some("26-3159"), None, Some("Castle of Tharoggad (1988) (26-3159) (Tandy) (Coco 3).ccc")),
    rom!(0xd45e59e3, 0x2000, RomPak, Any, "Dungeons of Daggorath", Some(1982), Some("Tandy"), Some("26-3093"), None, Some("Dungeons of Daggorath (1982) (26-3093) (Tandy).ccc")),
    rom!(0x899978e7, 0x8000, RomPak, Coco3, "GFL Championship Football II", Some(1988), Some("ZCT Systems"), None, None, Some("GFL Championship Football II (1988) (26-3172) (Tandy) (Coco 3).ccc")),
    rom!(0x83bd6056, 0x8000, BankedRomPak, Any, "Mind Roll", Some(1988), Some("Tandy"), Some("26-3100"), None, Some("Mind-Roll (1988) (26-3100) (Tandy) (Coco 1-2) (Coco 3).ccc")),
    rom!(0xa9680ede, 0x10000, BankedRomPak, Coco3, "Predator", Some(1989), Some("Tandy"), Some("26-3165"), None, Some("Predator (1989) (26-3165) (Tandy) (Coco 3).ccc")),
    rom!(0xc8b64049, 0x8000, RomPak, Coco3, "RAD Warrior", Some(1987), Some("Tandy"), None, None, Some("RAD Warrior (1987) (26-3162) (Tandy) (Coco 3).ccc")),
    rom!(0x09c2e97d, 0x8000, RomPak, Coco3, "Rampage!", Some(1989), Some("Activision"), None, None, Some("Rampage! (1989) (26-3174) (Activision).ccc")),
    rom!(0xdd94dd06, 0x20000, BankedRomPak, Coco3, "RoboCop", Some(1988), Some("Tandy"), Some("26-3164"), None, Some("RoboCop (1988) (26-3164) (Data East).ccc")),
    rom!(0x3dc0ba73, 0x4000, RomPak, Coco3, "Shanghai", Some(1987), Some("Tandy"), Some("26-3084"), None, Some("Shanghai (1987) (26-3084) (Tandy) (Coco 3).ccc")),
    rom!(0xccfd0a0c, 0x8000, RomPak, Coco3, "Silpheed", Some(1988), Some("Sierra"), None, None, Some("Silpheed (1988) (26-3054) (Tandy) (Coco 1-2) (Coco 3).ccc")),
    rom!(0xf47d3880, 0x4000, RomPak, Coco3, "Springster", Some(1987), Some("Tandy"), Some("26-3078"), None, Some("Springster (1987) (26-3078) (Tandy) (Coco 3).ccc")),
    rom!(0xe8e54cbe, 0x8000, RomPak, Coco3, "Super Pitfall", Some(1988), Some("Activision"), None, None, Some("Super Pitfall (1988) (26-3171) (Activision).ccc")),
    rom!(0x8375f98b, 0x4000, RomPak, Any, "Tetris", Some(1987), Some("Tandy"), Some("26-3163"), None, Some("Tetris (1987) (26-3163) (Tandy) (Coco 1-2) (Coco 3).ccc")),
    rom!(0x0ef0df20, 0x4000, RomPak, Coco3, "Thexder", Some(1987), Some("Tandy"), Some("26-3072"), None, Some("Thexder (1987) (26-3072) (Tandy) (Coco 3).ccc")),
    rom!(0xc985282a, 0x2000, RomPak, Any, "Dungeons of Daggorath", Some(1982), Some("Tandy"), Some("26-3093"), Some("f shield, Aaron Oliver"), Some("Dungeons of Daggorath (Shield Fix) (Aaron Oliver).ccc")),
    rom!(0x878906fe, 0x8000, BankedRomPak, Any, "Mind Roll", Some(1988), Some("Tandy"), Some("26-3100"), Some("f plane1"), Some("Mind-Roll (1988) (26-3100) (Tandy) (Plane 1 fix) (Coco 1-2) (Coco 3).ccc")),
    rom!(0xabe7bb9e, 0x4000, GamesMaster, Any, "Blockdown", Some(2021), Some("Teipen Mwnci"), None, None, None),
    rom!(0x58716b7f, 0x10000, GamesMaster, Any, "Dunjunz", Some(2020), Some("Teipen Mwnci"), None, None, None),
    rom!(0x808b2a0a, 0x2000, GamesMaster, Any, "CyD Games Master Cartridge ROM", None, None, None, None, Some("cyd_gmc.ccc")),
];
