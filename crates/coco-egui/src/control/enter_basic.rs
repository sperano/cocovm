//! `enter_basic`: types a multi-line BASIC listing as a sequence of
//! [`Action::TypeText`] requests, one listing line each, so no single
//! deferred reply grows with the listing. After each line it reads the
//! screen back and stops at the first BASIC error message.

use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::{Value, json};

use super::jsonrpc::RpcError;
use super::protocol::{Action, ControlError, Reply, Request, ScreenSnapshot};
use super::tools::{
    Backend, error_result, format_control_error, format_screen, parse_args, screen_json,
    structured_result,
};
use super::{MAX_ENTER_BASIC_CHARS, MAX_TYPE_TEXT_CHARS, key_names};

/// Most characters Color BASIC's line input accepts for one line; it drops
/// further keystrokes without any error. Measured against `coco3.rom`: all
/// 249 typed characters echo, a 250th does not.
pub const BASIC_LINE_MAX_CHARS: usize = 249;

// Each line goes out as one `type_text` request, ENTER included.
const _: () = assert!(BASIC_LINE_MAX_CHARS < MAX_TYPE_TEXT_CHARS);

/// The direct-mode command `new: true` types before the listing.
const NEW_COMMAND: &str = "NEW";

/// BASIC's error report, `?` + two-character code + ` ERROR`, with
/// ` IN <line>` only while a program runs (verified against `coco3.rom`'s
/// error printer at `$AC5D` and its code table at `$ABAF`). Disk BASIC adds
/// codes beyond that table, so any two non-blank characters count.
static BASIC_ERROR: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^\?\S{2} ERROR(?: IN \d+)?$").expect("BASIC error pattern compiles")
});
/// The prompt BASIC's main loop prints on the row after an error report.
const OK_PROMPT: &str = "OK";
/// Rows from an error report down to the input row: the report, then `OK`.
const ERROR_REPORT_ROWS_ABOVE_CURSOR: usize = 2;

#[derive(Deserialize)]
struct EnterBasicArgs {
    #[serde(default)]
    vm: Option<String>,
    listing: String,
    #[serde(default)]
    new: bool,
}

/// One non-blank line of the listing and its 1-based position in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ListingLine<'a> {
    number: usize,
    text: &'a str,
}

/// Typing stopped at `line` (`None`: at `NEW`) after `entered` listing
/// lines, because of `cause`.
struct Stopped {
    entered: usize,
    line: Option<String>,
    cause: Failure,
}

/// Why typing stopped early.
enum Failure {
    /// BASIC printed an error report for the line.
    Basic {
        message: String,
        screen: ScreenSnapshot,
    },
    /// A request for the line failed (paused VM, client gone, ...).
    Control(ControlError),
}

pub(super) fn dispatch(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let EnterBasicArgs { vm, listing, new } = parse_args(args)?;
    let lines = match parse_listing(&listing) {
        Ok(lines) => lines,
        Err(message) => return Ok(error_result(message)),
    };
    if lines.is_empty() && !new {
        return Ok(error_result(
            "listing has no lines; nothing was typed".into(),
        ));
    }
    Ok(match enter(backend, &vm, new, &lines) {
        Ok(screen) => structured_result(
            format!(
                "Entered {} line(s).\n{}",
                lines.len(),
                format_screen(&screen)
            ),
            json!({"lines": lines.len(), "screen": screen_json(&screen)}),
        ),
        Err(stopped) => error_result(describe_stop(&stopped, lines.len())),
    })
}

/// Split `listing` into its non-blank lines (`\n`, `\r\n`, or `\r` ends a
/// line) and reject it whole — before anything is typed — when it is too
/// long, holds a character with no CoCo key, or has a line BASIC would cut.
fn parse_listing(listing: &str) -> Result<Vec<ListingLine<'_>>, String> {
    let length = listing.chars().count();
    if length > MAX_ENTER_BASIC_CHARS {
        return Err(format!(
            "listing is {length} characters; at most {MAX_ENTER_BASIC_CHARS} per call"
        ));
    }
    key_names::text_taps(listing)?;
    let lines: Vec<ListingLine<'_>> = listing
        .lines()
        .flat_map(|line| line.split('\r'))
        .enumerate()
        .filter(|(_, text)| !text.trim().is_empty())
        .map(|(index, text)| ListingLine {
            number: index + 1,
            text,
        })
        .collect();
    if let Some(line) = lines
        .iter()
        .find(|line| line.text.chars().count() > BASIC_LINE_MAX_CHARS)
    {
        return Err(format!(
            "listing line {} is {} characters; BASIC accepts at most \
             {BASIC_LINE_MAX_CHARS} per line; nothing was typed",
            line.number,
            line.text.chars().count()
        ));
    }
    Ok(lines)
}

/// Type `NEW` if asked, then every line. Returns the screen after the last
/// line. The error is boxed: it carries a whole screen snapshot.
fn enter(
    backend: &mut dyn Backend,
    vm: &Option<String>,
    new: bool,
    lines: &[ListingLine<'_>],
) -> Result<ScreenSnapshot, Box<Stopped>> {
    let stop = |entered, line, cause| {
        Box::new(Stopped {
            entered,
            line,
            cause,
        })
    };
    let mut screen = None;
    if new {
        let typed = type_line(backend, vm, NEW_COMMAND);
        screen = Some(typed.map_err(|cause| stop(0, None, cause))?);
    }
    for (entered, line) in lines.iter().enumerate() {
        let label = format!("listing line {} ({:?})", line.number, line.text);
        let typed = type_line(backend, vm, line.text);
        screen = Some(typed.map_err(|cause| stop(entered, Some(label), cause))?);
    }
    Ok(screen.expect("dispatch rejects an empty listing without new"))
}

/// Type `text` and ENTER, then read the screen back.
fn type_line(
    backend: &mut dyn Backend,
    vm: &Option<String>,
    text: &str,
) -> Result<ScreenSnapshot, Failure> {
    let type_text = Action::TypeText {
        text: format!("{text}\n"),
    };
    match request(backend, vm, type_text).map_err(Failure::Control)? {
        Reply::Done => {}
        _ => return Err(unexpected_reply()),
    }
    let screen = match request(backend, vm, Action::ScreenText).map_err(Failure::Control)? {
        Reply::Screen(screen) => screen,
        _ => return Err(unexpected_reply()),
    };
    match basic_error(&screen) {
        Some(message) => Err(Failure::Basic { message, screen }),
        None => Ok(screen),
    }
}

fn request(
    backend: &mut dyn Backend,
    vm: &Option<String>,
    action: Action,
) -> Result<Reply, ControlError> {
    backend.call(&Request {
        vm: vm.clone(),
        action,
    })
}

fn unexpected_reply() -> Failure {
    Failure::Control("cocovm returned an unexpected reply".into())
}

/// The error BASIC reported for the line just entered: after an error,
/// BASIC prints the report, then `OK`, and waits for input on the next row,
/// so the report sits two rows above the cursor (verified against
/// `coco3.rom` by typing `PRINT 1/0`). `None` when the cursor is unknown.
fn basic_error(screen: &ScreenSnapshot) -> Option<String> {
    let report_row = screen
        .cursor?
        .row
        .checked_sub(ERROR_REPORT_ROWS_ABOVE_CURSOR)?;
    let report = screen.lines.get(report_row)?.trim_end();
    let prompt = screen.lines.get(report_row + 1)?.trim_end();
    (prompt == OK_PROMPT && BASIC_ERROR.is_match(report)).then(|| report.to_string())
}

fn describe_stop(stopped: &Stopped, total: usize) -> String {
    let progress = format!(
        "entered {} of {total} listing line(s); stopped there",
        stopped.entered
    );
    let line = stopped.line.as_deref().unwrap_or(NEW_COMMAND);
    match &stopped.cause {
        Failure::Basic { message, screen } => format!(
            "BASIC reported {message} after {line}; {progress}.\n{}",
            format_screen(screen)
        ),
        Failure::Control(error) => format!(
            "typing {line} failed; {progress}: {}",
            format_control_error(error)
        ),
    }
}

#[cfg(test)]
#[path = "enter_basic_test.rs"]
mod tests;
