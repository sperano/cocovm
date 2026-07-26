//! The byte-level protocol state machine: [`DwServer::data_write`] feeds one
//! host byte at a time into [`DwServer::feed`], which either dispatches a
//! fresh opcode ([`DwServer::handle_opcode`]) or advances whatever
//! multi-byte transaction [`State`] says is in progress. Completed
//! READ/READEX headers and WRITE bodies hand off to
//! `drivewire::transfer`'s [`DwServer::execute_read`]/
//! [`DwServer::execute_write`].

use serde::{Deserialize, Serialize};

use super::{
    COMST_PAYLOAD_LEN, DW_PROTOCOL_VERSION, DwServer, HEADER_LEN, SS_COMST, STAT_PAYLOAD_LEN,
    TIME_REPLY_YEAR_BASE, TRANSACTION_TIMEOUT_CYCLES, WRITE_BODY_LEN, error, opcode,
};

/// One in-progress DriveWire transaction. An opcode byte is only ever
/// parsed from [`State::Idle`] — a byte arriving mid-transaction is always
/// consumed as more of that transaction's payload, never reinterpreted as a
/// fresh opcode (that's what [`DwServer::data_write`]'s timeout check is
/// for). This is also why [`opcode::RESET1`](opcode)/`RESET2`/`RESET3` need
/// no special handling beyond being normal opcodes: by the time an opcode
/// byte is parsed, whatever transaction there was has already ended
/// (successfully, on error, or via timeout).
#[derive(Serialize, Deserialize)]
pub(super) enum State {
    Idle,
    /// [`opcode::DWINIT`] sent; awaiting the client's 1-byte driver version
    /// (ignored) before replying with [`DW_PROTOCOL_VERSION`].
    AwaitDwInitVersion,
    /// A "consume `remaining` more bytes, then send no reply" transaction.
    /// This wire shape is shared by several unrelated opcodes:
    /// [`opcode::GETSTAT`]/[`opcode::SETSTAT`] (drive, statcode),
    /// [`opcode::SERGETSTAT`] (channel, statcode),
    /// [`opcode::SERWRITE`] (channel, data byte),
    /// [`opcode::SERINIT`]/[`opcode::SERTERM`] (channel), the
    /// [`opcode::FASTWRITE_BASE`]..=[`opcode::FASTWRITE_LAST`] family
    /// (data byte), and the trailing [`COMST_PAYLOAD_LEN`]-byte option
    /// table after an [`SS_COMST`] [`opcode::SERSETSTAT`] (see
    /// [`State::AwaitSerSetStat`]) — hence one shared variant rather than
    /// a sibling per opcode.
    AwaitDiscard {
        remaining: u8,
    },
    /// [`opcode::SERREADM`] sent; awaiting its 2-byte payload (channel,
    /// count). Once both bytes arrive, replies with `count` zero bytes
    /// (this server never has real virtual-serial data pending, and
    /// [`opcode::SERREAD`] always reports "idle" first, so a real client
    /// never actually reaches this opcode — but the wire shape is still
    /// honored for one that does).
    AwaitSerReadM {
        buf: Vec<u8>,
    },
    /// [`opcode::SERSETSTAT`] sent; awaiting its 2-byte payload (channel,
    /// statcode). Once both bytes arrive, branches on the statcode: if it
    /// is [`SS_COMST`], transitions to [`State::AwaitDiscard`] with
    /// [`COMST_PAYLOAD_LEN`] more bytes to consume (the SCF
    /// device-descriptor option table that always accompanies `SS.ComSt`
    /// on the wire); otherwise the transaction is already complete (no
    /// reply either way).
    AwaitSerSetStat {
        buf: Vec<u8>,
    },
    /// A READ-family opcode sent; awaiting the rest of the 4-byte header.
    /// `ex` distinguishes the READEX/REREADEX group (always sends 256 bytes
    /// and awaits a client checksum) from READ/REREAD (single status byte,
    /// or status + data + checksum on success).
    AwaitReadHeader {
        ex: bool,
        buf: Vec<u8>,
    },
    /// A READEX-family read's 256 data bytes have been sent; awaiting the
    /// client's 2-byte checksum. `expected` is this server's own checksum
    /// of the bytes it sent; `pending_error` is the read's outcome
    /// ([`error::OK`] or the error code the plain READ path would have sent
    /// as its status byte) to reply with if the client's checksum matches —
    /// a checksum mismatch overrides it with [`error::CRC`] instead.
    AwaitReadExChecksum {
        expected: u16,
        pending_error: u8,
        buf: Vec<u8>,
    },
    /// A WRITE-family opcode sent; awaiting the rest of the 262-byte body
    /// (header + 256 data bytes + 2-byte checksum).
    AwaitWriteBody {
        buf: Vec<u8>,
    },
}

impl DwServer {
    /// Becker-port data register write: feed one byte from the client into
    /// the protocol state machine. `cycle` is a monotonically increasing
    /// CPU cycle count, used only to detect a stalled transaction (see
    /// [`TRANSACTION_TIMEOUT_CYCLES`]); compared against the previous
    /// byte's cycle with `wrapping_sub` so a 64-bit wraparound (unreachable
    /// in practice, but cheap to get right) can't misfire.
    pub fn data_write(&mut self, byte: u8, cycle: u64) {
        if let Some(prev) = self.last_byte_cycle {
            let idle = matches!(self.state, State::Idle);
            if !idle && cycle.wrapping_sub(prev) > TRANSACTION_TIMEOUT_CYCLES {
                self.state = State::Idle;
            }
        }
        self.last_byte_cycle = Some(cycle);
        self.feed(byte);
    }

    /// Route one byte through the state machine.
    fn feed(&mut self, byte: u8) {
        let state = std::mem::replace(&mut self.state, State::Idle);
        match state {
            State::Idle => self.handle_opcode(byte),
            State::AwaitDwInitVersion => {
                // The client driver version byte itself is ignored.
                self.reply.push_back(DW_PROTOCOL_VERSION);
            }
            State::AwaitDiscard { remaining } => self.feed_discard(remaining),
            State::AwaitSerReadM { buf } => self.feed_ser_readm(buf, byte),
            State::AwaitSerSetStat { buf } => self.feed_ser_setstat(buf, byte),
            State::AwaitReadHeader { ex, buf } => self.feed_read_header(ex, buf, byte),
            State::AwaitReadExChecksum { expected, pending_error, buf } => {
                self.feed_read_ex_checksum(expected, pending_error, buf, byte)
            }
            State::AwaitWriteBody { buf } => self.feed_write_body(buf, byte),
        }
    }

    /// [`State::AwaitDiscard`]: consume one of `remaining` filler bytes.
    fn feed_discard(&mut self, remaining: u8) {
        if remaining > 1 {
            self.state = State::AwaitDiscard { remaining: remaining - 1 };
        }
    }

    /// [`State::AwaitSerReadM`]: accumulate the 2-byte (channel, count)
    /// payload, then reply with `count` zero bytes.
    fn feed_ser_readm(&mut self, mut buf: Vec<u8>, byte: u8) {
        buf.push(byte);
        if buf.len() == 2 {
            let count = buf[1];
            self.reply.extend(std::iter::repeat_n(0u8, count as usize));
        } else {
            self.state = State::AwaitSerReadM { buf };
        }
    }

    /// [`State::AwaitSerSetStat`]: accumulate the 2-byte (channel, statcode)
    /// payload, then branch on the statcode (see [`State::AwaitSerSetStat`]'s
    /// doc comment).
    fn feed_ser_setstat(&mut self, mut buf: Vec<u8>, byte: u8) {
        buf.push(byte);
        if buf.len() == 2 {
            let statcode = buf[1];
            if statcode == SS_COMST {
                self.state = State::AwaitDiscard { remaining: COMST_PAYLOAD_LEN as u8 };
            }
            // Otherwise the transaction is complete: no reply, state stays
            // Idle (already set at the top of `feed`).
        } else {
            self.state = State::AwaitSerSetStat { buf };
        }
    }

    /// [`State::AwaitReadHeader`]: accumulate the 4-byte header, then
    /// execute the read.
    fn feed_read_header(&mut self, ex: bool, mut buf: Vec<u8>, byte: u8) {
        buf.push(byte);
        if buf.len() == HEADER_LEN {
            self.execute_read(ex, &buf);
        } else {
            self.state = State::AwaitReadHeader { ex, buf };
        }
    }

    /// [`State::AwaitReadExChecksum`]: accumulate the client's 2-byte
    /// checksum, then reply with [`error::CRC`] on mismatch or
    /// `pending_error` otherwise.
    fn feed_read_ex_checksum(&mut self, expected: u16, pending_error: u8, mut buf: Vec<u8>, byte: u8) {
        buf.push(byte);
        if buf.len() == 2 {
            let client_sum = (u16::from(buf[0]) << 8) | u16::from(buf[1]);
            let status = if client_sum != expected { error::CRC } else { pending_error };
            self.reply.push_back(status);
        } else {
            self.state = State::AwaitReadExChecksum { expected, pending_error, buf };
        }
    }

    /// [`State::AwaitWriteBody`]: accumulate the 262-byte body, then
    /// execute the write.
    fn feed_write_body(&mut self, mut buf: Vec<u8>, byte: u8) {
        buf.push(byte);
        if buf.len() == WRITE_BODY_LEN {
            self.execute_write(&buf);
        } else {
            self.state = State::AwaitWriteBody { buf };
        }
    }

    /// Dispatch a byte parsed as a fresh opcode (always called with
    /// `self.state == State::Idle`).
    fn handle_opcode(&mut self, op: u8) {
        match op {
            opcode::NOP
            | opcode::INIT
            | opcode::TERM
            | opcode::RESET1
            | opcode::RESET2
            | opcode::RESET3 => {
                // Single byte, no reply, no state change.
            }
            opcode::TIME => self.reply_time(),
            opcode::DWINIT => self.state = State::AwaitDwInitVersion,
            opcode::GETSTAT | opcode::SETSTAT => {
                self.state = State::AwaitDiscard { remaining: STAT_PAYLOAD_LEN };
            }
            opcode::SERREAD => self.handle_serread(),
            opcode::SERREADM => self.handle_serreadm(),
            opcode::SERWRITE | opcode::SERGETSTAT => self.handle_ser_discard(2),
            opcode::SERSETSTAT => self.handle_sersetstat(),
            opcode::SERINIT | opcode::SERTERM => self.handle_ser_discard(1),
            opcode::FASTWRITE_BASE..=opcode::FASTWRITE_LAST => self.handle_ser_discard(1),
            opcode::READ | opcode::REREAD => self.handle_read_opcode(false),
            opcode::READEX | opcode::REREADEX => self.handle_read_opcode(true),
            opcode::WRITE | opcode::REWRITE => {
                self.state = State::AwaitWriteBody { buf: Vec::with_capacity(WRITE_BODY_LEN) };
            }
            _ => {
                self.unknown_opcodes += 1;
            }
        }
    }

    /// [`opcode::TIME`]: reply with the injected clock's 6-byte encoding.
    fn reply_time(&mut self) {
        let t = (self.clock)();
        self.reply.push_back(t.year.wrapping_sub(TIME_REPLY_YEAR_BASE) as u8);
        self.reply.push_back(t.month);
        self.reply.push_back(t.day);
        self.reply.push_back(t.hour);
        self.reply.push_back(t.minute);
        self.reply.push_back(t.second);
    }

    /// [`opcode::SERREAD`]: always "idle, no data" — this server never has
    /// real virtual-serial input pending.
    fn handle_serread(&mut self) {
        self.vserial_ops += 1;
        self.reply.push_back(0x00);
        self.reply.push_back(0x00);
    }

    /// [`opcode::SERREADM`]: await its 2-byte (channel, count) payload.
    fn handle_serreadm(&mut self) {
        self.vserial_ops += 1;
        self.state = State::AwaitSerReadM { buf: Vec::with_capacity(2) };
    }

    /// [`opcode::SERSETSTAT`]: await its 2-byte (channel, statcode) payload.
    fn handle_sersetstat(&mut self) {
        self.vserial_ops += 1;
        self.state = State::AwaitSerSetStat { buf: Vec::with_capacity(2) };
    }

    /// The vserial-family opcodes whose entire remaining payload is `n`
    /// bytes to discard with no reply
    /// ([`opcode::SERWRITE`]/[`opcode::SERGETSTAT`],
    /// [`opcode::SERINIT`]/[`opcode::SERTERM`], and the
    /// [`opcode::FASTWRITE_BASE`]..=[`opcode::FASTWRITE_LAST`] family).
    fn handle_ser_discard(&mut self, n: u8) {
        self.vserial_ops += 1;
        self.state = State::AwaitDiscard { remaining: n };
    }

    /// [`opcode::READ`]/[`opcode::REREAD`]/[`opcode::READEX`]/
    /// [`opcode::REREADEX`]: await the rest of the 4-byte header.
    fn handle_read_opcode(&mut self, ex: bool) {
        self.state = State::AwaitReadHeader { ex, buf: Vec::with_capacity(HEADER_LEN) };
    }
}
