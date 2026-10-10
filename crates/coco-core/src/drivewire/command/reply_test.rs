use super::*;

#[test]
fn ok_prefixes_the_java_status_line() {
    assert_eq!(ok(b"data"), b"OK command successful\n\rdata");
}

#[test]
fn fail_uses_a_three_digit_code() {
    assert_eq!(
        fail(&Failure::new(code::SYNTAX_ERROR, "Syntax error")),
        b"FAIL 010 Syntax error\n\r"
    );
}

#[test]
fn a_long_failure_fits_the_client_buffer() {
    let line = fail(&Failure::new(code::INVALID_DRIVE, "x".repeat(1000)));
    assert_eq!(line.len(), MAX_STATUS_LINE_BYTES);
    assert!(line.ends_with(b"x\n\r"));
}

#[test]
fn echoed_input_is_printable_and_bounded() {
    assert_eq!(sanitize(b"a\x01b\xFFc"), "a?b?c");
    let long = sanitize(&[b'y'; MAX_ECHO_BYTES + 1]);
    assert_eq!(long.len(), MAX_ECHO_BYTES + ECHO_ELLIPSIS.len());
}

#[test]
fn share_failures_name_the_guest_path() {
    let failure = share_failure(ShareError::NotFound, b"games/missing.bin");
    assert_eq!(failure.code, 201);
    assert_eq!(failure.text, "games/missing.bin: not found");
    assert_eq!(
        insert_failure(ShareError::NotFound, b"games/a.dsk").code,
        code::SERVER_FILE_NOT_FOUND
    );
    assert_eq!(insert_failure(ShareError::Busy, b"games/a.dsk").code, 202);
}

#[test]
fn help_wraps_like_col_layout() {
    let verbs = ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "b", "c"];
    let text = String::from_utf8(help(&verbs)).unwrap();
    let rows: Vec<&str> = text.split("\r\n").collect();
    // 37 columns per name: two fit in 79.
    assert_eq!(rows[2].len(), 2 * 37);
    assert_eq!(rows[3].trim_end(), "c");
}

#[test]
fn dir_lines_end_in_crlf_and_count_entries() {
    let (lines, entries) = dir_lines(b"My File.txt\nsub/\n");
    assert_eq!(lines, b"My File.txt\r\nsub/\r\n");
    assert_eq!(entries, 2);
    assert_eq!(dir_lines(b""), (Vec::new(), 0));
}

#[test]
fn disk_list_marks_write_protected_drives() {
    let host = DriveMedia {
        name: None,
        origin: MediaOrigin::Host,
        write_protected: false,
    };
    let guest = DriveMedia {
        name: Some("/games/a.dsk".to_string()),
        origin: MediaOrigin::Guest,
        write_protected: true,
    };
    let text = disk_list([(0, &host), (12, &guest)]);
    assert_eq!(
        String::from_utf8(text).unwrap(),
        "\r\nCurrent DriveWire disks:\r\n\r\nX0   (configured image)\r\nX12 */games/a.dsk\r\n"
    );
    let details = String::from_utf8(disk_details(1, &guest)).unwrap();
    assert!(details.starts_with("Details for disk in drive #1:\r\n\r\n/games/a.dsk\r\n"));
    assert!(details.contains("Mounted by: dw disk insert (this session only)\r\n"));
    assert!(details.contains("Access: read-only\r\n"));
}
