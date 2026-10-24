use super::*;
use crate::drivewire::DWTime;

const STAMP: DWTime = DWTime {
    year: 2026,
    month: 10,
    day: 13,
    hour: 9,
    minute: 5,
    second: 7,
};

fn job(number: u64, part: u32, end: JobEnd) -> PrintJob {
    PrintJob {
        number,
        part,
        end,
        stamp: STAMP,
        bytes: b"x".to_vec(),
        leftover: None,
    }
}

#[test]
fn host_text_turns_every_guest_line_end_into_lf() {
    assert_eq!(to_host_text(b"a\rb\r\nc\n\rd\r"), b"a\nb\nc\n\nd\n");
    assert_eq!(to_host_text(b"\r\r\n"), b"\n\n");
    assert_eq!(to_host_text([0, 0x1B, 0xFF]), [0, 0x1B, 0xFF]);
}

#[test]
fn file_names_carry_time_job_part_and_end() {
    assert_eq!(
        file_stem(&job(7, 1, JobEnd::Flushed)),
        "print-20261013-090507-job0007"
    );
    assert_eq!(
        file_stem(&job(7, 1, JobEnd::Split)),
        "print-20261013-090507-job0007-part1"
    );
    assert_eq!(
        file_stem(&job(7, 2, JobEnd::Flushed)),
        "print-20261013-090507-job0007-part2"
    );
    assert_eq!(
        file_stem(&job(12345, 3, JobEnd::Unterminated)),
        "print-20261013-090507-job12345-part3-incomplete"
    );
}

#[test]
fn write_job_never_overwrites_an_existing_file() {
    let dir = std::env::temp_dir().join(format!("cocovm-dw-print-names-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let first = write_job(&mut job(1, 1, JobEnd::Flushed), &dir, PrintFormat::Raw).unwrap();
    let second = write_job(&mut job(1, 1, JobEnd::Flushed), &dir, PrintFormat::Text).unwrap();
    let third = write_job(&mut job(1, 1, JobEnd::Flushed), &dir, PrintFormat::Raw).unwrap();
    let name = |path: &PathBuf| path.file_name().unwrap().to_str().unwrap().to_owned();
    assert_eq!(name(&first), "print-20261013-090507-job0001.prn");
    assert_eq!(name(&second), "print-20261013-090507-job0001.txt");
    assert_eq!(name(&third), "print-20261013-090507-job0001-1.prn");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn write_job_creates_only_the_last_folder() {
    let root = std::env::temp_dir().join(format!("cocovm-dw-print-leaf-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let nested = root.join("missing").join("prints");
    let error = write_job(&mut job(1, 1, JobEnd::Flushed), &nested, PrintFormat::Raw).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(!root.exists());
}

#[test]
fn a_leftover_partial_file_blocks_its_folder_until_it_is_removed() {
    let dir = std::env::temp_dir().join(format!("cocovm-dw-print-left-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let leftover = dir.join("print-20261013-090507-job0001.prn");
    // A non-empty folder at the leftover's path makes its removal fail.
    fs::create_dir_all(leftover.join("stuck")).unwrap();
    let mut first = job(1, 1, JobEnd::Flushed);
    first.leftover = Some(leftover.clone());

    let error = write_job(&mut first, &dir, PrintFormat::Raw).unwrap_err();
    assert!(error.to_string().contains("not removed"), "{error}");
    assert_eq!(first.leftover.as_ref(), Some(&leftover));
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

    fs::remove_dir_all(&leftover).unwrap();
    fs::write(&leftover, b"partial").unwrap();
    let written = write_job(&mut first, &dir, PrintFormat::Raw).unwrap();
    assert_eq!(written, leftover);
    assert_eq!(fs::read(&written).unwrap(), b"x");
    assert_eq!(first.leftover, None);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_leftover_in_another_folder_does_not_block_the_write() {
    let root = std::env::temp_dir().join(format!("cocovm-dw-print-away-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let leftover = root.join("old").join("print-20261013-090507-job0001.prn");
    fs::create_dir_all(leftover.join("stuck")).unwrap();
    let mut moved = job(1, 1, JobEnd::Flushed);
    moved.leftover = Some(leftover);

    let written = write_job(&mut moved, &root.join("new"), PrintFormat::Raw).unwrap();
    assert_eq!(fs::read(written).unwrap(), b"x");
    assert_eq!(moved.leftover, None);
    fs::remove_dir_all(&root).unwrap();
}
