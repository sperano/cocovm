//! Bounded listener, connection workers, UI request queue, and shutdown.

use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(test)]
use super::{Action, MCP_PATH, tool_defs};
use super::{
    CONTROL_IO_TIMEOUT, CONTROL_REPLY_TIMEOUT, Incoming, MAX_CONTROL_CONNECTIONS,
    MAX_INCOMING_CONTROL_REQUESTS, Reply, ReplyHandle, Request, Response, Wake, http, mcp, tools,
};

/// How often a waiting connection checks for server shutdown.
const CONTROL_SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(25);
/// How often an accepted call checks whether its client closed the socket.
const CONTROL_DISCONNECT_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Bounds the accept thread's attempt to explain connection overload.
const CONTROL_OVERLOAD_WRITE_TIMEOUT: Duration = Duration::from_secs(1);

const CONTROL_OVERLOADED: &str = "control server overloaded; retry later";
const CONTROL_REPLY_TIMED_OUT: &str = "control request timed out";
const CONTROL_CLIENT_DISCONNECTED: &str = "control client disconnected";
const CONTROL_SHUTTING_DOWN: &str = "control server shutting down";

/// The listener and its request queue. Dropping it stops all workers.
pub struct ControlServer {
    addr: SocketAddr,
    incoming: Receiver<Incoming>,
    shutdown: Arc<AtomicBool>,
    active: Arc<Mutex<ActiveConnections>>,
    wake: Arc<WakeGate>,
    accept_thread: Option<JoinHandle<()>>,
}

impl ControlServer {
    /// Bind `127.0.0.1:port` (`0` picks a free port, see [`Self::port`]).
    pub fn bind(port: u16, wake: Wake) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        let addr = listener.local_addr()?;
        let (tx, incoming) = mpsc::sync_channel(MAX_INCOMING_CONTROL_REQUESTS);
        let shutdown = Arc::new(AtomicBool::new(false));
        let active = Arc::new(Mutex::new(ActiveConnections::default()));
        let wake = Arc::new(WakeGate::new(wake));
        let accept_thread = spawn_accept_thread(
            listener,
            tx,
            Arc::clone(&wake),
            Arc::clone(&shutdown),
            Arc::clone(&active),
        )?;
        Ok(Self {
            addr,
            incoming,
            shutdown,
            active,
            wake,
            accept_thread: Some(accept_thread),
        })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The next queued request, if any. Never blocks.
    pub fn try_recv(&self) -> Option<Incoming> {
        self.incoming.try_recv().ok()
    }

    /// Mark the prior wake handled before the UI starts a bounded drain.
    pub(crate) fn begin_dispatch(&self) {
        self.wake.clear();
    }

    /// Ensure another UI update runs after a dispatch budget is exhausted.
    pub(crate) fn request_dispatch(&self) {
        self.wake.request();
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .shutdown_all();
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

fn spawn_accept_thread(
    listener: TcpListener,
    tx: SyncSender<Incoming>,
    wake: Arc<WakeGate>,
    stop: Arc<AtomicBool>,
    active: Arc<Mutex<ActiveConnections>>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("cocovm-mcp-accept".into())
        .spawn(move || accept_loop(listener, tx, wake, stop, active))
}

fn accept_loop(
    listener: TcpListener,
    tx: SyncSender<Incoming>,
    wake: Arc<WakeGate>,
    stop: Arc<AtomicBool>,
    active: Arc<Mutex<ActiveConnections>>,
) {
    let sessions = Arc::new(Mutex::new(http::SessionStore::default()));
    let mut workers = Vec::new();
    let mut next_connection_id = 0_u64;
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        reap_finished(&mut workers);
        let connection_id = next_connection_id;
        next_connection_id = next_connection_id.wrapping_add(1);
        if !register_connection(&active, connection_id, &stream) {
            reject_overloaded(stream);
            continue;
        }
        if let Ok(worker) = spawn_connection(
            stream,
            tx.clone(),
            Arc::clone(&wake),
            Arc::clone(&sessions),
            Arc::clone(&stop),
            ConnectionGuard::new(connection_id, Arc::clone(&active)),
        ) {
            workers.push(worker);
        }
    }
    active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .shutdown_all();
    for worker in workers {
        let _ = worker.join();
    }
}

fn spawn_connection(
    stream: TcpStream,
    tx: SyncSender<Incoming>,
    wake: Arc<WakeGate>,
    sessions: Arc<Mutex<http::SessionStore>>,
    stop: Arc<AtomicBool>,
    guard: ConnectionGuard,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("cocovm-mcp-conn".into())
        .spawn(move || {
            let _guard = guard;
            serve_connection(stream, tx, wake, sessions, stop);
        })
}

#[derive(Default)]
struct ActiveConnections {
    streams: HashMap<u64, TcpStream>,
}

impl ActiveConnections {
    fn remove(&mut self, id: u64) {
        self.streams.remove(&id);
    }

    fn shutdown_all(&self) {
        for stream in self.streams.values() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

struct ConnectionGuard {
    active: Arc<Mutex<ActiveConnections>>,
    id: u64,
}

impl ConnectionGuard {
    fn new(id: u64, active: Arc<Mutex<ActiveConnections>>) -> Self {
        Self { active, id }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(self.id);
    }
}

fn register_connection(
    active: &Arc<Mutex<ActiveConnections>>,
    id: u64,
    stream: &TcpStream,
) -> bool {
    let mut active = active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if active.streams.len() >= MAX_CONTROL_CONNECTIONS {
        return false;
    }
    let Ok(shutdown_handle) = stream.try_clone() else {
        return false;
    };
    active.streams.insert(id, shutdown_handle);
    true
}

fn reject_overloaded(mut stream: TcpStream) {
    let _ = stream.set_write_timeout(Some(CONTROL_OVERLOAD_WRITE_TIMEOUT));
    let _ = http::write_close_response(
        &mut stream,
        503,
        "text/plain",
        CONTROL_OVERLOADED.as_bytes(),
    );
}

fn reap_finished(workers: &mut Vec<JoinHandle<()>>) {
    let mut running = Vec::with_capacity(workers.len());
    for worker in std::mem::take(workers) {
        if worker.is_finished() {
            let _ = worker.join();
        } else {
            running.push(worker);
        }
    }
    *workers = running;
}

fn serve_connection(
    stream: TcpStream,
    tx: SyncSender<Incoming>,
    wake: Arc<WakeGate>,
    sessions: Arc<Mutex<http::SessionStore>>,
    stop: Arc<AtomicBool>,
) {
    if stream.set_read_timeout(Some(CONTROL_IO_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(CONTROL_IO_TIMEOUT)).is_err()
    {
        return;
    }
    let Ok(disconnect_probe) = stream.try_clone() else {
        return;
    };
    let backend = QueueBackend {
        tx,
        wake,
        stop,
        disconnect_probe: Some(disconnect_probe),
    };
    let mut mcp = mcp::Mcp::new(Box::new(backend));
    http::serve_connection(stream, &mut mcp, &sessions);
}

struct ReplyWait {
    rx: Receiver<Response>,
    abandoned: Arc<AtomicBool>,
}

impl ReplyWait {
    fn pair() -> (ReplyHandle, Self) {
        let (tx, rx) = mpsc::channel();
        let abandoned = Arc::new(AtomicBool::new(false));
        (
            ReplyHandle {
                tx,
                abandoned: Arc::clone(&abandoned),
            },
            Self { rx, abandoned },
        )
    }

    fn abandon(&self) {
        self.abandoned.store(true, Ordering::Release);
    }
}

struct QueueBackend {
    tx: SyncSender<Incoming>,
    wake: Arc<WakeGate>,
    stop: Arc<AtomicBool>,
    disconnect_probe: Option<TcpStream>,
}

impl tools::Backend for QueueBackend {
    fn call(&mut self, req: &Request) -> Result<Reply, String> {
        let (reply, wait) = ReplyWait::pair();
        let incoming = Incoming {
            request: req.clone(),
            reply,
        };
        match self.tx.try_send(incoming) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(CONTROL_OVERLOADED.to_string()),
            Err(TrySendError::Disconnected(_)) => return Err(CONTROL_SHUTTING_DOWN.to_string()),
        }
        self.wake.request();
        self.wait_for_reply(wait)
    }
}

impl QueueBackend {
    fn wait_for_reply(&self, wait: ReplyWait) -> Result<Reply, String> {
        self.wait_for_reply_until(wait, Instant::now() + CONTROL_REPLY_TIMEOUT)
    }

    fn wait_for_reply_until(&self, wait: ReplyWait, deadline: Instant) -> Result<Reply, String> {
        let mut next_disconnect_check = Instant::now() + CONTROL_DISCONNECT_POLL_INTERVAL;
        loop {
            match wait.rx.recv_timeout(CONTROL_SHUTDOWN_POLL_INTERVAL) {
                Ok(Response::Ok(reply)) => return Ok(reply),
                Ok(Response::Err(msg)) => return Err(msg),
                Err(RecvTimeoutError::Disconnected) => {
                    wait.abandon();
                    return Err(CONTROL_SHUTTING_DOWN.to_string());
                }
                Err(RecvTimeoutError::Timeout) if self.stop.load(Ordering::Acquire) => {
                    wait.abandon();
                    return Err(CONTROL_SHUTTING_DOWN.to_string());
                }
                Err(RecvTimeoutError::Timeout)
                    if Instant::now() >= next_disconnect_check && self.client_disconnected() =>
                {
                    wait.abandon();
                    return Err(CONTROL_CLIENT_DISCONNECTED.to_string());
                }
                Err(RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                    wait.abandon();
                    return Err(CONTROL_REPLY_TIMED_OUT.to_string());
                }
                Err(RecvTimeoutError::Timeout) => {
                    if Instant::now() >= next_disconnect_check {
                        next_disconnect_check = Instant::now() + CONTROL_DISCONNECT_POLL_INTERVAL;
                    }
                }
            }
        }
    }

    fn client_disconnected(&self) -> bool {
        self.disconnect_probe
            .as_ref()
            .is_some_and(socket_disconnected)
    }
}

fn socket_disconnected(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let mut byte = [0_u8; 1];
    let result = stream.peek(&mut byte);
    if stream.set_nonblocking(false).is_err() {
        return true;
    }
    match result {
        Ok(0) => true,
        Ok(_) => false,
        Err(error) => !matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
        ),
    }
}

struct WakeGate {
    wake: Wake,
    pending: AtomicBool,
}

impl WakeGate {
    fn new(wake: Wake) -> Self {
        Self {
            wake,
            pending: AtomicBool::new(false),
        }
    }

    fn request(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            (self.wake)();
        }
    }

    fn clear(&self) {
        self.pending.store(false, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "control_test.rs"]
mod tests;
