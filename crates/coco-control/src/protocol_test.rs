use super::*;

#[test]
fn request_round_trips_with_flattened_action() {
    let req = Request {
        vm: Some("coco3".into()),
        action: Action::PressKeys {
            keys: vec!["A".into(), "SHIFT".into()],
            hold_fields: Some(3),
        },
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"cmd\":\"press_keys\""));
    assert!(json.contains("\"vm\":\"coco3\""));
    assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), req);
}

#[test]
fn unit_action_without_vm_parses() {
    let req: Request = serde_json::from_str(r#"{"cmd":"list_vms"}"#).unwrap();
    assert_eq!(req.vm, None);
    assert_eq!(req.action, Action::ListVms);
}

#[test]
fn joystick_defaults_are_optional() {
    let req: Request = serde_json::from_str(r#"{"cmd":"joystick","stick":"left","x":10}"#).unwrap();
    assert_eq!(
        req.action,
        Action::Joystick {
            stick: Stick::Left,
            x: Some(10),
            y: None,
            button1: None,
            button2: None,
            release: false,
        }
    );
}

#[test]
fn responses_are_externally_tagged() {
    assert_eq!(
        serde_json::to_string(&Response::Ok(Reply::Done)).unwrap(),
        r#"{"ok":"done"}"#
    );
    assert_eq!(
        serde_json::to_string(&Response::Err("nope".into())).unwrap(),
        r#"{"err":"nope"}"#
    );
    let screen = Response::Ok(Reply::Screen {
        lines: vec!["OK".into()],
        mode: "VDG text".into(),
    });
    let json = serde_json::to_string(&screen).unwrap();
    assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), screen);
}

#[test]
fn unknown_command_is_rejected() {
    assert!(serde_json::from_str::<Request>(r#"{"cmd":"explode"}"#).is_err());
}
