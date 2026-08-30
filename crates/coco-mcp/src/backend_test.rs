use super::*;
use coco_control::{Action, Request};

#[test]
fn unreachable_message_names_the_port() {
    let msg = unreachable_message(6809);
    assert!(msg.contains("127.0.0.1:6809"));
}

#[test]
fn app_backend_reports_io_error_when_nothing_listens() {
    // Port 0 never accepts a real connection, so `connect` fails immediately.
    let mut backend = AppBackend::new(0);
    let req = Request {
        vm: None,
        action: Action::ListVms,
    };
    let err = backend.call(&req).unwrap_err();
    assert!(matches!(err, ControlError::Io(_)));
}

#[test]
fn mock_backend_records_calls_and_replays_responses() {
    let mut mock = MockBackend::new(vec![
        Ok(Reply::Done),
        Err(ControlError::Remote("nope".into())),
    ]);
    let req = Request {
        vm: None,
        action: Action::ListVms,
    };
    assert!(matches!(mock.call(&req), Ok(Reply::Done)));
    assert!(matches!(mock.call(&req), Err(ControlError::Remote(_))));
    assert_eq!(mock.calls.len(), 2);
}
