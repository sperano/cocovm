//! Shared `egui_kittest` test infrastructure for every topic module under
//! `ui_tests`: harness construction (direct-boot and manager), the click/
//! hover/combo-select interaction helpers (see the parent module doc for
//! the interaction conventions they encode), and small fixture builders
//! (`sample_entry`, `sample_coco2_entry`).

use egui_kittest::kittest::Queryable;

use coco_core::{MachineVariant, MemorySize, VDGVariant, VideoStandard};

use crate::rom_load::load_default_rom;

use crate::*;

pub(super) type AppHarness = egui_kittest::Harness<'static, CocoApp>;
pub(super) type ManagerHarness = egui_kittest::Harness<'static, manager::ManagerApp>;

/// Boot a default (CoCo 3) machine into a kittest harness, exactly as
/// `main()` would with no CLI arguments.
pub(super) fn boot_harness() -> AppHarness {
    let roms_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms");
    let rom = load_default_rom(MachineVariant::Coco3, &roms_dir)
        .expect("roms/coco3.rom is required (git-ignored, local-only)");
    let rom_source = ROMSource::File(roms_dir.join("coco3.rom"));
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        CocoApp::new(
            MachineConfig::default(),
            rom,
            rom_source,
            None,
            [None, None],
            [None, None],
            std::array::from_fn(|_| None),
            false,
            false,
            false,
        )
    });
    // Room for the full Machine menu: egui only puts on-screen widgets in
    // the AccessKit tree, so a too-small viewport hides the lower items.
    harness.set_size(egui::vec2(1024.0, 768.0));
    harness.step();
    harness
}

/// Click the widget labelled exactly `label`: hover one frame (see module
/// docs), then press and release across the following two frames — egui
/// fires `clicked` on the release. Generic over the app type so the same
/// helper drives both `CocoApp` and `manager::ManagerApp` harnesses.
pub(super) fn click<S: 'static>(harness: &mut egui_kittest::Harness<'static, S>, label: &str) {
    harness.get_by_label(label).hover();
    harness.step();
    harness.get_by_label(label).click();
    harness.step();
    harness.step();
}

/// [`click`] at a screen position rather than at a labelled widget — for
/// click targets that paint themselves and so never enter the
/// accessibility tree, like the status bar's keyboard icon
/// ([`crate::status_icons::keyboard_icon`]). Same hover-then-press-then-
/// release frame sequence [`click`] uses.
pub(super) fn click_at<S: 'static>(
    harness: &mut egui_kittest::Harness<'static, S>,
    pos: egui::Pos2,
) {
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(pos));
    harness.step();
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
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

/// [`click`] with `modifiers` held down for the press/release — Cmd/Ctrl-
/// click and Shift-click on a machine-list row (`egui_kittest::Node::
/// click_modifiers`, which both applies and then resets the modifiers, so
/// nothing leaks into whatever's clicked next).
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

/// [`click`] matching by substring — for widgets whose accessible label
/// carries decoration beyond the visible caption, like submenu buttons'
/// trailing arrow ("MultiPak Interface ⏵", "Slot 1 ⏵").
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

/// Select an item in a form combo box (Machine, Cartridge, …),
/// disambiguated by position: among all combo buttons currently showing
/// `current` as their value, open the `index`-th in top-to-bottom (then
/// left-to-right) screen order, then click the wanted item. The combo
/// button exposes the selected text as its accessibility *value* (egui
/// sets `WidgetInfo::current_text_value`, not a label), so it is addressed
/// with `get_by_value`; the popup items are plain selectables, addressed
/// by label. Positional because more than one combo can show the same
/// value at once (e.g. several drive combos on "None").
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

/// Lowest-on-screen widget labelled `label` — the open-menu copy of a label
/// the toolbar shows too ("Reset"): the menu popup hangs below the toolbar
/// row.
pub(super) fn lowest_by_label<'t>(
    harness: &'t AppHarness,
    label: &'t str,
) -> egui_kittest::Node<'t> {
    harness
        .get_all_by_label(label)
        .max_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y))
        .unwrap_or_else(|| panic!("no node labelled {label:?}"))
}

/// Topmost widget labelled `label` — e.g. the right stick's copy of a source
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

/// [`click`] via [`lowest_by_label`].
pub(super) fn click_in_menu(harness: &mut AppHarness, label: &str) {
    lowest_by_label(harness, label).hover();
    harness.step();
    lowest_by_label(harness, label).click();
    harness.step();
    harness.step();
}

/// A minimal valid entry: a CoCo 3 default config under `name`, built
/// through [`machine_def::MachineDef::from_config`] like the manager's own
/// "New…" flow does, so tests don't hand-roll a second copy of the DTO
/// shape.
pub(super) fn sample_entry(slug: &str, name: &str) -> manager::MachineEntry {
    manager::MachineEntry::new(
        slug.to_string(),
        machine_def::MachineDef::from_config(name.to_string(), None, &MachineConfig::default()),
    )
}

/// A minimal valid CoCo 2 `Ok` entry — [`sample_entry`]'s default is CoCo 3,
/// so pairing this with it gives two distinct machine families for
/// `starting_two_machines_runs_both` (`docs/plan-machine-persistence.md`
/// step 5's acceptance scenario: "a CoCo 3 and a newly created, launched
/// CoCo 2").
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
/// machines directory for Create/Save to write into — never the user's real
/// config dir.
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
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();
    harness
}

/// Whether `label` matches at least one accessible node — unlike
/// `get_by_label`/`query_by_label` (which require *at most* one match), used
/// where a status word like "Running" is deliberately shown twice at once
/// (the list row's `weak()` copy and the detail pane header's `strong()`
/// copy, both driven by `manager::vm_status_label`).
pub(super) fn label_exists<S: 'static>(
    harness: &egui_kittest::Harness<'static, S>,
    label: &str,
) -> bool {
    harness.get_all_by_label(label).next().is_some()
}
