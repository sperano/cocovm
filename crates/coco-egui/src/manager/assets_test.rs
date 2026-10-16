use super::*;

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
