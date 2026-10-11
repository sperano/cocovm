use super::*;

fn object(leased: bool, write_protected: bool, attached: bool) -> NamedObjectInfo {
    NamedObjectInfo {
        drive: u8::MAX,
        name: "boot.bin".into(),
        guest_path: "/games/boot.bin".into(),
        write_protected,
        leased,
        attached,
    }
}

#[test]
fn the_selection_follows_a_renamed_share_and_clears_on_removal() {
    let mut named = Some("games".to_string());
    follow_share(&mut named, "other", Some("renamed"));
    assert_eq!(named.as_deref(), Some("games"), "another share changed");
    follow_share(&mut named, "GAMES", Some("Games2"));
    assert_eq!(named.as_deref(), Some("Games2"));
    follow_share(&mut named, "games2", None);
    assert_eq!(named, None);
}

#[test]
fn object_lines_name_the_drive_path_and_state() {
    assert_eq!(
        object_line(&object(false, false, true)),
        "Named object DW255: /games/boot.bin"
    );
    assert_eq!(
        object_line(&object(true, true, false)),
        "Named object DW255: /games/boot.bin (read-only, leased, not reopened)"
    );
}
