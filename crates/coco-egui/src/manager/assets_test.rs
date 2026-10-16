use super::*;

/// `Resizable(true)` must come before the limit clears: on winit's Wayland
/// backend it re-applies the recorded min/max size, which would pin the
/// window at the dialog size again. The resize comes last so the window
/// manager no longer clamps it.
#[test]
fn growing_from_dialog_clears_size_limits_before_resizing() {
    assert_eq!(
        grow_to_manager_commands(),
        [
            egui::ViewportCommand::Resizable(true),
            egui::ViewportCommand::MinInnerSize(egui::Vec2::ZERO),
            egui::ViewportCommand::MaxInnerSize(egui::Vec2::INFINITY),
            egui::ViewportCommand::InnerSize(WINDOW_SIZE.into()),
        ]
    );
}
