use std::fs;
use std::time::Duration;

use super::*;
use crate::machine_def::tests::TempDir;
use crate::nitros9::EOU_IMAGES;

/// Stand-in bytes for the downloaded EOU images.
const FAKE_EOU_IMAGE: &[u8] = b"eou image";

/// The slug the template's first machine gets.
const EOU_SLUG: &str = "nitros-9-eou";

/// How often, and how many times, a test polls for the install's end.
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const POLL_ATTEMPTS: usize = 500;

/// Directories for a manager that can create the EOU template.
struct Fixture {
    machines: TempDir,
    artifacts: TempDir,
    eou: TempDir,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let eou = TempDir::new(&format!("{name}-eou"));
        for image in EOU_IMAGES {
            fs::write(eou.path().join(image), FAKE_EOU_IMAGE).unwrap();
        }
        Self {
            machines: TempDir::new(&format!("{name}-machines")),
            artifacts: TempDir::new(&format!("{name}-artifacts")),
            eou,
        }
    }

    fn app(&self) -> ManagerApp {
        let mut app = ManagerApp::new(
            None,
            Some(self.machines.path().to_path_buf()),
            Some(self.artifacts.path().to_path_buf()),
            Vec::new(),
            None,
        );
        app.eou_dir = Some(self.eou.path().to_path_buf());
        app
    }

    /// Whether the machine's definition and both image copies are on disk.
    fn installed(&self) -> bool {
        crate::machine_def::def_path(self.machines.path(), EOU_SLUG).is_file()
            && EOU_IMAGES
                .into_iter()
                .all(|image| self.artifacts.path().join(EOU_SLUG).join(image).is_file())
    }
}

/// Poll `app` until its install has ended.
fn wait_for_install(app: &mut ManagerApp, ctx: &egui::Context) {
    for _ in 0..POLL_ATTEMPTS {
        app.poll_eou_install(ctx);
        if app.eou_install.is_none() {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    panic!("the install must end");
}

/// Picking the template returns before the copy is done; polling inserts
/// and selects the machine once it is.
#[test]
fn template_installs_in_the_background() {
    let fixture = Fixture::new("eou-install-background");
    let mut app = fixture.app();
    let ctx = egui::Context::default();

    app.create_machine_from_preset(Preset::NitrOS9Eou);
    assert!(
        app.eou_install.is_some(),
        "the install runs on its own thread"
    );
    wait_for_install(&mut app, &ctx);

    assert_eq!(app.save_error, None);
    assert_eq!(app.entries.len(), 1);
    assert_eq!(app.entries[0].slug, EOU_SLUG);
    assert_eq!(app.selection.single(), Some(0));
    assert!(fixture.installed());
}

/// When the user selects something else while the copy runs, the new
/// machine is listed without taking the selection or the Name field.
#[test]
fn finished_install_leaves_a_changed_selection_alone() {
    let fixture = Fixture::new("eou-install-moved-on");
    let mut app = fixture.app();
    app.create_machine_from_preset(Preset::CoCo3DiskBasic);
    let ctx = egui::Context::default();

    app.create_machine_from_preset(Preset::NitrOS9Eou);
    app.selection.clear();
    app.focus_name = false;
    wait_for_install(&mut app, &ctx);

    assert_eq!(app.entries.len(), 2);
    assert!(app.entries.iter().any(|entry| entry.slug == EOU_SLUG));
    assert!(app.selection.is_empty());
    assert!(!app.focus_name);
}

/// A name being typed in the selected machine's Name field while the copy
/// runs keeps the selection and the draft.
#[test]
fn finished_install_keeps_a_name_draft() {
    const DRAFT: &str = "Half-typ";
    let fixture = Fixture::new("eou-install-draft");
    let mut app = fixture.app();
    app.create_machine_from_preset(Preset::CoCo3DiskBasic);
    let ctx = egui::Context::default();

    app.create_machine_from_preset(Preset::NitrOS9Eou);
    let selected = &app.entries[0];
    let mut edit = app.seed_edit(&selected.slug, &selected.def);
    edit.name = DRAFT.to_string();
    app.edit = Some(edit);
    wait_for_install(&mut app, &ctx);

    assert_eq!(app.entries.len(), 2);
    let selected = app.selection.single().expect("one machine stays selected");
    assert_eq!(app.entries[selected].slug, "coco-3-disk-basic");
    assert_eq!(
        app.edit.as_ref().map(|edit| edit.name.as_str()),
        Some(DRAFT)
    );
}

/// Quitting mid-install waits for it, so the machine is complete on disk
/// for the next launch.
#[test]
fn quitting_finishes_the_install() {
    let fixture = Fixture::new("eou-install-quit");
    let mut app = fixture.app();

    app.create_machine_from_preset(Preset::NitrOS9Eou);
    app.finish_eou_install();

    assert!(app.eou_install.is_none());
    assert!(fixture.installed());
}

/// A failed install reports its error and inserts nothing.
#[test]
fn failed_install_reports_its_error() {
    let fixture = Fixture::new("eou-install-failed");
    let mut app = fixture.app();
    let (tx, rx) = mpsc::channel();
    tx.send(Err("disk full".to_string())).unwrap();
    app.eou_install = Some(PendingInstall::new(EOU_SLUG.to_string(), rx));

    app.poll_eou_install(&egui::Context::default());

    assert!(app.eou_install.is_none());
    assert!(app.entries.is_empty());
    assert_eq!(app.save_error.as_deref(), Some("disk full"));
}

/// Two instances creating the template at once get different slugs: the
/// first claims its artifact dir before its copy starts, so the second
/// never copies into, or cleans up, the same files.
#[test]
fn concurrent_instances_claim_different_slugs() {
    let fixture = Fixture::new("eou-install-concurrent");
    let mut first = fixture.app();
    let mut second = fixture.app();
    let ctx = egui::Context::default();

    first.create_machine_from_preset(Preset::NitrOS9Eou);
    second.create_machine_from_preset(Preset::NitrOS9Eou);
    wait_for_install(&mut first, &ctx);
    wait_for_install(&mut second, &ctx);

    assert_eq!(first.entries[0].slug, EOU_SLUG);
    assert_eq!(second.entries[0].slug, format!("{EOU_SLUG}-2"));
    assert!(fixture.installed());
}

/// A slug whose artifact dir can't be created because something already
/// sits there, even where the slug check misses it, is skipped.
#[cfg(unix)]
#[test]
fn claim_skips_a_slug_already_present() {
    let fixture = Fixture::new("eou-install-dangling");
    let dangling = fixture.artifacts.path().join(EOU_SLUG);
    std::os::unix::fs::symlink(fixture.artifacts.path().join("gone"), &dangling).unwrap();
    let mut app = fixture.app();

    app.create_machine_from_preset(Preset::NitrOS9Eou);
    wait_for_install(&mut app, &egui::Context::default());

    assert_eq!(app.save_error, None);
    assert_eq!(app.entries[0].slug, format!("{EOU_SLUG}-2"));
    assert!(fs::symlink_metadata(&dangling).unwrap().is_symlink());
}

/// An install that fails after claiming its slug removes the claimed dir,
/// so the slug stays free.
#[test]
fn failed_install_releases_its_slug() {
    let fixture = Fixture::new("eou-install-release");
    let mut app = fixture.app();
    fs::remove_file(fixture.eou.path().join(EOU_IMAGES[0])).unwrap();

    app.create_machine_from_preset(Preset::NitrOS9Eou);
    wait_for_install(&mut app, &egui::Context::default());

    assert_eq!(app.save_error.as_deref(), Some(presets::EOU_NOT_DOWNLOADED));
    assert!(app.entries.is_empty());
    assert!(!fixture.artifacts.path().join(EOU_SLUG).exists());
}

/// A slug that can't be claimed for another reason than an existing dir
/// (here, the artifacts root is a file) is reported, and nothing starts.
#[test]
fn unclaimable_slug_reports_its_error() {
    let fixture = Fixture::new("eou-install-unclaimable");
    let not_a_dir = fixture.artifacts.path().join("file");
    fs::write(&not_a_dir, FAKE_EOU_IMAGE).unwrap();
    let mut app = fixture.app();
    app.artifacts_root = Some(not_a_dir.clone());

    app.create_machine_from_preset(Preset::NitrOS9Eou);

    assert!(app.eou_install.is_none());
    let error = app.save_error.expect("the claim error is reported");
    assert!(
        error.starts_with(&not_a_dir.display().to_string()),
        "{error}"
    );
}

/// An install whose thread ended without a result removes the dir it
/// claimed, so the slug stays free.
#[test]
fn exited_install_releases_its_slug() {
    let fixture = Fixture::new("eou-install-exited");
    let mut app = fixture.app();
    let artifact_dir = fixture.artifacts.path().join(EOU_SLUG);
    fs::create_dir(&artifact_dir).unwrap();
    fs::write(artifact_dir.join(EOU_IMAGES[0]), FAKE_EOU_IMAGE).unwrap();
    let (tx, rx) = mpsc::channel();
    drop(tx);
    app.eou_install = Some(PendingInstall::new(EOU_SLUG.to_string(), rx));

    app.poll_eou_install(&egui::Context::default());

    assert_eq!(app.save_error.as_deref(), Some(INSTALL_EXITED));
    assert!(!artifact_dir.exists());
}
