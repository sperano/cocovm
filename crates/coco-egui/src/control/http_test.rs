use std::io::Cursor;

use super::*;

fn parse(input: &str) -> HttpRequest {
    parse_request(&mut Cursor::new(input.as_bytes()))
        .expect("parse succeeds")
        .expect("not EOF")
}

#[test]
fn request_line_and_headers_parse_with_lowercased_names() {
    let req = parse("POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\n\r\n");
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/mcp");
    assert_eq!(req.headers.get("host"), Some(&"x".to_string()));
    assert_eq!(
        req.headers.get("content-type"),
        Some(&"application/json".to_string())
    );
    assert!(req.body.is_empty());
}

#[test]
fn content_length_body_is_read_exactly() {
    let req = parse("POST /mcp HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello");
    assert_eq!(req.body, b"hello");
}

#[test]
fn missing_content_length_defaults_to_an_empty_body() {
    let req = parse("GET /mcp HTTP/1.1\r\n\r\n");
    assert!(req.body.is_empty());
}

#[test]
fn oversize_body_is_rejected() {
    let head = format!(
        "POST /mcp HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        MAX_BODY_BYTES + 1
    );
    let err = parse_request(&mut Cursor::new(head.as_bytes()))
        .expect_err("body over the limit must be rejected");
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn chunked_transfer_encoding_is_rejected() {
    let input = "POST /mcp HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
    let err = parse_request(&mut Cursor::new(input.as_bytes()))
        .expect_err("chunked bodies must be rejected");
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn malformed_content_length_is_rejected_not_treated_as_empty() {
    let input = "POST /mcp HTTP/1.1\r\nContent-Length: abc\r\n\r\n{}";
    let err = parse_request(&mut Cursor::new(input.as_bytes()))
        .expect_err("an unparseable length must fail closed");
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn clean_eof_between_requests_is_none() {
    let result = parse_request(&mut Cursor::new(b"".as_slice())).expect("EOF is not an error");
    assert!(result.is_none());
}

#[test]
fn write_response_formats_status_headers_and_body() {
    let mut out = Vec::new();
    write_response(
        &mut out,
        200,
        "application/json",
        b"{}",
        &[("Mcp-Session-Id", "abc")],
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(text.contains("Content-Type: application/json\r\n"));
    assert!(text.contains("Content-Length: 2\r\n"));
    assert!(text.contains("Connection: keep-alive\r\n"));
    assert!(text.contains("Mcp-Session-Id: abc\r\n"));
    assert!(text.ends_with("{}"));
}

#[test]
fn origin_is_local_accepts_loopback_hosts_and_rejects_others() {
    assert!(origin_is_local("http://localhost:6809"));
    assert!(origin_is_local("http://127.0.0.1:6809"));
    assert!(origin_is_local("http://[::1]:6809"));
    assert!(!origin_is_local("http://evil.example"));
}

#[test]
fn session_store_rejects_capacity_until_a_session_expires() {
    let now = Instant::now();
    let mut store = SessionStore::default();
    for _ in 0..MAX_CONTROL_SESSIONS {
        assert!(store.create(now, ProtocolVersion::June2025).is_some());
    }
    assert!(store.create(now, ProtocolVersion::June2025).is_none());

    let expired = now + CONTROL_SESSION_IDLE_TIMEOUT;
    assert!(store.create(expired, ProtocolVersion::June2025).is_some());
    assert_eq!(store.sessions.len(), 1);
}

#[test]
fn session_lookup_refreshes_its_idle_deadline() {
    let now = Instant::now();
    let mut store = SessionStore::default();
    let id = store
        .create(now, ProtocolVersion::March2025)
        .expect("session admitted");
    let refreshed = now + CONTROL_SESSION_IDLE_TIMEOUT / 2;

    assert_eq!(
        store.get_and_touch(&id, refreshed),
        Some(ProtocolVersion::March2025)
    );
    assert_eq!(
        store.get_and_touch(&id, now + CONTROL_SESSION_IDLE_TIMEOUT),
        Some(ProtocolVersion::March2025)
    );
}

#[test]
fn close_response_advertises_connection_close() {
    let mut out = Vec::new();
    write_close_response(&mut out, 503, "text/plain", b"busy").unwrap();
    let text = String::from_utf8(out).unwrap();

    assert!(text.starts_with("HTTP/1.1 503 Service Unavailable\r\n"));
    assert!(text.contains("Connection: close\r\n"));
}
