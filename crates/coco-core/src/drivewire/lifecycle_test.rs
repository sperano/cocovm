use super::*;
use crate::drivewire::host::{HostExecutor, HostState, ManualHost};
use crate::drivewire::{DWImage, TRANSACTION_TIMEOUT_CYCLES, checksum_of, opcode};

const TEST_DRIVE: usize = 0;
const TEST_BYTE: u8 = 0xA5;
const QUEUE_CAPACITY: usize = 1;
const COCO3_ROM_SIZE: usize = 32 * 1024;

fn manual_server() -> (DWServer, ManualHost) {
    let (host, manual) = HostExecutor::manual(QUEUE_CAPACITY);
    let mut server = DWServer::new();
    server.host = host;
    server.mount(TEST_DRIVE, DWImage::Memory(vec![TEST_BYTE; SECTOR_SIZE]));
    (server, manual)
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

fn begin_read(server: &mut DWServer, ex: bool) {
    server.state = State::AwaitHostRead { ex };
    server.begin_host_request(TEST_DRIVE, Box::new(|_| Ok(vec![TEST_BYTE; SECTOR_SIZE])));
}

fn complete_one(server: &mut DWServer, manual: &ManualHost) {
    manual
        .complete(manual.take_request().unwrap().run())
        .unwrap();
    server.poll_host();
}

#[test]
fn protocol_reset_preserves_media_and_mode_but_clears_statistics() {
    for reset in [opcode::RESET1, opcode::RESET2, opcode::RESET3] {
        let (mut server, _) = manual_server();
        server.set_hdbdos_mode(true);
        feed(&mut server, &[opcode::READ, 0, 0, 0, 0]);
        assert_eq!(server.sectors_read(), 1);
        server.dirty[TEST_DRIVE] = true;
        feed(&mut server, &[reset]);
        assert!(drain(&mut server).is_empty());
        assert!(server.is_mounted(TEST_DRIVE));
        assert!(server.hdbdos_mode());
        assert!(server.dirty(TEST_DRIVE));
        assert_eq!(server.sectors_read(), 0);
        assert_eq!(server.drive_ops(TEST_DRIVE), 0);
    }
}

#[test]
fn init_and_term_do_not_cancel_disk_or_host_state() {
    let (mut server, manual) = manual_server();
    server.host.submit(Box::new(|_| Ok(Vec::new()))).unwrap();
    feed(
        &mut server,
        &[opcode::READ, 0, 0, 0, 0, opcode::INIT, opcode::TERM],
    );
    assert_eq!(server.sectors_read(), 1);
    assert_eq!(server.host_diagnostics().cancelled, 0);
    complete_one(&mut server, &manual);
    assert!(server.take_host_completion().unwrap().result.is_ok());
    assert!(server.host_is_idle());
}

#[test]
fn dwinit_cancels_old_work_without_changing_selected_mode() {
    let (mut server, manual) = manual_server();
    server.set_hdbdos_mode(true);
    server
        .host
        .submit(Box::new(|_| Ok(vec![TEST_BYTE])))
        .unwrap();
    let old_request = manual.take_request().unwrap();
    feed(&mut server, &[opcode::DWINIT, 1]);
    assert_eq!(drain(&mut server), [super::super::DW_PROTOCOL_VERSION]);
    assert!(server.hdbdos_mode());
    assert!(server.is_mounted(TEST_DRIVE));
    assert!(!server.host_is_idle());
    manual.complete(old_request.run()).unwrap();
    server.poll_host();
    assert!(drain(&mut server).is_empty());
    assert!(server.host_is_idle());
}

#[test]
fn delayed_read_does_not_block_another_machine() {
    let (mut server, manual) = manual_server();
    begin_read(&mut server, false);
    assert_eq!(server.status_read(), 0);
    let mut other = DWServer::new();
    feed(&mut other, &[opcode::DWINIT, 1]);
    assert_eq!(drain(&mut other), [super::super::DW_PROTOCOL_VERSION]);
    complete_one(&mut server, &manual);
    let reply = drain(&mut server);
    assert_eq!(reply[0], error::OK);
    assert_eq!(&reply[1..=SECTOR_SIZE], &[TEST_BYTE; SECTOR_SIZE]);
    assert_eq!(server.sectors_read(), 1);
}

#[test]
fn full_host_queue_retains_disk_job_for_retry() {
    let (mut server, manual) = manual_server();
    server.host.submit(Box::new(|_| Ok(Vec::new()))).unwrap();
    begin_read(&mut server, false);
    assert!(server.pending_host.as_ref().unwrap().job.is_some());
    assert_eq!(server.status_read(), 0);
    complete_one(&mut server, &manual);
    assert!(server.take_host_completion().unwrap().result.is_ok());
    server.poll_host();
    complete_one(&mut server, &manual);
    assert_eq!(drain(&mut server).len(), 1 + SECTOR_SIZE + 2);
    assert_eq!(server.sectors_read(), 1);
    assert!(server.host_is_idle());
}

#[test]
fn reset_during_request_rejects_stale_completion() {
    let (mut server, manual) = manual_server();
    begin_read(&mut server, false);
    let old_request = manual.take_request().unwrap();
    server.reset_session();
    manual.complete(old_request.run()).unwrap();
    server.poll_host();
    assert!(drain(&mut server).is_empty());
    assert_eq!(server.sectors_read(), 0);
    begin_read(&mut server, false);
    complete_one(&mut server, &manual);
    assert_eq!(drain(&mut server)[0], error::OK);
}

#[test]
fn host_latency_does_not_expire_extended_read_checksum() {
    let (mut server, manual) = manual_server();
    begin_read(&mut server, true);
    server.last_byte_cycle = Some(0);
    complete_one(&mut server, &manual);
    let sector = drain(&mut server);
    let sum = checksum_of(&sector).to_be_bytes();
    let cycle = TRANSACTION_TIMEOUT_CYCLES * 2;
    server.data_write(sum[0], cycle);
    server.data_write(sum[1], cycle + 1);
    assert_eq!(drain(&mut server), [error::OK]);
}

#[test]
fn suspend_aborts_pending_transfer_and_resume_accepts_work() {
    let (mut server, manual) = manual_server();
    begin_read(&mut server, false);
    server.suspend_host();
    assert_eq!(server.host_diagnostics().state, HostState::Suspended);
    assert_eq!(drain(&mut server), [error::READ]);
    complete_one(&mut server, &manual);
    server.resume_host();
    begin_read(&mut server, false);
    complete_one(&mut server, &manual);
    assert_eq!(drain(&mut server)[0], error::OK);
    server.stop_host();
    assert_eq!(server.host_diagnostics().state, HostState::Stopped);
}

#[test]
fn restore_preserves_completed_disk_reply_but_does_not_replay_host_job() {
    let (mut server, _manual) = manual_server();
    feed(&mut server, &[opcode::READ, 0, 0, 0, 0]);
    let mut restored: DWServer =
        serde_json::from_str(&serde_json::to_string(&server).unwrap()).unwrap();
    restored.after_restore();
    assert_eq!(drain(&mut restored), drain(&mut server));

    begin_read(&mut server, true);
    server.last_byte_cycle = Some(0);
    let mut restored: DWServer =
        serde_json::from_str(&serde_json::to_string(&server).unwrap()).unwrap();
    restored.after_restore();
    assert!(restored.host_is_idle());
    assert_eq!(drain(&mut restored), [0; SECTOR_SIZE]);
    restored.data_write(0, TRANSACTION_TIMEOUT_CYCLES * 2);
    restored.data_write(0, TRANSACTION_TIMEOUT_CYCLES * 2 + 1);
    assert_eq!(drain(&mut restored), [error::READ]);
}

#[test]
fn snapshot_rejects_unfinished_host_side_effects_even_after_reset() {
    let mut machine = crate::Machine::new(
        crate::MachineConfig::default(),
        vec![0; COCO3_ROM_SIZE].into_boxed_slice(),
    );
    machine.bus.enable_drivewire();
    let (host, manual) = HostExecutor::manual(QUEUE_CAPACITY);
    let dw = machine.bus.drivewire.as_mut().unwrap();
    dw.host = host;
    dw.host.submit(Box::new(|_| Ok(Vec::new()))).unwrap();
    let request = manual.take_request().unwrap();
    machine.reset();
    let result = crate::snapshot::save(&machine, &crate::snapshot::MediaRefs::default());
    assert!(matches!(
        result,
        Err(crate::snapshot::SnapshotError::DriveWireBusy)
    ));
    manual.complete(request.run()).unwrap();
    machine.bus.drivewire.as_mut().unwrap().poll_host();
    assert!(crate::snapshot::save(&machine, &crate::snapshot::MediaRefs::default()).is_ok());
}

#[test]
fn service_results_survive_bus_polling_and_apply_backpressure() {
    let (mut server, manual) = manual_server();
    for value in 0..SERVICE_COMPLETION_CAPACITY {
        server
            .submit_host_service(Box::new(move |_| Ok(vec![value as u8])))
            .unwrap();
        complete_one(&mut server, &manual);
    }
    assert!(!server.host_is_idle());
    let last_id = server
        .submit_host_service(Box::new(|_| Ok(vec![TEST_BYTE])))
        .unwrap();
    complete_one(&mut server, &manual);
    assert_eq!(server.host_diagnostics().outstanding, 1);
    for value in 0..SERVICE_COMPLETION_CAPACITY {
        let completion = server.take_host_completion().unwrap();
        assert_eq!(completion.result.unwrap(), [value as u8]);
    }
    server.poll_host();
    let completion = server.take_host_completion().unwrap();
    assert_eq!(completion.id, last_id);
    assert_eq!(completion.result.unwrap(), [TEST_BYTE]);
    assert!(server.host_is_idle());
}

#[test]
fn reset_discards_unconsumed_service_results() {
    let (mut server, manual) = manual_server();
    server
        .submit_host_service(Box::new(|_| Ok(vec![TEST_BYTE])))
        .unwrap();
    complete_one(&mut server, &manual);
    server.reset_session();
    assert!(server.take_host_completion().is_none());
    assert!(server.host_is_idle());
}

#[test]
fn reset_abandons_partial_guest_write_without_touching_media() {
    let (mut server, _) = manual_server();
    feed(&mut server, &[opcode::WRITE, 0, 0, 0, 0, TEST_BYTE]);
    server.reset_session();
    feed(&mut server, &[opcode::DWINIT, 1]);
    assert_eq!(drain(&mut server), [super::super::DW_PROTOCOL_VERSION]);
    assert_eq!(
        server.image(TEST_DRIVE).unwrap().as_memory().unwrap(),
        &[TEST_BYTE; SECTOR_SIZE]
    );
    assert!(!server.dirty(TEST_DRIVE));
    assert_eq!(server.sectors_written(), 0);
}

#[test]
fn replacing_disk_preserves_unrelated_service_jobs_and_results() {
    const CAPACITY: usize = 3;
    let (executor, manual) = HostExecutor::manual(CAPACITY);
    let mut server = DWServer::with_host_executor(executor);
    let first = server
        .submit_host_service(Box::new(|_| Ok(vec![TEST_BYTE])))
        .unwrap();
    complete_one(&mut server, &manual);
    let second = server
        .submit_host_service(Box::new(|_| Ok(vec![TEST_BYTE])))
        .unwrap();
    begin_read(&mut server, false);
    let service = manual.take_request().unwrap();
    let disk = manual.take_request().unwrap();
    server.mount(TEST_DRIVE, DWImage::Memory(vec![TEST_BYTE; SECTOR_SIZE]));
    manual.complete(service.run()).unwrap();
    manual.complete(disk.run()).unwrap();
    server.poll_host();
    server.poll_host();
    for id in [first, second] {
        let completion = server.take_host_completion().unwrap();
        assert_eq!(completion.id, id);
        assert_eq!(completion.result.unwrap(), [TEST_BYTE]);
    }
    assert_eq!(drain(&mut server), [error::READ]);
    assert!(server.host_is_idle());
}
