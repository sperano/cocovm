use crate::control::Reply;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::{AppParams, CocoApp, MachineConfig, ROMSource};

/// Boots a default CoCo 3 from the installed `coco3.rom`, like
/// `frame_test.rs`'s `boot()`.
fn boot() -> CocoApp {
    let rom_path = crate::installed_roms_dir().join(crate::rom_load::COCO3_ROM_FILE);
    let rom = std::fs::read(&rom_path)
        .expect("installed coco3.rom is required (first-run asset download)")
        .into_boxed_slice();
    CocoApp::new(
        MachineConfig::default(),
        rom,
        ROMSource::File(rom_path),
        AppParams::default(),
        crate::joy::SharedGamepad::without_backend(),
    )
}

#[test]
fn screen_text_reports_lines_mode_and_cursor() {
    let mut app = boot();
    let Reply::Screen {
        lines,
        mode,
        cursor,
    } = app.screen_text()
    else {
        panic!("expected Reply::Screen");
    };
    assert!(!lines.is_empty());
    assert!(!mode.is_empty());
    assert_eq!(cursor, app.machine.basic_text_cursor());
}

#[test]
fn screenshot_encodes_a_decodable_png_at_framebuffer_size() {
    let app = boot();
    let Reply::Screenshot {
        png_base64,
        width,
        height,
    } = app.screenshot().expect("screenshot should succeed")
    else {
        panic!("expected Reply::Screenshot");
    };
    assert_eq!(width, app.machine.fb_width);
    assert_eq!(height, app.machine.fb_height);
    let bytes = BASE64.decode(&png_base64).expect("valid base64");
    let decoded = image::load_from_memory(&bytes).expect("valid PNG");
    assert_eq!(decoded.width(), width);
    assert_eq!(decoded.height(), height);
}

#[test]
fn start_remote_typing_rejects_a_paused_vm() {
    let mut app = boot();
    app.set_running(false);
    let err = app
        .start_remote_typing("HI")
        .expect_err("paused VM must reject type_text");
    assert!(err.contains("paused"));
}

#[test]
fn start_remote_typing_queues_mapped_characters_and_reports_fields() {
    let mut app = boot();
    let fields = app.start_remote_typing("HI").expect("running VM accepts");
    assert_eq!(app.remote_type_ahead.queue.len(), 2);
    assert_eq!(fields, 2 * crate::typeahead::FIELDS_PER_TAP);
}

#[test]
fn start_remote_typing_rejects_overlong_text_and_a_draining_burst() {
    let mut app = boot();
    let too_long = "A".repeat(crate::control::MAX_TYPE_TEXT_CHARS + 1);
    let err = app
        .start_remote_typing(&too_long)
        .expect_err("overlong text must be rejected");
    assert!(err.contains("at most"));

    app.start_remote_typing("HI").expect("running VM accepts");
    let err = app
        .start_remote_typing("X")
        .expect_err("a second burst must wait for the first");
    assert!(err.contains("draining"));
}

#[test]
fn start_remote_hold_rejects_a_paused_vm() {
    let mut app = boot();
    app.set_running(false);
    let err = app
        .start_remote_hold(&["ENTER".to_string()], None)
        .expect_err("paused VM must reject press_keys");
    assert!(err.contains("paused"));
}

#[test]
fn start_remote_hold_rejects_an_unknown_key_name() {
    let mut app = boot();
    let err = app
        .start_remote_hold(&["NOT_A_KEY".to_string()], None)
        .expect_err("unknown key name must be rejected");
    assert!(err.contains("NOT_A_KEY"));
}

#[test]
fn start_remote_hold_rejects_a_second_concurrent_hold() {
    let mut app = boot();
    app.start_remote_hold(&["A".to_string()], None)
        .expect("first hold succeeds");
    let err = app
        .start_remote_hold(&["B".to_string()], None)
        .expect_err("a second hold must be rejected while one is in progress");
    assert!(err.contains("already in progress"));
}

#[test]
fn start_remote_hold_clamps_to_max_hold_fields() {
    let mut app = boot();
    app.start_remote_hold(
        &["A".to_string()],
        Some(crate::control::MAX_HOLD_FIELDS + 500),
    )
    .expect("hold succeeds");
    let hold = app.remote_held.as_ref().expect("hold recorded");
    assert_eq!(hold.fields_left, crate::control::MAX_HOLD_FIELDS);
}

#[test]
fn start_remote_hold_presses_every_resolved_key_immediately() {
    const ALL_COLUMNS_STROBED: u8 = 0x00;
    const NO_KEYS_DOWN: u8 = 0xFF;

    let mut app = boot();
    app.start_remote_hold(&["A".to_string()], None)
        .expect("hold succeeds");
    assert_ne!(
        app.machine.bus.keyboard.sense(ALL_COLUMNS_STROBED),
        NO_KEYS_DOWN
    );
}

#[test]
fn apply_remote_joystick_sets_then_release_clears() {
    let mut app = boot();
    app.apply_remote_joystick(
        crate::control::Stick::Left,
        Some(10),
        Some(20),
        None,
        None,
        false,
    );
    let left = coco_core::joystick::LEFT;
    let state = app.remote_joy[left].as_ref().expect("override recorded");
    assert_eq!((state.x, state.y), (10, 20));

    app.apply_remote_joystick(crate::control::Stick::Left, None, None, None, None, true);
    assert!(app.remote_joy[left].is_none());
}

#[test]
fn apply_remote_joystick_merges_into_an_existing_override() {
    let mut app = boot();
    let right = coco_core::joystick::RIGHT;
    app.apply_remote_joystick(
        crate::control::Stick::Right,
        Some(5),
        None,
        None,
        None,
        false,
    );
    app.apply_remote_joystick(
        crate::control::Stick::Right,
        None,
        Some(7),
        Some(true),
        None,
        false,
    );
    let state = app.remote_joy[right].as_ref().expect("override recorded");
    assert_eq!((state.x, state.y), (5, 7));
    assert_eq!(state.buttons, [true, false]);
}

#[test]
fn apply_remote_joystick_clamps_axes_to_axis_max() {
    let mut app = boot();
    let left = coco_core::joystick::LEFT;
    app.apply_remote_joystick(
        crate::control::Stick::Left,
        Some(255),
        Some(255),
        None,
        None,
        false,
    );
    let state = app.remote_joy[left].as_ref().expect("override recorded");
    assert_eq!(state.x, coco_core::joystick::AXIS_MAX);
    assert_eq!(state.y, coco_core::joystick::AXIS_MAX);
}

#[test]
fn peek_bytes_reads_what_poke_bytes_wrote() {
    let mut app = boot();
    app.poke_bytes(0x0400, &[1, 2, 3, 4]).unwrap();
    assert_eq!(app.peek_bytes(0x0400, 4), vec![1, 2, 3, 4]);
}

#[test]
fn poke_bytes_rejects_more_than_max_poke_len() {
    let mut app = boot();
    let too_many = vec![0; crate::control::MAX_POKE_LEN + 1];
    let err = app
        .poke_bytes(0x0400, &too_many)
        .expect_err("oversized poke must be rejected");
    assert!(err.contains("at most"));
}

#[test]
fn peek_bytes_clamps_to_max_peek_len() {
    let app = boot();
    let bytes = app.peek_bytes(0x0000, crate::control::MAX_PEEK_LEN + 1000);
    assert_eq!(bytes.len(), crate::control::MAX_PEEK_LEN as usize);
}

#[test]
fn remote_reset_soft_preserves_ram_hard_power_cycle_clears_it() {
    let mut app = boot();
    app.poke_bytes(0x0400, &[0xAB]).unwrap();
    app.remote_reset(false);
    assert_eq!(app.peek_bytes(0x0400, 1), vec![0xAB]);

    app.poke_bytes(0x0400, &[0xCD]).unwrap();
    app.remote_reset(true);
    assert_ne!(app.peek_bytes(0x0400, 1), vec![0xCD]);
}
