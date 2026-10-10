//! The byte-level protocol state machine: [`DWServer::data_write`] feeds one
//! host byte at a time into [`DWServer::feed`], which either dispatches a
//! fresh opcode ([`DWServer::handle_opcode`]) or advances whatever
//! multi-byte transaction [`State`] says is in progress. Completed
//! READ/READEX headers and WRITE bodies hand off to
//! `drivewire::transfer`'s [`DWServer::execute_read`]/
//! [`DWServer::execute_write`].

use serde::{Deserialize, Serialize};

use super::serial::{SerialOp, SerialRequest};
use super::{
    DW_PROTOCOL_VERSION, DWServer, HEADER_LEN, STAT_PAYLOAD_LEN, TIME_REPLY_YEAR_BASE,
    TRANSACTION_TIMEOUT_CYCLES, WRITE_BODY_LEN, error, opcode,
};

/// One in-progress DriveWire transaction. An opcode byte is only ever
/// parsed from [`State::Idle`] — a byte arriving mid-transaction is always
/// consumed as more of that transaction's payload, never reinterpreted as a
/// fresh opcode (that's what [`DWServer::data_write`]'s timeout check is
/// for). This is also why [`opcode::RESET1`](opcode)/`RESET2`/`RESET3` need
/// no special handling beyond being normal opcodes: by the time an opcode
/// byte is parsed, whatever transaction there was has already ended
/// (successfully, on error, or after a timeout).
#[derive(Serialize, Deserialize)]
pub(super) enum State {
    Idle,
    /// [`opcode::DWINIT`] sent; awaiting the client's 1-byte driver version
    /// (ignored) before replying with [`DW_PROTOCOL_VERSION`].
    AwaitDwInitVersion,
    /// A "consume `remaining` more bytes, then send no reply" transaction:
    /// the (drive, statcode) payload of [`opcode::GETSTAT`]/[`opcode::SETSTAT`],
    /// and the [`COMST_PAYLOAD_LEN`](super::COMST_PAYLOAD_LEN)-byte SCF option
    /// table after an [`SS_COMST`](super::SS_COMST) [`opcode::SERSETSTAT`].
    AwaitDiscard {
        remaining: u8,
    },
    /// A virtual-serial request (`drivewire::serial`).
    Serial(SerialRequest),
    /// An [`opcode::SERREADM`] header from a snapshot saved before virtual
    /// channels existed; continues as a [`State::Serial`] block read.
    AwaitSerReadM {
        buf: Vec<u8>,
    },
    /// An [`opcode::SERSETSTAT`] header from a snapshot saved before virtual
    /// channels existed; continues as a [`State::Serial`] setstat.
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
    /// A complete request waits for host I/O; no payload bytes remain.
    AwaitHostRead {
        ex: bool,
    },
    AwaitHostWrite,
}

impl DWServer {
    /// Write one byte to the Becker-port data register and feed it into the
    /// protocol state machine. `cycle` detects a stalled transaction by
    /// comparing it with the previous byte's cycle using `wrapping_sub`.
    pub fn data_write(&mut self, byte: u8, cycle: u64) {
        self.poll_host();
        if let Some(prev) = self.last_byte_cycle {
            let idle = matches!(self.state, State::Idle);
            if !idle && cycle.wrapping_sub(prev) > TRANSACTION_TIMEOUT_CYCLES {
                self.cancel_host_request();
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
                self.reset_session();
                self.reply.push_back(DW_PROTOCOL_VERSION);
            }
            State::AwaitDiscard { remaining } => self.feed_discard(remaining),
            State::Serial(request) => self.feed_serial(request, byte),
            State::AwaitSerReadM { buf } => self.feed_serial(
                SerialRequest::Header {
                    op: SerialOp::ReadBlock,
                    buf,
                },
                byte,
            ),
            State::AwaitSerSetStat { buf } => self.feed_serial(
                SerialRequest::Header {
                    op: SerialOp::SetStat,
                    buf,
                },
                byte,
            ),
            State::AwaitReadHeader { ex, buf } => self.feed_read_header(ex, buf, byte),
            State::AwaitReadExChecksum {
                expected,
                pending_error,
                buf,
            } => self.feed_read_ex_checksum(expected, pending_error, buf, byte),
            State::AwaitWriteBody { buf } => self.feed_write_body(buf, byte),
            State::AwaitHostRead { .. } | State::AwaitHostWrite => {
                // A disk client must wait for its reply before sending another
                // request. An early opcode abandons the unanswered request.
                self.cancel_host_request();
                self.handle_opcode(byte);
            }
        }
    }

    /// [`State::AwaitDiscard`]: consume one of `remaining` filler bytes.
    fn feed_discard(&mut self, remaining: u8) {
        if remaining > 1 {
            self.state = State::AwaitDiscard {
                remaining: remaining - 1,
            };
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

    /// [`State::AwaitReadExChecksum`]: accumulate the client's 2-byte checksum, then reply
    /// with [`error::CRC`] on mismatch or `pending_error` otherwise.
    fn feed_read_ex_checksum(
        &mut self,
        expected: u16,
        pending_error: u8,
        mut buf: Vec<u8>,
        byte: u8,
    ) {
        buf.push(byte);
        if buf.len() == 2 {
            let client_sum = (u16::from(buf[0]) << 8) | u16::from(buf[1]);
            let status = if client_sum != expected {
                error::CRC
            } else {
                pending_error
            };
            self.reply.push_back(status);
        } else {
            self.state = State::AwaitReadExChecksum {
                expected,
                pending_error,
                buf,
            };
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
            opcode::NOP | opcode::INIT | opcode::TERM => {
                // Single byte, no reply, no state change.
            }
            opcode::RESET1 | opcode::RESET2 | opcode::RESET3 => self.protocol_reset(),
            opcode::TIME => self.reply_time(),
            opcode::DWINIT => self.state = State::AwaitDwInitVersion,
            opcode::GETSTAT | opcode::SETSTAT => {
                self.state = State::AwaitDiscard {
                    remaining: STAT_PAYLOAD_LEN,
                };
            }
            opcode::SERREAD => self.reply_poll(),
            opcode::SERREADM => self.begin_serial(SerialOp::ReadBlock),
            opcode::SERWRITE => self.begin_serial(SerialOp::Write),
            opcode::SERWRITEM => self.begin_serial(SerialOp::WriteBlock),
            opcode::SERGETSTAT => self.begin_serial(SerialOp::GetStat),
            opcode::SERSETSTAT => self.begin_serial(SerialOp::SetStat),
            opcode::SERINIT => self.begin_serial(SerialOp::Init),
            opcode::SERTERM => self.begin_serial(SerialOp::Term),
            opcode::FASTWRITE_BASE..=opcode::FASTWRITE_LAST => {
                self.begin_serial(SerialOp::FastWrite {
                    channel: op - opcode::FASTWRITE_BASE,
                });
            }
            opcode::READ | opcode::REREAD => self.handle_read_opcode(false),
            opcode::READEX | opcode::REREADEX => self.handle_read_opcode(true),
            opcode::WRITE | opcode::REWRITE => {
                self.state = State::AwaitWriteBody {
                    buf: Vec::with_capacity(WRITE_BODY_LEN),
                };
            }
            _ => {
                self.unknown_opcodes += 1;
            }
        }
    }

    /// [`opcode::TIME`]: reply with the injected clock's 6-byte encoding.
    fn reply_time(&mut self) {
        let t = (self.clock)();
        self.reply
            .push_back(t.year.wrapping_sub(TIME_REPLY_YEAR_BASE) as u8);
        self.reply.push_back(t.month);
        self.reply.push_back(t.day);
        self.reply.push_back(t.hour);
        self.reply.push_back(t.minute);
        self.reply.push_back(t.second);
    }

    /// [`opcode::READ`]/[`opcode::REREAD`]/[`opcode::READEX`]/
    /// [`opcode::REREADEX`]: await the rest of the 4-byte header.
    fn handle_read_opcode(&mut self, ex: bool) {
        self.state = State::AwaitReadHeader {
            ex,
            buf: Vec::with_capacity(HEADER_LEN),
        };
    }
}
