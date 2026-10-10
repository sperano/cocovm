//! DriveWire host shares across two machines: per-VM sessions over one
//! host folder, cross-VM write coordination, and Becker-port traffic that
//! keeps flowing while a share job waits on the host executor.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coco_core::drivewire::host::{HostCompletion, HostError, HostExecutor, RequestId};
use coco_core::drivewire::share::{
    LeaseOwner, ShareAccess, ShareError, ShareHandle, ShareOp, ShareSpec, ShareTable,
};
use coco_core::drivewire::{DWImage, DWServer, SECTOR_SIZE, error, opcode};
use coco_core::{MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

const ROM_SIZE: usize = 32 * 1024;
const BECKER_STATUS: u16 = 0xFF41;
const BECKER_DATA: u16 = 0xFF42;
const QUEUE_CAPACITY: usize = 2;
const SECTOR_FILL: u8 = 0x5A;
/// Generous bound for a real worker thread to finish one small job.
const WORKER_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(1);

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("cocovm-shares-it-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bus() -> SystemBus {
    let rom = vec![0_u8; ROM_SIZE].into_boxed_slice();
    let mut bus = SystemBus::new(MachineVariant::Coco3, MemorySize::K512, rom);
    bus.gime.write_init0(coco_core::gime::init0::MC2);
    bus
}

fn shares(root: &Path, access: ShareAccess) -> ShareTable {
    ShareTable::new(vec![ShareSpec {
        name: "shared".to_string(),
        root: root.to_path_buf(),
        access,
    }])
    .unwrap()
}

fn machine_with_shares(root: &Path, access: ShareAccess) -> SystemBus {
    let mut bus = bus();
    bus.enable_drivewire();
    dw(&mut bus).set_shares(shares(root, access), LeaseOwner::new());
    bus
}

fn dw(bus: &mut SystemBus) -> &mut DWServer {
    bus.drivewire.as_mut().expect("DriveWire enabled")
}

/// Submits `op` to the machine's real host worker and waits for its reply.
fn run(bus: &mut SystemBus, op: ShareOp) -> Result<Vec<u8>, HostError> {
    let id = dw(bus)
        .submit_share_op(op)
        .expect("executor accepts the job");
    wait_for(dw(bus), id).result
}

fn wait_for(server: &mut DWServer, id: RequestId) -> HostCompletion {
    let deadline = Instant::now() + WORKER_TIMEOUT;
    loop {
        server.poll_host();
        if let Some(completion) = server.take_host_completion() {
            assert_eq!(completion.id, id);
            return completion;
        }
        assert!(Instant::now() < deadline, "host worker did not finish");
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn path_op(op: fn(Vec<u8>) -> ShareOp, path: &str) -> ShareOp {
    op(path.as_bytes().to_vec())
}

fn change_dir(path: Vec<u8>) -> ShareOp {
    ShareOp::ChangeDir { path }
}

fn open_read(path: Vec<u8>) -> ShareOp {
    ShareOp::OpenRead { path }
}

fn create(path: Vec<u8>) -> ShareOp {
    ShareOp::Create { path }
}

fn list(path: &str) -> ShareOp {
    ShareOp::List {
        path: path.as_bytes().to_vec(),
        start: 0,
    }
}

#[test]
fn two_machines_share_a_root_with_independent_sessions() {
    let scratch = Scratch::new("independent");
    fs::create_dir_all(scratch.0.join("left")).unwrap();
    fs::create_dir_all(scratch.0.join("right")).unwrap();
    fs::write(scratch.0.join("left/l.txt"), b"left").unwrap();
    fs::write(scratch.0.join("right/r.txt"), b"right").unwrap();
    let mut vm_a = machine_with_shares(&scratch.0, ShareAccess::ReadOnly);
    let mut vm_b = machine_with_shares(&scratch.0, ShareAccess::ReadOnly);

    run(&mut vm_a, path_op(change_dir, "shared/left")).unwrap();
    run(&mut vm_b, path_op(change_dir, "shared/right")).unwrap();
    assert_eq!(run(&mut vm_a, list("")).unwrap(), b"l.txt\n");
    assert_eq!(run(&mut vm_b, list("")).unwrap(), b"r.txt\n");

    let handle_a = run(&mut vm_a, path_op(open_read, "l.txt")).unwrap();
    let handle_b = run(&mut vm_b, path_op(open_read, "r.txt")).unwrap();
    assert_eq!(handle_a, handle_b, "handle numbers are per machine");
    let read = |handle: &[u8]| ShareOp::Read {
        handle: ShareHandle::from_byte(handle[0]),
        max: SECTOR_SIZE,
    };
    assert_eq!(run(&mut vm_a, read(&handle_a)).unwrap(), b"left");
    assert_eq!(run(&mut vm_b, read(&handle_b)).unwrap(), b"right");

    dw(&mut vm_a).reset_session();
    assert_eq!(dw(&mut vm_a).share_session().status().cwd, "/");
    assert_eq!(dw(&mut vm_b).share_session().status().cwd, "/shared/right");
}

#[test]
fn a_second_machine_cannot_write_a_file_the_first_has_open() {
    let scratch = Scratch::new("conflict");
    fs::write(scratch.0.join("game.dsk"), b"image").unwrap();
    let mut vm_a = machine_with_shares(&scratch.0, ShareAccess::ReadWrite);
    let mut vm_b = machine_with_shares(&scratch.0, ShareAccess::ReadWrite);

    run(&mut vm_a, path_op(open_read, "shared/game.dsk")).unwrap();
    assert_eq!(
        run(&mut vm_b, path_op(create, "shared/game.dsk")),
        Err(HostError::Share(ShareError::Busy))
    );
    assert_eq!(fs::read(scratch.0.join("game.dsk")).unwrap(), b"image");
    assert_eq!(
        dw(&mut vm_b).host_diagnostics().last_error,
        Some(HostError::Share(ShareError::Busy))
    );

    vm_a.disable_drivewire();
    assert!(run(&mut vm_b, path_op(create, "shared/game.dsk")).is_ok());
}

#[test]
fn becker_disk_traffic_continues_while_a_share_job_waits() {
    let scratch = Scratch::new("nonblocking");
    let (host, manual) = HostExecutor::manual(QUEUE_CAPACITY);
    let mut vm = bus();
    vm.drivewire = Some(DWServer::with_host_executor(host));
    let server = dw(&mut vm);
    server.set_shares(shares(&scratch.0, ShareAccess::ReadOnly), LeaseOwner::new());
    server.mount(0, DWImage::Memory(vec![SECTOR_FILL; SECTOR_SIZE]));
    let id = server.submit_share_op(list("shared")).unwrap();

    for byte in [opcode::READ, 0, 0, 0, 0] {
        vm.write(BECKER_DATA, byte);
    }
    let mut reply = Vec::new();
    while vm.read(BECKER_STATUS) != 0 {
        reply.push(vm.read(BECKER_DATA));
    }
    assert_eq!(
        reply[0],
        error::OK,
        "the disk read answers while the share job is queued"
    );
    assert_eq!(&reply[1..=SECTOR_SIZE], &[SECTOR_FILL; SECTOR_SIZE][..]);

    manual
        .complete(manual.take_request().unwrap().run())
        .unwrap();
    let completion = wait_for(dw(&mut vm), id);
    assert_eq!(completion.result.unwrap(), b"");
}
