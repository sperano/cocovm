use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::drivewire::host::HostExecutor;
use crate::drivewire::{DWServer, checksum_of, error, opcode};

const DRIVE: usize = 0;
const OLD_BYTE: u8 = 0xA5;
const NEW_BYTE: u8 = 0x5A;
const QUEUE_CAPACITY: usize = 1;
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct ScratchImage(PathBuf);

impl ScratchImage {
    fn new() -> (Self, File) {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cocovm-dw-{}-{id}.dsk", std::process::id()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.write_all(&[OLD_BYTE; SECTOR_SIZE]).unwrap();
        (Self(path), file)
    }
}

impl Drop for ScratchImage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn feed(server: &mut DWServer, bytes: &[u8]) {
    for (cycle, byte) in bytes.iter().enumerate() {
        server.data_write(*byte, cycle as u64);
    }
}

fn drain(server: &mut DWServer) -> Vec<u8> {
    let mut reply = Vec::new();
    while server.status_read() != 0 {
        reply.push(server.data_read());
    }
    reply
}

fn request(write: bool) -> Vec<u8> {
    let mut request = vec![
        if write { opcode::WRITE } else { opcode::READ },
        DRIVE as u8,
        0,
        0,
        0,
    ];
    if write {
        let sector = [NEW_BYTE; SECTOR_SIZE];
        request.extend(sector);
        request.extend(checksum_of(&sector).to_be_bytes());
    }
    request
}

#[test]
fn file_operations_finish_only_after_injected_host_completion() {
    let (scratch, file) = ScratchImage::new();
    let (executor, manual) = HostExecutor::manual(QUEUE_CAPACITY);
    let mut server = DWServer::with_host_executor(executor);
    server.mount(DRIVE, DWImage::File(file));
    for write in [false, true] {
        feed(&mut server, &request(write));
        assert!(drain(&mut server).is_empty());
        manual
            .complete(manual.take_request().unwrap().run())
            .unwrap();
        server.poll_host();
        let reply = drain(&mut server);
        assert_eq!(reply[0], error::OK);
        if !write {
            assert_eq!(&reply[1..=SECTOR_SIZE], &[OLD_BYTE; SECTOR_SIZE]);
        }
    }
    assert_eq!(std::fs::read(&scratch.0).unwrap(), [NEW_BYTE; SECTOR_SIZE]);
    assert_eq!(server.sectors_read(), 1);
    assert_eq!(server.sectors_written(), 1);
}

#[test]
fn changing_media_invalidates_old_file_results() {
    for write in [false, true] {
        for eject in [false, true] {
            let (_scratch, file) = ScratchImage::new();
            let (executor, manual) = HostExecutor::manual(QUEUE_CAPACITY);
            let mut server = DWServer::with_host_executor(executor);
            server.mount(DRIVE, DWImage::File(file));
            feed(&mut server, &request(write));
            // Simulates I/O that finished before replacement but whose result
            // has not yet reached the emulated device.
            let completion = manual.take_request().unwrap().run();
            if eject {
                server.eject(DRIVE);
            } else {
                server.mount(DRIVE, DWImage::Memory(vec![NEW_BYTE; SECTOR_SIZE]));
            }
            let expected = if write { error::WRITE } else { error::READ };
            assert_eq!(drain(&mut server), [expected]);
            manual.complete(completion).unwrap();
            server.poll_host();
            assert!(drain(&mut server).is_empty());
            assert!(!server.dirty(DRIVE));
            assert_eq!(server.drive_ops(DRIVE), 0);
            assert!(server.host_is_idle());
        }
    }
}

#[test]
fn replacing_media_cancels_a_write_before_it_touches_the_old_file() {
    let (scratch, file) = ScratchImage::new();
    let (executor, manual) = HostExecutor::manual(QUEUE_CAPACITY);
    let mut server = DWServer::with_host_executor(executor);
    server.mount(DRIVE, DWImage::File(file));
    feed(&mut server, &request(true));
    let request = manual.take_request().unwrap();
    server.reattach(DRIVE, DWImage::Memory(vec![NEW_BYTE; SECTOR_SIZE]));
    manual.complete(request.run()).unwrap();
    server.poll_host();
    assert_eq!(std::fs::read(&scratch.0).unwrap(), [OLD_BYTE; SECTOR_SIZE]);
    assert_eq!(drain(&mut server), [error::WRITE]);
    assert_eq!(server.sectors_written(), 0);
}

#[test]
fn asynchronous_file_failures_keep_disk_error_framing() {
    let (scratch, file) = ScratchImage::new();
    drop(file);
    let readonly = File::open(&scratch.0).unwrap();
    let (executor, manual) = HostExecutor::manual(QUEUE_CAPACITY);
    let mut server = DWServer::with_host_executor(executor);
    server.mount(DRIVE, DWImage::File(readonly));

    feed(&mut server, &request(true));
    manual
        .complete(manual.take_request().unwrap().run())
        .unwrap();
    server.poll_host();
    assert_eq!(drain(&mut server), [error::WRITE]);
    assert_eq!(std::fs::read(&scratch.0).unwrap(), [OLD_BYTE; SECTOR_SIZE]);

    feed(&mut server, &[opcode::READEX, DRIVE as u8, 0, 0, 1]);
    manual
        .complete(manual.take_request().unwrap().run())
        .unwrap();
    server.poll_host();
    assert_eq!(drain(&mut server), [0; SECTOR_SIZE]);
    feed(&mut server, &[0, 0]);
    assert_eq!(drain(&mut server), [error::READ]);
    assert_eq!(server.host_diagnostics().errors, 2);
}
