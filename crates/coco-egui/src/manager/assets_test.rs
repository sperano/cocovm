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
        "http://unused.invalid/bundle.tgz".to_string(),
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

/// The running download's bar sits under the prompt, which already names
/// the files, so the bar carries no label of its own; the footer says the
/// work is under way instead of offering buttons.
#[test]
fn running_download_shows_an_unlabelled_progress_bar() {
    let bundle = Arc::new(Progress::default());
    bundle.set_total(Some(4_000_000));
    bundle.add(1_000_000);
    let (_bundle_tx, bundle_rx) = mpsc::channel();
    let mut dialog = dialog();
    dialog.download = Some(Download::running(Source::Bundle, bundle, bundle_rx));

    let harness = dialog_harness(dialog);

    harness.get_by_label(PROMPT_TEXT);
    assert!(harness.query_by_label(Source::Bundle.label()).is_none());
    harness.get_by_label("1.0 MB of 4.0 MB");
    harness.get_by_label("Downloading and installing…");
    assert!(harness.query_by_label("Cancel").is_none());
}

/// A failed download offers a retry.
#[test]
fn failed_download_offers_a_retry() {
    let (bundle_tx, bundle_rx) = mpsc::channel();
    bundle_tx.send(Err("404".to_string())).unwrap();
    let mut failed = Download::running(Source::Bundle, Arc::default(), bundle_rx);
    failed.poll();
    let mut dialog = dialog();
    dialog.error = Some(format!("{}: 404", Source::Bundle.label()));
    dialog.download = Some(failed);

    let harness = dialog_harness(dialog);

    harness.get_by_label("Try again");
    harness.get_by_label("Cancel");
    let spinner = harness.query_by_label("Downloading and installing…");
    assert!(spinner.is_none());
}

/// A failed download's error is named after the bundle.
#[test]
fn failure_details_name_the_bundle() {
    let mut dialog = dialog();
    let (tx, rx) = mpsc::channel();
    let mut failed = Download::running(Source::Bundle, Arc::default(), rx);
    tx.send(Err("404".to_string())).unwrap();
    failed.poll();
    dialog.download = Some(failed);

    assert_eq!(
        dialog.failure_details(),
        format!("{}: 404", Source::Bundle.label())
    );

    dialog.download = None;
    assert!(dialog.failure_details().contains("did not provide"));
}
