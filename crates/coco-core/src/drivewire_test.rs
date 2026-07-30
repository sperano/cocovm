use super::*;

/// A recognizable, non-repeating 256-byte pattern: byte `i` has value
/// `i as u8`.
fn pattern_sector() -> [u8; SECTOR_SIZE] {
    let mut s = [0u8; SECTOR_SIZE];
    for (i, b) in s.iter_mut().enumerate() {
        *b = i as u8;
    }
    s
}

fn lsn_bytes(lsn: u32) -> [u8; 3] {
    [(lsn >> 16) as u8, (lsn >> 8) as u8, lsn as u8]
}

/// Feed every byte of `bytes` into `server`, each at its own
/// (small, strictly increasing) fake cycle count, then drain and return
/// whatever reply bytes are queued afterward.
fn feed_and_drain(server: &mut DwServer, bytes: &[u8]) -> Vec<u8> {
    feed_at(server, bytes, 0);
    drain(server)
}

/// Like [`feed_and_drain`] but the first byte lands at cycle `start`
/// (subsequent bytes increment by 1), without draining afterward.
fn feed_at(server: &mut DwServer, bytes: &[u8], start: u64) {
    for (i, &b) in bytes.iter().enumerate() {
        server.data_write(b, start + i as u64);
    }
}

fn drain(server: &mut DwServer) -> Vec<u8> {
    let mut out = Vec::new();
    while server.status_read() != 0 {
        out.push(server.data_read());
    }
    out
}

#[test]
fn checksum_vectors() {
    assert_eq!(checksum_of(&[0u8; SECTOR_SIZE]), 0);
    assert_eq!(checksum_of(&[0xFFu8; SECTOR_SIZE]), 65_280);
    assert_eq!(checksum_of(&pattern_sector()), 32_640);
}

#[test]
fn dwinit_replies_protocol_version() {
    let mut server = DwServer::new();
    let reply = feed_and_drain(&mut server, &[opcode::DWINIT, 0x03]);
    assert_eq!(reply, vec![0x04]);
}

#[test]
fn time_uses_injected_clock() {
    let mut server = DwServer::new();
    server.set_clock(Box::new(|| DwTime {
        year: 2024,
        month: 6,
        day: 15,
        hour: 12,
        minute: 30,
        second: 45,
    }));
    let reply = feed_and_drain(&mut server, &[opcode::TIME]);
    assert_eq!(reply, vec![124, 6, 15, 12, 30, 45]);
}

#[test]
fn time_default_clock_is_fixed_date() {
    let mut server = DwServer::new();
    let reply = feed_and_drain(&mut server, &[opcode::TIME]);
    assert_eq!(reply, vec![90, 1, 1, 0, 0, 0]);
}

#[test]
fn read_success() {
    let mut server = DwServer::new();
    let mut image = vec![0u8; SECTOR_SIZE * 2];
    image[SECTOR_SIZE..].copy_from_slice(&pattern_sector());
    server.mount(0, DwImage::Memory(image));

    let mut req = vec![opcode::READ, 0];
    req.extend(lsn_bytes(1));
    let reply = feed_and_drain(&mut server, &req);

    assert_eq!(reply[0], error::OK);
    assert_eq!(&reply[1..1 + SECTOR_SIZE], &pattern_sector());
    let checksum = checksum_of(&pattern_sector());
    assert_eq!(reply[1 + SECTOR_SIZE], (checksum >> 8) as u8);
    assert_eq!(reply[2 + SECTOR_SIZE], (checksum & 0xFF) as u8);
    assert_eq!(server.sectors_read(), 1);
}

#[test]
fn read_bumps_drive_ops_for_that_drive_only() {
    let mut server = DwServer::new();
    let mut image = vec![0u8; SECTOR_SIZE * 2];
    image[SECTOR_SIZE..].copy_from_slice(&pattern_sector());
    server.mount(0, DwImage::Memory(image));

    let mut req = vec![opcode::READ, 0];
    req.extend(lsn_bytes(1));
    feed_and_drain(&mut server, &req);

    assert_eq!(server.drive_ops(0), 1);
    assert_eq!(server.drive_ops(1), 0, "an unrelated drive must not be touched");
}

#[test]
fn read_unmounted_drive() {
    let mut server = DwServer::new();
    let mut req = vec![opcode::READ, 0];
    req.extend(lsn_bytes(0));
    let reply = feed_and_drain(&mut server, &req);
    assert_eq!(reply, vec![error::NOT_READY]);
    assert_eq!(server.drive_ops(0), 0, "a NOT_READY attempt must not bump drive_ops");
}

#[test]
fn read_lsn_past_end() {
    let mut server = DwServer::new();
    server.mount(0, DwImage::Memory(vec![0u8; SECTOR_SIZE]));
    let mut req = vec![opcode::READ, 0];
    req.extend(lsn_bytes(1));
    let reply = feed_and_drain(&mut server, &req);
    assert_eq!(reply, vec![error::READ]);
}

#[test]
fn readex_round_trip_and_retry() {
    let mut server = DwServer::new();
    let mut image = vec![0u8; SECTOR_SIZE];
    image.copy_from_slice(&pattern_sector());
    server.mount(0, DwImage::Memory(image));

    let mut req = vec![opcode::READEX, 0];
    req.extend(lsn_bytes(0));
    feed_at(&mut server, &req, 0);
    let data = drain(&mut server);
    assert_eq!(data.len(), SECTOR_SIZE);
    assert_eq!(data, pattern_sector());

    let checksum = checksum_of(&data);
    feed_at(
        &mut server,
        &[(checksum >> 8) as u8, (checksum & 0xFF) as u8],
        100,
    );
    assert_eq!(drain(&mut server), vec![error::OK]);

    // Wrong checksum -> CRC error.
    feed_at(&mut server, &req, 200);
    let _ = drain(&mut server);
    feed_at(&mut server, &[0x00, 0x00], 300);
    assert_eq!(drain(&mut server), vec![error::CRC]);

    // REREADEX retries successfully.
    let mut retry = vec![opcode::REREADEX, 0];
    retry.extend(lsn_bytes(0));
    feed_at(&mut server, &retry, 400);
    let data = drain(&mut server);
    let checksum = checksum_of(&data);
    feed_at(
        &mut server,
        &[(checksum >> 8) as u8, (checksum & 0xFF) as u8],
        500,
    );
    assert_eq!(drain(&mut server), vec![error::OK]);
}

#[test]
fn readex_unmounted_drive_sends_zeros_with_pending_error() {
    let mut server = DwServer::new();
    let mut req = vec![opcode::READEX, 0];
    req.extend(lsn_bytes(0));
    feed_at(&mut server, &req, 0);
    let data = drain(&mut server);
    assert_eq!(data, vec![0u8; SECTOR_SIZE]);

    // Matching (zero-sum) checksum -> pending NOT_READY error preserved.
    feed_at(&mut server, &[0x00, 0x00], 100);
    assert_eq!(drain(&mut server), vec![error::NOT_READY]);

    // Wrong checksum overrides with CRC even though the drive is unmounted.
    feed_at(&mut server, &req, 200);
    let _ = drain(&mut server);
    feed_at(&mut server, &[0xFF, 0xFF], 300);
    assert_eq!(drain(&mut server), vec![error::CRC]);
}

#[test]
fn write_success_sets_dirty_and_persists() {
    let mut server = DwServer::new();
    server.mount(0, DwImage::Memory(vec![0u8; SECTOR_SIZE]));

    let sector = pattern_sector();
    let checksum = checksum_of(&sector);
    let mut req = vec![opcode::WRITE, 0];
    req.extend(lsn_bytes(0));
    req.extend(sector);
    req.push((checksum >> 8) as u8);
    req.push((checksum & 0xFF) as u8);

    let reply = feed_and_drain(&mut server, &req);
    assert_eq!(reply, vec![error::OK]);
    assert!(server.dirty(0));
    assert_eq!(server.image(0).unwrap().as_memory().unwrap(), &sector);
    assert_eq!(server.sectors_written(), 1);
}

#[test]
fn write_bad_checksum_leaves_image_untouched() {
    let mut server = DwServer::new();
    server.mount(0, DwImage::Memory(vec![0u8; SECTOR_SIZE]));

    let sector = pattern_sector();
    let mut req = vec![opcode::WRITE, 0];
    req.extend(lsn_bytes(0));
    req.extend(sector);
    req.push(0x00); // wrong checksum
    req.push(0x00);

    let reply = feed_and_drain(&mut server, &req);
    assert_eq!(reply, vec![error::CRC]);
    assert!(!server.dirty(0));
    assert_eq!(
        server.image(0).unwrap().as_memory().unwrap(),
        &[0u8; SECTOR_SIZE]
    );

    // REWRITE with the correct checksum then succeeds.
    let checksum = checksum_of(&sector);
    let mut retry = vec![opcode::REWRITE, 0];
    retry.extend(lsn_bytes(0));
    retry.extend(sector);
    retry.push((checksum >> 8) as u8);
    retry.push((checksum & 0xFF) as u8);
    assert_eq!(feed_and_drain(&mut server, &retry), vec![error::OK]);
}

#[test]
fn write_unmounted_drive_still_consumes_all_bytes() {
    let mut server = DwServer::new();
    let sector = pattern_sector();
    let checksum = checksum_of(&sector);
    let mut req = vec![opcode::WRITE, 0];
    req.extend(lsn_bytes(0));
    req.extend(sector);
    req.push((checksum >> 8) as u8);
    req.push((checksum & 0xFF) as u8);

    let reply = feed_and_drain(&mut server, &req);
    assert_eq!(reply, vec![error::NOT_READY]);

    // Server is back in sync: next opcode parses cleanly.
    let reply = feed_and_drain(&mut server, &[opcode::DWINIT, 0x00]);
    assert_eq!(reply, vec![0x04]);
}

#[test]
fn write_past_end_extends_image() {
    let mut server = DwServer::new();
    server.mount(0, DwImage::Memory(vec![0u8; SECTOR_SIZE]));

    let sector = pattern_sector();
    let checksum = checksum_of(&sector);
    let mut req = vec![opcode::WRITE, 0];
    req.extend(lsn_bytes(1));
    req.extend(sector);
    req.push((checksum >> 8) as u8);
    req.push((checksum & 0xFF) as u8);

    let reply = feed_and_drain(&mut server, &req);
    assert_eq!(reply, vec![error::OK]);
    let bytes = server.image(0).unwrap().as_memory().unwrap();
    assert_eq!(bytes.len(), SECTOR_SIZE * 2);
    assert_eq!(&bytes[SECTOR_SIZE..], &sector);
}

#[test]
fn getstat_setstat_consume_exactly_two_bytes() {
    let mut server = DwServer::new();
    // GETSTAT + drive + statcode, then a fresh DWINIT — no leftover
    // reply from GETSTAT, and DWINIT parses cleanly right after.
    let reply = feed_and_drain(
        &mut server,
        &[opcode::GETSTAT, 0x00, 0x01, opcode::DWINIT, 0x00],
    );
    assert_eq!(reply, vec![0x04]);

    let reply = feed_and_drain(
        &mut server,
        &[opcode::SETSTAT, 0x00, 0x01, opcode::DWINIT, 0x00],
    );
    assert_eq!(reply, vec![0x04]);
}

#[test]
fn serread_always_reports_idle() {
    let mut server = DwServer::new();
    for i in 0..3u64 {
        let reply = feed_and_drain(&mut server, &[opcode::SERREAD]);
        assert_eq!(reply, vec![0x00, 0x00], "iteration {i}");
    }
    assert_eq!(server.vserial_ops(), 3);
    assert_eq!(server.unknown_opcodes(), 0);
}

#[test]
fn serinit_serterm_sergetstat_consume_bytes_with_no_reply() {
    let mut server = DwServer::new();
    let reply = feed_and_drain(
        &mut server,
        &[
            opcode::SERINIT,
            0x00, // channel
            opcode::SERTERM,
            0x00, // channel
            opcode::SERGETSTAT,
            0x00, // channel
            0x01, // statcode
            opcode::DWINIT,
            0x03, // driver version, ignored
        ],
    );
    // Only DWINIT produces a reply: proves the state machine is back
    // in sync, not desynced by any of the preceding SER* opcodes.
    assert_eq!(reply, vec![0x04]);
    assert_eq!(server.vserial_ops(), 3);
    assert_eq!(server.unknown_opcodes(), 0);
}

#[test]
fn sersetstat_non_comst_consumes_two_bytes_only() {
    let mut server = DwServer::new();
    let reply = feed_and_drain(
        &mut server,
        &[
            opcode::SERSETSTAT,
            0x00, // channel
            0x00, // statcode, not SS_COMST
            opcode::DWINIT,
            0x00,
        ],
    );
    assert_eq!(reply, vec![0x04]);
    assert_eq!(server.vserial_ops(), 1);
    assert_eq!(server.unknown_opcodes(), 0);
}

#[test]
fn sersetstat_comst_consumes_payload_without_dispatching_it_as_opcodes() {
    let mut server = DwServer::new();
    let mut req = vec![
        opcode::SERSETSTAT,
        0x00,     // channel
        SS_COMST, // statcode
    ];
    // One of the 26 payload bytes deliberately equals a real opcode
    // value (DWINIT) to prove it is consumed as raw payload, not
    // dispatched — if it were misparsed as an opcode, a stray 0x04
    // reply would appear before the real DWINIT below.
    let mut payload = vec![0u8; COMST_PAYLOAD_LEN];
    payload[10] = opcode::DWINIT;
    req.extend(payload);
    feed_at(&mut server, &req, 0);
    assert!(
        drain(&mut server).is_empty(),
        "SERSETSTAT + its ComSt payload must produce no reply"
    );

    let reply = feed_and_drain(&mut server, &[opcode::DWINIT, 0x00]);
    assert_eq!(reply, vec![0x04]);
    assert_eq!(server.vserial_ops(), 1);
    assert_eq!(server.unknown_opcodes(), 0);
}

#[test]
fn fastwrite_and_serwrite_consume_bytes_with_no_reply() {
    let mut server = DwServer::new();
    let reply = feed_and_drain(
        &mut server,
        &[
            opcode::FASTWRITE_BASE,
            0xAB, // data byte, channel 0
            opcode::FASTWRITE_LAST,
            0xCD, // data byte, channel 15
            opcode::SERWRITE,
            0x00, // channel
            0xEF, // data byte
            opcode::DWINIT,
            0x00,
        ],
    );
    assert_eq!(reply, vec![0x04]);
    assert_eq!(server.vserial_ops(), 3);
    assert_eq!(server.unknown_opcodes(), 0);
}

#[test]
fn serreadm_replies_with_count_zero_bytes() {
    let mut server = DwServer::new();
    let reply = feed_and_drain(&mut server, &[opcode::SERREADM, 0x00, 0x05]);
    assert_eq!(reply, vec![0u8; 5]);
    assert_eq!(server.vserial_ops(), 1);
    assert_eq!(server.unknown_opcodes(), 0);
}

#[test]
fn unknown_opcode_is_silently_skipped() {
    let mut server = DwServer::new();
    let reply = feed_and_drain(&mut server, &[0xAB, opcode::DWINIT, 0x00]);
    assert_eq!(reply, vec![0x04]);
    assert_eq!(server.unknown_opcodes(), 1);
}

#[test]
fn stalled_transaction_times_out() {
    let mut server = DwServer::new();
    // Start a READ but only send 2 of its 4 header bytes.
    feed_at(&mut server, &[opcode::READ, 0x00], 0);
    assert!(drain(&mut server).is_empty());

    // Next byte arrives well past the timeout: treated as a fresh
    // opcode (DWINIT) instead of header byte 3.
    let timeout_cycle = 1 + TRANSACTION_TIMEOUT_CYCLES + 1;
    feed_at(&mut server, &[opcode::DWINIT], timeout_cycle);
    feed_at(&mut server, &[0x00], timeout_cycle + 1);
    assert_eq!(drain(&mut server), vec![0x04]);
}

#[test]
fn hdbdos_mode_remaps_drive_and_lsn() {
    let mut server = DwServer::new();
    server.set_hdbdos_mode(true);
    server.mount(0, DwImage::Memory(vec![0xAAu8; SECTOR_SIZE]));
    server.mount(1, DwImage::Memory(pattern_sector().to_vec()));

    // Wire drive byte 0 is ignored; LSN 630 -> drive 1, local LSN 0.
    let mut req = vec![opcode::READ, 0];
    req.extend(lsn_bytes(HDBDOS_SECTORS_PER_DISK as u32));
    let reply = feed_and_drain(&mut server, &req);
    assert_eq!(reply[0], error::OK);
    assert_eq!(&reply[1..1 + SECTOR_SIZE], &pattern_sector());

    // Same remap applies to WRITE.
    let sector = [0x42u8; SECTOR_SIZE];
    let checksum = checksum_of(&sector);
    let mut write_req = vec![opcode::WRITE, 0];
    write_req.extend(lsn_bytes(HDBDOS_SECTORS_PER_DISK as u32));
    write_req.extend(sector);
    write_req.push((checksum >> 8) as u8);
    write_req.push((checksum & 0xFF) as u8);
    let reply = feed_and_drain(&mut server, &write_req);
    assert_eq!(reply, vec![error::OK]);
    assert_eq!(server.image(1).unwrap().as_memory().unwrap(), &sector);
    assert_eq!(
        server.image(0).unwrap().as_memory().unwrap(),
        &[0xAAu8; SECTOR_SIZE]
    );
}
