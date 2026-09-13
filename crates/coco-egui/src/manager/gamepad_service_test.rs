use super::*;

#[test]
fn services_gamepad_without_live_vms_and_schedules_next_drain() {
    let mut manager = ManagerApp::new(None, None, None, Vec::new(), None);
    manager.gamepad = crate::joy::SharedGamepad::service_test_backend();
    let gamepad = manager.gamepad.clone();
    let ctx = egui::Context::default();
    let _ = ctx.run(Default::default(), |_| {});

    let output = ctx.run(Default::default(), |ctx| manager.service_gamepad(ctx));

    assert_eq!(gamepad.service_count(), 1);
    let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    assert!(!delay.is_zero());
    assert!(delay <= GAMEPAD_SERVICE_INTERVAL);
}
