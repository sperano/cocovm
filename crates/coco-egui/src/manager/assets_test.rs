use std::sync::{Arc, mpsc};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;
use crate::download::Progress;

/// A dialog drawn by a harness the size of the real window.
fn dialog_harness(dialog: AssetDialog) -> Harness<'static, AssetDialog> {
    let mut harness = Harness::builder()
        .with_size(egui::Vec2::from(DIALOG_WINDOW_SIZE))
        .build_state(
            |ctx, dialog: &mut AssetDialog| {
                dialog.draw(ctx);
            },
            dialog,
        );
    harness.run_steps(2);
    harness
}

/// A dialog fetching into an unused directory.
fn dialog() -> AssetDialog {
    AssetDialog::new(
        vec!["coco3.rom".to_string()],
        SourceUrls {
            bundle: "http://unused.invalid/bundle.tgz".to_string(),
            eou: crate::nitros9::EOU_URL.to_string(),
        },
        std::env::temp_dir().join("cocovm-asset-dialog-unused"),
    )
}

/// The resize must come last. Sent while the window is still non-resizable,
/// winit's X11 backend pins the window at the requested size; sent before
/// the limit clears, the window manager clamps it to the dialog size.
#[test]
fn growing_from_dialog_resizes_after_unpinning_the_window() {
    let commands = grow_to_manager_commands();
    let (resize, unpin) = commands.split_last().expect("at least one command");

    assert_eq!(
        *resize,
        egui::ViewportCommand::InnerSize(WINDOW_SIZE.into())
    );
    for expected in [
        egui::ViewportCommand::Resizable(true),
        egui::ViewportCommand::MinInnerSize(egui::Vec2::ZERO),
        egui::ViewportCommand::MaxInnerSize(egui::Vec2::INFINITY),
    ] {
        assert!(
            unpin.contains(&expected),
            "missing {expected:?} before the resize"
        );
    }
}

/// Both downloads show at once, each under its label with its own bar, and
/// the footer says the work is under way instead of offering buttons.
#[test]
fn running_downloads_each_show_a_progress_bar() {
    let bundle = Arc::new(Progress::default());
    bundle.set_total(Some(4_000_000));
    bundle.add(1_000_000);
    let eou = Arc::new(Progress::default());
    eou.set_total(Some(58_226_559));
    eou.add(29_100_000);
    let (_bundle_tx, bundle_rx) = mpsc::channel();
    let (_eou_tx, eou_rx) = mpsc::channel();
    let mut dialog = dialog();
    dialog.downloads = vec![
        Download::running(Source::Bundle, bundle, bundle_rx),
        Download::running(Source::Eou, eou, eou_rx),
    ];

    let harness = dialog_harness(dialog);

    harness.get_by_label(Source::Bundle.label());
    harness.get_by_label(Source::Eou.label());
    harness.get_by_label("1.0 MB of 4.0 MB");
    harness.get_by_label("29.1 MB of 58.2 MB");
    harness.get_by_label("Downloading and installing…");
    assert!(harness.query_by_label("Cancel").is_none());
}

/// A failed bundle offers a retry while the EOU download carries on: the
/// footer waits only for required downloads.
#[test]
fn optional_download_does_not_hold_the_buttons() {
    let (bundle_tx, bundle_rx) = mpsc::channel();
    bundle_tx.send(Err("404".to_string())).unwrap();
    let (_eou_tx, eou_rx) = mpsc::channel();
    let mut failed = Download::running(Source::Bundle, Arc::default(), bundle_rx);
    failed.poll();
    let mut dialog = dialog();
    dialog.error = Some(format!("{}: 404", Source::Bundle.label()));
    dialog.downloads = vec![
        failed,
        Download::running(Source::Eou, Arc::default(), eou_rx),
    ];

    let harness = dialog_harness(dialog);

    harness.get_by_label("Try again");
    harness.get_by_label("Cancel");
    harness.get_by_label(Source::Eou.label());
    let spinner = harness.query_by_label("Downloading and installing…");
    assert!(spinner.is_none());
}

/// The EOU download, still running, outlives the dialog for the New Machine dialog
/// to show.
#[test]
fn eou_download_is_handed_over() {
    let (_bundle_tx, bundle_rx) = mpsc::channel();
    let (_eou_tx, eou_rx) = mpsc::channel();
    let mut dialog = dialog();
    dialog.downloads = vec![
        Download::running(Source::Bundle, Arc::default(), bundle_rx),
        Download::running(Source::Eou, Arc::default(), eou_rx),
    ];

    let handed = dialog.into_eou_download().expect("the EOU download");

    assert_eq!(handed.source(), Source::Eou);
    assert!(handed.is_running());
}

/// Each failed download's error is named after its source.
#[test]
fn failure_details_name_each_failed_source() {
    let mut dialog = dialog();
    let (tx, rx) = mpsc::channel();
    let mut failed = Download::running(Source::Eou, Arc::default(), rx);
    tx.send(Err("404".to_string())).unwrap();
    failed.poll();
    dialog.downloads = vec![failed];

    assert_eq!(
        dialog.failure_details(),
        format!("{}: 404", Source::Eou.label())
    );

    dialog.downloads.clear();
    assert!(dialog.failure_details().contains("did not provide"));
}
