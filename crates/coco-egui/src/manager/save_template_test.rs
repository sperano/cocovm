use std::fs;
use std::time::Duration;

use super::*;
use crate::machine_def::StatsDTO;
use crate::machine_def::tests::TempDir;
use crate::manager::MachineEntry;
use crate::manager::presets::Preset;
use crate::manager::templates::tests::TemplateFixture;

/// The machine the tests save.
const SLUG: &str = "games";
const NAME: &str = "Games CoCo";

/// How often, and how many times, a test polls for a background save.
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const POLL_ATTEMPTS: usize = 500;

/// A manager with one Disk BASIC machine whose disk holds `disk` bytes,
/// and template directories holding `templates`.
struct Fixture {
    templates: TemplateFixture,
    machines: TempDir,
    artifacts: TempDir,
}

impl Fixture {
    fn new(name: &str, templates: TemplateFixture, disk: &[u8]) -> Self {
        let fixture = Self {
            templates,
            machines: TempDir::new(&format!("{name}-machines")),
            artifacts: TempDir::new(&format!("{name}-artifacts")),
        };
        fs::create_dir_all(fixture.artifact_dir()).unwrap();
        fs::write(fixture.artifact_dir().join("disk0.dsk"), disk).unwrap();
        fixture
    }

    fn artifact_dir(&self) -> PathBuf {
        self.artifacts.path().join(SLUG)
    }

    fn app(&self) -> ManagerApp {
        let mut def = Preset::CoCo3DiskBasic.def(Some("2020-01-01".to_string()));
        def.name = NAME.to_string();
        def.stats.starts = 3;
        machine_def::save(self.machines.path(), SLUG, &def).unwrap();
        let mut app = ManagerApp::new(
            None,
            Some(self.machines.path().to_path_buf()),
            Some(self.artifacts.path().to_path_buf()),
            vec![MachineEntry::new(SLUG.to_string(), def)],
            None,
        );
        app.templates = self.templates.state();
        app
    }
}

/// Make the machine's disk (sparsely) large enough to copy on a thread.
fn grow_disk(fixture: &Fixture) {
    fs::OpenOptions::new()
        .write(true)
        .open(fixture.artifact_dir().join("disk0.dsk"))
        .and_then(|file| file.set_len(media::BACKGROUND_COPY_BYTES + 1))
        .unwrap();
}

/// Open the dialog for [`SLUG`] on `app`, as the context menu does.
fn open(app: &mut ManagerApp) -> SaveTemplateDialog {
    app.request_save_template(SLUG.to_string());
    app.open_requested_save_template();
    app.templates.save_dialog.take().expect("the dialog opens")
}

/// Saving writes a template named and described as typed, with a copy of
/// the machine's disk, and leaves the machine alone.
#[test]
fn save_writes_the_template_and_copies_the_disk() {
    let fixture = Fixture::new(
        "save-template",
        TemplateFixture::new("save-template"),
        b"data",
    );
    let mut app = fixture.app();
    let mut dialog = open(&mut app);
    assert_eq!(dialog.name, NAME);
    dialog.name = "  Game Night ".to_string();
    dialog.description = "Ready to play".to_string();

    assert!(app.commit_save_template(&mut dialog));

    let template = app
        .templates
        .find("game-night")
        .expect("the template is listed");
    assert_eq!(template.name(), "Game Night");
    assert_eq!(template.description, "Ready to play");
    assert_eq!(template.def.stats, StatsDTO::default());
    assert_eq!(template.def.created, Some(created_today()));
    let copy = fixture.templates.media_dir("game-night").join("disk0.dsk");
    assert_eq!(fs::read(copy).unwrap(), b"data");
    assert_eq!(app.entries[0].def.name, NAME);
    assert_eq!(app.entries[0].def.stats.starts, 3);
    assert!(fixture.artifact_dir().join("disk0.dsk").is_file());
}

/// A name another template has (ignoring case) replaces that template:
/// the new one is written, and the old one, file and media, goes.
#[test]
fn save_over_a_name_replaces_the_template() {
    let templates = TemplateFixture::seeded("save-template-replace");
    let fixture = Fixture::new("save-template-replace", templates, b"mine");
    let mut app = fixture.app();
    let mut dialog = open(&mut app);
    dialog.name = "coco 3 disk basic".to_string();
    assert_eq!(
        dialog.clash(&app.templates.list).map(|t| t.slug.as_str()),
        Some("coco-3-disk-basic")
    );

    assert!(app.commit_save_template(&mut dialog));

    assert!(app.templates.find("coco-3-disk-basic").is_none());
    assert!(!fixture.templates.media_dir("coco-3-disk-basic").exists());
    let replacement = app.templates.find("coco-3-disk-basic-2").expect("listed");
    assert_eq!(replacement.name(), "coco 3 disk basic");
    let copy = fixture
        .templates
        .media_dir("coco-3-disk-basic-2")
        .join("disk0.dsk");
    assert_eq!(fs::read(copy).unwrap(), b"mine");
}

/// Replace is refused while a machine uses a file in the replaced
/// template's folder, and nothing is written.
#[test]
fn replace_is_refused_while_a_machine_uses_the_template() {
    let templates = TemplateFixture::seeded("save-template-in-use");
    let used = templates.media_dir("coco-3-disk-basic").join("disk0.dsk");
    let fixture = Fixture::new("save-template-in-use", templates, b"mine");
    let mut app = fixture.app();
    app.entries[0].def.media.disk1 = Some(used.display().to_string());
    let mut dialog = open(&mut app);
    dialog.name = Preset::CoCo3DiskBasic.name().to_string();

    assert!(!app.commit_save_template(&mut dialog));

    let error = dialog.error.expect("the refusal is shown");
    assert!(error.contains(NAME), "{error}");
    assert!(app.templates.find("coco-3-disk-basic").is_some());
    assert!(!fixture.templates.media_dir("coco-3-disk-basic-2").exists());
}

/// A machine that runs or is suspended can't be saved: the request is
/// dropped.
#[test]
fn live_machine_opens_no_dialog() {
    let fixture = Fixture::new(
        "save-template-live",
        TemplateFixture::new("save-template-live"),
        b"",
    );
    let mut app = fixture.app();
    app.entries[0].suspended = true;

    app.request_save_template(SLUG.to_string());
    app.open_requested_save_template();

    assert!(app.templates.save_dialog.is_none());
    assert!(app.templates.save_request.is_none());
}

/// A Name-field draft is committed before the dialog opens, renaming the
/// machine, and the dialog starts with that name.
#[test]
fn name_draft_is_saved_first() {
    let fixture = Fixture::new(
        "save-template-draft",
        TemplateFixture::new("save-template-draft"),
        b"",
    );
    let mut app = fixture.app();
    let mut edit = app.seed_edit(SLUG, &app.entries[0].def);
    edit.name = "Renamed CoCo".to_string();
    app.edit = Some(edit);

    let dialog = open(&mut app);

    assert_eq!(dialog.name, "Renamed CoCo");
    assert_eq!(app.entries[0].slug, "renamed-coco");
    assert_eq!(
        dialog.artifact_dir,
        Some(fixture.artifacts.path().join("renamed-coco"))
    );
}

/// A file the machine names but lacks is listed as missing, which keeps
/// Save off.
#[test]
fn missing_file_is_listed() {
    let fixture = Fixture::new(
        "save-template-missing",
        TemplateFixture::new("save-template-missing"),
        b"",
    );
    fs::remove_file(fixture.artifact_dir().join("disk0.dsk")).unwrap();
    let mut app = fixture.app();

    let dialog = open(&mut app);

    assert_eq!(
        dialog.files,
        [FileLine {
            path: "disk0.dsk".to_string(),
            fate: FileFate::Missing
        }]
    );
}

/// A large copy runs on its own thread; the dialog stays open meanwhile
/// and closes once the template is written.
#[test]
fn large_save_runs_in_the_background() {
    let fixture = Fixture::new(
        "save-template-large",
        TemplateFixture::new("save-template-large"),
        b"",
    );
    grow_disk(&fixture);
    let mut app = fixture.app();
    let mut dialog = open(&mut app);
    let ctx = egui::Context::default();

    assert!(!app.commit_save_template(&mut dialog));
    assert!(dialog.pending.is_some());
    let mut done = false;
    for _ in 0..POLL_ATTEMPTS {
        if app.poll_save_template(&ctx, &mut dialog) {
            done = true;
            break;
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    assert!(done, "the save must end");
    assert_eq!(dialog.error, None);
    assert!(app.templates.find("games-coco").is_some());
}

/// Quitting while a save copies waits for it.
#[test]
fn quitting_finishes_the_save() {
    let fixture = Fixture::new(
        "save-template-quit",
        TemplateFixture::new("save-template-quit"),
        b"",
    );
    grow_disk(&fixture);
    let mut app = fixture.app();
    let mut dialog = open(&mut app);
    assert!(!app.commit_save_template(&mut dialog));
    app.templates.save_dialog = Some(dialog);

    app.finish_template_save();

    let defs = fixture.templates.dirs().defs.unwrap();
    assert!(machine_def::def_path(&defs, "games-coco").is_file());
}
