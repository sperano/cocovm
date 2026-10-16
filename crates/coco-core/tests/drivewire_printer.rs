//! Real-guest acceptance for DriveWire printing: the stock NitrOS-9 Level 2
//! 3.3.0 Becker image (`tests/nos96809l2v030300coco3_becker.dsk`) has the
//! `scdwp` driver and its `/P` descriptor in its bootfile. `scdwp` sends
//! `OP_PRINT` per byte and `OP_PRINTFLUSH` when the last path to `/P`
//! closes, so each redirected command is one job. Skips when an asset is
//! absent.

#[allow(dead_code)]
#[path = "drivewire_channels/common.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::Machine;
use coco_core::drivewire::{PrintFormat, PrintOutput};

use common::{boot_nitros9, dw, run_until, run_until_prompt_after, type_line};

const JOB_TEXT: &str = "HELLO FROM THE DRIVEWIRE PRINTER";

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("cocovm-dw-print-it-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn files(&self) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<_> = fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name().into_string().unwrap();
                (name, fs::read(entry.path()).unwrap())
            })
            .collect();
        files.sort();
        files
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn boot_printing(dir: &Path) -> Option<Machine> {
    let mut m = boot_nitros9()?;
    dw(&mut m).set_print_output(PrintOutput::Folder {
        dir: dir.to_path_buf(),
        format: PrintFormat::Text,
    });
    Some(m)
}

fn run_until_printed(m: &mut Machine, jobs: u64) {
    run_until(m, &format!("{jobs} printed jobs"), |m| {
        let server = dw(m);
        server.poll_host();
        server.print_diagnostics().printed >= jobs
    });
}

#[test]
fn echo_to_p_prints_one_job_into_the_folder() {
    let scratch = Scratch::new("echo");
    let Some(mut m) = boot_printing(&scratch.0) else {
        return;
    };
    let command = format!("echo {JOB_TEXT}>/p");
    type_line(&mut m, &command);
    run_until_prompt_after(&mut m, &command);
    run_until_printed(&mut m, 1);
    let files = scratch.files();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].0.ends_with("-job0001.txt"), "{}", files[0].0);
    assert_eq!(files[0].1, format!("{JOB_TEXT}\n").as_bytes());
    let diagnostics = dw(&mut m).print_diagnostics();
    assert_eq!((diagnostics.open_bytes, diagnostics.waiting), (0, 0));
}

#[test]
fn each_closed_path_is_its_own_job() {
    let scratch = Scratch::new("jobs");
    let Some(mut m) = boot_printing(&scratch.0) else {
        return;
    };
    for line in ["echo FIRST JOB>/p", "dir >/p"] {
        type_line(&mut m, line);
        run_until_prompt_after(&mut m, line);
    }
    run_until_printed(&mut m, 2);
    let files = scratch.files();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(files[0].0.contains("-job0001"));
    assert_eq!(files[0].1, b"FIRST JOB\n");
    let listing = String::from_utf8_lossy(&files[1].1);
    assert!(files[1].0.contains("-job0002"));
    assert!(listing.contains("CMDS"), "{listing}");
    assert!(listing.lines().count() > 1, "{listing}");
}

#[test]
fn printer_port_output_reaches_the_bit_banger_printer() {
    let Some(mut m) = boot_nitros9() else {
        return;
    };
    let paper = m
        .bus
        .bitbanger
        .start_printer(coco_core::dmp::DmpModel::Dmp105);
    dw(&mut m).set_print_output(PrintOutput::PrinterPort);
    let before = m.bus.bitbanger.bytes_out();
    let command = format!("echo {JOB_TEXT}>/p");
    type_line(&mut m, &command);
    run_until_prompt_after(&mut m, &command);
    run_until(&mut m, "the job on paper", |m| {
        m.bus.bitbanger.bytes_out() > before
    });
    // The job's text plus the CR that `echo` ends it with.
    let job_len = JOB_TEXT.len() as u64 + 1;
    assert_eq!(m.bus.bitbanger.bytes_out() - before, job_len);
    assert!(paper.paper_extent().dot_count > 0);
}

#[test]
fn printing_off_by_default_discards_the_job() {
    let Some(mut m) = boot_nitros9() else {
        return;
    };
    let before = m.bus.bitbanger.bytes_out();
    let command = format!("echo {JOB_TEXT}>/p");
    type_line(&mut m, &command);
    run_until_prompt_after(&mut m, &command);
    run_until(&mut m, "the discarded job", |m| {
        dw(m).print_diagnostics().discarded == 1
    });
    assert_eq!(m.bus.bitbanger.bytes_out(), before);
}
