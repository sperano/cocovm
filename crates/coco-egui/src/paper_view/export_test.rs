use std::sync::Mutex;

use coco_core::printer::Paper;

use super::*;

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn scratch_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "cocovm-paper-job-test-{}-{name}",
        std::process::id()
    ))
}

#[test]
fn process_limit_rejects_an_unbounded_third_export() {
    let _lock = TEST_LOCK.lock().unwrap();
    let permits: Vec<_> = (0..MAX_CONCURRENT_PRINTER_EXPORTS)
        .map(|_| ExportPermit::acquire().unwrap())
        .collect();

    let error = ExportPermit::acquire().err().unwrap();

    assert!(error.contains("already running"));
    drop(permits);
    assert_eq!(ACTIVE_EXPORTS.load(Ordering::Acquire), 0);
}

#[test]
fn controller_rejects_a_duplicate_window_job() {
    let _lock = TEST_LOCK.lock().unwrap();
    let path = scratch_path("duplicate.png");
    let mut controller = Controller::default();
    let handle = DmpHandle::new();
    controller
        .start(
            &handle,
            path.clone(),
            ExportFormat::PagePng { page: 0 },
            1,
            false,
        )
        .unwrap();

    let error = controller
        .start(&handle, path.clone(), ExportFormat::RollPng, 1, false)
        .unwrap_err();

    assert!(error.contains("already running"));
    drop(controller);
    let _ = std::fs::remove_file(path);
}

#[test]
fn dropping_controller_cancels_joins_and_releases_permit() {
    let _lock = TEST_LOCK.lock().unwrap();
    let path = scratch_path("drop.png");
    let permit = ExportPermit::acquire().unwrap();
    let request = ExportRequest {
        paper: Paper::new(),
        page_count: 100,
        green_bar: false,
        target: path.clone(),
        format: ExportFormat::RollPng,
    };
    let mut controller = Controller::default();
    controller.start_request(request, permit).unwrap();

    drop(controller);

    assert_eq!(ACTIVE_EXPORTS.load(Ordering::Acquire), 0);
    assert!(!path.exists());
}

#[test]
fn completed_job_records_an_accurate_visible_notice() {
    let _lock = TEST_LOCK.lock().unwrap();
    let path = scratch_path("complete.png");
    let mut controller = Controller::default();
    let handle = DmpHandle::new();
    controller
        .start(
            &handle,
            path.clone(),
            ExportFormat::PagePng { page: 0 },
            1,
            false,
        )
        .unwrap();
    while controller.is_active() {
        controller.poll();
        std::thread::yield_now();
    }

    let notice = controller.notice.as_deref().unwrap();
    assert!(notice.contains("Export complete"));
    assert!(notice.contains(path.to_str().unwrap()));
    std::fs::remove_file(path).unwrap();
}
