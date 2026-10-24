use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;
use crate::download::Progress;
use crate::machine_def::tests::TempDir;

/// A URL whose download fails at once, without reaching the network.
const REFUSED_URL: &str = "http://127.0.0.1:1/eou.zip";

/// How often, and how many times, the retry test polls for its download's end.
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const POLL_ATTEMPTS: usize = 500;

/// The entry's download button label.
fn download_label() -> String {
    format!("Download {EOU_TITLE} ({})", megabytes(EOU_ARCHIVE_BYTES))
}

/// `app`'s EOU entry for a template whose media folder is `media_dir`,
/// drawn on its own.
fn entry_harness(app: ManagerApp, media_dir: Option<&Path>) -> Harness<'static, ManagerApp> {
    let media_dir = media_dir.map(Path::to_path_buf);
    let mut harness = Harness::new_ui_state(
        move |ui, app: &mut ManagerApp| app.eou_download_ui(ui, media_dir.as_deref()),
        app,
    );
    harness.run_steps(2);
    harness
}

fn app() -> ManagerApp {
    ManagerApp::new(None, None, None, Vec::new(), None)
}

/// Before any download, the entry offers one, with its size.
#[test]
fn offers_the_download() {
    let eou = TempDir::new("eou-menu-offer");
    let harness = entry_harness(app(), Some(eou.path()));

    harness.get_by_label(&download_label());
}

/// Without a media folder (no data directory) there is nowhere to put the
/// images.
#[test]
fn download_needs_a_data_dir() {
    let harness = entry_harness(app(), None);

    assert!(
        harness
            .get_by_label(&download_label())
            .accesskit_node()
            .is_disabled()
    );
}

/// An app with a download running into `dir`, 29.1 MB in, while the
/// returned sender lives.
fn downloading_into(dir: &Path) -> (ManagerApp, mpsc::Sender<Result<(), String>>) {
    let progress = Arc::new(Progress::default());
    progress.set_total(Some(EOU_ARCHIVE_BYTES));
    progress.add(29_100_000);
    let (tx, rx) = mpsc::channel();
    let mut app = app();
    app.eou_download = Some(Download::running(Source::Eou, progress, rx));
    app.eou_download_dir = Some(dir.to_path_buf());
    (app, tx)
}

/// A running download shows its progress bar in place of the button.
#[test]
fn running_download_shows_its_progress() {
    let eou = TempDir::new("eou-menu-running");

    let (app, _tx) = downloading_into(eou.path());
    let harness = entry_harness(app, Some(eou.path()));

    harness.get_by_label(Source::Eou.label());
    harness.get_by_label("29.1 MB of 58.2 MB");
    assert!(harness.query_by_label(&download_label()).is_none());
}

/// Beneath another template, a running download shows no progress, only
/// the button, off until it ends.
#[test]
fn download_for_another_template_turns_the_button_off() {
    let eou = TempDir::new("eou-menu-elsewhere");
    let other = TempDir::new("eou-menu-elsewhere-other");

    let (app, _tx) = downloading_into(other.path());
    let harness = entry_harness(app, Some(eou.path()));

    assert!(harness.query_by_label(Source::Eou.label()).is_none());
    assert!(
        harness
            .get_by_label(&download_label())
            .accesskit_node()
            .is_disabled()
    );
}

/// A failed download says so and offers the button again for a retry.
#[test]
fn failed_download_offers_a_retry() {
    let eou = TempDir::new("eou-menu-failed");
    let (tx, rx) = mpsc::channel();
    tx.send(Err("404".to_string())).unwrap();
    let mut app = app();
    app.eou_download = Some(Download::running(Source::Eou, Arc::default(), rx));
    app.eou_download_dir = Some(eou.path().to_path_buf());

    let harness = entry_harness(app, Some(eou.path()));

    harness.get_by_label(FAILED_TEXT);
    harness.get_by_label(&download_label());
}

/// Another template's failed download isn't reported here.
#[test]
fn failed_download_for_another_template_is_not_shown() {
    let eou = TempDir::new("eou-menu-failed-here");
    let other = TempDir::new("eou-menu-failed-other");
    let (tx, rx) = mpsc::channel();
    tx.send(Err("404".to_string())).unwrap();
    let mut app = app();
    app.eou_download = Some(Download::running(Source::Eou, Arc::default(), rx));
    app.eou_download_dir = Some(other.path().to_path_buf());

    let harness = entry_harness(app, Some(eou.path()));

    assert!(harness.query_by_label(FAILED_TEXT).is_none());
    assert!(
        !harness
            .get_by_label(&download_label())
            .accesskit_node()
            .is_disabled()
    );
}

/// Clicking the button after a failure starts a new download.
#[test]
fn retry_starts_a_new_download() {
    let eou = TempDir::new("eou-menu-retry");
    let (tx, rx) = mpsc::channel();
    tx.send(Err("404".to_string())).unwrap();
    let mut app = app();
    app.eou_url = REFUSED_URL.to_string();
    app.eou_download = Some(Download::running(Source::Eou, Arc::default(), rx));
    let mut harness = entry_harness(app, Some(eou.path()));

    harness.get_by_label(&download_label()).click();
    harness.step();
    for _ in 0..POLL_ATTEMPTS {
        harness.step();
        if !harness.state().eou_download.as_ref().unwrap().is_running() {
            break;
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    let download = harness.state().eou_download.as_ref().unwrap();
    assert!(!download.is_running(), "the retry must end");
    let error = download.error().expect("a refused connection fails");
    assert_ne!(error, "404", "a new download replaced the failed one");
}
