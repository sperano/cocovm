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

use super::{CartridgeHardware, KnownCartridgeROM};

macro_rules! rom {
    ($crc32:expr, $size:expr, $hardware:ident, $desc:expr) => {
        KnownCartridgeROM {
            size: $size,
            crc32: $crc32,
            desc: $desc,
            hardware: CartridgeHardware::$hardware,
        }
    };
}

/// Known CoCo cartridge ROMs adapted from XRoar, plus CocoVM additions.
#[rustfmt::skip]
pub const KNOWN_CARTRIDGE_ROMS: &[KnownCartridgeROM] = &[
    rom!(0x7d1cac0e, 0x2000, RomPak, "Androne (1983) (Tandy) (26-3096)"),
    rom!(0x2fdf5b58, 0x1000, RomPak, "Art Gallery (1981) (Robert G. Kilgus) (26-3061)"),
    rom!(0xf76f6fbe, 0x4000, RomPak, "Atom (1983) (Tandy) (26-3149)"),
    rom!(0x16d2d946, 0x0800, RomPak, "Audio Spectrum Analyzer (1981) (Tandy) (26-3156)"),
    rom!(0x0d964862, 0x0800, RomPak, "Audio Spectrum Analyzer v2 (1983) (Tandy) (26-3156)"),
    rom!(0xa3b8ba85, 0x1000, RomPak, "Backgammon (1980) (Tandy) (26-3059)"),
    rom!(0x3be0cf60, 0x1000, RomPak, "Bingo Math (1980) (Tandy) (26-3150)"),
    rom!(0xd6b940f1, 0x2000, RomPak, "Bridge Tutor I (1982) (Philidor Software) (26-3158)"),
    rom!(0xc7ed9d30, 0x1000, RomPak, "Bustout (1981) (Tandy) (26-3056)"),
    rom!(0x41dd5a1a, 0x2000, RomPak, "Canyon Climber (1982) (Tandy) (26-3089)"),
    rom!(0x8869eddc, 0x1000, RomPak, "Castle Guard (1981) (The Image Producers) (26-3079)"),
    rom!(0x05dc5ef3, 0x1000, RomPak, "Checker King (1980) (Personal Software) (26-3055)"),
    rom!(0xfe4c93e4, 0x2000, RomPak, "Clowns & Balloons (1982) (Tandy) (26-3087)"),
    rom!(0xad1937c1, 0x2000, RomPak, "Color Baseball (1980) (Dale A. Lear) (26-3095)"),
    rom!(0xf4be90bc, 0x1000, RomPak, "Color Cubes (1981) (Robert G. Kilgus) (26-3075)"),
    rom!(0xd78be10a, 0x1000, RomPak, "Color File (1981) (Tandy) (26-3103)"),
    rom!(0x8acd7ea2, 0x2800, RomPak, "Color Forth (1981) (Microworks)"),
    rom!(0x9fb1e7d9, 0x2000, RomPak, "Color Logo (1983) (Larry Kheriaty & George Gerhold) (26-2722)"),
    rom!(0x3aca199b, 0x2000, RomPak, "Color Scripsit (1981) (Tandy) (26-3105)"),
    rom!(0x7bffd03a, 0x4000, RomPak, "Color Scripsit II (1986) (Tandy) (26-3109)"),
    rom!(0x06075f2a, 0x1000, RomPak, "Crosswords (1981) (26-3082)"),
    rom!(0xbfa3585d, 0x4000, RomPak, "Cyrus World Class Chess (1983) (Tandy) (26-3064)"),
    rom!(0xd990e1f9, 0x1000, RomPak, "Deluxe RS-232 Program Pak (1983) (26-2226) (Tandy)"),
    rom!(0x1199d27f, 0x2000, RomPak, "Demolition Derby (1984) (Tandy) (26-3044)"),
    rom!(0xb7a1aa3e, 0x3f00, RomPak, "Demon Attack (1984) (Tandy) (26-3099)"),
    rom!(0xd5257b50, 0x0800, RomPak, "Diagnostics (1980) (Tandy) (26-3019)"),
    rom!(0x8cd56308, 0x0800, RomPak, "Diagnostics v2.0 (1982) (Tandy) (26-3019)"),
    rom!(0xba1d9e81, 0x2000, RomPak, "Dino Wars (1981) (Tandy) (26-3057)"),
    rom!(0x667bc55d, 0x2000, RomPak, "Direct Connect Modem Pak (1985) (26-2228) (Tandy)"),
    rom!(0x819fc9fb, 0x2000, RomPak, "Don Pan (1985) (Tandy) (26-3097)"),
    rom!(0xb86830a9, 0x3f00, RomPak, "Doodle Bug 1 (1982) (26-xxxx) (Computerware)"),
    rom!(0xc3d41232, 0x2000, RomPak, "Doodle Bug 2 (1982) (26-xxxx) (Computerware)"),
    rom!(0xea820c39, 0x2000, RomPak, "Doodle Bug 3 (1982) (26-xxxx) (Computerware)"),
    rom!(0x4cc44337, 0x1000, RomPak, "Doubleback (1982) (Tandy) (26-3091)"),
    rom!(0xa923f5a2, 0x2000, RomPak, "Downland v1.0 (1983) (Tandy) (26-3046)"),
    rom!(0x0229c319, 0x2000, RomPak, "Downland v1.1 (1983) (Tandy) (26-3046)"),
    rom!(0x6f1e913a, 0x4000, RomPak, "Dragonfire (1984) (Tandy) (26-3098)"),
    rom!(0xf4374a55, 0x4000, RomPak, "EDTASM+ (1982) (26-3250) (Tandy)"),
    rom!(0x7a1c2f2d, 0x2000, RomPak, "Facemaker (1984) (Tandy) (26-3166)"),
    rom!(0xf17b5b38, 0x2000, RomPak, "Football (1980) (26-3053) (Tandy)"),
    rom!(0x93153b15, 0x2000, RomPak, "Fraction Fever (1984) (Spinnaker) (26-3169)"),
    rom!(0x984ee0d9, 0x1000, RomPak, "Galactic Attack (1982) (Tandy) (26-3066)"),
    rom!(0xfd84fc5c, 0x2000, RomPak, "Gomoku-Renju (1983) (Tandy) (26-3069)"),
    rom!(0xb2e463b6, 0x2000, RomPak, "Kids on Keys (1984) (Spinnaker) (26-3167)"),
    rom!(0x36eecac4, 0x2000, RomPak, "Kindercomp (1984) (Tandy) (26-3168)"),
    rom!(0x4cee1c54, 0x2000, RomPak, "Mega-Bug (1982) (Tandy) (26-3076)"),
    rom!(0x66130daa, 0x2000, RomPak, "Micro Chess V2.0 (1980) (Personal Software) (26-3050)"),
    rom!(0xd4320300, 0x1000, RomPak, "Micro Painter (1982) (Datasoft) (26-3077)"),
    rom!(0x24772d4f, 0x1000, RomPak, "Microbes (1981) (Tandy) (26-3085)"),
    rom!(0xf7653118, 0x1000, RomPak, "Monster Maze (1983) (Tandy) (26-3081)"),
    rom!(0x90e48836, 0x1000, RomPak, "Music (1980) (Tandy) (26-3151)"),
    rom!(0x54094dea, 0x2000, RomPak, "Panic Button (1983) (Tandy) (26-3147)"),
    rom!(0xd577436a, 0x2000, RomPak, "Personal Finance (1980) (Tandy) (26-3101)"),
    rom!(0x73c41fde, 0x2000, RomPak, "Personal Finance II (1983) (Tandy) (26-3106)"),
    rom!(0xd5baf3ad, 0x1000, RomPak, "Pinball (1980) (Tandy) (26-3052)"),
    rom!(0xf7dcc3bb, 0x1000, RomPak, "Polaris (1981) (Tandy) (26-3065)"),
    rom!(0x770e5f3a, 0x2000, RomPak, "Poltergeist (1982) (Tandy) (26-3073)"),
    rom!(0x7f507089, 0x0800, RomPak, "Popcorn (1981) (Tandy) (26-3090)"),
    rom!(0x4123fb50, 0x2000, RomPak, "Project Nebula (1981) (Tandy) (26-3063)"),
    rom!(0x3218e0f5, 0x1000, RomPak, "Quasar Commander (1980) (Tandy) (26-3051)"),
    rom!(0xfad4c7e3, 0x1000, RomPak, "Reactoid (1983) (Tandy) (26-3092)"),
    rom!(0x3bf566bb, 0x2000, RomPak, "Roman Checkers (1981) (Tandy) (26-3071)"),
    rom!(0x1f1ba95d, 0x2000, RomPak, "Shooting Gallery (1982) (Tandy) (26-3088)"),
    rom!(0x1a05a395, 0x2000, RomPak, "Skiing (1981) (Tandy) (26-3058)"),
    rom!(0xb98d5eaf, 0x2000, RomPak, "Slay the Nereis (1983) (Tandy) (26-3086)"),
    rom!(0x44390e55, 0x4000, RomPak, "Soko-Ban (1988) (Tandy) (26-3161)"),
    rom!(0xf4700120, 0x1000, RomPak, "Space Assault (1981) (Tandy) (26-3060)"),
    rom!(0x80d7ac8b, 0x3e00, RomPak, "Spectaculator (1983) (Tandy) (26-3104)"),
    rom!(0xec9d0199, 0x1000, RomPak, "Spidercide (1983) (Tandy) (26-3049)"),
    rom!(0x36f31eb7, 0x2000, RomPak, "Starblaze (1983) (Greg Zumwalt) (26-3094)"),
    rom!(0xaeb38e39, 0x2000, RomPak, "Stellar Lifeline (1983) (Tandy) (26-3047)"),
    rom!(0xf4f2b0a0, 0x2000, RomPak, "Temple of Rom (1984) (Tandy) (26-3045)"),
    rom!(0x99de1a03, 0x1000, RomPak, "Tennis (1981) (Tandy) (26-3080)"),
    rom!(0x54f38ab8, 0x3ff0, RomPak, "TypeMate (1988) (26-3155) (ZCT Systems)"),
    rom!(0xcebd3196, 0x1000, RomPak, "Typing Tutor (1980) (Leah R. O'Connor) (26-3152)"),
    rom!(0x8987b1e3, 0x0800, RomPak, "Videotex v1.1 (1981) (Tandy) (26-2222)"),
    rom!(0x1cd04106, 0x0800, RomPak, "Videotex v1.2 (1981) (Tandy) (26-2222)"),
    rom!(0xab96914a, 0x1000, RomPak, "Wildcatting (1982) (Tandy) (26-3067)"),
    rom!(0x498fde21, 0x4000, RomPak, "Arkanoid (1987) (Taito) [coco12]"),
    rom!(0xbc930185, 0x2000, RomPak, "Color Scripsit (1981) (Tandy) (26-3105) [alt]"),
    rom!(0x7abb161b, 0x2000, RomPak, "Micro Chess V2.0 (1980) (Personal Software) (26-3050) [alt]"),
    rom!(0xbed7dcde, 0x4000, RomPak, "Silpheed (1988) (Sierra) [coco12]"),
    rom!(0xd4bbe731, 0x4000, RomPak, "A Mazing World of Malcom Mortar (1987) (Tandy) (26-3160)"),
    rom!(0x2fab4955, 0x8000, RomPak, "Arkanoid (1987) (Taito)"),
    rom!(0x82929650, 0x4000, RomPak, "Castle of Tharoggad (1988) (Tandy) (26-3159)"),
    rom!(0xd45e59e3, 0x2000, RomPak, "Dungeons of Daggorath (1982) (Tandy) (26-3093)"),
    rom!(0x899978e7, 0x8000, RomPak, "GFL Championship Football II (1988) (ZCT Systems)"),
    rom!(0x83bd6056, 0x8000, BankedRomPak, "Mind Roll (1988) (Tandy) (26-3100)"),
    rom!(0xa9680ede, 0x10000, BankedRomPak, "Predator (1989) (Tandy) (26-3165)"),
    rom!(0xc8b64049, 0x8000, RomPak, "RAD Warrior (1987) (Tandy)"),
    rom!(0x09c2e97d, 0x8000, RomPak, "Rampage! (1989) (Activision)"),
    rom!(0xdd94dd06, 0x20000, BankedRomPak, "RoboCop (1988) (Tandy) (26-3164)"),
    rom!(0x3dc0ba73, 0x4000, RomPak, "Shanghai (1987) (Tandy) (26-3084)"),
    rom!(0xccfd0a0c, 0x8000, RomPak, "Silpheed (1988) (Sierra)"),
    rom!(0xf47d3880, 0x4000, RomPak, "Springster (1987) (Tandy) (26-3078)"),
    rom!(0xe8e54cbe, 0x8000, RomPak, "Super Pitfall (1988) (Activision)"),
    rom!(0x8375f98b, 0x4000, RomPak, "Tetris (1987) (Tandy) (26-3163)"),
    rom!(0x0ef0df20, 0x4000, RomPak, "Thexder (1987) (Tandy) (26-3072)"),
    rom!(0xc985282a, 0x2000, RomPak, "Dungeons of Daggorath (1982) (Tandy) (26-3093) [f shield, Aaron Oliver]"),
    rom!(0x878906fe, 0x8000, BankedRomPak, "Mind Roll (1988) (Tandy) (26-3100) [f plane1]"),
    rom!(0xabe7bb9e, 0x4000, GamesMaster, "Blockdown (2021) (Teipen Mwnci)"),
    rom!(0x58716b7f, 0x10000, GamesMaster, "Dunjunz (2020) (Teipen Mwnci)"),
    rom!(0x808b2a0a, 0x2000, GamesMaster, "CyD Games Master Cartridge ROM"),
];
