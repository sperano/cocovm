use super::*;

fn parse_str(line: &str) -> Result<Command, Failure> {
    parse(line.as_bytes())
}

fn failure_code(line: &str) -> u16 {
    parse_str(line).expect_err(line).code
}

#[test]
fn server_commands_take_the_rest_of_the_line_as_the_path() {
    assert_eq!(
        parse_str("dw server dir games"),
        Ok(Command::ServerDir {
            path: b"games".to_vec()
        })
    );
    assert_eq!(
        parse_str("dw  server   list  games/My File.txt  "),
        Ok(Command::ServerList {
            path: b"games/My File.txt".to_vec()
        })
    );
}

#[test]
fn verbs_ignore_case_and_accept_unique_prefixes() {
    assert_eq!(
        parse_str("DW SERV D /"),
        Ok(Command::ServerDir {
            path: b"/".to_vec()
        })
    );
    assert_eq!(
        parse_str("dw d i 1 games/a.dsk"),
        Ok(Command::DiskInsert {
            drive: 1,
            path: b"games/a.dsk".to_vec()
        })
    );
}

#[test]
fn unknown_and_ambiguous_verbs_are_syntax_errors() {
    let unknown = parse_str("dw config show").unwrap_err();
    assert_eq!(unknown.code, code::SYNTAX_ERROR);
    assert_eq!(unknown.text, "Unknown command 'config'");
    assert_eq!(failure_code("dw disk create 1 x"), code::SYNTAX_ERROR);
}

#[test]
fn a_missing_path_is_a_syntax_error() {
    let failure = parse_str("dw server dir   ").unwrap_err();
    assert_eq!(
        failure,
        Failure::new(
            code::SYNTAX_ERROR,
            "dw server dir requires a path as an argument"
        )
    );
    assert_eq!(failure_code("dw server list"), code::SYNTAX_ERROR);
    assert_eq!(failure_code("dw disk insert 1"), code::SYNTAX_ERROR);
    assert_eq!(failure_code("dw disk eject"), code::SYNTAX_ERROR);
    assert_eq!(failure_code("dw disk eject 1 2"), code::SYNTAX_ERROR);
    assert_eq!(failure_code("dw disk show 1 2"), code::SYNTAX_ERROR);
}

#[test]
fn a_verb_alone_lists_its_subcommands() {
    for (line, verbs) in [
        ("dw", "disk    server"),
        ("dw server", "dir   list"),
        ("dw disk", "eject   insert  show"),
    ] {
        let Ok(Command::Text(text)) = parse_str(line) else {
            panic!("{line} should list verbs");
        };
        let text = String::from_utf8(text).unwrap();
        assert!(text.starts_with("Possible commands:\r\n\r\n"), "{text:?}");
        assert_eq!(text.lines().nth(2).unwrap().trim_end(), verbs, "{line}");
    }
}

#[test]
fn disk_commands_parse_drives() {
    assert_eq!(
        parse_str("dw disk show"),
        Ok(Command::DiskShow { drive: None })
    );
    assert_eq!(
        parse_str("dw disk show 3"),
        Ok(Command::DiskShow { drive: Some(3) })
    );
    assert_eq!(
        parse_str("dw disk eject all"),
        Ok(Command::DiskEject { drive: None })
    );
    assert_eq!(
        parse_str("dw disk eject 0"),
        Ok(Command::DiskEject { drive: Some(0) })
    );
}

#[test]
fn bad_drive_numbers_are_invalid_drives() {
    let range = parse_str("dw disk insert 4 games/a.dsk").unwrap_err();
    assert_eq!(range.code, code::INVALID_DRIVE);
    assert_eq!(
        range.text,
        "There is no drive 4. Valid drive numbers are 0 - 3"
    );
    let numeric = parse_str("dw disk eject one").unwrap_err();
    assert_eq!(
        numeric,
        Failure::new(code::INVALID_DRIVE, "Drive numbers must be numeric")
    );
    assert_eq!(failure_code("dw disk show -1"), code::INVALID_DRIVE);
    // Like the Java server, only lowercase `all` ejects every drive.
    assert_eq!(failure_code("dw disk eject ALL"), code::INVALID_DRIVE);
}
