//! In-process end-to-end coverage of the MCP HTTP server: real `TcpStream`s
//! talk to a real [`ControlServer`], proving `http`/`jsonrpc`/`mcp`/`tools`
//! are wired together correctly. Unit coverage for each layer lives in that
//! layer's own test file.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::*;

fn bind() -> ControlServer {
    ControlServer::bind(0, Arc::new(|| {})).expect("bind ephemeral port")
}

/// Send one HTTP request and read back its status, lower-cased headers, and
/// body — a client-side mirror of `http::parse_request`/`write_response`.
fn send(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, HashMap<String, String>, String) {
    let mut stream = connect(port);
    let request = request_text(method, path, headers, body, true);
    stream.write_all(request.as_bytes()).unwrap();
    read_response(&mut BufReader::new(stream))
}

fn connect(port: u16) -> TcpStream {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
}

/// One HTTP/1.1 request as text; `close` adds `Connection: close`.
fn request_text(
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
    close: bool,
) -> String {
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    if close {
        request.push_str("Connection: close\r\n");
    }
    request.push_str(&format!("\r\n{body}"));
    request
}

/// Read one response off `reader`, leaving the stream positioned at the next.
fn read_response(reader: &mut BufReader<TcpStream>) -> (u16, HashMap<String, String>, String) {
    let mut status_line = String::new();
    reader.read_line(&mut status_line).unwrap();
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .expect("status line has a code")
        .parse()
        .unwrap();
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        let (name, value) = trimmed.split_once(':').expect("header line has a colon");
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).unwrap();
    (status, headers, String::from_utf8(body).unwrap())
}

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;

/// A real client keeps one connection open for the whole session: three
/// requests on one socket, each answered in order, the third ending it.
#[test]
fn keep_alive_connection_serves_requests_in_order() {
    let server = bind();
    let stream = connect(server.port());
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = stream;

    writer
        .write_all(request_text("POST", MCP_PATH, &[], INITIALIZE, false).as_bytes())
        .unwrap();
    let (status, headers, _) = read_response(&mut reader);
    assert_eq!(status, 200);
    let session_id = headers["mcp-session-id"].clone();

    let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
    let session = [("Mcp-Session-Id", session_id.as_str())];
    writer
        .write_all(request_text("POST", MCP_PATH, &session, list, false).as_bytes())
        .unwrap();
    let (status, _, body) = read_response(&mut reader);
    assert_eq!(status, 200);
    let response: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["id"], json!(2));
    assert!(response["result"]["tools"].is_array());

    writer
        .write_all(request_text("DELETE", MCP_PATH, &session, "", true).as_bytes())
        .unwrap();
    let (status, _, _) = read_response(&mut reader);
    assert_eq!(status, 200);
    // The session is gone: the same id is now unknown.
    let (status, _, _) = send(server.port(), "POST", MCP_PATH, &session, list);
    assert_eq!(status, 404);
}

#[test]
fn mcp_session_lifecycle_over_real_http() {
    let server = bind();
    let port = server.port();

    // initialize: 200, and a fresh Mcp-Session-Id to use for the rest.
    let (status, headers, body) = send(port, "POST", MCP_PATH, &[], INITIALIZE);
    assert_eq!(status, 200);
    let session_id = headers
        .get("mcp-session-id")
        .expect("initialize must mint a session id")
        .clone();
    let response: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["id"], json!(1));

    // notifications/initialized with the session header: 202, empty body.
    let (status, _, body) = send(
        port,
        "POST",
        MCP_PATH,
        &[("Mcp-Session-Id", &session_id)],
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );
    assert_eq!(status, 202);
    assert!(body.is_empty());

    // tools/list without the session header: 400.
    let list_request = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
    let (status, _, _) = send(port, "POST", MCP_PATH, &[], list_request);
    assert_eq!(status, 400);

    // tools/list with an unknown session id: 404.
    let (status, _, _) = send(
        port,
        "POST",
        MCP_PATH,
        &[("Mcp-Session-Id", "no-such-session")],
        list_request,
    );
    assert_eq!(status, 404);

    // tools/list with the right header: 200, every tool listed.
    let (status, _, body) = send(
        port,
        "POST",
        MCP_PATH,
        &[("Mcp-Session-Id", &session_id)],
        list_request,
    );
    assert_eq!(status, 200);
    let response: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        response["result"]["tools"].as_array().unwrap().len(),
        tool_defs::definitions().len()
    );

    // tools/call list_vms: goes out on the request queue; answer it like the
    // frame loop would while the client thread blocks on the HTTP response.
    let call_request = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_vms","arguments":{}}}"#;
    let session_for_thread = session_id.clone();
    let client = thread::spawn(move || {
        send(
            port,
            "POST",
            MCP_PATH,
            &[("Mcp-Session-Id", &session_for_thread)],
            call_request,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(incoming) = server.try_recv() {
            assert_eq!(incoming.request.action, Action::ListVms);
            incoming.reply(Response::Ok(Reply::Vms(vec![])));
            break;
        }
        assert!(Instant::now() < deadline, "no request reached the queue");
        thread::sleep(Duration::from_millis(2));
    }
    let (status, _, body) = client.join().unwrap();
    assert_eq!(status, 200);
    let response: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["result"]["isError"], json!(false));
    assert_eq!(response["result"]["content"][0]["type"], json!("text"));
}

#[test]
fn get_is_not_allowed_no_sse_stream_offered() {
    let server = bind();
    let (status, _, _) = send(server.port(), "GET", MCP_PATH, &[], "");
    assert_eq!(status, 405);
}

#[test]
fn a_foreign_origin_is_forbidden() {
    let server = bind();
    let (status, _, _) = send(
        server.port(),
        "POST",
        MCP_PATH,
        &[("Origin", "http://evil.example")],
        INITIALIZE,
    );
    assert_eq!(status, 403);
}

#[test]
fn a_non_mcp_path_is_not_found() {
    let server = bind();
    let (status, _, _) = send(server.port(), "POST", "/other", &[], "{}");
    assert_eq!(status, 404);
}
