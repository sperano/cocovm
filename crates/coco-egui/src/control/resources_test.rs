use super::*;
use crate::control::protocol::{ScreenSnapshot, VmInfo};
use crate::control::tools::MockBackend;
use crate::control::tools::vms::tests::sample_vms;

/// `list`/`templates` as a protocol 2025-06-18 client sees them.
const TITLED: bool = true;
/// `list`/`templates` as an older client sees them.
const UNTITLED: bool = false;

fn screen_reply(lines: &[&str]) -> Reply {
    Reply::Screen(ScreenSnapshot {
        lines: lines.iter().map(|line| line.to_string()).collect(),
        mode: "video mode: CoCo-compatible text, base=$0400".into(),
        cursor: Some(coco_core::TextCursor { row: 1, col: 0 }),
    })
}

fn screenshot_reply() -> Reply {
    Reply::Screenshot {
        png_base64: "cGljdHVyZQ==".into(),
        width: 640,
        height: 240,
    }
}

fn read_params(uri: &str) -> Value {
    json!({"uri": uri})
}

#[test]
fn capability_declines_subscriptions_and_list_change_notices() {
    assert_eq!(
        capability(),
        json!({"subscribe": false, "listChanged": false})
    );
}

#[test]
fn list_offers_text_and_png_for_every_vm() {
    let vms = sample_vms();
    let mut mock = MockBackend::new(vec![Ok(Reply::Vms(vms.clone()))]);

    let result = list(&mut mock, Value::Null, TITLED).unwrap();

    assert_eq!(
        mock.calls,
        vec![Request {
            vm: None,
            action: Action::ListVms
        }]
    );
    let resources = result["resources"].as_array().unwrap();
    assert_eq!(resources.len(), vms.len() * ScreenResource::ALL.len());
    let VmInfo { slug, name, .. } = &vms[0];
    assert_eq!(
        resources[0]["uri"],
        json!(format!("cocovm://vm/{slug}/screen.txt"))
    );
    assert_eq!(resources[0]["mimeType"], json!("text/plain"));
    assert_eq!(resources[0]["name"], json!(format!("{slug} screen text")));
    assert_eq!(resources[0]["title"], json!(format!("{name} screen text")));
    assert_eq!(
        resources[1]["uri"],
        json!(format!("cocovm://vm/{slug}/screen.png"))
    );
    assert_eq!(resources[1]["mimeType"], json!("image/png"));
    assert_eq!(resources[1]["name"], json!(format!("{slug} screenshot")));
    assert_eq!(resources[1]["title"], json!(format!("{name} screenshot")));
    assert!(result.get("nextCursor").is_none());
}

#[test]
fn list_describes_each_vm_by_name_slug_and_status() {
    let vms = sample_vms();
    let mut mock = MockBackend::new(vec![Ok(Reply::Vms(vms.clone()))]);

    let result = list(&mut mock, json!({}), UNTITLED).unwrap();

    let descriptions: Vec<&str> = result["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|resource| resource["description"].as_str().unwrap())
        .collect();
    for (vm, chunk) in vms.iter().zip(descriptions.chunks(2)) {
        for description in chunk {
            assert!(
                description.contains(&format!("'{}'", vm.name)),
                "{description}"
            );
            assert!(description.contains(&vm.slug), "{description}");
            assert!(
                description.contains(status_text(vm.status)),
                "{description}"
            );
        }
    }
}

#[test]
fn list_omits_titles_for_older_clients() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Vms(sample_vms()))]);
    let result = list(&mut mock, Value::Null, UNTITLED).unwrap();
    assert!(
        result["resources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|resource| resource.get("title").is_none())
    );
}

#[test]
fn list_rejects_a_cursor_it_never_issued() {
    let mut mock = MockBackend::new(vec![]);
    let err = list(&mut mock, json!({"cursor": "page2"}), TITLED).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
    assert!(mock.calls.is_empty());
}

#[test]
fn list_reports_a_backend_failure_as_an_internal_error() {
    let mut mock = MockBackend::new(vec![Err("control server shutting down".into())]);
    let err = list(&mut mock, Value::Null, TITLED).unwrap_err();
    assert_eq!(err.code, INTERNAL_ERROR);
    assert_eq!(err.message, "control server shutting down");
}

#[test]
fn templates_name_the_slug_variable() {
    let result = templates(TITLED);
    let titled = result["resourceTemplates"].as_array().unwrap();
    assert_eq!(titled.len(), 2);
    assert_eq!(
        titled[0]["uriTemplate"],
        json!("cocovm://vm/{slug}/screen.txt")
    );
    assert_eq!(titled[0]["mimeType"], json!("text/plain"));
    assert_eq!(
        titled[1]["uriTemplate"],
        json!("cocovm://vm/{slug}/screen.png")
    );
    assert_eq!(titled[1]["mimeType"], json!("image/png"));
    assert!(titled.iter().all(|t| t["title"].is_string()));
    assert!(
        templates(UNTITLED)["resourceTemplates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t.get("title").is_none())
    );
}

#[test]
fn read_text_returns_the_screen_lines_as_plain_text() {
    let mut mock = MockBackend::new(vec![Ok(screen_reply(&["OK", ""]))]);

    let result = read(&mut mock, read_params("cocovm://vm/vm0/screen.txt")).unwrap();

    assert_eq!(
        mock.calls,
        vec![Request {
            vm: Some("vm0".into()),
            action: Action::ScreenText
        }]
    );
    assert_eq!(
        result,
        json!({"contents": [{
            "uri": "cocovm://vm/vm0/screen.txt",
            "mimeType": "text/plain",
            "text": "OK\n"
        }]})
    );
}

#[test]
fn read_png_returns_the_screenshot_as_a_blob() {
    let mut mock = MockBackend::new(vec![Ok(screenshot_reply())]);

    let result = read(&mut mock, read_params("cocovm://vm/vm0/screen.png")).unwrap();

    assert_eq!(
        mock.calls,
        vec![Request {
            vm: Some("vm0".into()),
            action: Action::Screenshot
        }]
    );
    assert_eq!(
        result,
        json!({"contents": [{
            "uri": "cocovm://vm/vm0/screen.png",
            "mimeType": "image/png",
            "blob": "cGljdHVyZQ=="
        }]})
    );
}

#[test]
fn read_without_a_uri_is_invalid_params() {
    let mut mock = MockBackend::new(vec![]);
    for params in [Value::Null, json!({}), json!({"uri": 7})] {
        let err = read(&mut mock, params).unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
    }
    assert!(mock.calls.is_empty());
}

#[test]
fn read_of_a_uri_this_server_does_not_serve_is_not_found() {
    let mut mock = MockBackend::new(vec![]);
    for uri in [
        "file:///etc/passwd",
        "cocovm://vm/",
        "cocovm://vm//screen.txt",
        "cocovm://vm/vm0",
        "cocovm://vm/vm0/screen.gif",
        "cocovm://vm/vm0/screen.txt/extra",
        "COCOVM://vm/vm0/screen.txt",
    ] {
        let err = read(&mut mock, read_params(uri)).unwrap_err();
        assert_eq!(err.code, RESOURCE_NOT_FOUND, "{uri}");
        assert!(err.message.contains(uri), "{}", err.message);
    }
    assert!(mock.calls.is_empty());
}

#[test]
fn read_of_an_unknown_vm_is_not_found() {
    let mut mock = MockBackend::new(vec![
        Err("no VM named 'ghost'".into()),
        Ok(Reply::Vms(sample_vms())),
    ]);

    let err = read(&mut mock, read_params("cocovm://vm/ghost/screen.txt")).unwrap_err();

    assert_eq!(err.code, RESOURCE_NOT_FOUND);
    assert_eq!(mock.calls.len(), 2);
    assert_eq!(mock.calls[1].action, Action::ListVms);
}

#[test]
fn read_of_a_vm_that_is_not_running_is_an_internal_error() {
    let message = "VM 'vm1' is not running; call start_vm first";
    let mut mock = MockBackend::new(vec![Err(message.into()), Ok(Reply::Vms(sample_vms()))]);

    let err = read(&mut mock, read_params("cocovm://vm/vm1/screen.png")).unwrap_err();

    assert_eq!(err.code, INTERNAL_ERROR);
    assert_eq!(err.message, message);
}

#[test]
fn read_keeps_the_original_error_when_the_listing_fails_too() {
    let mut mock = MockBackend::new(vec![
        Err("control server overloaded; retry later".into()),
        Err("control server overloaded; retry later".into()),
    ]);

    let err = read(&mut mock, read_params("cocovm://vm/vm0/screen.txt")).unwrap_err();

    assert_eq!(err.code, INTERNAL_ERROR);
    assert_eq!(err.message, "control server overloaded; retry later");
}

#[test]
fn read_with_the_wrong_reply_variant_is_an_internal_error() {
    let mut mock = MockBackend::new(vec![Ok(screenshot_reply())]);
    let err = read(&mut mock, read_params("cocovm://vm/vm0/screen.txt")).unwrap_err();
    assert_eq!(err.code, INTERNAL_ERROR);
}

#[test]
fn every_uri_round_trips_through_parse_uri() {
    for resource in ScreenResource::ALL {
        let uri = resource.uri("coco-3-2");
        assert_eq!(
            ScreenResource::parse_uri(&uri),
            Some(("coco-3-2", resource))
        );
    }
}
