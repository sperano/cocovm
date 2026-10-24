use std::fs;
use std::time::Duration;

use super::*;
use crate::machine_def::tests::TempDir;
use crate::manager::presets::{BLANK_DISK_DRIVE, Preset};
use crate::manager::templates::tests::{EOU_TEMPLATE, TemplateFixture};
use crate::nitros9::EOU_IMAGES;

/// The slug the EOU template's first machine gets.
const EOU_SLUG: &str = "nitros-9-eou";

/// The seeded CoCo 3 Disk BASIC template.
const DISK_BASIC: &str = "coco-3-disk-basic";

/// The seeded DriveWire template, which names no media.
const DRIVEWIRE: &str = "coco-3-drivewire-hdb-dos";

/// Each EOU stand-in image's size: together just past the background
/// threshold, so creating the template copies on a thread.
const LARGE_IMAGE_BYTES: u64 = media::BACKGROUND_COPY_BYTES / 2 + 1;

/// How often, and how many times, a test polls for the install's end.
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const POLL_ATTEMPTS: usize = 500;

/// Seeded templates, with large EOU images, and a manager's machine
/// directories.
struct Fixture {
    templates: TemplateFixture,
    machines: TempDir,
    artifacts: TempDir,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let templates = TemplateFixture::seeded(name);
        templates.fill_eou(LARGE_IMAGE_BYTES);
        Self {
            templates,
            machines: TempDir::new(&format!("{name}-machines")),
            artifacts: TempDir::new(&format!("{name}-artifacts")),
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
        app.templates = self.templates.state();
        app
    }

    /// Whether the EOU machine's definition and both image copies are on disk.
    fn installed(&self) -> bool {
        crate::machine_def::def_path(self.machines.path(), EOU_SLUG).is_file()
            && EOU_IMAGES
                .into_iter()
                .all(|image| self.artifacts.path().join(EOU_SLUG).join(image).is_file())
    }
}

/// Poll `app` until its installs have ended.
fn wait_for_install(app: &mut ManagerApp, ctx: &egui::Context) {
    for _ in 0..POLL_ATTEMPTS {
        app.poll_template_installs(ctx);
        if app.templates.installs.is_empty() {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    panic!("the install must end");
}

/// A template's blank disk is copied at once: the machine is inserted,
/// selected, and named after the template, with its own disk.
#[test]
fn small_template_installs_at_once() {
    let fixture = Fixture::new("install-small");
    let mut app = fixture.app();

    app.create_machine_from_template(DISK_BASIC);

    assert!(app.templates.installs.is_empty());
    assert_eq!(app.save_error, None);
    assert_eq!(app.entries.len(), 1);
    assert_eq!(app.entries[0].slug, DISK_BASIC);
    assert_eq!(app.entries[0].def.name, Preset::CoCo3DiskBasic.name());
    assert_eq!(app.selection.single(), Some(0));
    let disk = fixture.artifacts.path().join(DISK_BASIC).join("disk0.dsk");
    assert!(disk.is_file());
}

/// A template naming no media makes a machine without an artifact dir.
#[test]
fn template_without_media_writes_only_the_definition() {
    let fixture = Fixture::new("install-no-media");
    let mut app = fixture.app();

    app.create_machine_from_template(DRIVEWIRE);

    assert_eq!(app.save_error, None);
    assert_eq!(app.entries[0].slug, DRIVEWIRE);
    assert!(!fixture.artifacts.path().join(DRIVEWIRE).exists());
}

/// Picking the same template twice makes two independent machines under
/// distinct slugs, each with its own blank floppy.
#[test]
fn creating_a_template_twice_gives_each_copy_its_own_slug() {
    let fixture = Fixture::new("install-twice");
    let mut app = fixture.app();

    app.create_machine_from_template(DISK_BASIC);
    app.create_machine_from_template(DISK_BASIC);

    let mut slugs: Vec<&str> = app.entries.iter().map(|e| e.slug.as_str()).collect();
    slugs.sort_unstable();
    assert_eq!(slugs, [DISK_BASIC, "coco-3-disk-basic-2"]);
    for slug in slugs {
        assert!(
            fixture
                .artifacts
                .path()
                .join(slug)
                .join("disk0.dsk")
                .is_file()
        );
    }
}

/// A slug whose artifact dir outlived its machine is not reused: the new
/// machine's blank disk must not be the old machine's disk.
#[test]
fn new_machine_skips_a_slug_with_leftover_artifacts() {
    let fixture = Fixture::new("install-leftover");
    let old_disk = fixture.artifacts.path().join(DISK_BASIC).join("disk0.dsk");
    fs::create_dir_all(old_disk.parent().unwrap()).unwrap();
    fs::write(&old_disk, b"old machine's files").unwrap();
    let mut app = fixture.app();

    app.create_machine_from_template(DISK_BASIC);

    assert_eq!(app.entries[0].slug, "coco-3-disk-basic-2");
    let new_disk = fixture
        .artifacts
        .path()
        .join("coco-3-disk-basic-2")
        .join("disk0.dsk");
    assert_eq!(fs::metadata(new_disk).unwrap().len(), 0);
}

/// A template with media needs the artifacts root; without one nothing
/// is written.
#[test]
fn template_media_needs_an_artifacts_root() {
    let fixture = Fixture::new("install-no-root");
    let mut app = fixture.app();
    app.artifacts_root = None;

    app.create_machine_from_template(DISK_BASIC);

    assert_eq!(app.save_error.as_deref(), Some(NO_DATA_DIR));
    assert!(app.entries.is_empty());
    assert!(!crate::machine_def::def_path(fixture.machines.path(), DISK_BASIC).exists());
}

/// A template deleted since the dialog listed it is reported.
#[test]
fn vanished_template_is_reported() {
    let fixture = Fixture::new("install-gone");
    let mut app = fixture.app();

    app.create_machine_from_template("no-such-template");

    assert_eq!(app.save_error.as_deref(), Some(TEMPLATE_GONE));
}

/// Picking a large template returns before the copy is done; polling
/// inserts and selects the machine once it is, and the template's own
/// images stay.
#[test]
fn large_template_installs_in_the_background() {
    let fixture = Fixture::new("install-background");
    let mut app = fixture.app();
    let ctx = egui::Context::default();

    app.create_machine_from_template(EOU_TEMPLATE);
    assert!(app.template_installing(EOU_TEMPLATE));
    assert!(app.installing_slug(EOU_SLUG));
    wait_for_install(&mut app, &ctx);

    assert_eq!(app.save_error, None);
    assert_eq!(app.entries.len(), 1);
    assert_eq!(app.entries[0].slug, EOU_SLUG);
    assert_eq!(app.selection.single(), Some(0));
    assert!(fixture.installed());
    for image in EOU_IMAGES {
        assert!(
            fixture
                .templates
                .media_dir(EOU_TEMPLATE)
                .join(image)
                .is_file()
        );
    }
}

/// When the user selects something else while the copy runs, the new
/// machine is listed without taking the selection or the Name field.
#[test]
fn finished_install_leaves_a_changed_selection_alone() {
    let fixture = Fixture::new("install-moved-on");
    let mut app = fixture.app();
    app.create_machine_from_template(DISK_BASIC);
    let ctx = egui::Context::default();

    app.create_machine_from_template(EOU_TEMPLATE);
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
    let fixture = Fixture::new("install-draft");
    let mut app = fixture.app();
    app.create_machine_from_template(DISK_BASIC);
    let ctx = egui::Context::default();

    app.create_machine_from_template(EOU_TEMPLATE);
    let selected = &app.entries[0];
    let mut edit = app.seed_edit(&selected.slug, &selected.def);
    edit.name = DRAFT.to_string();
    app.edit = Some(edit);
    wait_for_install(&mut app, &ctx);

    assert_eq!(app.entries.len(), 2);
    let selected = app.selection.single().expect("one machine stays selected");
    assert_eq!(app.entries[selected].slug, DISK_BASIC);
    assert_eq!(
        app.edit.as_ref().map(|edit| edit.name.as_str()),
        Some(DRAFT)
    );
}

/// Quitting mid-install waits for it, so the machine is complete on disk
/// for the next launch.
#[test]
fn quitting_finishes_the_install() {
    let fixture = Fixture::new("install-quit");
    let mut app = fixture.app();

    app.create_machine_from_template(EOU_TEMPLATE);
    app.finish_template_installs();

    assert!(app.templates.installs.is_empty());
    assert!(fixture.installed());
}

/// A failed install reports its error and inserts nothing.
#[test]
fn failed_install_reports_its_error() {
    let fixture = Fixture::new("install-failed");
    let mut app = fixture.app();
    let (tx, rx) = mpsc::channel();
    tx.send(Err("disk full".to_string())).unwrap();
    let pending = PendingInstall::new(EOU_SLUG.to_string(), EOU_TEMPLATE.to_string(), rx);
    app.templates.installs.push(pending);

    app.poll_template_installs(&egui::Context::default());

    assert!(app.templates.installs.is_empty());
    assert!(app.entries.is_empty());
    assert_eq!(app.save_error.as_deref(), Some("disk full"));
}

/// Two instances creating the template at once get different slugs: the
/// first claims its artifact dir before its copy starts, so the second
/// never copies into, or cleans up, the same files.
#[test]
fn concurrent_instances_claim_different_slugs() {
    let fixture = Fixture::new("install-concurrent");
    let mut first = fixture.app();
    let mut second = fixture.app();
    let ctx = egui::Context::default();

    first.create_machine_from_template(EOU_TEMPLATE);
    second.create_machine_from_template(EOU_TEMPLATE);
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
    let fixture = Fixture::new("install-dangling");
    let dangling = fixture.artifacts.path().join(EOU_SLUG);
    std::os::unix::fs::symlink(fixture.artifacts.path().join("gone"), &dangling).unwrap();
    let mut app = fixture.app();

    app.create_machine_from_template(EOU_TEMPLATE);
    wait_for_install(&mut app, &egui::Context::default());

    assert_eq!(app.save_error, None);
    assert_eq!(app.entries[0].slug, format!("{EOU_SLUG}-2"));
    assert!(fs::symlink_metadata(&dangling).unwrap().is_symlink());
}

/// An install that fails after claiming its slug removes the claimed dir,
/// so the slug stays free.
#[test]
fn failed_install_releases_its_slug() {
    let fixture = Fixture::new("install-release");
    let mut app = fixture.app();
    let media_dir = fixture.templates.media_dir(EOU_TEMPLATE);
    // Large enough on its own to copy on a thread, which then finds the
    // boot disk gone.
    fs::File::create(media_dir.join(EOU_IMAGES[1]))
        .and_then(|file| file.set_len(media::BACKGROUND_COPY_BYTES + 1))
        .unwrap();
    fs::remove_file(media_dir.join(EOU_IMAGES[0])).unwrap();

    app.create_machine_from_template(EOU_TEMPLATE);
    wait_for_install(&mut app, &egui::Context::default());

    let error = app.save_error.clone().expect("the copy fails");
    assert!(error.contains(EOU_IMAGES[0]), "{error}");
    assert!(app.entries.is_empty());
    assert!(!fixture.artifacts.path().join(EOU_SLUG).exists());
}

/// A slug that can't be claimed for another reason than an existing dir
/// (here, the artifacts root is a file) is reported, and nothing starts.
#[test]
fn unclaimable_slug_reports_its_error() {
    let fixture = Fixture::new("install-unclaimable");
    let not_a_dir = fixture.artifacts.path().join("file");
    fs::write(&not_a_dir, b"not a directory").unwrap();
    let mut app = fixture.app();
    app.artifacts_root = Some(not_a_dir.clone());

    app.create_machine_from_template(EOU_TEMPLATE);

    assert!(app.templates.installs.is_empty());
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
    let fixture = Fixture::new("install-exited");
    let mut app = fixture.app();
    let artifact_dir = fixture.artifacts.path().join(EOU_SLUG);
    fs::create_dir(&artifact_dir).unwrap();
    fs::write(artifact_dir.join(EOU_IMAGES[0]), b"partial").unwrap();
    let (tx, rx) = mpsc::channel();
    drop(tx);
    let pending = PendingInstall::new(EOU_SLUG.to_string(), EOU_TEMPLATE.to_string(), rx);
    app.templates.installs.push(pending);

    app.poll_template_installs(&egui::Context::default());

    assert_eq!(app.save_error.as_deref(), Some(INSTALL_EXITED));
    assert!(!artifact_dir.exists());
}

/// Every built-in template boots through the production launch path with
/// the installed ROMs: its DOS ROM loads and its media mount. The EOU
/// template gets the test bundle's EOU images. Relative media are pointed
/// at the copies, since launch resolves them against the real data
/// directory.
#[test]
fn every_builtin_template_launches() {
    let fixture = Fixture::new("install-launch");
    let eou_dir = test_assets::disk(test_assets::disk::EOU_BOOT)
        .parent()
        .expect("test disks live in a directory")
        .to_path_buf();
    let eou_present = crate::nitros9::missing_eou_images(&eou_dir).is_empty();
    for name in EOU_IMAGES {
        let target = fixture.templates.media_dir(EOU_TEMPLATE).join(name);
        if eou_present {
            fs::copy(eou_dir.join(name), target).unwrap();
        }
    }
    let mut app = fixture.app();
    for template in app.templates.list.clone() {
        if template.slug == EOU_TEMPLATE && !eou_present {
            eprintln!(
                "skipping {}: the EOU test images are not present",
                template.slug
            );
            continue;
        }
        app.create_machine_from_template(&template.slug);
        wait_for_install(&mut app, &egui::Context::default());
        assert_eq!(app.save_error, None, "{}", template.slug);
        let entry = app
            .entries
            .iter()
            .find(|entry| entry.def.name == template.name())
            .expect("the machine is listed");
        let mut def = entry.def.clone();
        for media in [&mut def.media.disk0, &mut def.media.vhd0]
            .into_iter()
            .flatten()
        {
            let copy = fixture.artifacts.path().join(&entry.slug).join(&*media);
            assert!(
                copy.is_file(),
                "{}: {} must be installed",
                template.slug,
                copy.display()
            );
            *media = copy.display().to_string();
        }

        let mut vm = crate::launch::launch_machine(&def, &entry.slug)
            .unwrap_or_else(|e| panic!("{} must launch: {e}", template.slug));
        assert!(
            vm.machine.bus.cart.as_disk_cart().is_some(),
            "{} must have its FD-502 installed",
            template.slug
        );
        assert_eq!(
            vm.disk_paths[BLANK_DISK_DRIVE].is_some(),
            def.media.disk0.is_some(),
            "{}: drive 0 mount",
            template.slug
        );
        assert_eq!(
            vm.vhd_paths[0].is_some(),
            def.media.vhd0.is_some(),
            "{}: VHD mount",
            template.slug
        );
    }
}
