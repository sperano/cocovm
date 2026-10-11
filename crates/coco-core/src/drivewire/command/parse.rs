//! Parsing a `dw` command line into one of the implemented commands.
//!
//! Grammar and messages follow the DriveWire 4 Java server
//! (`DWCommandList.parse`, `DWCmdServerDir`, `DWCmdServerList`,
//! `DWCmdDiskShow`, `DWCmdDiskInsert`, `DWCmdDiskEject`): verbs match in
//! any case and may be shortened to a unique prefix, a missing verb lists
//! the possible ones, and a path is the rest of the line, so it may contain
//! spaces. Only `server dir`, `server list`, and `disk show/insert/eject`
//! exist here; other Java verbs are unknown commands.

use super::super::DRIVE_COUNT;
use super::reply::{help, sanitize};

/// `dw` command result codes, from the Java server's `DWDefs.RC_*`.
pub(crate) mod code {
    pub const SYNTAX_ERROR: u16 = 10;
    pub const INVALID_DRIVE: u16 = 101;
    pub const DRIVE_NOT_LOADED: u16 = 102;
    pub const SERVER_FILE_NOT_FOUND: u16 = 203;
    /// `RC_SERVER_NOT_READY`: the host service cannot take the request.
    pub const SERVER_NOT_READY: u16 = 205;
}

const TOP_VERBS: &[&str] = &["disk", "server"];
const SERVER_VERBS: &[&str] = &["dir", "list"];
const DISK_VERBS: &[&str] = &["eject", "insert", "show"];
const EJECT_ALL: &str = "all";

/// A parsed command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Command {
    /// A ready-made successful reply: help for a verb with no subcommand.
    Text(Vec<u8>),
    ServerDir {
        path: Vec<u8>,
    },
    ServerList {
        path: Vec<u8>,
    },
    DiskShow {
        drive: Option<usize>,
    },
    DiskInsert {
        drive: usize,
        path: Vec<u8>,
    },
    DiskEject {
        drive: Option<usize>,
    },
}

/// A failed command: its result code and explanation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Failure {
    pub code: u16,
    pub text: String,
}

impl Failure {
    pub fn new(code: u16, text: impl Into<String>) -> Self {
        Self {
            code,
            text: text.into(),
        }
    }

    fn syntax(text: impl Into<String>) -> Self {
        Self::new(code::SYNTAX_ERROR, text)
    }
}

/// Parses a complete line whose first word is `dw` (see `line::classify`).
pub(super) fn parse(line: &[u8]) -> Result<Command, Failure> {
    let (_, rest) = split_word(line);
    let (word, rest) = split_word(rest);
    if word.is_empty() {
        return Ok(Command::Text(help(TOP_VERBS)));
    }
    match verb(word, TOP_VERBS)? {
        "server" => parse_server(rest),
        _ => parse_disk(rest),
    }
}

fn parse_server(line: &[u8]) -> Result<Command, Failure> {
    let (word, rest) = split_word(line);
    if word.is_empty() {
        return Ok(Command::Text(help(SERVER_VERBS)));
    }
    let name = verb(word, SERVER_VERBS)?;
    let path = rest.trim_ascii().to_vec();
    if path.is_empty() {
        return Err(Failure::syntax(format!(
            "dw server {name} requires a path as an argument"
        )));
    }
    Ok(match name {
        "dir" => Command::ServerDir { path },
        _ => Command::ServerList { path },
    })
}

fn parse_disk(line: &[u8]) -> Result<Command, Failure> {
    let (word, rest) = split_word(line);
    if word.is_empty() {
        return Ok(Command::Text(help(DISK_VERBS)));
    }
    let (argument, rest) = split_word(rest);
    let rest = rest.trim_ascii();
    match verb(word, DISK_VERBS)? {
        "show" if argument.is_empty() => Ok(Command::DiskShow { drive: None }),
        "show" if rest.is_empty() => Ok(Command::DiskShow {
            drive: Some(drive_number(argument)?),
        }),
        "insert" if !argument.is_empty() && !rest.is_empty() => Ok(Command::DiskInsert {
            drive: drive_number(argument)?,
            path: rest.to_vec(),
        }),
        "eject" if !argument.is_empty() && rest.is_empty() => {
            // Java compares `all` case-sensitively, unlike the verbs.
            let drive = if argument == EJECT_ALL.as_bytes() {
                None
            } else {
                Some(drive_number(argument)?)
            };
            Ok(Command::DiskEject { drive })
        }
        _ => Err(Failure::syntax("Syntax error")),
    }
}

/// Splits off the first word, skipping leading whitespace.
fn split_word(line: &[u8]) -> (&[u8], &[u8]) {
    let line = line.trim_ascii_start();
    let end = line
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(line.len());
    line.split_at(end)
}

/// The verb in `verbs` that `word` names or uniquely abbreviates.
fn verb(word: &[u8], verbs: &[&'static str]) -> Result<&'static str, Failure> {
    let matches: Vec<&'static str> = verbs
        .iter()
        .copied()
        .filter(|verb| {
            verb.len() >= word.len() && verb.as_bytes()[..word.len()].eq_ignore_ascii_case(word)
        })
        .collect();
    match matches.as_slice() {
        [verb] => Ok(verb),
        [] => Err(Failure::syntax(format!(
            "Unknown command '{}'",
            sanitize(word)
        ))),
        _ => Err(Failure::syntax(format!(
            "Ambiguous command, '{}' matches {}",
            sanitize(word),
            matches.join(" or ")
        ))),
    }
}

/// A drive number from 0 to `DRIVE_COUNT - 1`.
fn drive_number(word: &[u8]) -> Result<usize, Failure> {
    let number = std::str::from_utf8(word)
        .ok()
        .and_then(|text| text.parse::<i64>().ok())
        .ok_or_else(|| Failure::new(code::INVALID_DRIVE, "Drive numbers must be numeric"))?;
    usize::try_from(number)
        .ok()
        .filter(|&drive| drive < DRIVE_COUNT)
        .ok_or_else(|| {
            Failure::new(
                code::INVALID_DRIVE,
                format!(
                    "There is no drive {number}. Valid drive numbers are 0 - {}",
                    DRIVE_COUNT - 1
                ),
            )
        })
}

#[cfg(test)]
#[path = "parse_test.rs"]
mod tests;
