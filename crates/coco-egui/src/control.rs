//! The app's built-in MCP server: `cocovm` itself answers MCP over HTTP
//! ("streamable HTTP" transport, JSON responses only) on `127.0.0.1:<port>`,
//! rather than a separate proxy process. An accept thread ([`ControlServer`])
//! hands each connection to its own thread, which parses HTTP and JSON-RPC
//! and answers `initialize`/`ping`/`tools/list`/notifications itself
//! (`control::http`, `control::jsonrpc`, `control::mcp`); a `tools/call`
//! becomes a [`Request`] queued for the frame loop
//! (`manager::control::drain_control`). The connection thread blocks on the
//! [`ReplyHandle`] channel for that request's [`Response`], then
//! [`control::tools`] formats it into MCP content.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

pub mod http;
pub mod jsonrpc;
pub mod key_names;
pub mod mcp;
pub mod protocol;
mod server;
pub mod tool_defs;
pub mod tools;

pub use protocol::{
    Action, ControlError, Reply, Request, Response, ScreenSnapshot, Stick, TextMatcher, VmInfo,
    VmStatus,
};
pub use server::ControlServer;

/// Loopback port the app listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 6809;
/// Environment variable naming the control port, read by `cli.rs`.
pub const PORT_ENV: &str = "COCOVM_CONTROL_PORT";
/// Path the MCP endpoint answers on; anything else is a 404.
pub(crate) const MCP_PATH: &str = "/mcp";
/// Upper bound on a single `wait` request, in fields (one minute at 60 Hz).
pub const MAX_WAIT_FIELDS: u32 = 3600;
/// Most characters accepted in a `wait_for_text` literal or regex pattern.
pub const MAX_WAIT_PATTERN_CHARS: usize = 1024;
/// Upper bound on a single `press_keys` hold, in fields.
pub const MAX_HOLD_FIELDS: u32 = 600;
/// Default hold for `press_keys` when the request gives none: long enough
/// for the ROM's keyboard scan to register the tap (see `TYPE_HOLD_FIELDS`
/// in `main.rs`, which this mirrors).
pub const DEFAULT_HOLD_FIELDS: u32 = 2;
/// Largest `peek` a single request may ask for.
pub const MAX_PEEK_LEN: u16 = 4096;
/// Most bytes a single `poke` may write.
pub const MAX_POKE_LEN: usize = 4096;
/// Most characters a single `type_text` may queue — about a minute of typing
/// at the nominal tap pace, so the deferred reply stays bounded.
pub const MAX_TYPE_TEXT_CHARS: usize = 600;
/// Most accepted client sockets served at once. Additional clients receive
/// HTTP 503 without getting a connection thread.
pub(crate) const MAX_CONTROL_CONNECTIONS: usize = 32;
/// Most `tools/call` requests waiting for the UI thread.
pub(crate) const MAX_INCOMING_CONTROL_REQUESTS: usize = 16;
/// Most HTTP sessions retained across all connections.
pub(crate) const MAX_CONTROL_SESSIONS: usize = 64;
/// Idle HTTP session lifetime. A later request with an expired ID receives
/// the same 404 as any other unknown session.
pub(crate) const CONTROL_SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Bounds reads from slow clients and writes to clients that stop reading.
pub(crate) const CONTROL_IO_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest a connection thread waits for the UI to answer an accepted call.
/// This exceeds the nominal length of any deferred operation and its margin;
/// a `type_text` into a target that stalls between keys can still hit it.
pub(crate) const CONTROL_REPLY_TIMEOUT: Duration = Duration::from_secs(90);
/// Called from the accept/connection threads whenever a request lands, so a
/// frame loop that only repaints on events wakes up to service it.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// A request waiting for the frame loop, with the channel its reply goes
/// back on. Holding it past the frame it arrived in is the deferral
/// mechanism: reply whenever the work is done.
pub struct Incoming {
    pub request: Request,
    reply: ReplyHandle,
}

impl Incoming {
    /// Build one directly from its parts — for tests that need an
    /// [`Incoming`] without a real HTTP connection behind it (see
    /// [`ReplyHandle::new`]).
    #[cfg(test)]
    pub(crate) fn new(request: Request, reply: ReplyHandle) -> Self {
        Self { request, reply }
    }

    /// Send the reply; a connection that has since closed is not an error.
    /// Production code goes through [`Self::into_parts`] instead, since a
    /// deferred request needs its [`ReplyHandle`] kept past the frame the
    /// request arrived in — this is the direct, immediate-reply shape tests
    /// use.
    #[cfg(test)]
    pub(crate) fn reply(self, response: Response) {
        self.reply.reply(response);
    }

    /// Split into the request (to consume) and the handle that answers it.
    pub fn into_parts(self) -> (Request, ReplyHandle) {
        (self.request, self.reply)
    }
}

/// The channel a request's reply goes back on, once it is done.
pub struct ReplyHandle {
    tx: mpsc::Sender<Response>,
    abandoned: Arc<AtomicBool>,
}

impl ReplyHandle {
    /// Build a handle directly from its reply channel — for tests that need
    /// a [`ReplyHandle`]/[`Incoming`] without a real HTTP connection.
    #[cfg(test)]
    pub(crate) fn new(tx: mpsc::Sender<Response>) -> Self {
        Self {
            tx,
            abandoned: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Send the reply; a connection that has since closed is not an error.
    pub fn reply(self, response: Response) {
        let _ = self.tx.send(response);
    }

    /// Whether the connection stopped waiting for this reply.
    pub(crate) fn is_abandoned(&self) -> bool {
        self.abandoned.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn abandon_for_test(&self) {
        self.abandoned.store(true, Ordering::Release);
    }
}
