use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use super::*;

/// How long [`wait_for_result`] lets a loopback check run.
const CHECK_DEADLINE: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// Read size for the request `fetch_latest` sends.
const REQUEST_BUFFER_SIZE: usize = 4096;

/// A release object as the API returns it, trimmed to a few fields.
pub(crate) fn release_json(tag: &str) -> String {
    format!(
        r#"{{"tag_name":"{tag}","name":"{tag}","draft":false,"prerelease":false,"html_url":"https://github.com/sperano/cocovm/releases/tag/{tag}","assets":[]}}"#
    )
}

/// Serve one HTTP response with `status_line` (`"200 OK"`) and `body` on a
/// loopback port; returns its URL. The listener answers a single request.
pub(crate) fn serve_once(status_line: &'static str, body: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let url = format!("http://{}/releases/latest", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        // Read the whole request head (a GET has no body) before answering.
        let mut request = Vec::with_capacity(REQUEST_BUFFER_SIZE);
        let mut chunk = [0; REQUEST_BUFFER_SIZE];
        while !request.ends_with(b"\r\n\r\n") {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => request.extend_from_slice(&chunk[..n]),
            }
        }
        let _ = write!(
            stream,
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    });
    url
}

/// A version one minor step past the build, so the test holds after bumps.
fn next_version() -> semver::Version {
    let current = current_version();
    semver::Version::new(current.major, current.minor + 1, 0)
}

/// Poll `check` until it leaves `Checking`, or panic after the deadline.
pub(crate) fn wait_for_result(check: &mut UpdateCheck) {
    let deadline = Instant::now() + CHECK_DEADLINE;
    while matches!(check.status(), Status::Checking(_)) {
        assert!(Instant::now() < deadline, "update check did not finish");
        std::thread::sleep(POLL_INTERVAL);
        check.poll();
    }
}

#[test]
fn parse_release_strips_the_tag_prefix() {
    let release = parse_release(&release_json("v0.8.0")).unwrap();
    assert_eq!(release.version, semver::Version::new(0, 8, 0));
    assert_eq!(
        release.page_url,
        "https://github.com/sperano/cocovm/releases/tag/v0.8.0"
    );
}

#[test]
fn parse_release_accepts_a_bare_version_tag() {
    let release = parse_release(&release_json("1.2.3")).unwrap();
    assert_eq!(release.version, semver::Version::new(1, 2, 3));
}

#[test]
fn parse_release_rejects_a_tag_that_is_not_a_version() {
    let error = parse_release(&release_json("nightly")).unwrap_err();
    assert!(error.contains("nightly"), "{error}");
}

#[test]
fn parse_release_rejects_a_page_outside_the_repository() {
    let json = r#"{"tag_name":"v9.0.0","html_url":"https://example.com/cocovm"}"#;
    let error = parse_release(json).unwrap_err();
    assert!(error.contains("example.com"), "{error}");
}

#[test]
fn parse_release_rejects_a_body_without_the_fields() {
    let error = parse_release(r#"{"message":"Not Found"}"#).unwrap_err();
    assert!(error.contains("tag_name"), "{error}");
}

#[test]
fn current_version_is_the_package_version() {
    assert_eq!(current_version().to_string(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn fetch_latest_reads_the_release_from_the_endpoint() {
    let url = serve_once("200 OK", release_json("v0.8.0"));
    let release = fetch_latest(&url).unwrap();
    assert_eq!(release.version, semver::Version::new(0, 8, 0));
}

#[test]
fn fetch_latest_reports_an_http_error() {
    let url = serve_once("403 Forbidden", r#"{"message":"rate limited"}"#.to_string());
    let error = fetch_latest(&url).unwrap_err();
    assert!(error.contains("403"), "{error}");
}

#[test]
fn a_newer_release_is_available_and_shows_the_notice() {
    let tag = format!("v{}", next_version());
    let mut check = UpdateCheck::new(serve_once("200 OK", release_json(&tag)));
    check.start(&egui::Context::default(), false);
    wait_for_result(&mut check);
    match check.status() {
        Status::Available(release) => assert_eq!(release.version, next_version()),
        other => panic!("expected Available, got {other:?}"),
    }
    assert!(check.notice().is_some());
    assert!(!check.requested());
}

#[test]
fn the_built_version_is_up_to_date() {
    let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    let mut check = UpdateCheck::new(serve_once("200 OK", release_json(&tag)));
    check.start(&egui::Context::default(), false);
    wait_for_result(&mut check);
    assert!(matches!(check.status(), Status::UpToDate));
    assert!(check.notice().is_none());
}

#[test]
fn an_older_release_is_up_to_date() {
    let mut check = UpdateCheck::new(serve_once("200 OK", release_json("v0.0.1")));
    check.start(&egui::Context::default(), false);
    wait_for_result(&mut check);
    assert!(matches!(check.status(), Status::UpToDate));
}

#[test]
fn a_failed_request_is_failed() {
    let url = serve_once("500 Internal Server Error", String::new());
    let mut check = UpdateCheck::new(url);
    check.start(&egui::Context::default(), true);
    wait_for_result(&mut check);
    assert!(matches!(check.status(), Status::Failed(_)));
    assert!(check.requested());
}

#[test]
fn dismiss_hides_the_notice_until_a_requested_check() {
    let mut check = UpdateCheck::new(serve_once(
        "200 OK",
        release_json(&format!("v{}", next_version())),
    ));
    check.set_status(Status::Available(Release {
        version: next_version(),
        page_url: format!("{RELEASE_PAGE_PREFIX}tag/v{}", next_version()),
    }));
    check.dismiss();
    assert!(check.notice().is_none());

    check.start(&egui::Context::default(), true);
    wait_for_result(&mut check);
    assert!(check.notice().is_some());
}
