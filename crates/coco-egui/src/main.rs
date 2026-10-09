//! `coco-egui` — eframe frontend. See `DESIGN.md` §8.
//!
//! Boots the real Super Extended Color BASIC ROM, shows the GIME/VDG output as an
//! integer-scaled texture, and feeds host keyboard input into the CoCo matrix in
//! one of two modes (toggle with F12):
//!
//! - Positional — physical key → CoCo matrix position (CoCo applies its own shift
//!   semantics, like MAME). The default.
//! - Symbolic — the character you type is injected through the CoCo keys that produce it.
//!
//! Cartridges and other peripherals are configured in the machine's
//! `[peripherals]` definition and mounted at launch, not from a menu.
//! With the `debug-ui` feature enabled, the toolbar's Debug tile (or ⌘D / Ctrl+D)
//! toggles the interactive debugger (Controls/Registers/Disassembly/Memory/Stack/Hardware
//! panels — `debugger.rs`, §3).

#![deny(rustdoc::broken_intra_doc_links)]

mod about;
mod app;
mod audio;
pub(crate) mod chrome;
mod cli;
mod config;
mod control;
mod debugger;
mod defaults;
mod display;
mod host;
mod hotkeys;
mod joy;
mod kbd_help;
mod keymap;
mod launch;
mod machine_def;
mod manager;
mod media;
mod mpi;
mod new_vm;
mod orch90_meters;
mod paper_export;
mod paper_render;
mod paper_view;
mod path_remap;
mod paths;
mod perf;
mod photo_view;
mod rom_load;
mod rs232;
mod runtime_fmt;
mod save_state;
mod startup;
mod status_icons;
mod typeahead;
mod widgets;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

pub(crate) use app::{AppParams, CocoApp};
use chrono::{Datelike, Timelike};
use clap::Parser;
use cli::Cli;
use coco_core::cart::{BankedROMPak, CoCoMaxModule, GamesMasterCartridge, MultiPak, ROMPak};
use coco_core::drivewire::{self, DWImage, DWTime};
use coco_core::fdc::{DiskCart, JVCDisk};
use coco_core::keyboard::{self as kbd, Pos};
use coco_core::orch90::Orch90;
use coco_core::rtc::{DistoRTC, RTCTime};
use coco_core::ssc::SoundSpeechCartridge;
use coco_core::vhd::VHDImage;
use coco_core::{Machine, MachineConfig, MachineVariant};
use eframe::egui;
use joy::JoystickInputs;
// Re-exported rather than plainly imported: the modules carved out of this file
// were all crate-root items until recently, and `manager`, `save_state` and the
// `ui_tests` harness still reach for them as `crate::…`.
pub(crate) use defaults::{default_ram, default_vdg, machine_label};
pub(crate) use display::Display;
pub(crate) use host::{host_dw_clock, host_time_source};
pub(crate) use keymap::{control_key_pos, is_joystick_key, key_to_pos};
#[cfg(test)]
pub(crate) use launch::launch_machine;
pub(crate) use launch::launch_machine_with_gamepad;
pub(crate) use mpi::{DEFAULT_MPI_SWITCH_SLOT, MPI_SLOT_COUNT, MPISlot, MPIState};
pub(crate) use rom_load::{
    Coco12ROMResult, ROM_DB_PSEUDO_PATH_PREFIX, ROMSource, compose_coco12_rom, dos_rom_path,
    installed_roms_dir, orch90_rom_path, report_rom_validation, rom_db_pseudo_path,
    rs232_eprom_default_path,
};
pub(crate) use rs232::{RS232_TCP_DEFAULT_ADDR, RS232Endpoint, RS232EndpointKind};
pub(crate) use runtime_fmt::humanize_runtime;
pub(crate) use startup::{
    StartupInfo, banner, cartridge_count, load_dotenv, missing_assets, renderer_info,
    require_data_dir, rom_count, setup_logging, use_color,
};
pub(crate) use status_icons::{
    StatusActivity, cart_icon, cassette_icon, drivewire_icon, floppy_icon, joystick_icon,
    keyboard_icon, monitor_icon, mpi_icon, printer_icon, rs232_icon, speaker_icon, tv_icon,
    vhd_icon,
};
pub(crate) use typeahead::{KbMode, TypeAhead};
pub(crate) use widgets::{
    BUTTON_GAP, BUTTON_SIZE, PLAY_GLYPH, RESET_GLYPH, RESET_LABEL, START_LABEL, STOP_GLYPH,
    STOP_LABEL, SUSPEND_GLYPH, SUSPEND_HOVER, SUSPEND_LABEL, UI_DRIVES, titled_group,
    toolbar_button, toolbar_button_width, toolbar_separator, toolbar_separator_width, toolbar_tile,
    window_title,
};

/// Wayland app id and eframe persistence name; must match
/// `packaging/linux/cocovm.desktop` so launchers pair every window with its icon.
pub(crate) const APP_ID: &str = "cocovm";

/// Viewport builder for a native window. Every window, child viewports
/// included, carries the app id: eframe passes the icon down but not the id.
pub(crate) fn window_builder(title: impl Into<String>) -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_app_id(APP_ID)
        .with_title(title)
}

/// Integer scale factor for the canvas rows when sizing a VM window.
pub(crate) const SCALE: f32 = 3.0;
/// Physical aspect the CoCo frame fills on an NTSC set (4:3). The canvas is
/// 640×240 (two pixels per VDG dot); the image is fitted to this ratio
/// independently of texture dimensions and display effects.
pub(crate) const TARGET_ASPECT: f32 = 4.0 / 3.0;
/// Cap on emulated fields run in one UI update: catches up after short host
/// stalls (~130 ms) but drops time beyond that instead of spiralling.
pub(crate) const MAX_FIELDS_PER_UPDATE: usize = 8;
/// Longest wall-clock gap credited to the emulation clock, in seconds. Gaps
/// beyond this (window drag, app hidden, debugger pause) are discarded.
pub(crate) const MAX_FRAME_DT: f64 = 0.25;
/// Audio cushion for an unfocused or minimized VM. The host service deadline
/// reserves one field of this interval for callback and presentation jitter.
/// Cushion plus bounded catch-up remains below the 0.25 s audio ring at NTSC/PAL.
pub(crate) const BACKGROUND_REPAINT_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);
/// Horizontal inner margin `chrome::toolbar`'s `TopBottomPanel::top
/// ("toolbar")` gives its content, using an explicit `.frame(...)` rather than
/// egui's `TopBottomPanel` default — matches `egui::Frame::side_top_panel`'s
/// own default (`Margin::symmetric(8, 2)`, the `8` here) so the toolbar
/// panel doesn't look different from the app's other panels, but as our own
/// named constant it can't silently drift if a future egui version changes
/// that default. `i8`: the field type `egui::Margin` uses.
pub(crate) const TOOLBAR_PANEL_MARGIN_X: i8 = 8;
/// Vertical inner margin `chrome::toolbar`'s toolbar panel gives its content
/// above and below, set the same explicit way as
/// [`TOOLBAR_PANEL_MARGIN_X`] — also matches `Frame::side_top_panel`'s
/// default (the `2` in `Margin::symmetric(8, 2)`). The panel's separator
/// line is drawn on the boundary itself and adds no extra height, so this is
/// exactly the frame's overhead. `i8`, cast to `f32` later for the window-
/// sizing formula.
pub(crate) const TOOLBAR_PANEL_MARGIN_Y: i8 = 2;
/// Height reserved for the toolbar row when sizing the window: the toolbar
/// tiles' own height ([`BUTTON_SIZE`].y) plus the panel frame's vertical
/// margin on both edges ([`TOOLBAR_PANEL_MARGIN_Y`]).
pub(crate) const TOOLBAR_H: f32 = BUTTON_SIZE.y + 2.0 * TOOLBAR_PANEL_MARGIN_Y as f32;
/// Height of the bottom status bar row: both what the window-sizing math
/// reserves for it and the panel's own exact height
/// (`chrome::status_bar`'s `status_bar_ui`), so the two can't drift apart.
/// Roomier than the text alone needs — it has to clear the device icons,
/// which are drawn at `status_icons::paint`'s `ICON_SCALE`.
pub(crate) const STATUS_BAR_H: f32 = 28.0;
/// Symbolic-mode key timing, in fields: the minimum a synthesized key is held,
/// then the minimum it stays released (see `typeahead::TypeAhead`).
pub(crate) const TYPE_HOLD_FIELDS: u8 = 2;
pub(crate) const TYPE_GAP_FIELDS: u8 = 1;
/// Longest a type-ahead hold or gap waits for the CPU to scan the key's
/// column before moving on regardless.
pub(crate) const TYPE_SCAN_TIMEOUT_FIELDS: u8 = 60;

fn main() -> eframe::Result<()> {
    // Before anything reads the environment: RUST_LOG and clap's env fallbacks need `.env`
    // loaded first.
    load_dotenv();

    // Before anything writes to stdout: legacy Windows conhost needs the VT opt-in this performs.
    let use_color = use_color();

    // Parsed first: the global log subscriber can't be built before the flags it reads are known.
    let mut cli = Cli::parse();
    // The slug is a command, not a global setting: it never falls through to
    // `config.toml`, so it leaves `Cli` before `config::resolve` consumes it.
    let machine = cli.machine.take();

    // A malformed config.toml is fatal at startup, same severity as a bad machine definition
    // (`machine_def::load_all`).
    let config_path = paths::config_dir().map(|dir| dir.join(config::CONFIG_FILE_NAME));
    if let Some(path) = config_path.as_deref() {
        config::seed_default_file(path);
    }
    let file_config = config::load(config_path.as_deref()).unwrap_or_else(|e| {
        eprintln!("coco: cannot load config file: {e}");
        std::process::exit(1);
    });
    let config = config::resolve(cli, file_config);
    let log_reload = setup_logging(use_color, config.log_level.into());

    // The app always opens the CoCoVM manager window; a named machine is started from that
    // window's own machine definitions (`manager::run`).
    manager::run(config, config_path, log_reload, machine)
}

#[cfg(test)]
mod ui_tests;
