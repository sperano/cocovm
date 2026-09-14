use super::*;
use crate::bitbanger::BitBanger;

#[test]
fn both_printers_resume_partial_commands_and_paper_after_serialization() {
    for model in [DmpModel::Dmp105, DmpModel::Dmp130] {
        let mut port = BitBanger::new();
        let mut handle = port.start_printer(model);
        for &byte in b"A\r\x12\x81\x1b\x10\x01" {
            handle.write_byte(byte);
        }
        let mut bytes = Vec::new();
        ciborium::into_writer(&port, &mut bytes).unwrap();
        let restored: BitBanger = ciborium::from_reader(bytes.as_slice()).unwrap();
        let mut restored_handle = restored.printer_handle().unwrap();
        assert_eq!(restored_handle.model(), model);
        for &byte in &[0x00, 0xFF, 0x0D, 0x81, 0x1E, b'B', 0x0D] {
            handle.write_byte(byte);
            restored_handle.write_byte(byte);
        }
        assert_eq!(
            restored_handle.dots_in_range(0, u32::MAX),
            handle.dots_in_range(0, u32::MAX)
        );
        assert!(restored_handle.paper_extent().dot_count > 0);
        restored_handle.tear_off();
        assert_eq!(restored_handle.paper_extent().dot_count, 0);
        assert!(handle.paper_extent().dot_count > 0);
    }
}

#[test]
fn paper_snapshot_is_independent_of_later_printer_changes() {
    let mut handle = DmpHandle::new();
    handle.write_byte(b'A');
    let snapshot = handle.paper_snapshot();
    let snapshot_extent = snapshot.extent();

    handle.write_byte(b'B');

    assert_eq!(snapshot.extent(), snapshot_extent);
    assert!(handle.paper_extent().dot_count > snapshot_extent.dot_count);
}

#[test]
fn paper_snapshot_limit_rejects_before_cloning_large_paper() {
    let mut handle = DmpHandle::new();
    handle.write_byte(b'A');
    let limit_bytes = 0;

    let error = handle.paper_snapshot_with_limit(limit_bytes).unwrap_err();

    assert!(error.estimated_bytes > 0);
    assert_eq!(error.limit_bytes, limit_bytes);
}
