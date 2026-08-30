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

use std::collections::HashSet;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

pub mod http;
pub mod jsonrpc;
pub mod key_names;
pub mod mcp;
pub mod protocol;
pub mod tool_defs;
pub mod tools;

pub use protocol::{Action, Reply, Request, Response, Stick, VmInfo, VmStatus};

/// Loopback port the app listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 6809;
/// Environment variable naming the control port, read by `cli.rs`.
pub const PORT_ENV: &str = "COCOVM_CONTROL_PORT";
/// Path the MCP endpoint answers on; anything else is a 404.
pub(crate) const MCP_PATH: &str = "/mcp";
/// Upper bound on a single `wait` request, in fields (one minute at 60 Hz).
pub const MAX_WAIT_FIELDS: u32 = 3600;
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
/// Most characters a single `type_text` may queue — about a minute of
/// typing at the app's tap timing, so the deferred reply stays bounded.
pub const MAX_TYPE_TEXT_CHARS: usize = 600;

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
pub struct ReplyHandle(Sender<Response>);

impl ReplyHandle {
    /// Build a handle directly from its reply channel — for tests that need
    /// a [`ReplyHandle`]/[`Incoming`] without a real HTTP connection.
    pub(crate) fn new(tx: Sender<Response>) -> Self {
        Self(tx)
    }

    /// Send the reply; a connection that has since closed is not an error.
    pub fn reply(self, response: Response) {
        let _ = self.0.send(response);
    }
}

/// The listener and its request queue. Dropping it stops accepting.
pub struct ControlServer {
    addr: SocketAddr,
    incoming: Receiver<Incoming>,
    shutdown: Arc<AtomicBool>,
}

impl ControlServer {
    /// Bind `127.0.0.1:port` (`0` picks a free port, see [`Self::port`]).
    pub fn bind(port: u16, wake: Wake) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        let addr = listener.local_addr()?;
        let (tx, incoming) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&shutdown);
        thread::Builder::new()
            .name("cocovm-mcp-accept".into())
            .spawn(move || accept_loop(listener, tx, wake, stop))?;
        Ok(Self {
            addr,
            incoming,
            shutdown,
        })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The next queued request, if any. Never blocks.
    pub fn try_recv(&self) -> Option<Incoming> {
        self.incoming.try_recv().ok()
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Unblock `accept` so the thread sees the flag and exits.
        let _ = TcpStream::connect(self.addr);
    }
}

fn accept_loop(listener: TcpListener, tx: Sender<Incoming>, wake: Wake, stop: Arc<AtomicBool>) {
    let sessions: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        let tx = tx.clone();
        let wake = Arc::clone(&wake);
        let sessions = Arc::clone(&sessions);
        let _ = thread::Builder::new()
            .name("cocovm-mcp-conn".into())
            .spawn(move || serve_connection(stream, tx, wake, sessions));
    }
}

/// One connection: an HTTP client bound to the MCP endpoint. Its
/// `tools/call` requests are relayed to the frame loop through `tx`/`wake`
/// and blocked on ([`QueueBackend`]); `initialize`/`tools/list`/notifications
/// are answered inline ([`mcp::Mcp`]).
fn serve_connection(
    stream: TcpStream,
    tx: Sender<Incoming>,
    wake: Wake,
    sessions: Arc<Mutex<HashSet<String>>>,
) {
    let backend = QueueBackend { tx, wake };
    let mut mcp = mcp::Mcp::new(Box::new(backend));
    http::serve_connection(stream, &mut mcp, &sessions);
}

/// The real [`tools::Backend`]: queues a [`Request`] for the frame loop and
/// blocks for its [`Response`].
struct QueueBackend {
    tx: Sender<Incoming>,
    wake: Wake,
}

impl tools::Backend for QueueBackend {
    fn call(&mut self, req: &Request) -> Result<Reply, String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let incoming = Incoming {
            request: req.clone(),
            reply: ReplyHandle::new(reply_tx),
        };
        if self.tx.send(incoming).is_err() {
            return Err("cocovm dropped the request".to_string());
        }
        (self.wake)();
        match reply_rx.recv() {
            Ok(Response::Ok(reply)) => Ok(reply),
            Ok(Response::Err(msg)) => Err(msg),
            Err(_) => Err("cocovm dropped the request".to_string()),
        }
    }
}

#[cfg(test)]
#[path = "control/control_test.rs"]
mod tests;
