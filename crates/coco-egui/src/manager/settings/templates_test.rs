use super::*;
use crate::machine_def::tests::TempDir;
use crate::manager::MachineEntry;
use crate::manager::templates::tests::{EOU_TEMPLATE, TemplateFixture, sample_template};

/// A draft over templates named `names`, slugged by position.
fn draft(names: &[&str]) -> TemplatesDraft {
    let templates: Vec<Template> = names
        .iter()
        .enumerate()
        .map(|(index, name)| sample_template(&format!("t{index}"), name))
        .collect();
    TemplatesDraft::new(&templates)
}

/// An untouched draft has nothing to apply.
#[test]
fn untouched_draft_has_no_edits() {
    assert_eq!(
        draft(&["A", "B"]).edits().unwrap(),
        TemplateEdits::default()
    );
}

/// A changed name is a rename (trimmed), a marked row a delete.
#[test]
fn edits_collect_renames_and_deletes() {
    let mut draft = draft(&["A", "B", "C"]);
    draft.rows[0].name = " Alpha ".to_string();
    draft.rows[1].delete = true;

    let edits = draft.edits().unwrap();

    assert_eq!(edits.renames, [("t0".to_string(), "Alpha".to_string())]);
    assert_eq!(edits.deletes, ["t1"]);
}

/// An emptied name is refused, unless the row is being deleted.
#[test]
fn empty_name_is_refused_unless_deleted() {
    let mut draft = draft(&["A"]);
    draft.rows[0].name = "  ".to_string();
    assert_eq!(draft.edits().unwrap_err(), EMPTY_NAME_ERROR);
    draft.rows[0].delete = true;
    assert_eq!(draft.edits().unwrap().deletes, ["t0"]);
}

/// Two kept templates can't share a name, ignoring case; a deleted one
/// gives its name up.
#[test]
fn duplicate_names_are_refused() {
    let mut draft = draft(&["A", "B"]);
    draft.rows[1].name = "a".to_string();
    assert!(draft.edits().unwrap_err().contains("“a”"));
    draft.rows[0].delete = true;
    assert!(draft.edits().is_ok());
}

/// After a save, gone templates leave the draft, and a failed rename
/// stays pending.
#[test]
fn sync_keeps_what_failed() {
    let mut draft = draft(&["A", "B"]);
    draft.rows[0].delete = true;
    draft.rows[1].name = "Beta".to_string();

    draft.sync(&[sample_template("t1", "B")]);

    assert_eq!(draft.rows.len(), 1);
    assert_eq!(
        draft.edits().unwrap().renames,
        [("t1".to_string(), "Beta".to_string())]
    );
}

/// A manager over the seeded templates.
fn app(templates: &TemplateFixture, artifacts: &TempDir) -> ManagerApp {
    let mut app = ManagerApp::new(
        None,
        None,
        Some(artifacts.path().to_path_buf()),
        Vec::new(),
        None,
    );
    app.templates = templates.state();
    app
}

/// Applying renames a template in its file, keeping its slug, and deletes
/// another with its media.
#[test]
fn apply_renames_and_deletes() {
    let templates = TemplateFixture::seeded("settings-templates-apply");
    let artifacts = TempDir::new("settings-templates-apply-artifacts");
    let mut app = app(&templates, &artifacts);
    let edits = TemplateEdits {
        renames: vec![("coco-2-disk-basic".to_string(), "My CoCo 2".to_string())],
        deletes: vec!["coco-3-disk-basic".to_string()],
    };

    app.apply_template_edits(&edits).unwrap();

    let renamed = app.templates.find("coco-2-disk-basic").expect("kept");
    assert_eq!(renamed.name(), "My CoCo 2");
    assert!(app.templates.find("coco-3-disk-basic").is_none());
    assert!(!templates.media_dir("coco-3-disk-basic").exists());
}

/// A delete refused because a machine uses a file in the template's folder
/// is reported, and the other edits still apply.
#[test]
fn refused_delete_leaves_the_rest_applied() {
    let templates = TemplateFixture::seeded("settings-templates-refused");
    let artifacts = TempDir::new("settings-templates-refused-artifacts");
    let mut app = app(&templates, &artifacts);
    let def = crate::machine_def::MachineDef::from_config(
        "User CoCo".to_string(),
        None,
        &coco_core::MachineConfig::default(),
    );
    let mut entry = MachineEntry::new("user".to_string(), def);
    let used = templates.media_dir("coco-3-disk-basic").join("disk0.dsk");
    entry.def.media.disk0 = Some(used.display().to_string());
    app.entries.push(entry);
    let edits = TemplateEdits {
        renames: vec![("coco-2-disk-basic".to_string(), "My CoCo 2".to_string())],
        deletes: vec![
            "coco-3-disk-basic".to_string(),
            "coco-3-drivewire-hdb-dos".to_string(),
        ],
    };

    let error = app.apply_template_edits(&edits).unwrap_err();

    assert!(error.contains("“User CoCo”"), "{error}");
    assert!(app.templates.find("coco-3-disk-basic").is_some());
    assert!(app.templates.find("coco-3-drivewire-hdb-dos").is_none());
    assert_eq!(
        app.templates.find("coco-2-disk-basic").unwrap().name(),
        "My CoCo 2"
    );
}

/// The machine a deleted template made keeps its own copy of the media.
#[test]
fn deleting_a_template_keeps_its_machines() {
    let templates = TemplateFixture::seeded("settings-templates-machines");
    let artifacts = TempDir::new("settings-templates-machines-artifacts");
    let machines = TempDir::new("settings-templates-machines-defs");
    let mut app = app(&templates, &artifacts);
    app.machines_dir = Some(machines.path().to_path_buf());
    app.create_machine_from_template("coco-3-disk-basic");
    let edits = TemplateEdits {
        deletes: vec!["coco-3-disk-basic".to_string()],
        ..TemplateEdits::default()
    };

    app.apply_template_edits(&edits).unwrap();

    assert!(
        app.entries
            .iter()
            .any(|entry| entry.slug == "coco-3-disk-basic")
    );
    assert!(
        artifacts
            .path()
            .join("coco-3-disk-basic/disk0.dsk")
            .is_file()
    );
}

/// A template a machine is being copied from, or whose EOU images are
/// downloading into its folder, can't be deleted meanwhile: the copy or
/// the download would fail, or refill a folder nothing lists.
#[test]
fn busy_template_is_not_deleted() {
    use crate::manager::asset_downloads::{Download, Source};
    use crate::manager::template_install::PendingInstall;

    let templates = TemplateFixture::seeded("settings-templates-busy");
    let artifacts = TempDir::new("settings-templates-busy-artifacts");
    let mut app = app(&templates, &artifacts);
    let (_install_tx, install_rx) = std::sync::mpsc::channel();
    let installing = "coco-3-disk-basic".to_string();
    let pending = PendingInstall::new("new-machine".to_string(), installing.clone(), install_rx);
    app.templates.installs.push(pending);
    let (_download_tx, download_rx) = std::sync::mpsc::channel();
    app.eou_download = Some(Download::running(
        Source::Eou,
        std::sync::Arc::default(),
        download_rx,
    ));
    app.eou_download_dir = Some(templates.media_dir(EOU_TEMPLATE));
    let edits = TemplateEdits {
        deletes: vec![installing, EOU_TEMPLATE.to_string()],
        ..TemplateEdits::default()
    };

    let error = app.apply_template_edits(&edits).unwrap_err();

    assert!(error.contains("copied from it"), "{error}");
    assert!(error.contains("download"), "{error}");
    assert!(app.templates.find("coco-3-disk-basic").is_some());
    assert!(app.templates.find(EOU_TEMPLATE).is_some());
}
