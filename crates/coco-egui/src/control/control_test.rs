//! In-process end-to-end coverage of the MCP HTTP server: real `TcpStream`s
//! talk to a real [`ControlServer`], proving `http`/`jsonrpc`/`mcp`/`tools`
//! are wired together correctly. Unit coverage for each layer lives in that
//! layer's own test file.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
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

fn initialize_session(server: &ControlServer, protocol_version: &str) -> String {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": protocol_version},
    })
    .to_string();
    let (status, headers, response) = send(server.port(), "POST", MCP_PATH, &[], &body);
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_str::<Value>(&response).unwrap()["result"]["protocolVersion"],
        json!(protocol_version)
    );
    headers["mcp-session-id"].clone()
}

#[test]
fn batch_rules_follow_the_session_protocol_across_connections() {
    let server = bind();
    let batch = r#"[{"jsonrpc":"2.0","id":3,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/initialized"},{"jsonrpc":"2.0","id":2,"method":"ping"}]"#;
    for version in [
        mcp::PROTOCOL_VERSION_2024_11_05,
        mcp::PROTOCOL_VERSION_2025_06_18,
    ] {
        let session_id = initialize_session(&server, version);
        let headers = [("Mcp-Session-Id", session_id.as_str())];
        assert_eq!(
            send(server.port(), "POST", MCP_PATH, &headers, batch).0,
            400
        );
    }

    let session_id = initialize_session(&server, mcp::PROTOCOL_VERSION_2025_03_26);
    let headers = [("Mcp-Session-Id", session_id.as_str())];
    let (status, _, body) = send(server.port(), "POST", MCP_PATH, &headers, batch);
    assert_eq!(status, 200);
    let responses: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(responses.as_array().unwrap().len(), 2);
    assert_eq!(responses[0]["id"], json!(3));
    assert_eq!(responses[1]["id"], json!(2));

    let notifications = r#"[{"jsonrpc":"2.0","method":"notifications/initialized"},{"jsonrpc":"2.0","method":"notifications/cancelled"}]"#;
    let (status, _, body) = send(server.port(), "POST", MCP_PATH, &headers, notifications);
    assert_eq!(status, 202);
    assert!(body.is_empty());
    assert_eq!(send(server.port(), "POST", MCP_PATH, &headers, "[]").0, 400);
    assert_eq!(
        send(
            server.port(),
            "POST",
            MCP_PATH,
            &headers,
            &format!("[{INITIALIZE}]")
        )
        .0,
        400
    );
}

/// The `MCP-Protocol-Version` header, not the session, decides whether a
/// batch is accepted: batches exist only in 2025-03-26.
#[test]
fn protocol_version_header_decides_batch_support() {
    let server = bind();
    let batch = r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#;
    for (session_version, header_version, status) in [
        (
            mcp::PROTOCOL_VERSION_2025_03_26,
            mcp::PROTOCOL_VERSION_2025_06_18,
            400,
        ),
        (
            mcp::PROTOCOL_VERSION_2025_06_18,
            mcp::PROTOCOL_VERSION_2025_03_26,
            200,
        ),
    ] {
        let session_id = initialize_session(&server, session_version);
        let headers = [
            ("Mcp-Session-Id", session_id.as_str()),
            ("MCP-Protocol-Version", header_version),
        ];
        assert_eq!(
            send(server.port(), "POST", MCP_PATH, &headers, batch).0,
            status,
            "session {session_version}, header {header_version}"
        );
    }
}

#[test]
fn tools_list_uses_the_session_protocol_across_connections() {
    let server = bind();
    let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
    for (version, annotations_expected) in [
        (mcp::PROTOCOL_VERSION_2024_11_05, false),
        (mcp::PROTOCOL_VERSION_2025_03_26, true),
        (mcp::PROTOCOL_VERSION_2025_06_18, true),
    ] {
        let session_id = initialize_session(&server, version);
        let headers = [("Mcp-Session-Id", session_id.as_str())];
        let (status, _, body) = send(server.port(), "POST", MCP_PATH, &headers, list);
        assert_eq!(status, 200);
        let response: Value = serde_json::from_str(&body).unwrap();
        for tool in response["result"]["tools"].as_array().unwrap() {
            assert_eq!(tool.get("annotations").is_some(), annotations_expected);
        }
    }
}

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
        tool_defs::definitions(true).len()
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

/// Whether `tools/list` over `headers` declares any `outputSchema`, or the
/// HTTP status when the request is refused.
fn lists_output_schemas(port: u16, headers: &[(&str, &str)]) -> Result<bool, u16> {
    let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
    let (status, _, body) = send(port, "POST", MCP_PATH, headers, list);
    if status != 200 {
        return Err(status);
    }
    let response: Value = serde_json::from_str(&body).unwrap();
    let tools = response["result"]["tools"].as_array().unwrap().clone();
    Ok(tools.iter().any(|t| t.get("outputSchema").is_some()))
}

/// The session's `initialize` version decides structured output on later
/// connections; an `MCP-Protocol-Version` header overrides it per request.
#[test]
fn negotiated_protocol_version_gates_structured_output() {
    let server = bind();
    let port = server.port();
    let initialize = INITIALIZE.replace("2025-06-18", "2025-03-26");
    let (status, headers, _) = send(port, "POST", MCP_PATH, &[], &initialize);
    assert_eq!(status, 200);
    let session_id = headers["mcp-session-id"].clone();
    let session = ("Mcp-Session-Id", session_id.as_str());

    assert_eq!(lists_output_schemas(port, &[session]), Ok(false));
    let newer = ("MCP-Protocol-Version", "2025-06-18");
    assert_eq!(lists_output_schemas(port, &[session, newer]), Ok(true));
    let unsupported = ("MCP-Protocol-Version", "1999-01-01");
    assert_eq!(
        lists_output_schemas(port, &[session, unsupported]),
        Err(400)
    );

    // `initialize` ignores the header (even an unknown one) and negotiates
    // from its body.
    let (status, headers, _) = send(port, "POST", MCP_PATH, &[unsupported], INITIALIZE);
    assert_eq!(status, 200);
    let latest = ("Mcp-Session-Id", headers["mcp-session-id"].as_str());
    assert_eq!(lists_output_schemas(port, &[latest]), Ok(true));
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

#[test]
fn wake_requests_are_coalesced_until_dispatch_begins() {
    let wakes = Arc::new(AtomicUsize::new(0));
    let wake_count = Arc::clone(&wakes);
    let gate = WakeGate::new(Arc::new(move || {
        wake_count.fetch_add(1, Ordering::Relaxed);
    }));

    gate.request();
    gate.request();
    assert_eq!(wakes.load(Ordering::Relaxed), 1);

    gate.clear();
    gate.request();
    assert_eq!(wakes.load(Ordering::Relaxed), 2);
}

#[test]
fn a_full_incoming_queue_returns_an_overload_error() {
    let (tx, _rx) = mpsc::sync_channel(1);
    let (occupied_reply, _occupied_wait) = ReplyWait::pair();
    tx.try_send(Incoming {
        request: Request {
            vm: None,
            action: Action::ListVms,
        },
        reply: occupied_reply,
    })
    .expect("occupy queue");
    let mut backend = QueueBackend {
        tx,
        wake: Arc::new(WakeGate::new(Arc::new(|| {}))),
        stop: Arc::new(AtomicBool::new(false)),
        disconnect_probe: None,
    };

    let err = tools::Backend::call(
        &mut backend,
        &Request {
            vm: None,
            action: Action::ListVms,
        },
    )
    .expect_err("a full queue must reject admission");

    assert!(err.contains("overloaded"));
}

#[test]
fn reply_timeout_marks_queued_work_abandoned() {
    let (tx, rx) = mpsc::sync_channel(1);
    let backend = QueueBackend {
        tx,
        wake: Arc::new(WakeGate::new(Arc::new(|| {}))),
        stop: Arc::new(AtomicBool::new(false)),
        disconnect_probe: None,
    };
    let request = Request {
        vm: None,
        action: Action::ListVms,
    };

    let (reply, wait) = ReplyWait::pair();
    backend
        .tx
        .try_send(Incoming { request, reply })
        .expect("queue request");
    let err = backend
        .wait_for_reply_until(wait, Instant::now())
        .expect_err("past deadline must time out");
    let queued = rx.try_recv().expect("queued request remains available");

    assert!(err.contains("timed out"));
    assert!(queued.reply.is_abandoned());
}

#[test]
fn dropping_the_server_joins_idle_connection_threads() {
    const SHUTDOWN_LIMIT: Duration = Duration::from_secs(1);

    let server = bind();
    let _idle = connect(server.port());
    let deadline = Instant::now() + Duration::from_secs(1);
    while server
        .active
        .lock()
        .expect("active connections mutex")
        .streams
        .is_empty()
    {
        assert!(Instant::now() < deadline, "connection wasn't accepted");
        thread::yield_now();
    }

    let started = Instant::now();
    drop(server);

    assert!(started.elapsed() < SHUTDOWN_LIMIT);
}

#[test]
fn connections_beyond_the_limit_receive_service_unavailable() {
    let server = bind();
    let clients: Vec<_> = (0..MAX_CONTROL_CONNECTIONS)
        .map(|_| connect(server.port()))
        .collect();
    let deadline = Instant::now() + Duration::from_secs(2);
    while server
        .active
        .lock()
        .expect("active connections mutex")
        .streams
        .len()
        < MAX_CONTROL_CONNECTIONS
    {
        assert!(Instant::now() < deadline, "connections weren't accepted");
        thread::yield_now();
    }

    let overloaded = connect(server.port());
    let (status, headers, body) = read_response(&mut BufReader::new(overloaded));

    assert_eq!(status, 503);
    assert_eq!(headers["connection"], "close");
    assert!(body.contains("overloaded"));
    drop(clients);
}

#[test]
fn a_disconnected_client_marks_its_queued_request_abandoned() {
    let server = bind();
    let (status, headers, _) = send(server.port(), "POST", MCP_PATH, &[], INITIALIZE);
    assert_eq!(status, 200);
    let session_id = headers["mcp-session-id"].clone();
    let mut client = connect(server.port());
    let call = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"list_vms","arguments":{}}}"#;
    client
        .write_all(
            request_text(
                "POST",
                MCP_PATH,
                &[("Mcp-Session-Id", &session_id)],
                call,
                false,
            )
            .as_bytes(),
        )
        .unwrap();
    drop(client);

    let deadline = Instant::now() + Duration::from_secs(2);
    let incoming = loop {
        if let Some(incoming) = server.try_recv() {
            break incoming;
        }
        assert!(Instant::now() < deadline, "request wasn't queued");
        thread::yield_now();
    };
    while !incoming.reply.is_abandoned() {
        assert!(Instant::now() < deadline, "disconnect wasn't detected");
        thread::yield_now();
    }
}
