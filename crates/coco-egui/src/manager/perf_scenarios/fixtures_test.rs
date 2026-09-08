use super::*;

#[test]
fn sparse_printer_fixture_reaches_every_requested_page() {
    const MAX_DOTS_PER_PAGE: usize = 1000;
    let mut handle = coco_core::dmp105::DMP105Handle::new();
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
    assert!(operate(&mut app, &config, 0).expect("first snapshot round trip"));
    assert!(root.join(SNAPSHOT_FILE).is_file());
    assert!(operate(&mut app, &config, 1).expect("replace existing snapshot"));
    let vm = app.entries[0].vm.as_ref().expect("VM remains live");
    assert_eq!(vm.machine.cpu.pc, saved_pc);
    assert_eq!(vm.machine.bus.ram[0], MEMORY_SENTINEL);
}
