use coco_core::TextCursor;

use super::*;
use crate::control::tools::{MockBackend, STRUCTURED_CONTENT, call};
use crate::{AppParams, CocoApp, MachineConfig, ROMSource};

/// `call` as a protocol 2025-06-18 client sees it.
const STRUCTURED: bool = true;

fn enter_basic_params(arguments: Value) -> Value {
    json!({"name": "enter_basic", "arguments": arguments})
}

fn text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap()
}

/// A screen of `rows` with the cursor at column 0 of `cursor_row`.
fn screen(rows: &[&str], cursor_row: usize) -> ScreenSnapshot {
    ScreenSnapshot {
        lines: rows.iter().map(|row| row.to_string()).collect(),
        mode: "text 32x16".into(),
        cursor: Some(TextCursor {
            row: cursor_row,
            col: 0,
        }),
    }
}

fn typed(calls: &[Request]) -> Vec<&str> {
    calls
        .iter()
        .filter_map(|request| match &request.action {
            Action::TypeText { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn parse_listing_splits_on_every_line_ending_and_skips_blank_lines() {
    let lines = parse_listing("10 A\r\n\r\n20 B\r30 C\n  \n40 D").unwrap();
    let expected = [(1, "10 A"), (3, "20 B"), (4, "30 C"), (6, "40 D")];
    let expected: Vec<ListingLine<'_>> = expected
        .into_iter()
        .map(|(number, text)| ListingLine { number, text })
        .collect();
    assert_eq!(lines, expected);
}

#[test]
fn unmappable_characters_reject_the_listing_before_typing() {
    let mut mock = MockBackend::new(vec![]);
    let listing = "10 PRINT \"OK\"\n20 PRINT \"[~]\"";
    let result = call(
        &mut mock,
        enter_basic_params(json!({"listing": listing})),
        STRUCTURED,
    )
    .unwrap();
    assert_eq!(result["isError"], json!(true));
    assert!(text(&result).contains("'['"), "{}", text(&result));
    assert!(text(&result).contains("'~'"), "{}", text(&result));
    assert!(mock.calls.is_empty());
}

#[test]
fn an_overlong_listing_is_rejected_before_typing() {
    let mut mock = MockBackend::new(vec![]);
    let listing = "1\n".repeat(MAX_ENTER_BASIC_CHARS / 2 + 1);
    let result = call(
        &mut mock,
        enter_basic_params(json!({"listing": listing})),
        STRUCTURED,
    )
    .unwrap();
    assert_eq!(result["isError"], json!(true));
    assert!(text(&result).contains(&MAX_ENTER_BASIC_CHARS.to_string()));
    assert!(mock.calls.is_empty());
}

#[test]
fn a_line_longer_than_basic_accepts_is_rejected_before_typing() {
    let mut mock = MockBackend::new(vec![]);
    let listing = format!("10 A=1\n20 '{}", "X".repeat(BASIC_LINE_MAX_CHARS));
    let result = call(
        &mut mock,
        enter_basic_params(json!({"listing": listing})),
        STRUCTURED,
    )
    .unwrap();
    assert_eq!(result["isError"], json!(true));
    assert!(
        text(&result).contains("listing line 2"),
        "{}",
        text(&result)
    );
    assert!(mock.calls.is_empty());
}

#[test]
fn a_line_of_exactly_the_basic_limit_is_typed() {
    let line = "X".repeat(BASIC_LINE_MAX_CHARS);
    let mut mock = MockBackend::new(vec![Ok(Reply::Done), Ok(Reply::Screen(screen(&[], 0)))]);
    let result = call(
        &mut mock,
        enter_basic_params(json!({"listing": line})),
        STRUCTURED,
    )
    .unwrap();
    assert_eq!(result["isError"], json!(false), "{}", text(&result));
    assert_eq!(typed(&mock.calls), [format!("{line}\n")]);
}

#[test]
fn an_empty_listing_needs_new() {
    let mut mock = MockBackend::new(vec![]);
    let result = call(
        &mut mock,
        enter_basic_params(json!({"listing": "\n \n"})),
        STRUCTURED,
    )
    .unwrap();
    assert_eq!(result["isError"], json!(true));
    assert!(mock.calls.is_empty());

    let mut mock = MockBackend::new(vec![Ok(Reply::Done), Ok(Reply::Screen(screen(&[], 0)))]);
    let result = call(
        &mut mock,
        enter_basic_params(json!({"listing": "", "new": true})),
        STRUCTURED,
    )
    .unwrap();
    assert_eq!(result["isError"], json!(false), "{}", text(&result));
    assert_eq!(typed(&mock.calls), ["NEW\n"]);
}

#[test]
fn new_is_typed_first_and_every_line_is_read_back() {
    let rows = ["NEW", "OK", "10 A=1", "20 B=2", ""];
    let mut mock = MockBackend::new(vec![
        Ok(Reply::Done),
        Ok(Reply::Screen(screen(&rows[..2], 2))),
        Ok(Reply::Done),
        Ok(Reply::Screen(screen(&rows[..3], 3))),
        Ok(Reply::Done),
        Ok(Reply::Screen(screen(&rows, 4))),
    ]);
    let result = call(
        &mut mock,
        enter_basic_params(json!({"listing": "10 A=1\n20 B=2\n", "new": true, "vm": "coco3"})),
        STRUCTURED,
    )
    .unwrap();
    assert_eq!(result["isError"], json!(false), "{}", text(&result));
    assert_eq!(typed(&mock.calls), ["NEW\n", "10 A=1\n", "20 B=2\n"]);
    assert!(
        mock.calls
            .iter()
            .all(|request| request.vm.as_deref() == Some("coco3"))
    );
    let actions: Vec<bool> = mock
        .calls
        .iter()
        .map(|request| request.action == Action::ScreenText)
        .collect();
    assert_eq!(actions, [false, true, false, true, false, true]);
    assert_eq!(result[STRUCTURED_CONTENT]["lines"], json!(2));
    assert_eq!(result[STRUCTURED_CONTENT]["screen"]["lines"], json!(rows));
}

#[test]
fn the_first_basic_error_stops_typing_and_is_reported() {
    let errored = screen(&["10 A=1", "CLEAR 99999", "?OM ERROR", "OK", ""], 4);
    let mut mock = MockBackend::new(vec![
        Ok(Reply::Done),
        Ok(Reply::Screen(screen(&["10 A=1", ""], 1))),
        Ok(Reply::Done),
        Ok(Reply::Screen(errored)),
    ]);
    let listing = "10 A=1\nCLEAR 99999\n20 B=2\n";
    let result = call(
        &mut mock,
        enter_basic_params(json!({"listing": listing})),
        STRUCTURED,
    )
    .unwrap();
    assert_eq!(result["isError"], json!(true));
    let message = text(&result);
    assert!(message.contains("?OM ERROR"), "{message}");
    assert!(message.contains("listing line 2"), "{message}");
    assert!(message.contains("entered 1 of 3"), "{message}");
    assert!(message.contains("CLEAR 99999"), "{message}");
    assert_eq!(typed(&mock.calls), ["10 A=1\n", "CLEAR 99999\n"]);
}

#[test]
fn basic_error_reads_the_report_two_rows_above_the_cursor() {
    let report = |rows: &[&str], cursor_row| basic_error(&screen(rows, cursor_row));
    assert_eq!(
        report(&["RUN", "?SN ERROR IN 20", "OK", ""], 3).as_deref(),
        Some("?SN ERROR IN 20")
    );
    assert_eq!(
        report(&["PRINT 1/0", "?/0 ERROR   ", "OK   ", ""], 3).as_deref(),
        Some("?/0 ERROR")
    );
    // A program line that prints error-like text is not BASIC's report.
    assert_eq!(report(&["?\"?SN ERROR\"", "?SN ERROR", ""], 2), None);
    // A report not followed by the OK prompt is old output.
    assert_eq!(report(&["?SN ERROR", "10 A=1", ""], 2), None);
    assert_eq!(report(&["OK", ""], 1), None);
    let mut unknown = screen(&["?SN ERROR", "OK", ""], 2);
    unknown.cursor = None;
    assert_eq!(basic_error(&unknown), None);
}

#[test]
fn a_failed_request_reports_the_line_and_progress() {
    let mut mock = MockBackend::new(vec![
        Ok(Reply::Done),
        Ok(Reply::Screen(screen(&["10 A=1", ""], 1))),
        Err("VM is paused; call set_running or start_vm first".into()),
    ]);
    let result = call(
        &mut mock,
        enter_basic_params(json!({"listing": "10 A=1\n20 B=2"})),
        STRUCTURED,
    )
    .unwrap();
    assert_eq!(result["isError"], json!(true));
    let message = text(&result);
    assert!(message.contains("listing line 2"), "{message}");
    assert!(message.contains("entered 1 of 2"), "{message}");
    assert!(message.contains("VM is paused"), "{message}");
    assert_eq!(mock.calls.len(), 3, "nothing is typed after the failure");
}

#[test]
fn an_unexpected_reply_is_an_error() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Bytes(vec![]))]);
    let result = call(
        &mut mock,
        enter_basic_params(json!({"listing": "10 A=1"})),
        STRUCTURED,
    )
    .unwrap();
    assert_eq!(result["isError"], json!(true));
    assert!(text(&result).contains("unexpected reply"));
}

/// Fields the real-ROM backend lets one boot or one typed line take before
/// failing the test, far beyond what either needs.
const ROM_FIELD_CAP: usize = 20_000;

/// Fields from power-on until BASIC reads the keyboard at its `OK` prompt,
/// as `typeahead_test.rs` boots.
const BOOT_FIELDS: usize = 600;

/// A [`Backend`] that runs a real CoCo 3 directly, field by field, the way
/// the manager's frame loop does.
struct RomBackend {
    app: CocoApp,
}

impl RomBackend {
    /// Boot the installed `coco3.rom` to BASIC's `OK` prompt.
    fn boot() -> Self {
        let rom_path = crate::installed_roms_dir().join(crate::rom_load::COCO3_ROM_FILE);
        let rom = std::fs::read(&rom_path)
            .expect("installed coco3.rom is required (first-run asset download)")
            .into_boxed_slice();
        let mut app = CocoApp::new(
            MachineConfig::default(),
            rom,
            ROMSource::File(rom_path),
            AppParams::default(),
            crate::joy::SharedGamepad::without_backend(),
        );
        app.run_fields(BOOT_FIELDS);
        let screen = app.screen_snapshot();
        assert!(
            screen.cursor.is_some() && screen.lines.iter().any(|line| line.trim() == "OK"),
            "BASIC never reached its OK prompt: {screen:?}"
        );
        Self { app }
    }
}

impl Backend for RomBackend {
    fn call(&mut self, req: &Request) -> Result<Reply, ControlError> {
        match &req.action {
            Action::TypeText { text } => {
                self.app.start_remote_typing(text)?;
                for _ in 0..ROM_FIELD_CAP {
                    if !self.app.remote_type_ahead.is_active() {
                        return Ok(Reply::Done);
                    }
                    self.app.run_fields(1);
                }
                panic!("typing {text:?} never drained");
            }
            Action::ScreenText => Ok(self.app.screen_text()),
            other => panic!("enter_basic sent {other:?}"),
        }
    }
}

fn enter_on_rom(backend: &mut RomBackend, arguments: Value) -> Value {
    call(backend, enter_basic_params(arguments), STRUCTURED).unwrap()
}

#[test]
fn real_rom_listing_is_entered_and_runs() {
    let mut backend = RomBackend::boot();
    let result = enter_on_rom(
        &mut backend,
        json!({"listing": "10 A=6*7\r\n20 PRINT A\r\n", "new": true}),
    );
    assert_eq!(result["isError"], json!(false), "{}", text(&result));
    assert_eq!(result[STRUCTURED_CONTENT]["lines"], json!(2));

    let result = enter_on_rom(&mut backend, json!({"listing": "RUN"}));
    assert_eq!(result["isError"], json!(false), "{}", text(&result));
    let rows = &result[STRUCTURED_CONTENT]["screen"]["lines"];
    let rows: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row.as_str().unwrap())
        .collect();
    assert!(rows.iter().any(|row| row.trim() == "42"), "{rows:?}");
}

#[test]
fn real_rom_direct_mode_error_stops_the_listing() {
    let mut backend = RomBackend::boot();
    let listing = "10 PRINT \"HI\"\n\nPRINT 1/0\n20 END\n";
    let result = enter_on_rom(&mut backend, json!({"listing": listing, "new": true}));
    assert_eq!(result["isError"], json!(true));
    let message = text(&result);
    assert!(message.contains("BASIC reported ?/0 ERROR"), "{message}");
    assert!(message.contains("listing line 3"), "{message}");
    assert!(message.contains("entered 1 of 3"), "{message}");
}

#[test]
fn real_rom_program_error_reports_its_line_number() {
    let mut backend = RomBackend::boot();
    let result = enter_on_rom(
        &mut backend,
        json!({"listing": "10 PRINT 1/0\nRUN", "new": true}),
    );
    assert_eq!(result["isError"], json!(true));
    let message = text(&result);
    assert!(message.contains("?/0 ERROR IN 10"), "{message}");
}
