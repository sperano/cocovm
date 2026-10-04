//! Internal request/reply types passed between the MCP layer
//! (`control::tools`) and the manager (`manager::control`). No longer a wire
//! format — everything crosses in-process on an `mpsc` channel — so only
//! [`Stick`] keeps a `Deserialize` impl, for parsing a tool call's arguments.

use serde::Deserialize;

/// A literal or compiled regex used by `wait_for_text`.
#[derive(Clone, Debug)]
pub enum TextMatcher {
    Literal(String),
    Regex { source: String, regex: regex::Regex },
}

impl TextMatcher {
    pub fn new(pattern: String, regex: bool) -> Result<Self, String> {
        let length = pattern.chars().count();
        if length > super::MAX_WAIT_PATTERN_CHARS {
            return Err(format!(
                "pattern is {length} characters; at most {} per call",
                super::MAX_WAIT_PATTERN_CHARS
            ));
        }
        if regex {
            let compiled = regex::Regex::new(&pattern)
                .map_err(|error| format!("invalid regular expression: {error}"))?;
            Ok(Self::Regex {
                source: pattern,
                regex: compiled,
            })
        } else {
            Ok(Self::Literal(pattern))
        }
    }

    pub fn is_match(&self, screen: &ScreenSnapshot) -> bool {
        let text = screen.lines.join("\n");
        match self {
            Self::Literal(pattern) => text.contains(pattern),
            Self::Regex { regex, .. } => regex.is_match(&text),
        }
    }
}

impl PartialEq for TextMatcher {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Literal(left), Self::Literal(right)) => left == right,
            (Self::Regex { source: left, .. }, Self::Regex { source: right, .. }) => left == right,
            _ => false,
        }
    }
}

impl Eq for TextMatcher {}

/// One request from a tool call. `vm` names the target by manager slug;
/// `None` selects "the only running VM", an error when there are several.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub vm: Option<String>,
    pub action: Action,
}

/// What a request asks the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Every machine the manager knows, with its lifecycle state.
    ListVms,
    /// Start (or resume) the named VM; a no-op if it is already running.
    StartVm,
    /// The text screen decoded as lines, plus the video-mode summary.
    ScreenText,
    /// The current framebuffer as a PNG.
    Screenshot,
    /// Type `text` through the symbolic type-ahead; replies once drained.
    TypeText {
        text: String,
    },
    /// Hold every named key together for `hold_fields` fields, then release.
    PressKeys {
        keys: Vec<String>,
        hold_fields: Option<u32>,
    },
    /// Set joystick axes/buttons; `None` leaves that input as it was.
    /// `release` hands the port back to the host's own input source.
    Joystick {
        stick: Stick,
        x: Option<u8>,
        y: Option<u8>,
        button1: Option<bool>,
        button2: Option<bool>,
        release: bool,
    },
    /// Mount the disk image at `path` in floppy `drive` (0-based).
    InsertDisk {
        drive: usize,
        path: String,
    },
    EjectDisk {
        drive: usize,
    },
    /// Reset the machine; `hard` power-cycles (clears RAM).
    Reset {
        hard: bool,
    },
    /// Pause (`false`) or resume (`true`) emulation.
    SetRunning {
        running: bool,
    },
    /// Let `fields` video fields elapse before replying.
    Wait {
        fields: u32,
    },
    /// Wait until decoded screen text matches, or until `timeout_fields` pass.
    WaitForText {
        matcher: TextMatcher,
        timeout_fields: u32,
    },
    /// Read `len` bytes from `addr` without side effects.
    Peek {
        addr: u16,
        len: u16,
    },
    /// Write `bytes` starting at `addr`, with bus side effects.
    Poke {
        addr: u16,
        bytes: Vec<u8>,
    },
}

/// Which joystick port a [`Action::Joystick`] request drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stick {
    Left,
    Right,
}

impl Stick {
    /// The `coco_core::joystick` port index.
    pub fn index(self) -> usize {
        match self {
            Stick::Left => coco_core::joystick::LEFT,
            Stick::Right => coco_core::joystick::RIGHT,
        }
    }
}

/// One reply from the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Ok(Reply),
    Err(ControlError),
}

/// A failed request, optionally carrying the last decoded screen snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlError {
    /// Human-readable error summary.
    pub message: String,
    /// Last decoded screen for condition-wait timeouts.
    pub screen: Option<ScreenSnapshot>,
}

impl ControlError {
    pub fn with_screen(message: impl Into<String>, screen: ScreenSnapshot) -> Self {
        Self {
            message: message.into(),
            screen: Some(screen),
        }
    }
}

impl From<String> for ControlError {
    fn from(message: String) -> Self {
        Self {
            message,
            screen: None,
        }
    }
}

impl From<&str> for ControlError {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

impl std::ops::Deref for ControlError {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ControlError {}

/// The successful payload of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// The action completed with nothing to report.
    Done,
    Vms(Vec<VmInfo>),
    Screen(ScreenSnapshot),
    Screenshot {
        png_base64: String,
        width: u32,
        height: u32,
    },
    Bytes(Vec<u8>),
}

/// A zero-based insertion position in a decoded text screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenCursor {
    /// Character row from the top of the decoded screen.
    pub row: usize,
    /// Character column from the left edge of the decoded screen.
    pub column: usize,
}

/// Decoded screen text, video mode, and a validated insertion position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenSnapshot {
    /// Fixed-width decoded character rows.
    pub lines: Vec<String>,
    /// Diagnostic summary of the active video mode.
    pub mode: String,
    /// Validated ROM insertion position, or `None` outside known conventions.
    pub cursor: Option<ScreenCursor>,
}

/// One manager entry as [`Action::ListVms`] reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmInfo {
    pub slug: String,
    pub name: String,
    pub status: VmStatus,
}

/// A manager entry's lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmStatus {
    Running,
    Suspended,
    PoweredOff,
}

impl VmStatus {
    /// The wire spelling, for human-readable listings.
    pub fn as_str(self) -> &'static str {
        match self {
            VmStatus::Running => "running",
            VmStatus::Suspended => "suspended",
            VmStatus::PoweredOff => "powered_off",
        }
    }
}

#[cfg(test)]
#[path = "protocol_test.rs"]
mod tests;
