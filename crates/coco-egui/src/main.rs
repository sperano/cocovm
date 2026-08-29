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
mod chrome;
mod cli;
mod debugger;
mod defaults;
mod display;
mod host;
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
mod paths;
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
use coco_core::cart::{GamesMasterCartridge, MultiPak, ROMPak};
use coco_core::drivewire::{self, DWImage, DWTime};
use coco_core::fdc::{DiskCart, JVCDisk};
use coco_core::keyboard::{self as kbd, Pos};
use coco_core::orch90::Orch90;
use coco_core::rtc::{DistoRTC, DistoRTCModel, RTCTime};
use coco_core::ssc::SoundSpeechCartridge;
use coco_core::vhd::VHDImage;
use coco_core::{Machine, MachineConfig};
use eframe::egui;
use joy::JoystickInputs;
// Re-exported rather than plainly imported: the modules carved out of this file
// were all crate-root items until recently, and `manager`, `save_state` and the
// `ui_tests` harness still reach for them as `crate::…`.
pub(crate) use defaults::{default_ram, default_vdg, machine_label};
pub(crate) use display::Display;
pub(crate) use host::{host_dw_clock, host_time_source};
pub(crate) use keymap::{control_key_pos, is_joystick_key, key_to_pos};
pub(crate) use launch::launch_machine;
pub(crate) use media::disk::NO_FD502_HINT;
pub(crate) use mpi::{DEFAULT_MPI_SWITCH_SLOT, MPI_SLOT_COUNT, MPISlot, MPIState};
pub(crate) use rom_load::{
    Coco12ROMResult, ROM_DB_PSEUDO_PATH_PREFIX, ROMSource, compose_coco12_rom, disk_basic_rom_path,
    installed_roms_dir, report_rom_validation, rom_db_pseudo_path, rs232_eprom_default_path,
};
pub(crate) use rs232::{RS232_TCP_DEFAULT_ADDR, RS232Endpoint, RS232EndpointKind};
pub(crate) use runtime_fmt::humanize_runtime;
pub(crate) use startup::{
    StartupInfo, banner, ensure_assets, load_dotenv, renderer_info, rom_count, setup_logging,
    use_color,
};
pub(crate) use status_icons::{
    StatusActivity, cart_icon, cassette_icon, drivewire_icon, floppy_icon, joystick_icon,
    keyboard_icon, monitor_icon, mpi_icon, printer_icon, rs232_icon, tv_icon, vhd_icon,
};
pub(crate) use typeahead::{KbMode, TypeAhead};
#[cfg(feature = "debug-ui")]
pub(crate) use widgets::toolbar_separator;
pub(crate) use widgets::{
    BUTTON_GAP, BUTTON_SIZE, PLAY_GLYPH, RESET_GLYPH, RESET_LABEL, START_LABEL, STOP_GLYPH,
    STOP_LABEL, SUSPEND_GLYPH, SUSPEND_HOVER, SUSPEND_LABEL, UI_DRIVES, titled_group,
    toolbar_button, window_title,
};

/// Integer scale factor for the (small) CoCo framebuffer.
pub(crate) const SCALE: f32 = 3.0;
/// Physical aspect the CoCo frame fills on an NTSC set (4:3). The framebuffer is
/// 288×224 (≈1.29:1); when aspect correction is on, the image is stretched
/// horizontally to this ratio so pixels are ~3% wider than tall, as on real hardware.
pub(crate) const TARGET_ASPECT: f32 = 4.0 / 3.0;
/// Cap on emulated fields run in one UI update: catches up after short host
/// stalls (~130 ms) but drops time beyond that instead of spiralling.
pub(crate) const MAX_FIELDS_PER_UPDATE: usize = 8;
/// Longest wall-clock gap credited to the emulation clock, in seconds. Gaps
/// beyond this (window drag, app hidden, debugger pause) are discarded.
pub(crate) const MAX_FRAME_DT: f64 = 0.25;
/// Repaint cadence for running VMs while no cocovm window has focus (egui
/// subtracts its 1/60 s `predicted_dt`, so wake-ups land every ~83 ms). Bounds:
/// the wake gap must stay under the one-interval audio cushion, and cushion +
/// `MAX_FIELDS_PER_UPDATE` fields must stay under the 0.25 s audio ring.
pub(crate) const BACKGROUND_REPAINT_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);
/// Height reserved for the top menu bar row when sizing the window.
pub(crate) const MENU_BAR_H: f32 = 22.0;
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
/// Symbolic-mode key timing, in fields: hold a synthesized key then release.
pub(crate) const TYPE_HOLD_FIELDS: u8 = 2;
pub(crate) const TYPE_GAP_FIELDS: u8 = 1;

fn main() -> eframe::Result<()> {
    // Before anything reads the environment: RUST_LOG and clap's env fallbacks need `.env`
    // loaded first.
    load_dotenv();

    // Before anything writes to stdout: legacy Windows conhost needs the VT opt-in this performs.
    let use_color = use_color();

    // Parsed first: the global log subscriber can't be built before the flags it reads are known.
    let cli = Cli::parse();
    setup_logging(use_color, cli.log_level.into());

    // The app always opens the CocoVM manager window; a future CLI will build on its machine
    // definitions.
    manager::run()
}

#[cfg(test)]
mod ui_tests;
