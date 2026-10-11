use super::*;

fn classify_str(text: &str) -> Line {
    classify(text.bytes())
}

#[test]
fn a_dw_line_is_complete_at_its_cr() {
    assert_eq!(classify_str("dw server dir games\r"), Line::Complete(20));
    assert_eq!(classify_str("DW disk show\rmore"), Line::Complete(13));
    assert_eq!(classify_str("dw\r"), Line::Complete(3));
}

#[test]
fn leading_blanks_and_nul_bytes_are_skipped() {
    assert_eq!(classify_str("\r \0\tdw disk\r"), Line::Complete(12));
    assert_eq!(classify([b'd', 0, b'w', b'\r']), Line::Complete(4));
}

#[test]
fn a_partial_line_waits_for_more_bytes() {
    for partial in ["", "\r", "d", "dw", "dw ", "dw server dir games"] {
        assert_eq!(classify_str(partial), Line::Undecided, "{partial:?}");
    }
}

#[test]
fn other_first_words_are_not_commands() {
    for line in [
        "HELLO FROM NITROS9 \r",
        "dwx\r",
        "x",
        "d \r",
        "tcp connect\r",
        "ui x\r",
    ] {
        assert_eq!(classify_str(line), Line::NotCommand, "{line:?}");
    }
}

#[test]
fn a_dw_line_without_cr_is_too_long_at_the_limit() {
    let mut line = b"dw server list ".to_vec();
    line.resize(MAX_LINE_BYTES, b'a');
    assert_eq!(classify(line.iter().copied()), Line::TooLong);
    line.truncate(MAX_LINE_BYTES - 1);
    assert_eq!(classify(line.iter().copied()), Line::Undecided);
    line.push(b'\r');
    assert_eq!(
        classify(line.iter().copied()),
        Line::Complete(MAX_LINE_BYTES)
    );
}

#[test]
fn endless_blank_bytes_are_not_a_command() {
    assert_eq!(classify(vec![b'\r'; MAX_LINE_BYTES + 1]), Line::NotCommand);
}
