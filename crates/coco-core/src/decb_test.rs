use super::*;

const DATA_MARKER: u8 = 0x00;
const TRAILER_MARKER: u8 = 0xFF;

fn data(address: u16, bytes: &[u8]) -> Vec<u8> {
    let mut record = vec![DATA_MARKER];
    record.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    record.extend_from_slice(&address.to_be_bytes());
    record.extend_from_slice(bytes);
    record
}

fn trailer(exec_address: u16) -> Vec<u8> {
    let mut record = vec![TRAILER_MARKER, 0, 0];
    record.extend_from_slice(&exec_address.to_be_bytes());
    record
}

#[test]
fn parses_multiple_data_blocks_and_execution_address() {
    let mut input = data(0x1200, &[1, 2, 3]);
    input.extend(data(0xFFFE, &[4, 5, 6]));
    input.extend(trailer(0x3456));

    let binary = DecbBinary::parse(&input).unwrap();

    assert_eq!(
        binary.segments,
        [
            DecbSegment {
                address: 0x1200,
                bytes: vec![1, 2, 3]
            },
            DecbSegment {
                address: 0xFFFE,
                bytes: vec![4, 5, 6]
            }
        ]
    );
    assert_eq!(binary.exec_address, 0x3456);
}

#[test]
fn accepts_a_trailer_without_data_blocks() {
    let binary = DecbBinary::parse(&trailer(0x8000)).unwrap();
    assert!(binary.segments.is_empty());
    assert_eq!(binary.exec_address, 0x8000);
}

#[test]
fn zero_data_count_means_65536_bytes() {
    const FULL_ADDRESS_SPACE: usize = u16::MAX as usize + 1;
    let mut input = vec![DATA_MARKER, 0, 0, 0x80, 0x00];
    input.extend((0..FULL_ADDRESS_SPACE).map(|value| value as u8));
    input.extend(trailer(0x8000));

    let binary = DecbBinary::parse(&input).unwrap();

    assert_eq!(binary.segments[0].bytes.len(), FULL_ADDRESS_SPACE);
    assert_eq!(binary.segments[0].bytes[0], 0);
    assert_eq!(binary.segments[0].bytes[FULL_ADDRESS_SPACE - 1], 0xFF);
}

#[test]
fn rejects_empty_truncated_missing_and_unknown_records() {
    let cases: &[(&[u8], &str)] = &[
        (&[], "empty"),
        (&[DATA_MARKER, 0], "header"),
        (&[DATA_MARKER, 0, 2, 0x10, 0, 1], "data block"),
        (&[DATA_MARKER, 0, 1, 0x10, 0, 1], "trailer"),
        (&[0x7E, 0, 0, 0, 0], "marker"),
    ];

    for (input, expected) in cases {
        let error = DecbBinary::parse(input).unwrap_err().to_string();
        assert!(
            error.contains(expected),
            "{error:?} did not contain {expected:?}"
        );
    }
}

#[test]
fn rejects_noncanonical_trailer_and_trailing_bytes() {
    let nonzero_count = [TRAILER_MARKER, 0, 1, 0x12, 0x34];
    assert!(
        DecbBinary::parse(&nonzero_count)
            .unwrap_err()
            .to_string()
            .contains("trailer count")
    );

    let mut trailing = trailer(0x1234);
    trailing.push(0);
    assert!(
        DecbBinary::parse(&trailing)
            .unwrap_err()
            .to_string()
            .contains("trailing")
    );
}

#[test]
fn does_not_return_partial_segments_after_a_late_error() {
    let mut input = data(0x1000, &[1, 2, 3]);
    input.extend([0x7E, 0, 0, 0, 0]);

    assert!(DecbBinary::parse(&input).is_err());
}
