//! Names a driver may use in `press_keys`, resolved to matrix positions.

use coco_core::keyboard::{self as kbd, Pos};

/// The named (non-character) keys, as `(name, position)`.
pub const NAMED_KEYS: &[(&str, Pos)] = &[
    ("ENTER", kbd::ENTER),
    ("SPACE", kbd::SPACE),
    ("BREAK", kbd::BREAK),
    ("CLEAR", kbd::CLEAR),
    ("UP", kbd::UP),
    ("DOWN", kbd::DOWN),
    ("LEFT", kbd::LEFT),
    ("RIGHT", kbd::RIGHT),
    ("SHIFT", kbd::SHIFT),
    ("ALT", kbd::ALT),
    ("CTRL", kbd::CTRL),
    ("F1", kbd::F1),
    ("F2", kbd::F2),
];

/// Resolve a key name: one of [`NAMED_KEYS`] (case-insensitive) or a single
/// printable character. Returns the position and whether SHIFT must be held
/// with it (as for `!`, which is SHIFT+1).
pub fn key_pos(name: &str) -> Option<(Pos, bool)> {
    let upper = name.to_ascii_uppercase();
    if let Some((_, pos)) = NAMED_KEYS.iter().find(|(n, _)| *n == upper) {
        return Some((*pos, false));
    }
    let mut chars = name.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    kbd::char_key(c)
}

/// Human-readable list of the accepted names, for tool descriptions.
pub fn describe() -> String {
    let named: Vec<&str> = NAMED_KEYS.iter().map(|(n, _)| *n).collect();
    format!(
        "{} or any single character (letters, digits, punctuation)",
        named.join(", ")
    )
}

#[cfg(test)]
#[path = "key_names_test.rs"]
mod tests;
