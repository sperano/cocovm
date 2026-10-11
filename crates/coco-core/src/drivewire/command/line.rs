//! Finding a `dw` command line at the start of a channel session.
//!
//! NitrOS-9's `dw` utility writes `dw `, then its argument line ending in
//! CR (`level1/cmds/dw.as`). The Java server's port handler collects bytes
//! up to CR, ignores NUL bytes, trims the line, ignores blank lines, and
//! starts its command thread when the first word is `dw` in any case
//! (`DWVPortHandler.takeInput`/`processAPICommand`). Lines whose first word
//! is something else are left on the channel for other host services.

const CR: u8 = b'\r';
const NUL: u8 = 0;
const COMMAND_WORD: &[u8] = b"dw";

/// Longest command line accepted, CR included: the `dw` client's own
/// response buffer is 256 bytes, and shell argument lines are no longer.
pub(super) const MAX_LINE_BYTES: usize = 256;

/// What the first bytes of a session hold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Line {
    /// Not enough bytes to tell yet.
    Undecided,
    /// The first word is not `dw`: leave the bytes alone.
    NotCommand,
    /// A `dw` line with no CR within [`MAX_LINE_BYTES`].
    TooLong,
    /// A `dw` line whose CR is the byte at this many bytes minus one.
    Complete(usize),
}

enum Scan {
    Leading,
    Word(usize),
    Arguments,
}

/// Classifies the bytes the guest has written so far.
pub(super) fn classify(bytes: impl IntoIterator<Item = u8>) -> Line {
    let mut scan = Scan::Leading;
    let mut scanned = 0;
    for (index, byte) in bytes.into_iter().take(MAX_LINE_BYTES).enumerate() {
        scanned = index + 1;
        if byte == NUL {
            continue;
        }
        let ends_word = byte == CR || byte.is_ascii_whitespace();
        scan = match scan {
            Scan::Leading if ends_word => Scan::Leading,
            Scan::Leading => Scan::Word(1),
            Scan::Word(len) if ends_word && len == COMMAND_WORD.len() => {
                if byte == CR {
                    return Line::Complete(index + 1);
                }
                Scan::Arguments
            }
            Scan::Word(_) if ends_word => return Line::NotCommand,
            Scan::Word(len) => Scan::Word(len + 1),
            Scan::Arguments if byte == CR => return Line::Complete(index + 1),
            Scan::Arguments => Scan::Arguments,
        };
        if let Scan::Word(len) = scan
            && !matches_command_prefix(byte, len)
        {
            return Line::NotCommand;
        }
    }
    match scan {
        _ if scanned < MAX_LINE_BYTES => Line::Undecided,
        Scan::Arguments => Line::TooLong,
        Scan::Leading | Scan::Word(_) => Line::NotCommand,
    }
}

/// Whether `byte`, as letter `len` of the first word, keeps it a prefix of
/// `dw` (case-insensitive).
fn matches_command_prefix(byte: u8, len: usize) -> bool {
    COMMAND_WORD
        .get(len - 1)
        .is_some_and(|expected| expected.eq_ignore_ascii_case(&byte))
}

#[cfg(test)]
#[path = "line_test.rs"]
mod tests;
