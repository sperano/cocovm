//! Shared `egui_kittest` test infrastructure for every topic module under
//! `ui_tests`: harness construction (a bare `CocoApp` window and the manager
//! window), the click, hover, and combo-select interaction helpers (see the
//! parent module doc for the interaction conventions they encode), and small
//! fixture builders
//! (`sample_entry`, `sample_coco2_entry`).

use egui_kittest::kittest::Queryable;

use coco_core::{MachineVariant, MemorySize, VDGVariant, VideoStandard};

use crate::rom_load::load_default_rom;

use crate::*;

pub(super) type AppHarness = egui_kittest::Harness<'static, CocoApp>;
pub(super) type ManagerHarness = egui_kittest::Harness<'static, manager::ManagerApp>;

/// Boot a default (CoCo 3) machine into a kittest harness — a bare
/// `CocoApp::new` call, unlike a manager-launched VM (`launch::launch_machine`).
pub(super) fn boot_harness() -> AppHarness {
    let roms_dir = test_assets::roms_dir();
    let (rom, rom_source) = load_default_rom(MachineVariant::Coco3, &roms_dir)
        .expect("coco3.rom is required in the cocovm XDG data directory");
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        CocoApp::new(
            MachineConfig::default(),
            rom,
            rom_source,
            AppParams::default(),
        )
    });
    // egui only puts on-screen widgets in the AccessKit tree, so size for the full Machine menu.
    harness.set_size(egui::vec2(1024.0, 768.0));
    harness.step();
    harness
}

/// Click the widget labelled exactly `label`: hover one frame, then press and
/// release across the next two — egui fires `clicked` on the release.
pub(super) fn click<S: 'static>(harness: &mut egui_kittest::Harness<'static, S>, label: &str) {
    harness.get_by_label(label).hover();
    harness.step();
    harness.get_by_label(label).click();
    harness.step();
    harness.step();
}

/// [`click`] with the secondary button — opens the machine-list rows'
/// context menu.
pub(super) fn right_click<S: 'static>(
    harness: &mut egui_kittest::Harness<'static, S>,
    label: &str,
) {
    harness.get_by_label(label).hover();
    harness.step();
    harness.get_by_label(label).click_secondary();
    harness.step();
    harness.step();
}

/// [`click`] with `modifiers` held for the press/release (for example, Cmd/Ctrl- or
/// Shift-click); modifiers reset afterward so they don't leak into the next click.
pub(super) fn click_modifiers<S: 'static>(
    harness: &mut egui_kittest::Harness<'static, S>,
    label: &str,
    modifiers: egui::Modifiers,
) {
    harness.get_by_label(label).hover();
    harness.step();
    harness.get_by_label(label).click_modifiers(modifiers);
    harness.step();
    harness.step();
}

/// [`click`] matching by substring — for widgets whose accessible label adds
/// decoration beyond the visible caption (submenu buttons' trailing "⏵").
pub(super) fn click_containing<S: 'static>(
    harness: &mut egui_kittest::Harness<'static, S>,
    label: &str,
) {
    harness.get_by_label_contains(label).hover();
    harness.step();
    harness.get_by_label_contains(label).click();
    harness.step();
    harness.step();
}

/// Select an item in a form combo box: among all combos currently showing
/// `current`, open the `index`-th in screen order and click `target`.
pub(super) fn select_combo_at<S: 'static>(
    harness: &mut egui_kittest::Harness<'static, S>,
    current: &str,
    index: usize,
    target: &str,
) {
    fn nth<'t, S>(
        harness: &'t egui_kittest::Harness<'static, S>,
        value: &'t str,
        index: usize,
    ) -> egui_kittest::Node<'t> {
        let mut nodes: Vec<_> = harness.get_all_by_value(value).collect();
        nodes.sort_by(|a, b| {
            (a.rect().min.y.total_cmp(&b.rect().min.y))
                .then(a.rect().min.x.total_cmp(&b.rect().min.x))
        });
        nodes
            .into_iter()
            .nth(index)
            .unwrap_or_else(|| panic!("no {index}-th node with value {value:?}"))
    }
    nth(harness, current, index).hover();
    harness.step();
    nth(harness, current, index).click();
    harness.step();
    harness.step();
    click(harness, target);
}

/// Lowest-on-screen widget labelled `label` — the open-menu copy when the
/// toolbar shows the same label too.
pub(super) fn lowest_by_label<'t>(
    harness: &'t AppHarness,
    label: &'t str,
) -> egui_kittest::Node<'t> {
    harness
        .get_all_by_label(label)
        .max_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y))
        .unwrap_or_else(|| panic!("no node labelled {label:?}"))
}

/// Topmost widget labelled `label` — for example, the right stick's copy of a source
/// label the Joysticks menu lists once per stick.
pub(super) fn topmost_by_label<'t>(
    harness: &'t AppHarness,
    label: &'t str,
) -> egui_kittest::Node<'t> {
    harness
        .get_all_by_label(label)
        .min_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y))
        .unwrap_or_else(|| panic!("no node labelled {label:?}"))
}

/// A minimal valid entry: a CoCo 3 default config under `name`, built
/// through [`machine_def::MachineDef::from_config`] like the manager's own "New…" flow.
pub(super) fn sample_entry(slug: &str, name: &str) -> manager::MachineEntry {
    manager::MachineEntry::new(
        slug.to_string(),
        machine_def::MachineDef::from_config(name.to_string(), None, &MachineConfig::default()),
    )
}

/// A minimal valid CoCo 2 entry — [`sample_entry`]'s default is CoCo 3, so
/// pairing the two gives distinct machine families.
pub(super) fn sample_coco2_entry(slug: &str, name: &str) -> manager::MachineEntry {
    manager::MachineEntry::new(
        slug.to_string(),
        machine_def::MachineDef::from_config(
            name.to_string(),
            None,
            &MachineConfig {
                variant: MachineVariant::Coco2,
                video: VideoStandard::NTSC,
                memory: MemorySize::K64,
                monitor: None,
                vdg: Some(VDGVariant::MC6847T1),
            },
        ),
    )
}

/// Boot a manager harness with injected entries and (optionally) a real
/// machines directory for Create/Save to write into (never the user's real config dir).
pub(super) fn manager_harness(
    machines_dir: Option<PathBuf>,
    entries: Vec<manager::MachineEntry>,
) -> ManagerHarness {
    manager_harness_with_artifacts(machines_dir, None, entries)
}

/// [`manager_harness`] with the artifact root injected too — for tests that
/// exercise thumbnail persistence (always a temp dir, never the real
/// `data_dir()`).
pub(super) fn manager_harness_with_artifacts(
    machines_dir: Option<PathBuf>,
    artifacts_root: Option<PathBuf>,
    entries: Vec<manager::MachineEntry>,
) -> ManagerHarness {
    let mut harness = egui_kittest::Harness::new_eframe(move |_cc| {
        manager::ManagerApp::new(None, machines_dir, artifacts_root, entries)
    });
    // Tall enough for the whole detail pane to land in the AccessKit tree (egui only reports
    // on-screen widgets).
    harness.set_size(egui::vec2(1080.0, 1400.0));
    harness.step();
    harness
}

/// Whether `label` matches at least one accessible node — unlike
/// `get_by_label`/`query_by_label`, which require *at most* one match.
pub(super) fn label_exists<S: 'static>(
    harness: &egui_kittest::Harness<'static, S>,
    label: &str,
) -> bool {
    harness.get_all_by_label(label).next().is_some()
}
