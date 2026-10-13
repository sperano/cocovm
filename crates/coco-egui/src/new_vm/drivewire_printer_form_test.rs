use coco_core::drivewire::PrintDiagnostics;

use super::*;

fn diagnostics() -> PrintDiagnostics {
    PrintDiagnostics::default()
}

#[test]
fn printed_line_names_the_last_file_by_name_only() {
    let mut d = diagnostics();
    d.printed = 1;
    assert_eq!(printed_line(&d), "Printed 1 job");

    d.printed = 3;
    d.last_file = Some("/home/me/prints/print-20260101-010101-job3.txt".into());
    assert_eq!(
        printed_line(&d),
        "Printed 3 jobs, last print-20260101-010101-job3.txt"
    );
}

#[test]
fn status_lines_include_only_nonzero_counters() {
    let d = diagnostics();
    assert_eq!(status_lines(&d), ["Printed 0 jobs"]);

    let mut d = diagnostics();
    d.printed = 2;
    d.waiting = 1;
    d.open_bytes = 42;
    d.discarded = 3;
    d.dropped = 1;
    d.incomplete = 2;
    assert_eq!(
        status_lines(&d),
        [
            "Printed 2 jobs",
            "1 job waiting to be written",
            "Open job: 42 bytes",
            "3 jobs discarded (printing was off)",
            "1 job dropped (the queue was full)",
            "2 jobs incomplete (ended by a reset or stop)",
        ]
    );
}

#[test]
fn output_and_format_labels_name_every_variant() {
    assert_eq!(output_label(PrintOutputDTO::Off), OFF_LABEL);
    assert_eq!(output_label(PrintOutputDTO::Folder), FOLDER_LABEL);
    assert_eq!(
        output_label(PrintOutputDTO::PrinterPort),
        PRINTER_PORT_LABEL
    );
    assert_eq!(format_label(PrintFormatDTO::Text), TEXT_FORMAT_LABEL);
    assert_eq!(format_label(PrintFormatDTO::Raw), RAW_FORMAT_LABEL);
}

#[test]
fn plural_s_only_marks_counts_other_than_one() {
    assert_eq!(plural_s(0), "s");
    assert_eq!(plural_s(1), "");
    assert_eq!(plural_s(2), "s");
}

#[test]
fn printer_hint_names_the_part_size_in_mib() {
    assert!(printer_hint().contains("1 MiB"), "{}", printer_hint());
}
