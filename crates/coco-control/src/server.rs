//! The app-side listener: accepts loopback connections on a background
//! thread and queues their requests for the frame loop.

use std::io::{self, BufReader, BufWriter};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::framing::{read_message, write_message};
use crate::protocol::{Request, Response};

/// Called from the accept/reader threads whenever a request lands, so a
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
    /// Send the reply; a connection that has since closed is not an error.
    pub fn reply(self, response: Response) {
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
            .name("coco-control-accept".into())
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
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        let tx = tx.clone();
        let wake = Arc::clone(&wake);
        let _ = thread::Builder::new()
            .name("coco-control-conn".into())
            .spawn(move || serve_connection(stream, tx, wake));
    }
}

/// One connection: read a request, queue it, wait for the reply, write it,
/// repeat. Any malformed line ends the connection — that also keeps a
/// browser's `fetch` to this port (whose first line is an HTTP request
/// line) from ever reaching the queue.
fn serve_connection(stream: TcpStream, tx: Sender<Incoming>, wake: Wake) {
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut writer = BufWriter::new(write_half);
    while let Ok(Some(request)) = read_message::<Request>(&mut reader) {
        let (reply_tx, reply_rx) = mpsc::channel();
        let incoming = Incoming {
            request,
            reply: ReplyHandle(reply_tx),
        };
        if tx.send(incoming).is_err() {
            return;
        }
        wake();
        let Ok(response) = reply_rx.recv() else {
            return;
        };
        if write_message(&mut writer, &response).is_err() {
            return;
        }
    }
}

#[cfg(test)]
#[path = "server_test.rs"]
mod tests;
