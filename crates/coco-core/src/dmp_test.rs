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
