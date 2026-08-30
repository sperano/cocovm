//! The driver-side connection: one blocking request/response at a time.

use std::io::{self, BufReader, BufWriter};
use std::net::{Ipv4Addr, TcpStream};
use std::time::Duration;

use crate::framing::{read_message, write_message};
use crate::protocol::{Reply, Request, Response};

/// How long a single request may take end to end. Deferred requests
/// (`wait`, `type_text`) are bounded by the app well inside this.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug)]
pub enum ControlError {
    /// The connection failed or dropped.
    Io(io::Error),
    /// The app replied with an error message.
    Remote(String),
    /// The stream ended before a reply arrived.
    Disconnected,
    /// No reply within [`REQUEST_TIMEOUT`]; the request may still be running.
    Timeout,
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ControlError::Io(e) => write!(f, "control connection: {e}"),
            ControlError::Remote(msg) => f.write_str(msg),
            ControlError::Disconnected => f.write_str("cocovm closed the control connection"),
            ControlError::Timeout => write!(
                f,
                "no reply from cocovm within {} s",
                REQUEST_TIMEOUT.as_secs()
            ),
        }
    }
}

impl std::error::Error for ControlError {}

impl From<io::Error> for ControlError {
    fn from(e: io::Error) -> Self {
        ControlError::Io(e)
    }
}

pub struct ControlClient {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

impl ControlClient {
    /// Connect to the app on `127.0.0.1:port`.
    pub fn connect(port: u16) -> io::Result<Self> {
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
        stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
        stream.set_nodelay(true)?;
        let writer = BufWriter::new(stream.try_clone()?);
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
        })
    }

    /// Send `request` and block for its reply.
    pub fn call(&mut self, request: &Request) -> Result<Reply, ControlError> {
        write_message(&mut self.writer, request)?;
        match read_message::<Response>(&mut self.reader) {
            Ok(Some(Response::Ok(reply))) => Ok(reply),
            Ok(Some(Response::Err(msg))) => Err(ControlError::Remote(msg)),
            Ok(None) => Err(ControlError::Disconnected),
            // A timeout isn't a stale connection: the request was delivered
            // and may still be running, so callers must not resend it.
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                Err(ControlError::Timeout)
            }
            Err(e) => Err(e.into()),
        }
    }
}
