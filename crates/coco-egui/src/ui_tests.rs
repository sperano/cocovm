//! Headless end-to-end drive of the full app through `egui_kittest`: real
//! `eframe::App::update` frames, with clicks and key presses dispatched
//! through the AccessKit tree — the closest a test gets to a user at the real
//! window. Like `coco-core`'s boot tests, these need the git-ignored ROM assets
//! installed on the test machine.
//!
//! Interaction conventions discovered the hard way:
//! - Clicks hover on one frame and press/release on the next: egui routes a
//!   press using the previous frame's hit-test data, so a press with no
//!   prior hover misses windows that were (re)anchored this frame.
//! - Menus close on *any* item click (egui's default menu close behavior),
//!   so every menu interaction reopens the menu from the bar.
//! - Submenu buttons expose their label with a trailing "⏵" arrow — match
//!   them with `_contains`, not exactly.
//!
//! Split by topic: [`harness`] holds the shared harness-construction and
//! click/hover/combo-select interaction helpers every other module builds
//! on; the rest are one topic apiece (the VM window's own dialogs/menus, its
//! status-bar disk, tape and joysticks entries/menus and icon-only mode, and the manager window's
//! scaffold/peripherals/lifecycle/settings).

mod cartridge_detection;
mod harness;
mod hotkeys;
mod keyboard_test;
mod manager_dos_rom_test;
mod manager_drivewire_test;
mod manager_lifecycle;
mod manager_peripherals;
mod manager_roms;
mod manager_selection;
mod manager_settings;
mod manager_settings_layout_test;
mod manager_settings_mcp_test;
mod manager_sort;
mod manager_window;
mod quick_states_test;
mod vm_window_disks;
mod vm_window_joysticks;
mod vm_window_menus;
mod vm_window_status_bar;
mod vm_window_suspend;
mod vm_window_tape;
