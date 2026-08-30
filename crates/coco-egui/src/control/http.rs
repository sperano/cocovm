//! Hand-rolled HTTP/1.1, just enough to carry MCP's "streamable HTTP"
//! transport (JSON responses only — no SSE, no chunked/streaming bodies).
//! [`parse_request`]/[`write_response`] are the wire-level primitives;
//! [`serve_connection`] is the per-connection loop: parse a request, route
//! it, dispatch its body through [`crate::control::jsonrpc::dispatch`], and
//! reply — repeating until the client closes the connection.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::MCP_PATH;
use super::jsonrpc::{self, Handler};

/// Longest request line + headers accepted, before the connection is closed
/// with a 400.
const MAX_HEADER_BYTES: u64 = 16 * 1024;
/// Longest request body accepted (a `poke`'s bytes are the largest
/// legitimate `tools/call` body and sit far below this).
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Hex digits `generate_session_id` produces — 64 bits of a counter plus 64
/// bits of wall-clock nanoseconds, so distinct only across connections, not
/// cryptographically unguessable; the `Origin` check is the real defense.
const SESSION_ID_HEX_CHARS: usize = 32;

/// One parsed HTTP request. Header names are lower-cased so callers don't
/// have to case-fold themselves.
#[derive(Debug)]
pub(crate) struct HttpRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: HashMap<String, String>,
    pub(crate) body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// Read one HTTP request: the request line, headers up to a blank line, then
/// exactly `Content-Length` body bytes. `Ok(None)` at a clean EOF between
/// requests (keep-alive's idle close). A `chunked` body or headers over
/// [`MAX_HEADER_BYTES`]/a body over [`MAX_BODY_BYTES`] is `Err`.
pub(crate) fn parse_request(reader: &mut impl BufRead) -> io::Result<Option<HttpRequest>> {
    let mut budget = (&mut *reader).take(MAX_HEADER_BYTES);
    let mut line = String::new();
    if budget.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let (method, path) = parse_request_line(&line)?;
    let headers = parse_headers(&mut budget)?;
    if headers
        .get("transfer-encoding")
        .is_some_and(|v| v.eq_ignore_ascii_case("chunked"))
    {
        return Err(protocol_error("chunked request bodies are not supported"));
    }
    // Fail closed: a present-but-unparseable length would leave the body in
    // the buffer to be read as the next request line.
    let content_length = match headers.get("content-length") {
        None => 0,
        Some(v) => v
            .trim()
            .parse::<usize>()
            .map_err(|_| protocol_error("malformed content-length"))?,
    };
    if content_length > MAX_BODY_BYTES {
        return Err(protocol_error("request body too large"));
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;
    Ok(Some(HttpRequest {
        method,
        path,
        headers,
        body,
    }))
}

fn protocol_error(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

fn parse_request_line(line: &str) -> io::Result<(String, String)> {
    if !line.ends_with('\n') {
        return Err(protocol_error("request line too long"));
    }
    let mut parts = line.trim_end().splitn(3, ' ');
    let method = parts.next().filter(|s| !s.is_empty());
    let path = parts.next().filter(|s| !s.is_empty());
    match (method, path) {
        (Some(method), Some(path)) => Ok((method.to_string(), path.to_string())),
        _ => Err(protocol_error("malformed request line")),
    }
}

fn parse_headers(reader: &mut impl BufRead) -> io::Result<HashMap<String, String>> {
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(protocol_error("connection closed mid-headers"));
        }
        if !line.ends_with('\n') {
            return Err(protocol_error("headers too long"));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            return Ok(headers);
        }
        let Some((name, value)) = trimmed.split_once(':') else {
            return Err(protocol_error("malformed header line"));
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
}

/// Write a status line, `Content-Type`/`Content-Length`/`Connection`
/// headers, `extra_headers`, then `body`.
pub(crate) fn write_response(
    writer: &mut impl Write,
    status: u16,
    content_type: &str,
    body: &[u8],
    extra_headers: &[(&str, &str)],
) -> io::Result<()> {
    write!(writer, "HTTP/1.1 {status} {}\r\n", reason_phrase(status))?;
    if !content_type.is_empty() {
        write!(writer, "Content-Type: {content_type}\r\n")?;
    }
    write!(writer, "Content-Length: {}\r\n", body.len())?;
    write!(writer, "Connection: keep-alive\r\n")?;
    for (name, value) in extra_headers {
        write!(writer, "{name}: {value}\r\n")?;
    }
    write!(writer, "\r\n")?;
    writer.write_all(body)?;
    writer.flush()
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    }
}

/// Serve one connection: parse-route-reply in a loop until EOF, a protocol
/// error, or a `Connection: close` request. `sessions` is shared across every
/// connection the listener accepts (MCP session IDs aren't per-connection).
pub(crate) fn serve_connection(
    stream: TcpStream,
    handler: &mut impl Handler,
    sessions: &Arc<Mutex<HashSet<String>>>,
) {
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut writer = BufWriter::new(write_half);
    loop {
        let request = match parse_request(&mut reader) {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err(_) => {
                let _ = write_response(&mut writer, 400, "text/plain", b"bad request", &[]);
                return;
            }
        };
        let close = request
            .header("connection")
            .is_some_and(|v| v.eq_ignore_ascii_case("close"));
        if handle_request(&request, handler, sessions, &mut writer).is_err() {
            return;
        }
        if close {
            return;
        }
    }
}

fn handle_request(
    request: &HttpRequest,
    handler: &mut impl Handler,
    sessions: &Arc<Mutex<HashSet<String>>>,
    writer: &mut impl Write,
) -> io::Result<()> {
    if request.path != MCP_PATH {
        return write_response(writer, 404, "text/plain", b"not found", &[]);
    }
    // Spec-required defense against a browser page's `fetch` reaching this
    // loopback port via DNS rebinding: only a same-host `Origin` (or none,
    // as every non-browser MCP client sends) is allowed through.
    if let Some(origin) = request.header("origin")
        && !origin_is_local(origin)
    {
        return write_response(writer, 403, "text/plain", b"forbidden origin", &[]);
    }
    match request.method.as_str() {
        "GET" => write_response(writer, 405, "text/plain", b"no SSE stream offered", &[]),
        "DELETE" => {
            if let Some(id) = request.header("mcp-session-id") {
                sessions.lock().expect("sessions mutex poisoned").remove(id);
            }
            write_response(writer, 200, "text/plain", b"", &[])
        }
        "POST" => handle_post(request, handler, sessions, writer),
        _ => write_response(writer, 405, "text/plain", b"method not allowed", &[]),
    }
}

fn handle_post(
    request: &HttpRequest,
    handler: &mut impl Handler,
    sessions: &Arc<Mutex<HashSet<String>>>,
    writer: &mut impl Write,
) -> io::Result<()> {
    let value: Value = match serde_json::from_slice(&request.body) {
        Ok(Value::Array(_)) | Err(_) => {
            return write_response(writer, 400, "text/plain", b"bad request", &[]);
        }
        Ok(v) => v,
    };
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    let is_initialize = method == "initialize";
    if !is_initialize && let Err(status) = check_session(request, sessions) {
        return write_response(writer, status, "text/plain", b"", &[]);
    }
    match jsonrpc::dispatch(handler, value) {
        None => write_response(writer, 202, "application/json", b"", &[]),
        Some(response) => {
            let body = serde_json::to_vec(&response).unwrap_or_default();
            if is_initialize {
                let session_id = generate_session_id();
                sessions
                    .lock()
                    .expect("sessions mutex poisoned")
                    .insert(session_id.clone());
                write_response(
                    writer,
                    200,
                    "application/json",
                    &body,
                    &[("Mcp-Session-Id", &session_id)],
                )
            } else {
                write_response(writer, 200, "application/json", &body, &[])
            }
        }
    }
}

/// Every request but `initialize` must carry a known `Mcp-Session-Id`: `400`
/// when the header is missing outright, `404` when it names a session the
/// server doesn't know (expired, or never `initialize`d) — the spec's cue
/// for the client to re-initialize.
fn check_session(request: &HttpRequest, sessions: &Arc<Mutex<HashSet<String>>>) -> Result<(), u16> {
    match request.header("mcp-session-id") {
        None => Err(400),
        Some(id)
            if sessions
                .lock()
                .expect("sessions mutex poisoned")
                .contains(id) =>
        {
            Ok(())
        }
        Some(_) => Err(404),
    }
}

fn generate_session_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Truncated to 64 bits: still ~16 significant hex digits of wall-clock
    // nanoseconds, plenty to keep this side of the id from repeating.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let id = format!("{counter:016x}{nanos:016x}");
    debug_assert_eq!(id.len(), SESSION_ID_HEX_CHARS);
    id
}

/// Whether an `Origin` header's host is this machine's loopback — the only
/// origins allowed to reach the MCP endpoint from a browser context.
fn origin_is_local(origin: &str) -> bool {
    let after_scheme = origin.split_once("://").map_or(origin, |(_, rest)| rest);
    let host = if let Some(bracketed) = after_scheme.strip_prefix('[') {
        match bracketed.find(']') {
            Some(end) => &after_scheme[..end + 2],
            None => after_scheme,
        }
    } else {
        after_scheme.split(['/', ':']).next().unwrap_or("")
    };
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

#[cfg(test)]
#[path = "http_test.rs"]
mod tests;
