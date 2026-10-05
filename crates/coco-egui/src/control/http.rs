//! Hand-rolled HTTP/1.1, just enough to carry MCP's "streamable HTTP"
//! transport (JSON responses only — no SSE, no chunked/streaming bodies).
//! [`parse_request`]/[`write_response`] are the wire-level primitives;
//! [`serve_connection`] is the per-connection loop: parse a request, route
//! it, dispatch its body through [`crate::control::jsonrpc::dispatch`], and
//! reply — repeating until the client closes the connection.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::Value;

use super::MCP_PATH;
use super::jsonrpc;
use super::mcp::{Mcp, ProtocolVersion};
use super::session::{ActiveSession, SessionStore};

/// Longest request line + headers accepted, before the connection is closed
/// with a 400.
const MAX_HEADER_BYTES: u64 = 16 * 1024;
/// Longest request body accepted (a `poke`'s bytes are the largest
/// legitimate `tools/call` body and sit far below this).
const MAX_BODY_BYTES: usize = 1024 * 1024;

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
    write_response_with_connection(
        writer,
        status,
        content_type,
        body,
        extra_headers,
        "keep-alive",
    )
}

/// Write a response that tells the peer this socket won't be reused.
pub(super) fn write_close_response(
    writer: &mut impl Write,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    write_response_with_connection(writer, status, content_type, body, &[], "close")
}

fn write_response_with_connection(
    writer: &mut impl Write,
    status: u16,
    content_type: &str,
    body: &[u8],
    extra_headers: &[(&str, &str)],
    connection: &str,
) -> io::Result<()> {
    write!(writer, "HTTP/1.1 {status} {}\r\n", reason_phrase(status))?;
    if !content_type.is_empty() {
        write!(writer, "Content-Type: {content_type}\r\n")?;
    }
    write!(writer, "Content-Length: {}\r\n", body.len())?;
    write!(writer, "Connection: {connection}\r\n")?;
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
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// Serve one connection: parse-route-reply in a loop until EOF, a protocol
/// error, or a `Connection: close` request. `sessions` is shared across every
/// connection the listener accepts (MCP session IDs aren't per-connection).
pub(super) fn serve_connection(
    stream: TcpStream,
    handler: &mut Mcp,
    sessions: &Arc<Mutex<SessionStore>>,
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
            Err(e) if is_timeout(&e) => return,
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

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

fn handle_request(
    request: &HttpRequest,
    handler: &mut Mcp,
    sessions: &Arc<Mutex<SessionStore>>,
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
    handler: &mut Mcp,
    sessions: &Arc<Mutex<SessionStore>>,
    writer: &mut impl Write,
) -> io::Result<()> {
    let Some(value) = parse_json_body(&request.body) else {
        return write_response(writer, 400, "text/plain", b"bad request", &[]);
    };
    if let Value::Array(messages) = value {
        return handle_batch_post(request, handler, messages, sessions, writer);
    }
    handle_single_post(request, handler, value, sessions, writer)
}

fn parse_json_body(body: &[u8]) -> Option<Value> {
    serde_json::from_slice(body).ok()
}

fn handle_single_post(
    request: &HttpRequest,
    handler: &mut Mcp,
    value: Value,
    sessions: &Arc<Mutex<SessionStore>>,
    writer: &mut impl Write,
) -> io::Result<()> {
    if method(&value) == Some("initialize") {
        return handle_initialize(handler, value, sessions, writer);
    }
    // `_session` holds the session open until the response is written.
    let (protocol_version, _session) = match request_version(request, sessions) {
        Ok(started) => started,
        Err((status, body)) => return write_response(writer, status, "text/plain", body, &[]),
    };
    handler.set_protocol_version(protocol_version);
    dispatch_single(handler, value, None, sessions, writer)
}

fn handle_initialize(
    handler: &mut Mcp,
    value: Value,
    sessions: &Arc<Mutex<SessionStore>>,
    writer: &mut impl Write,
) -> io::Result<()> {
    let protocol_version = ProtocolVersion::negotiate(&value["params"]);
    let Some(session_id) = sessions
        .lock()
        .expect("sessions mutex poisoned")
        .create(Instant::now(), protocol_version)
    else {
        return write_response(
            writer,
            503,
            "text/plain",
            b"session capacity reached; retry later",
            &[],
        );
    };
    handler.set_protocol_version(protocol_version);
    dispatch_single(handler, value, Some(session_id), sessions, writer)
}

fn handle_batch_post(
    request: &HttpRequest,
    handler: &mut Mcp,
    messages: Vec<Value>,
    sessions: &Arc<Mutex<SessionStore>>,
    writer: &mut impl Write,
) -> io::Result<()> {
    let (protocol_version, _session) = match request_version(request, sessions) {
        Ok(started) => started,
        Err((status, body)) => return write_response(writer, status, "text/plain", body, &[]),
    };
    if !protocol_version.accepts_batches()
        || messages.is_empty()
        || messages
            .iter()
            .any(|message| method(message) == Some("initialize"))
    {
        return write_response(writer, 400, "text/plain", b"bad request", &[]);
    }
    handler.set_protocol_version(protocol_version);
    let responses: Vec<Value> = messages
        .into_iter()
        .filter_map(|message| jsonrpc::dispatch(handler, message))
        .collect();
    if responses.is_empty() {
        return write_response(writer, 202, "application/json", b"", &[]);
    }
    let body = serde_json::to_vec(&responses).unwrap_or_default();
    write_response(writer, 200, "application/json", &body, &[])
}

/// The protocol version a request after `initialize` is answered in: its
/// `MCP-Protocol-Version` header, else the version its session negotiated,
/// plus the hold on that session for the request's lifetime.
/// `Err` holds the HTTP status and body for a missing or unknown session
/// (see [`check_session`]) or an unsupported header (a 400 per spec
/// 2025-06-18). `initialize` never comes here: it negotiates from its body.
fn request_version<'a>(
    request: &HttpRequest,
    sessions: &'a Mutex<SessionStore>,
) -> Result<(ProtocolVersion, ActiveSession<'a>), (u16, &'static [u8])> {
    let (session_version, session) =
        check_session(request, sessions, Instant::now()).map_err(|status| (status, &b""[..]))?;
    let version = match request.header("mcp-protocol-version") {
        None => session_version,
        Some(header) => {
            ProtocolVersion::parse(header).ok_or((400, &b"unsupported MCP-Protocol-Version"[..]))?
        }
    };
    Ok((version, session))
}

fn method(value: &Value) -> Option<&str> {
    value.get("method").and_then(Value::as_str)
}

fn dispatch_single(
    handler: &mut Mcp,
    value: Value,
    session_id: Option<String>,
    sessions: &Arc<Mutex<SessionStore>>,
    writer: &mut impl Write,
) -> io::Result<()> {
    match jsonrpc::dispatch(handler, value) {
        None => {
            if let Some(id) = session_id {
                sessions
                    .lock()
                    .expect("sessions mutex poisoned")
                    .remove(&id);
            }
            write_response(writer, 202, "application/json", b"", &[])
        }
        Some(response) => {
            let body = serde_json::to_vec(&response).unwrap_or_default();
            if let Some(session_id) = session_id {
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
fn check_session<'a>(
    request: &HttpRequest,
    sessions: &'a Mutex<SessionStore>,
    now: Instant,
) -> Result<(ProtocolVersion, ActiveSession<'a>), u16> {
    let Some(id) = request.header("mcp-session-id") else {
        return Err(400);
    };
    ActiveSession::start(sessions, id, now).ok_or(404)
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
