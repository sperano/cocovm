use crate::drivewire::{
    CHANNEL_BUFFER_BYTES, COMST_PAYLOAD_LEN, ChannelError, ChannelHandle, DWImage, DWServer,
    SECTOR_SIZE, SS_CLOSE, SS_COMST, SS_OPEN, TRANSACTION_TIMEOUT_CYCLES, error, opcode,
};

/// `/N3`: an ordinary NitrOS-9 numbered channel.
const CH: u8 = 3;
/// Poll reply byte 1 for a block on [`CH`]: the block flag plus `CH + 1`.
const BLOCK_CH: u8 = 0x10 | (CH + 1);
/// Poll reply byte 1 for a status reply.
const STATUS: u8 = 0x10;
const IDLE: [u8; 2] = [0x00, 0x00];

/// Feeds `bytes` one cycle apart from `*cycle`, advancing it, and returns
/// the reply bytes queued afterwards.
fn exchange(server: &mut DWServer, cycle: &mut u64, bytes: &[u8]) -> Vec<u8> {
    for &byte in bytes {
        server.data_write(byte, *cycle);
        *cycle += 1;
    }
    let mut reply = Vec::new();
    while server.status_read() != 0 {
        reply.push(server.data_read());
    }
    reply
}

struct Guest {
    server: DWServer,
    cycle: u64,
}

impl Guest {
    fn new() -> Self {
        Self {
            server: DWServer::new(),
            cycle: 0,
        }
    }

    fn send(&mut self, bytes: &[u8]) -> Vec<u8> {
        exchange(&mut self.server, &mut self.cycle, bytes)
    }

    fn open(&mut self, channel: u8) -> ChannelHandle {
        assert!(
            self.send(&[opcode::SERSETSTAT, channel, SS_OPEN])
                .is_empty()
        );
        self.handle(channel)
    }

    fn handle(&self, channel: u8) -> ChannelHandle {
        self.server.channel_info(channel).unwrap().handle.unwrap()
    }

    fn poll(&mut self) -> Vec<u8> {
        self.send(&[opcode::SERREAD])
    }

    fn read_block(&mut self, channel: u8, count: u8) -> Vec<u8> {
        self.send(&[opcode::SERREADM, channel, count])
    }

    /// Lets the transaction timeout expire before the next byte.
    fn stall(&mut self) {
        self.cycle += TRANSACTION_TIMEOUT_CYCLES + 1;
    }
}

#[test]
fn host_bytes_reach_guest_through_poll_and_block_read() {
    let mut guest = Guest::new();
    assert_eq!(guest.poll(), IDLE);
    let handle = guest.open(CH);
    assert_eq!(guest.server.channel_send(handle, b"hello"), Ok(5));

    assert_eq!(guest.poll(), vec![BLOCK_CH, 5]);
    assert_eq!(guest.read_block(CH, 5), b"hello");
    assert_eq!(guest.poll(), IDLE);
}

#[test]
fn guest_may_read_less_than_advertised() {
    let mut guest = Guest::new();
    let handle = guest.open(CH);
    guest.server.channel_send(handle, b"abcdef").unwrap();

    assert_eq!(guest.poll(), vec![BLOCK_CH, 6]);
    assert_eq!(guest.read_block(CH, 2), b"ab");
    assert_eq!(guest.poll(), vec![BLOCK_CH, 4]);
    assert_eq!(guest.read_block(CH, 4), b"cdef");
}

#[test]
fn single_byte_block_and_fast_writes_reach_host_in_order() {
    let mut guest = Guest::new();
    let handle = guest.open(CH);
    let mut request = vec![opcode::FASTWRITE_BASE + CH, b'a'];
    request.extend([opcode::SERWRITE, CH, b'b']);
    request.extend([opcode::SERWRITEM, CH, 3, b'c', b'd', b'e']);
    request.extend([opcode::SERWRITEM, CH, 0]);
    assert!(guest.send(&request).is_empty());

    assert_eq!(guest.server.channel_receive(handle, 100).unwrap(), b"abcde");
    assert_eq!(guest.server.unknown_opcodes(), 0);
}

#[test]
fn payload_bytes_equal_to_opcodes_stay_payload() {
    let mut guest = Guest::new();
    let handle = guest.open(CH);
    let tricky = [
        opcode::SERREAD,
        opcode::DWINIT,
        opcode::READ,
        opcode::RESET3,
        opcode::SERSETSTAT,
    ];
    let mut request = vec![opcode::SERWRITEM, CH, tricky.len() as u8];
    request.extend(tricky);
    for byte in tricky {
        request.extend([opcode::FASTWRITE_BASE + CH, byte]);
    }
    request.extend([opcode::SERWRITE, CH, opcode::RESET1]);
    assert!(guest.send(&request).is_empty(), "no payload byte may reply");

    let mut expected = tricky.repeat(2);
    expected.push(opcode::RESET1);
    assert_eq!(guest.server.channel_receive(handle, 100).unwrap(), expected);
    // A setstat's channel and statcode are payload too, as is its option table.
    let mut options = vec![opcode::SERSETSTAT, CH, SS_COMST];
    options.extend([opcode::SERREAD; COMST_PAYLOAD_LEN]);
    assert!(guest.send(&options).is_empty());
    assert_eq!(guest.send(&[opcode::DWINIT, 0]), vec![0x04]);
}

#[test]
fn timed_out_block_write_delivers_nothing_and_keeps_framing() {
    let mut guest = Guest::new();
    let handle = guest.open(CH);
    assert!(
        guest
            .send(&[opcode::SERWRITEM, CH, 4, b'a', b'b'])
            .is_empty()
    );
    guest.stall();

    // The stalled request is abandoned whole: this byte is a fresh poll.
    assert_eq!(guest.poll(), IDLE);
    assert!(guest.server.channel_receive(handle, 10).unwrap().is_empty());
    assert!(guest.send(&[opcode::FASTWRITE_BASE + CH, b'z']).is_empty());
    assert_eq!(guest.server.channel_receive(handle, 10).unwrap(), b"z");
}

#[test]
fn timed_out_headers_and_option_tables_recover() {
    let partials: [&[u8]; 5] = [
        &[opcode::SERREADM, CH],
        &[opcode::SERSETSTAT, CH],
        &[opcode::SERSETSTAT, CH, SS_COMST, 1, 2, 3],
        &[opcode::SERINIT],
        &[opcode::FASTWRITE_BASE + CH],
    ];
    for partial in partials {
        let mut guest = Guest::new();
        let handle = guest.open(CH);
        guest.server.channel_send(handle, b"kept").unwrap();
        assert!(guest.send(partial).is_empty());
        guest.stall();

        assert_eq!(guest.poll(), vec![BLOCK_CH, 4], "{partial:?}");
        assert_eq!(guest.read_block(CH, 4), b"kept");
        assert!(guest.server.channel_receive(handle, 10).unwrap().is_empty());
    }
}

#[test]
fn host_queue_applies_backpressure_and_block_advertisement_is_capped() {
    let mut guest = Guest::new();
    let handle = guest.open(CH);
    let data: Vec<u8> = (0..CHANNEL_BUFFER_BYTES + 10).map(|i| i as u8).collect();

    assert_eq!(
        guest.server.channel_send(handle, &data),
        Ok(CHANNEL_BUFFER_BYTES)
    );
    assert_eq!(guest.server.channel_send(handle, &data), Ok(0));
    assert_eq!(guest.poll(), vec![BLOCK_CH, u8::MAX]);
    assert_eq!(guest.read_block(CH, u8::MAX), &data[..usize::from(u8::MAX)]);
    assert_eq!(
        guest
            .server
            .channel_send(handle, &data[CHANNEL_BUFFER_BYTES..]),
        Ok(10)
    );

    let mut received = data[..usize::from(u8::MAX)].to_vec();
    loop {
        match guest.poll()[..] {
            [BLOCK_CH, count] => received.extend(guest.read_block(CH, count)),
            [0, 0] => break,
            ref other => panic!("unexpected poll reply {other:?}"),
        }
    }
    assert_eq!(received, data);
}

#[test]
fn guest_output_past_the_queue_limit_is_dropped_and_counted() {
    let mut guest = Guest::new();
    let handle = guest.open(CH);
    for _ in 0..CHANNEL_BUFFER_BYTES + 3 {
        guest.send(&[opcode::FASTWRITE_BASE + CH, b'x']);
    }
    let diagnostics = guest.server.channel_diagnostics();
    assert_eq!(diagnostics.from_guest, CHANNEL_BUFFER_BYTES);
    assert_eq!(diagnostics.dropped_bytes, 3);
    let received = guest.server.channel_receive(handle, usize::MAX).unwrap();
    assert_eq!(received.len(), CHANNEL_BUFFER_BYTES);
}

#[test]
fn polling_rotates_between_busy_channels() {
    let mut guest = Guest::new();
    let busy = guest.open(1);
    let quiet = guest.open(2);
    guest.server.channel_send(busy, &[b'b'; 1000]).unwrap();
    guest.server.channel_send(quiet, b"q").unwrap();

    // The guest takes nothing from channel 1, yet channel 2 is still served.
    assert_eq!(guest.poll(), vec![0x10 | 2, u8::MAX]);
    assert_eq!(guest.poll(), vec![0x10 | 3, 1]);
    assert_eq!(guest.read_block(2, 1), b"q");
    assert_eq!(guest.poll(), vec![0x10 | 2, u8::MAX]);
    assert_eq!(
        guest.poll(),
        vec![0x10 | 2, u8::MAX],
        "only channel 1 waits"
    );
}

#[test]
fn hangup_closes_only_after_queued_data_drains() {
    let mut guest = Guest::new();
    let handle = guest.open(CH);
    guest.server.channel_send(handle, b"tail").unwrap();
    guest.server.channel_hangup(handle).unwrap();
    assert_eq!(
        guest.server.channel_send(handle, b"more"),
        Err(ChannelError::Closing)
    );
    assert!(guest.server.channel_info(CH).unwrap().closing);

    assert_eq!(guest.poll(), vec![BLOCK_CH, 4]);
    assert_eq!(guest.read_block(CH, 3), b"tai");
    assert_eq!(guest.poll(), vec![BLOCK_CH, 1]);
    assert_eq!(guest.read_block(CH, 1), b"l");
    assert_eq!(
        guest.poll(),
        vec![STATUS, CH],
        "close status for the channel"
    );
    assert_eq!(guest.poll(), IDLE, "the close is reported once");

    let info = guest.server.channel_info(CH).unwrap();
    assert!(!info.open && !info.closing);
    assert_eq!(
        guest.server.channel_send(handle, b"x"),
        Err(ChannelError::NotOpen)
    );
    // The guest's own close after the hangup changes nothing.
    assert!(guest.send(&[opcode::SERSETSTAT, CH, SS_CLOSE]).is_empty());
    assert_eq!(guest.poll(), IDLE);
}

#[test]
fn guest_close_drops_host_data_but_keeps_guest_output() {
    let mut guest = Guest::new();
    let handle = guest.open(CH);
    guest.server.channel_send(handle, b"unread").unwrap();
    guest.send(&[opcode::FASTWRITE_BASE + CH, b'!']);
    guest.send(&[opcode::SERSETSTAT, CH, SS_CLOSE]);

    let info = guest.server.channel_info(CH).unwrap();
    assert!(!info.open);
    assert_eq!(info.to_guest, 0);
    assert_eq!(guest.poll(), IDLE, "no hangup echo for a guest close");
    assert_eq!(guest.server.channel_receive(handle, 10).unwrap(), b"!");
    assert_eq!(
        guest.server.channel_hangup(handle),
        Err(ChannelError::NotOpen)
    );
}

#[test]
fn opens_are_counted_and_descriptor_detach_closes() {
    let mut guest = Guest::new();
    let first = guest.open(CH);
    guest.open(CH);
    assert_eq!(guest.handle(CH), first, "a second path joins the session");
    guest.send(&[opcode::SERSETSTAT, CH, SS_CLOSE]);
    assert!(guest.server.channel_info(CH).unwrap().open);
    guest.send(&[opcode::SERSETSTAT, CH, SS_CLOSE]);
    assert!(!guest.server.channel_info(CH).unwrap().open);

    for detach in [opcode::SERTERM, opcode::SERINIT] {
        guest.open(CH);
        guest.open(CH);
        guest.send(&[detach, CH]);
        assert!(!guest.server.channel_info(CH).unwrap().open, "{detach:#x}");
    }
}

#[test]
fn reopening_a_channel_invalidates_the_old_handle() {
    let mut guest = Guest::new();
    let old = guest.open(CH);
    guest.send(&[opcode::FASTWRITE_BASE + CH, b'o']);
    guest.send(&[opcode::SERSETSTAT, CH, SS_CLOSE]);
    let new = guest.open(CH);

    assert_ne!(old, new);
    assert_eq!(
        guest.server.channel_send(old, b"x"),
        Err(ChannelError::Stale)
    );
    assert_eq!(
        guest.server.channel_receive(old, 1),
        Err(ChannelError::Stale)
    );
    assert!(guest.server.channel_receive(new, 1).unwrap().is_empty());
    assert_eq!(
        guest.server.channel_diagnostics().dropped_bytes,
        1,
        "the unread byte from the old session counts as dropped"
    );
}

#[test]
fn unknown_channels_are_consumed_without_losing_framing() {
    let mut guest = Guest::new();
    let unknown = 15;
    let window_fast_write = opcode::FASTWRITE_BASE + 20;
    let mut request = vec![opcode::SERSETSTAT, unknown, SS_OPEN];
    request.extend([opcode::SERWRITEM, unknown, 2, opcode::DWINIT, 0]);
    request.extend([window_fast_write, opcode::DWINIT]);
    request.extend([opcode::SERINIT, 0x80]);
    assert!(guest.send(&request).is_empty());
    assert_eq!(guest.read_block(unknown, 3), vec![0; 3], "framing is kept");

    let diagnostics = guest.server.channel_diagnostics();
    assert_eq!(diagnostics.unknown_channel_ops, 5);
    assert_eq!(diagnostics.short_reads, 1);
    assert_eq!(guest.server.channel_info(unknown), None);
    assert_eq!(guest.send(&[opcode::DWINIT, 0]), vec![0x04]);
}

#[test]
fn reading_more_than_queued_pads_and_counts() {
    let mut guest = Guest::new();
    let handle = guest.open(CH);
    guest.server.channel_send(handle, b"ab").unwrap();
    assert_eq!(guest.read_block(CH, 4), vec![b'a', b'b', 0, 0]);
    assert_eq!(guest.server.channel_diagnostics().short_reads, 1);
}

#[test]
fn protocol_reset_and_dwinit_close_every_channel() {
    for restart in [&[opcode::RESET1][..], &[opcode::DWINIT, 1][..]] {
        let mut guest = Guest::new();
        let handle = guest.open(CH);
        guest.server.channel_send(handle, b"lost").unwrap();
        guest.send(restart);

        assert!(!guest.server.channel_info(CH).unwrap().open);
        assert_eq!(guest.poll(), IDLE);
        assert_eq!(
            guest.server.channel_send(handle, b"x"),
            Err(ChannelError::Stale)
        );
    }
}

#[test]
fn machine_reset_and_stop_close_every_channel() {
    for stop in [false, true] {
        let mut guest = Guest::new();
        let handle = guest.open(CH);
        if stop {
            guest.server.stop_host();
        } else {
            guest.server.reset_session();
        }
        assert!(!guest.server.channel_info(CH).unwrap().open);
        assert_eq!(
            guest.server.channel_send(handle, b"x"),
            Err(ChannelError::Stale)
        );
    }
}

#[test]
fn restore_delivers_snapshot_data_then_hangs_up() {
    let mut guest = Guest::new();
    let handle = guest.open(CH);
    guest.server.channel_send(handle, b"saved").unwrap();
    guest.send(&[opcode::FASTWRITE_BASE + CH, b'g']);

    let mut cbor = Vec::new();
    ciborium::into_writer(&guest.server, &mut cbor).unwrap();
    let mut restored = Guest {
        server: ciborium::from_reader(cbor.as_slice()).unwrap(),
        cycle: guest.cycle,
    };
    restored.server.after_restore();

    let info = restored.server.channel_info(CH).unwrap();
    assert!(info.open && info.closing);
    assert_eq!(info.from_guest, 0, "guest output belonged to the old host");
    assert_eq!(
        restored.server.channel_send(handle, b"x"),
        Err(ChannelError::Stale)
    );
    assert_eq!(restored.poll(), vec![BLOCK_CH, 5]);
    assert_eq!(restored.read_block(CH, 5), b"saved");
    assert_eq!(restored.poll(), vec![STATUS, CH]);
}

#[test]
fn snapshot_mid_request_resumes_the_request() {
    let mut guest = Guest::new();
    guest.open(CH);
    guest.send(&[opcode::SERWRITEM, CH, 2, b'a']);
    let mut cbor = Vec::new();
    ciborium::into_writer(&guest.server, &mut cbor).unwrap();
    let mut restored: DWServer = ciborium::from_reader(cbor.as_slice()).unwrap();
    restored.after_restore();

    restored.data_write(b'b', guest.cycle);
    let handle = restored.channel_info(CH).unwrap().handle.unwrap();
    assert_eq!(restored.channel_receive(handle, 10).unwrap(), b"ab");
}

#[test]
fn two_servers_keep_channels_isolated() {
    let mut first = Guest::new();
    let mut second = Guest::new();
    let first_handle = first.open(CH);
    let second_handle = second.open(CH);
    first.server.channel_send(first_handle, b"one").unwrap();
    first.send(&[opcode::FASTWRITE_BASE + CH, b'1']);
    second.server.channel_send(second_handle, b"two").unwrap();

    assert_eq!(second.server.channel_diagnostics().from_guest, 0);
    assert_eq!(
        second.server.channel_send(first_handle, b"x"),
        Err(ChannelError::Stale),
        "a handle never addresses another VM's channel"
    );
    second.send(&[opcode::RESET1]);
    assert!(first.server.channel_info(CH).unwrap().open);
    assert_eq!(first.poll(), vec![BLOCK_CH, 3]);
    assert_eq!(first.read_block(CH, 3), b"one");
    assert_eq!(
        first.server.channel_receive(first_handle, 10).unwrap(),
        b"1"
    );
}

#[test]
fn disk_transactions_interleave_with_channel_traffic() {
    let mut guest = Guest::new();
    let sector: Vec<u8> = (0..SECTOR_SIZE).map(|i| i as u8).collect();
    guest.server.mount(0, DWImage::Memory(sector.clone()));
    let handle = guest.open(CH);
    guest.server.channel_send(handle, b"net").unwrap();

    assert_eq!(guest.poll(), vec![BLOCK_CH, 3]);
    let reply = guest.send(&[opcode::READ, 0, 0, 0, 0]);
    assert_eq!(reply[0], error::OK);
    assert_eq!(&reply[1..=SECTOR_SIZE], &sector[..]);
    guest.send(&[opcode::FASTWRITE_BASE + CH, opcode::WRITE]);
    assert_eq!(guest.read_block(CH, 3), b"net");
    assert_eq!(
        guest.server.channel_receive(handle, 10).unwrap(),
        [opcode::WRITE]
    );
    assert_eq!(guest.server.sectors_read(), 1);
}
