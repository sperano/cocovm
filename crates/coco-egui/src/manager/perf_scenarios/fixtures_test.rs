use super::*;

#[test]
fn scroll_operations_cycle_through_exact_fixture_positions() {
    let expected = [
        ("beginning", 0, 0),
        ("middle", PREVIEW_COUNT / 2, PRINTER_PAGE_COUNT / 2),
        ("end", PREVIEW_COUNT - 1, PRINTER_PAGE_COUNT - 1),
    ];

    for (operation, (position, preview_index, printer_page)) in expected.into_iter().enumerate() {
        let preview = scroll_operation(MANAGER_SCROLL_SURFACE, PREVIEW_COUNT, operation as u64)
            .scroll
            .expect("preview scroll");
        let printer =
            scroll_operation(PRINTER_SCROLL_SURFACE, PRINTER_PAGE_COUNT, operation as u64)
                .scroll
                .expect("printer scroll");

        assert_eq!(preview.position.name(), position);
        assert_eq!(preview.target_index, preview_index);
        assert_eq!(printer.position.name(), position);
        assert_eq!(printer.target_index, printer_page);
    }
}

#[test]
fn sparse_printer_fixture_reaches_every_requested_page() {
    const MAX_DOTS_PER_PAGE: usize = 1000;
    let mut handle = coco_core::dmp::DmpHandle::new();
    fill_printer(&mut handle);
    let extent = handle.paper_extent();
    let page_height =
        (crate::paper_render::PAGE_HEIGHT_IN * coco_core::printer::Y_UNITS_PER_INCH as f32) as u32;
    assert_eq!(extent.max_y / page_height + 1, PRINTER_PAGE_COUNT as u32);
    assert!(extent.dot_count > PRINTER_PAGE_COUNT);
    assert!(extent.dot_count < PRINTER_PAGE_COUNT * MAX_DOTS_PER_PAGE);
}

#[test]
fn snapshot_fixture_creates_missing_artifact_root_and_round_trips_real_vm() {
    use crate::machine_def::tests::TempDir;
    const TEST_DURATION: std::time::Duration = std::time::Duration::from_secs(1);
    const MEMORY_SENTINEL: u8 = 0x5A;
    let directory = TempDir::new("perf-snapshot-missing-root");
    let root = directory.path().join("missing/artifacts");
    let config = Config {
        name: "snapshot".into(),
        warmup: TEST_DURATION,
        duration: TEST_DURATION,
        output: directory.path().join("metrics.json"),
        vm_count: 1,
        display: "rgb".into(),
        variant: "coco3".into(),
    };
    let mut app = ManagerApp::new(
        None,
        Some(directory.path().join("config")),
        Some(root.clone()),
        Vec::new(),
        None,
    );
    assert!(!root.exists());
    prepare(&mut app, &config).expect("prepare isolated snapshot fixture");
    assert!(root.is_dir());
    let vm = app.entries[0].vm.as_mut().expect("real ROM booted");
    vm.machine.bus.ram[0] = MEMORY_SENTINEL;
    let saved_pc = vm.machine.cpu.pc;
    operate(&mut app, &config, 0)
        .expect("first snapshot round trip")
        .outcome
        .expect("first snapshot succeeds");
    assert!(root.join(SNAPSHOT_FILE).is_file());
    operate(&mut app, &config, 1)
        .expect("replace existing snapshot")
        .outcome
        .expect("replacement snapshot succeeds");
    let vm = app.entries[0].vm.as_ref().expect("VM remains live");
    assert_eq!(vm.machine.cpu.pc, saved_pc);
    assert_eq!(vm.machine.bus.ram[0], MEMORY_SENTINEL);
}

#[test]
fn lifecycle_fixture_names_warm_cold_and_recovery_transitions() {
    use crate::machine_def::tests::TempDir;
    const TEST_DURATION: std::time::Duration = std::time::Duration::from_secs(1);
    let directory = TempDir::new("perf-lifecycle-transitions");
    let config = Config {
        name: "lifecycle".into(),
        warmup: TEST_DURATION,
        duration: TEST_DURATION,
        output: directory.path().join("metrics.json"),
        vm_count: 1,
        display: "rgb".into(),
        variant: "coco3".into(),
    };
    let mut app = ManagerApp::new(
        None,
        Some(directory.path().join("config")),
        Some(directory.path().join("artifacts")),
        Vec::new(),
        None,
    );
    prepare(&mut app, &config).expect("prepare lifecycle fixture");
    assert!(
        app.entries[0]
            .vm
            .as_ref()
            .expect("fixture starts live")
            .joysticks
            .shares_gamepad(&app.gamepad)
    );
    let expected = [
        ("suspend-for-warm-resume", true, true),
        ("resume-live-vm", true, false),
        ("suspend-for-window-close", true, true),
        ("close-suspended-window", false, true),
        ("resume-cold-vm", true, false),
        ("stop-vm", false, false),
        ("start-vm", true, false),
        ("close-running-window", false, false),
        ("restart-after-window-close", true, false),
    ];

    for (step, (name, live, suspended)) in expected.into_iter().enumerate() {
        let attempt =
            operate(&mut app, &config, step as u64).expect("lifecycle operation is recorded");
        attempt.outcome.expect("lifecycle operation succeeds");
        assert_eq!(attempt.operation.name, name);
        assert_eq!(app.entries[0].vm.is_some(), live, "{name}");
        assert_eq!(app.entries[0].suspended, suspended, "{name}");
        if let Some(vm) = app.entries[0].vm.as_ref() {
            assert!(vm.joysticks.shares_gamepad(&app.gamepad), "{name}");
        }
    }
}
