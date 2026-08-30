//! The control protocol between a running `cocovm` app and an external
//! driver such as `cocovm-mcp`: newline-delimited JSON over a loopback TCP
//! connection, one request in flight per connection.
//!
//! The app side ([`server`]) accepts connections on a background thread and
//! hands each [`Request`] to the frame loop, which applies it to the VM it
//! names and replies — immediately, or once the VM has finished the work
//! (typed text drained, keys released, fields elapsed). The driver side
//! ([`client`]) is a blocking request/response call.

pub mod client;
pub mod framing;
pub mod key_names;
pub mod protocol;
pub mod server;

pub use client::{ControlClient, ControlError};
pub use protocol::{Action, Reply, Request, Response, Stick, VmInfo, VmStatus};
pub use server::{ControlServer, Incoming, ReplyHandle};

/// Loopback port the app listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 6809;
/// Environment variable naming the control port, read by both ends.
pub const PORT_ENV: &str = "COCOVM_CONTROL_PORT";
/// Upper bound on a single `wait` request, in fields (one minute at 60 Hz).
pub const MAX_WAIT_FIELDS: u32 = 3600;
/// Upper bound on a single `press_keys` hold, in fields.
pub const MAX_HOLD_FIELDS: u32 = 600;
/// Default hold for `press_keys` when the request gives none: long enough
/// for the ROM's keyboard scan to register the tap (see `TYPE_HOLD_FIELDS`
/// in coco-egui, which this mirrors).
pub const DEFAULT_HOLD_FIELDS: u32 = 2;
/// Largest `peek` a single request may ask for.
pub const MAX_PEEK_LEN: u16 = 4096;
/// Most bytes a single `poke` may write.
pub const MAX_POKE_LEN: usize = 4096;
/// Most characters a single `type_text` may queue — about a minute of
/// typing at the app's tap timing, so the deferred reply stays bounded.
pub const MAX_TYPE_TEXT_CHARS: usize = 600;
