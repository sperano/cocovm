//! Reply bytes: the status line, help, directory lines, and disk details.
//!
//! The status envelope is the Java server's (`DWVSerialPort`
//! `sendUtilityOKResponse`/`sendUtilityFailResponse`): `OK command
//! successful` or `FAIL nnn text`, each followed by LF then CR. NitrOS-9's
//! `dw` reads the status line up to CR into a 256-byte buffer, so a failure
//! line is cut to fit it. Payload text ends lines with CR LF as the Java
//! commands do.

use std::fmt::Write;

use super::super::share::ShareError;
use super::super::{DriveMedia, MediaOrigin};
use super::parse::{Failure, code};

/// The successful status line.
pub(super) const OK_STATUS: &[u8] = b"OK command successful\n\r";
const FAIL_PREFIX: &str = "FAIL";
const STATUS_END: &[u8] = b"\n\r";
/// The `dw` client's status buffer (`pbuffer rmb 256` in `dw.as`).
const MAX_STATUS_LINE_BYTES: usize = 256;
/// Bytes of a guest-typed word or path echoed in a message.
const MAX_ECHO_BYTES: usize = 64;
const ECHO_ELLIPSIS: &str = "...";
const UNPRINTABLE: char = '?';

const CRLF: &str = "\r\n";
/// `DWCommandList.getShortHelp`'s heading.
const HELP_HEADING: &str = "Possible commands:\r\n\r\n";
/// Columns `DWCommandList.colLayout` lays help out in, less one.
const HELP_COLUMNS: usize = 79;
/// Spaces `colLayout` leaves after the longest name on wide screens.
const HELP_GAP: usize = 2;
/// Width of the drive column in the `dw disk show` list (`X%-3d`).
const DRIVE_COLUMN: usize = 3;
const WRITE_PROTECT_MARK: char = '*';
const UNNAMED_IMAGE: &str = "(configured image)";

/// A successful reply: the status line, then `payload`.
pub(super) fn ok(payload: &[u8]) -> Vec<u8> {
    let mut reply = OK_STATUS.to_vec();
    reply.extend_from_slice(payload);
    reply
}

/// A failure status line, cut to fit the client's buffer.
pub(super) fn fail(failure: &Failure) -> Vec<u8> {
    let prefix = format!("{FAIL_PREFIX} {:03} ", failure.code);
    let room = MAX_STATUS_LINE_BYTES - prefix.len() - STATUS_END.len();
    let mut line = prefix.into_bytes();
    line.extend(failure.text.bytes().take(room));
    line.extend_from_slice(STATUS_END);
    line
}

/// Printable ASCII for echoing guest input in a message.
pub(super) fn sanitize(bytes: &[u8]) -> String {
    let mut text: String = bytes
        .iter()
        .take(MAX_ECHO_BYTES)
        .map(|&byte| match byte {
            b' '..=b'~' => char::from(byte),
            _ => UNPRINTABLE,
        })
        .collect();
    if bytes.len() > MAX_ECHO_BYTES {
        text.push_str(ECHO_ELLIPSIS);
    }
    text
}

/// A share failure naming the path the guest typed, never a host path.
pub(super) fn share_failure(error: ShareError, path: &[u8]) -> Failure {
    Failure::new(
        error.command_code(),
        format!("{}: {}", sanitize(path), error.message()),
    )
}

/// `dw disk insert` maps a missing image to the Java server's "file not
/// found" code; other failures keep their share code.
pub(super) fn insert_failure(error: ShareError, path: &[u8]) -> Failure {
    let mut failure = share_failure(error, path);
    if error == ShareError::NotFound {
        failure.code = code::SERVER_FILE_NOT_FOUND;
    }
    failure
}

/// The verb list for a command given without a subcommand.
pub(super) fn help(verbs: &[&str]) -> Vec<u8> {
    let width = verbs.iter().map(|verb| verb.len()).max().unwrap_or(1) + HELP_GAP;
    let per_row = (HELP_COLUMNS / width).max(1);
    let mut text = HELP_HEADING.to_string();
    for (index, verb) in verbs.iter().enumerate() {
        if index > 0 && index % per_row == 0 {
            text.push_str(CRLF);
        }
        // Writing to a `String` cannot fail.
        let _ = write!(text, "{verb:<width$}");
    }
    text.push_str(CRLF);
    text.into_bytes()
}

/// `DWCmdServerDir`'s heading line, then a blank line.
pub(super) fn dir_heading(name: &str) -> Vec<u8> {
    format!("Directory of {name}\r\n\n").into_bytes()
}

/// Converts a share listing (`name\n` lines) to CR LF lines and counts its
/// entries.
pub(super) fn dir_lines(listing: &[u8]) -> (Vec<u8>, usize) {
    let mut lines = Vec::with_capacity(listing.len() * 2);
    let mut entries = 0;
    for &byte in listing {
        if byte == b'\n' {
            lines.extend_from_slice(CRLF.as_bytes());
            entries += 1;
        } else {
            lines.push(byte);
        }
    }
    (lines, entries)
}

/// `dw disk show`: one `X<drive>` line per mounted drive, `*` marking a
/// write-protected one (`DWCmdDiskShow.doDiskShow`).
pub(super) fn disk_list<'a>(drives: impl IntoIterator<Item = (usize, &'a DriveMedia)>) -> Vec<u8> {
    let mut text = format!("{CRLF}Current DriveWire disks:{CRLF}{CRLF}");
    for (drive, media) in drives {
        let mark = if media.write_protected {
            WRITE_PROTECT_MARK
        } else {
            ' '
        };
        let _ = write!(
            text,
            "X{drive:<DRIVE_COLUMN$}{mark}{}{CRLF}",
            media_name(media)
        );
    }
    text.into_bytes()
}

/// `dw disk show N`: the drive's image and how it was mounted.
pub(super) fn disk_details(drive: usize, media: &DriveMedia) -> Vec<u8> {
    let source = match media.origin {
        MediaOrigin::Host => "VM settings",
        MediaOrigin::Guest => "dw disk insert (this session only)",
    };
    let access = if media.write_protected {
        "read-only"
    } else {
        "read/write"
    };
    format!(
        "Details for disk in drive #{drive}:{CRLF}{CRLF}{}{CRLF}{CRLF}\
         Mounted by: {source}{CRLF}Access: {access}{CRLF}",
        media_name(media)
    )
    .into_bytes()
}

fn media_name(media: &DriveMedia) -> &str {
    media.name.as_deref().unwrap_or(UNNAMED_IMAGE)
}

#[cfg(test)]
#[path = "reply_test.rs"]
mod tests;
