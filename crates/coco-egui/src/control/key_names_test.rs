use super::*;

#[test]
fn named_keys_resolve_case_insensitively() {
    assert_eq!(key_pos("enter"), Some((kbd::ENTER, false)));
    assert_eq!(key_pos("Break"), Some((kbd::BREAK, false)));
    assert_eq!(key_pos("F1"), Some((kbd::F1, false)));
}

#[test]
fn single_characters_go_through_char_key() {
    assert_eq!(key_pos("a"), kbd::char_key('a'));
    assert_eq!(key_pos("!"), kbd::char_key('!'));
    assert_eq!(key_pos("!").map(|(_, shift)| shift), Some(true));
}

#[test]
fn unknown_or_multi_char_names_are_rejected() {
    assert_eq!(key_pos("HOME"), None);
    assert_eq!(key_pos("ab"), None);
    assert_eq!(key_pos(""), None);
}

#[test]
fn describe_lists_every_named_key() {
    let text = describe();
    for (name, _) in NAMED_KEYS {
        assert!(text.contains(name), "{name} missing from {text}");
    }
}
