//! Templates from the user's side: a powered-off machine's context menu
//! saves it as a template, and the Settings dialog's Templates tab renames
//! and deletes templates on Save.

use egui_kittest::kittest::{NodeT, Queryable};

use crate::machine_def::tests::TempDir;
use crate::manager::presets::Preset;
use crate::manager::save_template::{SAVE_LABEL, SAVE_TEMPLATE_LABEL};
use crate::manager::settings::templates::{DELETE_LABEL, TEMPLATES_TAB_LABEL, UNDO_LABEL};
use crate::manager::templates::tests::TemplateFixture;

use super::harness::*;
use super::manager_settings::settings_harness_with;

/// Save as Template on a machine's context menu opens the dialog, and its
/// Save lists the new template under the machine's name.
#[test]
fn context_menu_saves_a_machine_as_a_template() {
    let templates = TemplateFixture::new("ui-save-template");
    let machines = TempDir::new("ui-save-template-machines");
    let mut harness = manager_harness(
        Some(machines.path().to_path_buf()),
        vec![sample_entry("alpha", "Alpha CoCo 3")],
    );
    harness.state_mut().templates = templates.state();

    right_click(&mut harness, "Alpha CoCo 3");
    click(&mut harness, SAVE_TEMPLATE_LABEL);
    harness.get_by_label("Save “Alpha CoCo 3” as a Template");
    click(&mut harness, SAVE_LABEL);

    assert!(
        harness.query_by_label(SAVE_LABEL).is_none(),
        "the dialog closes"
    );
    let state = harness.state();
    let template = state.templates.find("alpha-coco-3").expect("listed");
    assert_eq!(template.name(), "Alpha CoCo 3");
}

/// A suspended machine can't be saved as a template.
#[test]
fn context_menu_item_is_off_for_a_suspended_machine() {
    let mut entry = sample_entry("alpha", "Alpha CoCo 3");
    entry.suspended = true;
    let mut harness = manager_harness(None, vec![entry]);

    right_click(&mut harness, "Alpha CoCo 3");

    let item = harness.get_by_label(SAVE_TEMPLATE_LABEL);
    assert!(item.accesskit_node().is_disabled());
}

/// A settings harness over the seeded `templates`.
fn templates_settings(dir: &TempDir, templates: &TemplateFixture) -> ManagerHarness {
    let state = templates.state();
    let mut state = Some(state);
    let mut harness = settings_harness_with(dir.path().join("config.toml"), move |app| {
        app.templates = state.take().expect("set once");
    });
    click(&mut harness, "Settings");
    click(&mut harness, TEMPLATES_TAB_LABEL);
    harness
}

/// Delete marks a template, and Save deletes it.
#[test]
fn settings_delete_applies_on_save() {
    let templates = TemplateFixture::seeded("ui-settings-delete");
    let dir = TempDir::new("ui-settings-delete-config");
    let mut harness = templates_settings(&dir, &templates);
    harness.get_by_label(Preset::CoCo2DiskBasic.description());

    harness
        .get_all_by_label(DELETE_LABEL)
        .next()
        .unwrap()
        .click();
    harness.run_steps(2);
    harness.get_by_label(UNDO_LABEL);
    click(&mut harness, "Save");

    assert!(harness.state().settings.is_none(), "the dialog closes");
    let names: Vec<String> = harness
        .state()
        .templates
        .list
        .iter()
        .map(|template| template.name().to_string())
        .collect();
    assert!(
        !names.contains(&Preset::CoCo2DiskBasic.name().to_string()),
        "{names:?}"
    );
    assert!(!templates.media_dir("coco-2-disk-basic").exists());
}

/// Cancel drops a marked delete: the template stays.
#[test]
fn settings_cancel_keeps_a_marked_template() {
    let templates = TemplateFixture::seeded("ui-settings-cancel");
    let dir = TempDir::new("ui-settings-cancel-config");
    let mut harness = templates_settings(&dir, &templates);

    harness
        .get_all_by_label(DELETE_LABEL)
        .next()
        .unwrap()
        .click();
    harness.run_steps(2);
    click(&mut harness, "Cancel");

    assert_eq!(harness.state().templates.list.len(), Preset::ALL.len());
    assert!(templates.media_dir("coco-2-disk-basic").exists());
}
