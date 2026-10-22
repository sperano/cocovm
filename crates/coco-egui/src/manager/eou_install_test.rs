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
    for _ in 0..POLL_ATTEMPTS {
        app.poll_eou_install(&ctx);
        if app.eou_install.is_none() {
            break;
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    assert_eq!(app.save_error, None);
    assert_eq!(app.entries.len(), 1);
    assert_eq!(app.entries[0].slug, EOU_SLUG);
    assert_eq!(app.selection.single(), Some(0));
    assert!(fixture.installed());
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
