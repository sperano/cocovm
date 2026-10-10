use std::path::PathBuf;

use coco_core::drivewire::share::ShareAccess;

use super::*;
use crate::machine_def::{self, MachineDef};

const SHARES_TOML: &str = r#"
schema = 1
name = "Shares"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"

[drivewire]
enabled = true

[[drivewire.shares]]
name = "games"
path = "/srv/coco/games"
access = "read-write"

[[drivewire.shares]]
name = "docs"
path = "docs"

[[drivewire.shares]]
name = "pending"
"#;

fn parsed() -> MachineDef {
    toml::from_str(SHARES_TOML).expect("shares parse")
}

#[test]
fn shares_parse_with_read_only_as_the_default_access() {
    let shares = parsed().drivewire.shares;
    assert_eq!(
        shares,
        [
            DriveWireShareDTO {
                name: "games".into(),
                path: "/srv/coco/games".into(),
                access: ShareAccessDTO::ReadWrite,
            },
            DriveWireShareDTO {
                name: "docs".into(),
                path: "docs".into(),
                access: ShareAccessDTO::ReadOnly,
            },
            DriveWireShareDTO {
                name: "pending".into(),
                ..DriveWireShareDTO::default()
            },
        ]
    );
}

#[test]
fn shares_round_trip_through_toml_and_are_omitted_when_empty() {
    let def = parsed();
    let written = toml::to_string(&def).unwrap();
    assert!(written.contains("[[drivewire.shares]]"), "{written}");
    assert!(written.contains("access = \"read-write\""), "{written}");
    assert_eq!(toml::from_str::<MachineDef>(&written).unwrap(), def);

    let mut empty = def;
    empty.drivewire.shares.clear();
    assert!(!toml::to_string(&empty).unwrap().contains("shares"));
}

#[test]
fn the_share_table_resolves_folders_and_skips_entries_without_one() {
    let table = parsed().drivewire.share_table("shares").unwrap();
    let names: Vec<&str> = table.shares().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["games", "docs"]);
    let games = table.get("games").unwrap();
    assert_eq!(games.root, PathBuf::from("/srv/coco/games"));
    assert_eq!(games.access, ShareAccess::ReadWrite);
    assert_eq!(
        table.get("docs").unwrap().root,
        machine_def::resolve_media_path("docs", "shares")
    );
}

#[test]
fn bad_or_duplicate_names_fail_validation_even_without_a_folder() {
    let mut def = parsed();
    def.drivewire.shares[2].name = "GAMES".into();
    let error = def.validate_drivewire().unwrap_err();
    assert!(error.contains("used twice"), "{error}");
    def.drivewire.shares[2].name = "has space".into();
    assert!(def.validate_drivewire().unwrap_err().contains("has space"));
    assert!(def.to_machine_config().is_err());
}
