use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::drivewire::{TRANSACTION_TIMEOUT_CYCLES, opcode};

const CR: u8 = b'\r';
const WAIT_LIMIT: Duration = Duration::from_secs(10);
const WAIT_STEP: Duration = Duration::from_millis(2);
const LARGE_JOB_TAIL: usize = 3;
const COCO3_ROM_SIZE: usize = 32 * 1024;

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

/// A fresh scratch folder, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cocovm-dw-print-{name}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn files(&self) -> Vec<(String, Vec<u8>)> {
        files_in(&self.0)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn files_in(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .map(|entry| {
                    let entry = entry.unwrap();
                    let name = entry.file_name().into_string().unwrap();
                    (name, fs::read(entry.path()).unwrap())
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn folder(dir: &Path, format: PrintFormat) -> PrintOutput {
    PrintOutput::Folder {
        dir: dir.to_path_buf(),
        format,
    }
}

/// Sends `text` the way `scdwp` does: `OP_PRINT` and the byte, per byte.
fn print(server: &mut DWServer, text: &[u8]) {
    for &byte in text {
        server.data_write(opcode::PRINT, 0);
        server.data_write(byte, 0);
    }
}

fn flush(server: &mut DWServer) {
    server.data_write(opcode::PRINTFLUSH, 0);
}

fn wait_until(server: &mut DWServer, done: impl Fn(&PrintDiagnostics) -> bool) {
    let start = Instant::now();
    loop {
        server.poll_host();
        if done(&server.print_diagnostics()) {
            return;
        }
        assert!(
            start.elapsed() < WAIT_LIMIT,
            "{:?}",
            server.print_diagnostics()
        );
        std::thread::sleep(WAIT_STEP);
    }
}

fn wait_for_files(dir: &Path, count: usize) -> Vec<(String, Vec<u8>)> {
    let start = Instant::now();
    loop {
        let files = files_in(dir);
        if files.len() >= count {
            return files;
        }
        assert!(start.elapsed() < WAIT_LIMIT, "{files:?}");
        std::thread::sleep(WAIT_STEP);
    }
}

fn port_server() -> DWServer {
    let mut server = DWServer::new();
    server.set_print_output(PrintOutput::PrinterPort);
    server
}

#[test]
fn flushed_job_becomes_one_raw_file() {
    let scratch = Scratch::new("raw");
    let mut server = DWServer::new();
    server.set_print_output(folder(&scratch.0, PrintFormat::Raw));
    print(&mut server, b"HELLO\r");
    assert_eq!(server.print_diagnostics().open_bytes, 6);
    assert!(scratch.files().is_empty());
    flush(&mut server);
    wait_until(&mut server, |d| d.printed == 1);
    let files = scratch.files();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, "print-19900101-000000-job0001.prn");
    assert_eq!(files[0].1, b"HELLO\r");
    let diagnostics = server.print_diagnostics();
    assert_eq!(diagnostics.waiting, 0);
    assert_eq!(diagnostics.last_file, Some(scratch.0.join(&files[0].0)));
    assert!(server.status_read() == 0, "printing never replies");
}

#[test]
fn text_format_converts_line_ends() {
    let scratch = Scratch::new("text");
    let mut server = DWServer::new();
    server.set_print_output(folder(&scratch.0, PrintFormat::Text));
    print(&mut server, b"A\rB\r\nC");
    flush(&mut server);
    wait_until(&mut server, |d| d.printed == 1);
    assert_eq!(
        scratch.files(),
        [(
            "print-19900101-000000-job0001.txt".into(),
            b"A\nB\nC".to_vec()
        )]
    );
}

#[test]
fn opcode_values_are_print_payload() {
    let mut server = port_server();
    print(
        &mut server,
        &[opcode::PRINTFLUSH, opcode::PRINT, opcode::RESET3],
    );
    flush(&mut server);
    let job = server.take_port_print_job().unwrap();
    assert_eq!(
        job.bytes,
        [opcode::PRINTFLUSH, opcode::PRINT, opcode::RESET3]
    );
    assert_eq!(job.end, JobEnd::Flushed);
}

#[test]
fn empty_flush_prints_nothing_and_keeps_numbering() {
    let mut server = port_server();
    flush(&mut server);
    flush(&mut server);
    assert!(server.take_port_print_job().is_none());
    assert_eq!(server.print_diagnostics().empty, 2);
    print(&mut server, b"X");
    flush(&mut server);
    assert_eq!(server.take_port_print_job().unwrap().number, 1);
}

#[test]
fn timed_out_print_drops_only_its_own_byte() {
    let mut server = port_server();
    print(&mut server, b"A");
    server.data_write(opcode::PRINT, 0);
    server.data_write(opcode::PRINTFLUSH, TRANSACTION_TIMEOUT_CYCLES + 1);
    let job = server.take_port_print_job().unwrap();
    assert_eq!(job.bytes, b"A");
    assert_eq!(server.print_diagnostics().open_bytes, 0);
}

#[test]
fn large_job_is_delivered_in_numbered_parts() {
    let mut server = port_server();
    for _ in 0..PRINT_JOB_PART_BYTES + LARGE_JOB_TAIL {
        server.print_byte(b'x');
    }
    let first = server.take_port_print_job().unwrap();
    assert_eq!((first.number, first.part, first.end), (1, 1, JobEnd::Split));
    assert_eq!(first.bytes.len(), PRINT_JOB_PART_BYTES);
    assert_eq!(server.print_diagnostics().open_bytes, LARGE_JOB_TAIL);
    flush(&mut server);
    let last = server.take_port_print_job().unwrap();
    assert_eq!((last.number, last.part, last.end), (1, 2, JobEnd::Flushed));
    assert_eq!(last.bytes.len(), LARGE_JOB_TAIL);
    print(&mut server, b"y");
    flush(&mut server);
    let next = server.take_port_print_job().unwrap();
    assert_eq!((next.number, next.part), (2, 1));
}

#[test]
fn job_ending_exactly_at_a_part_boundary_starts_the_next_job() {
    let mut server = port_server();
    for _ in 0..PRINT_JOB_PART_BYTES {
        server.print_byte(b'x');
    }
    flush(&mut server);
    assert_eq!(server.take_port_print_job().unwrap().end, JobEnd::Split);
    assert!(server.take_port_print_job().is_none());
    assert_eq!(server.print_diagnostics().empty, 0);
    print(&mut server, b"y");
    flush(&mut server);
    assert_eq!(server.take_port_print_job().unwrap().number, 2);
}

#[test]
fn output_off_discards_and_counts_jobs() {
    let mut server = DWServer::new();
    assert_eq!(server.print_output(), &PrintOutput::Off);
    print(&mut server, b"secret");
    flush(&mut server);
    assert!(server.take_port_print_job().is_none());
    assert_eq!(server.print_diagnostics().discarded, 1);
}

#[test]
fn failed_write_keeps_the_job_until_output_works() {
    let scratch = Scratch::new("error");
    let blocker = scratch.0.join("not-a-folder");
    fs::write(&blocker, b"").unwrap();
    let good = Scratch::new("error-good");
    let mut server = DWServer::new();
    server.set_print_output(folder(&blocker.join("prints"), PrintFormat::Raw));
    print(&mut server, b"KEEP");
    flush(&mut server);
    wait_until(&mut server, |d| d.last_error.is_some());
    let diagnostics = server.print_diagnostics();
    assert_eq!((diagnostics.waiting, diagnostics.printed), (1, 0));
    server.set_print_output(folder(&good.0, PrintFormat::Raw));
    wait_until(&mut server, |d| d.printed == 1);
    assert_eq!(good.files().len(), 1);
    assert_eq!(good.files()[0].1, b"KEEP");
    let diagnostics = server.print_diagnostics();
    assert_eq!((diagnostics.waiting, diagnostics.last_error), (0, None));
}

#[test]
fn full_queue_drops_new_jobs_visibly() {
    let scratch = Scratch::new("full");
    let blocker = scratch.0.join("file");
    fs::write(&blocker, b"").unwrap();
    let mut server = DWServer::new();
    server.set_print_output(folder(&blocker.join("prints"), PrintFormat::Raw));
    for _ in 0..=PRINT_QUEUE_JOBS {
        print(&mut server, b"j");
        flush(&mut server);
    }
    let diagnostics = server.print_diagnostics();
    assert_eq!(diagnostics.waiting, PRINT_QUEUE_JOBS);
    assert_eq!(diagnostics.dropped, 1);
    assert!(diagnostics.last_error.unwrap().contains("dropped"));
}

#[test]
fn switching_to_off_discards_waiting_jobs() {
    let scratch = Scratch::new("off");
    let blocker = scratch.0.join("file");
    fs::write(&blocker, b"").unwrap();
    let mut server = DWServer::new();
    server.set_print_output(folder(&blocker.join("prints"), PrintFormat::Raw));
    print(&mut server, b"j");
    flush(&mut server);
    wait_until(&mut server, |d| d.last_error.is_some());
    server.set_print_output(PrintOutput::Off);
    let diagnostics = server.print_diagnostics();
    assert_eq!((diagnostics.waiting, diagnostics.discarded), (0, 1));
}

#[test]
fn protocol_resets_keep_the_open_job() {
    let mut server = port_server();
    print(&mut server, b"AB");
    server.data_write(opcode::DWINIT, 0);
    server.data_write(0, 0);
    server.data_write(opcode::RESET1, 0);
    print(&mut server, b"C");
    flush(&mut server);
    assert_eq!(server.take_port_print_job().unwrap().bytes, b"ABC");
}

#[test]
fn machine_reset_ends_the_open_job_as_incomplete() {
    let mut server = port_server();
    print(&mut server, b"half");
    server.machine_reset();
    let job = server.take_port_print_job().unwrap();
    assert_eq!(
        (job.bytes.as_slice(), job.end),
        (&b"half"[..], JobEnd::Unterminated)
    );
    assert_eq!(server.print_diagnostics().incomplete, 1);
    print(&mut server, b"next");
    flush(&mut server);
    assert_eq!(server.take_port_print_job().unwrap().number, 2);
}

#[test]
fn stop_writes_the_incomplete_job_after_the_session_ends() {
    let scratch = Scratch::new("stop");
    let mut server = DWServer::new();
    server.set_print_output(folder(&scratch.0, PrintFormat::Raw));
    print(&mut server, b"done");
    flush(&mut server);
    print(&mut server, b"half");
    server.stop_host();
    drop(server);
    let files = wait_for_files(&scratch.0, 2);
    assert_eq!(
        files[0],
        ("print-19900101-000000-job0001.prn".into(), b"done".to_vec())
    );
    assert_eq!(
        files[1],
        (
            "print-19900101-000000-job0002-incomplete.prn".into(),
            b"half".to_vec()
        )
    );
}

#[test]
fn stop_after_suspend_leaves_the_open_job_to_the_suspend_state() {
    let mut server = port_server();
    print(&mut server, b"half");
    server.suspend_host();
    server.stop_host();
    assert!(server.take_port_print_job().is_none());
    assert_eq!(server.print_diagnostics().open_bytes, 0);
}

#[test]
fn resume_lets_the_suspended_job_continue() {
    let mut server = port_server();
    print(&mut server, b"AB");
    server.suspend_host();
    server.resume_host();
    print(&mut server, b"C");
    flush(&mut server);
    assert_eq!(server.take_port_print_job().unwrap().bytes, b"ABC");
    server.stop_host();
    assert!(server.take_port_print_job().is_none());
}

#[test]
fn snapshot_carries_the_open_job_but_not_finished_ones() {
    let mut server = port_server();
    print(&mut server, b"old");
    flush(&mut server);
    print(&mut server, b"AB");
    server.data_write(opcode::PRINT, 0);
    let mut restored: DWServer =
        serde_json::from_str(&serde_json::to_string(&server).unwrap()).unwrap();
    restored.after_restore();
    assert_eq!(restored.print_output(), &PrintOutput::Off);
    restored.set_print_output(PrintOutput::PrinterPort);
    assert!(restored.take_port_print_job().is_none());
    restored.data_write(b'C', 1);
    flush(&mut restored);
    let job = restored.take_port_print_job().unwrap();
    assert_eq!((job.number, job.bytes.as_slice()), (2, &b"ABC"[..]));
}

#[test]
fn older_snapshot_without_a_spool_loads_with_no_open_job() {
    let server = DWServer::new();
    let mut json: serde_json::Value = serde_json::to_value(&server).unwrap();
    json.as_object_mut().unwrap().remove("printer");
    let mut restored: DWServer = serde_json::from_value(json).unwrap();
    restored.set_print_output(PrintOutput::PrinterPort);
    print(&mut restored, b"Z");
    flush(&mut restored);
    assert_eq!(restored.take_port_print_job().unwrap().number, 1);
}

#[test]
fn two_vms_print_into_one_folder_without_overwriting() {
    let scratch = Scratch::new("two-vms");
    let mut first = DWServer::new();
    let mut second = DWServer::new();
    for server in [&mut first, &mut second] {
        server.set_print_output(folder(&scratch.0, PrintFormat::Raw));
    }
    print(&mut first, b"first");
    print(&mut second, b"second");
    second.machine_reset();
    assert_eq!(first.print_diagnostics().open_bytes, b"first".len());
    flush(&mut first);
    wait_until(&mut first, |d| d.printed == 1);
    wait_until(&mut second, |d| d.printed == 1);
    let mut contents: Vec<_> = scratch.files().into_iter().map(|(_, b)| b).collect();
    contents.sort();
    assert_eq!(contents, [b"first".to_vec(), b"second".to_vec()]);
    assert_eq!(second.print_diagnostics().incomplete, 1);
    assert_eq!(first.print_diagnostics().incomplete, 0);
}

#[test]
fn machine_reset_and_stop_feed_the_printer_port() {
    let mut machine = crate::Machine::new(
        crate::MachineConfig::default(),
        vec![0; COCO3_ROM_SIZE].into_boxed_slice(),
    );
    machine.bus.enable_drivewire();
    let paper = machine
        .bus
        .bitbanger
        .start_printer(crate::dmp::DmpModel::Dmp105);
    let dw = machine.bus.drivewire.as_mut().unwrap();
    dw.set_print_output(PrintOutput::PrinterPort);
    print(dw, b"HI");
    machine.reset();
    assert_eq!(machine.bus.bitbanger.bytes_out(), 2);
    assert!(!machine.bus.drivewire.as_ref().unwrap().has_port_print_job());
    let dw = machine.bus.drivewire.as_mut().unwrap();
    dw.data_write(opcode::PRINT, 0);
    dw.data_write(CR, 0);
    machine.bus.stop_drivewire_host();
    assert_eq!(machine.bus.bitbanger.bytes_out(), 3);
    assert!(paper.paper_extent().dot_count > 0);
}
