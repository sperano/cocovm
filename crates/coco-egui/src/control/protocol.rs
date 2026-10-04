//! Internal request/reply types passed between the MCP layer
//! (`control::tools`) and the manager (`manager::control`). No longer a wire
//! format — everything crosses in-process on an `mpsc` channel — so only
//! [`Stick`] keeps a `Deserialize` impl, for parsing a tool call's arguments.

use serde::Deserialize;

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
    Err(String),
}

/// The successful payload of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// The action completed with nothing to report.
    Done,
    Vms(Vec<VmInfo>),
    Screen {
        lines: Vec<String>,
        mode: String,
    },
    Screenshot {
        png_base64: String,
        width: u32,
        height: u32,
    },
    Bytes(Vec<u8>),
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
    /// Every state, in declaration order.
    pub const ALL: [VmStatus; 3] = [VmStatus::Running, VmStatus::Suspended, VmStatus::PoweredOff];

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
