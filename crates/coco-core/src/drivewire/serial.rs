//! Wire handling for the virtual-serial `OP_SER*` family and the host-side
//! channel API. Channel state lives in `drivewire::channel`.
//!
//! Every request is parsed to its full length before it takes effect, so a
//! request cut short by the transaction timeout changes nothing. Until then
//! each byte is payload, even one equal to an opcode value. Request shapes:
//! DriveWire specification, "Virtual Serial Channels"; `scdwv.asm`; Java
//! `DWProtocolHandler.DoOP_SER*`.

use serde::{Deserialize, Serialize};

use super::channel::{ChannelDiagnostics, ChannelError, ChannelHandle, ChannelInfo, PollReply};
use super::protocol::State;
use super::{COMST_PAYLOAD_LEN, DWServer, SS_CLOSE, SS_COMST, SS_OPEN};

/// `OP_SERREAD` reply byte values (specification, "The SERREAD / POLL
/// Transaction"; `dwio.asm` `IRQSvc2`, `mode00`, and `dostat`).
mod poll {
    /// Byte 1: no data or status for any channel.
    pub const IDLE: u8 = 0x00;
    /// Byte 1 flag: byte 2 counts the bytes waiting for channel
    /// `(byte 1 & 0x0F) - 1`, which the guest fetches with `OP_SERREADM`.
    pub const BLOCK: u8 = 0x10;
    /// Byte 1 of a status reply. Byte 2 holds the status code in bits 7–4
    /// and the channel in bits 3–0.
    pub const STATUS: u8 = 0x10;
    /// Bit position of the status code in a status reply's byte 2.
    pub const STATUS_CODE_SHIFT: u8 = 4;
    /// Status code: the channel closed. `dwio` sends `S$HUP` to every
    /// process with a path open on it; it implements no other code.
    pub const CHANNEL_CLOSED: u8 = 0x0;
}

/// Byte length of the request fields that follow most `OP_SER*` opcodes:
/// channel, then a data byte, count, or status code.
const CHANNEL_AND_ARGUMENT_LEN: usize = 2;
/// `OP_SERINIT`/`OP_SERTERM` carry only the channel; a fast write carries
/// only its data byte (the opcode names the channel).
const SINGLE_FIELD_LEN: usize = 1;

/// A virtual-serial request whose fixed fields are still arriving.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(super) enum SerialOp {
    Init,
    Term,
    Write,
    FastWrite { channel: u8 },
    WriteBlock,
    ReadBlock,
    GetStat,
    SetStat,
}

impl SerialOp {
    fn header_len(self) -> usize {
        match self {
            Self::Init | Self::Term | Self::FastWrite { .. } => SINGLE_FIELD_LEN,
            _ => CHANNEL_AND_ARGUMENT_LEN,
        }
    }
}

/// A partially received virtual-serial request.
#[derive(Serialize, Deserialize)]
pub(super) enum SerialRequest {
    Header {
        op: SerialOp,
        buf: Vec<u8>,
    },
    /// `OP_SERWRITEM` data, delivered to the channel only once all `count`
    /// bytes arrive.
    WriteBlock {
        channel: u8,
        count: u8,
        data: Vec<u8>,
    },
}

impl DWServer {
    /// Starts a request of the `OP_SER*` family other than the poll.
    pub(super) fn begin_serial(&mut self, op: SerialOp) {
        self.vserial_ops += 1;
        self.state = State::Serial(SerialRequest::Header {
            op,
            buf: Vec::with_capacity(op.header_len()),
        });
    }

    /// `OP_SERREAD`: reply with the next channel that has data or a close.
    pub(super) fn reply_poll(&mut self) {
        self.vserial_ops += 1;
        let reply = match self.channels.poll() {
            PollReply::Idle => [poll::IDLE, 0],
            PollReply::Block { channel, count } => [poll::BLOCK | (channel + 1), count],
            PollReply::Closed { channel } => [
                poll::STATUS,
                (poll::CHANNEL_CLOSED << poll::STATUS_CODE_SHIFT) | channel,
            ],
        };
        self.reply.extend(reply);
    }

    pub(super) fn feed_serial(&mut self, request: SerialRequest, byte: u8) {
        match request {
            SerialRequest::Header { op, mut buf } => {
                buf.push(byte);
                if buf.len() < op.header_len() {
                    self.state = State::Serial(SerialRequest::Header { op, buf });
                } else {
                    self.finish_serial_header(op, &buf);
                }
            }
            SerialRequest::WriteBlock {
                channel,
                count,
                mut data,
            } => {
                data.push(byte);
                if data.len() < usize::from(count) {
                    self.state = State::Serial(SerialRequest::WriteBlock {
                        channel,
                        count,
                        data,
                    });
                } else {
                    self.channels.guest_write(channel, &data);
                }
            }
        }
    }

    fn finish_serial_header(&mut self, op: SerialOp, header: &[u8]) {
        let first = header[0];
        match op {
            SerialOp::Init | SerialOp::Term => self.channels.guest_detach(first),
            SerialOp::FastWrite { channel } => self.channels.guest_write(channel, &[first]),
            SerialOp::Write => self.channels.guest_write(first, &header[1..]),
            SerialOp::ReadBlock => {
                let bytes = self.channels.guest_read(first, header[1]);
                self.reply.extend(bytes);
            }
            SerialOp::WriteBlock if header[1] == 0 => self.channels.touch(first),
            SerialOp::WriteBlock => {
                let count = header[1];
                self.state = State::Serial(SerialRequest::WriteBlock {
                    channel: first,
                    count,
                    data: Vec::with_capacity(usize::from(count)),
                });
            }
            SerialOp::GetStat => self.channels.touch(first),
            SerialOp::SetStat => self.serial_setstat(first, header[1]),
        }
    }

    /// `OP_SERSETSTAT`: `SS.Open` and `SS.Close` open and close the channel;
    /// `SS.ComSt` carries the path's SCF option table, which no channel
    /// service uses yet. Other codes are notifications.
    fn serial_setstat(&mut self, channel: u8, code: u8) {
        match code {
            SS_OPEN => self.channels.guest_open(channel),
            SS_CLOSE => self.channels.guest_close(channel),
            SS_COMST => {
                self.channels.touch(channel);
                self.state = State::AwaitDiscard {
                    remaining: COMST_PAYLOAD_LEN as u8,
                };
            }
            _ => self.channels.touch(channel),
        }
    }

    /// The state of virtual serial `channel`, or `None` outside
    /// `0..CHANNEL_COUNT`.
    pub fn channel_info(&self, channel: u8) -> Option<ChannelInfo> {
        self.channels.info(channel)
    }

    /// Queues `bytes` for the guest to read and returns how many fit; a short
    /// count is backpressure, so send the rest later.
    pub fn channel_send(
        &mut self,
        handle: ChannelHandle,
        bytes: &[u8],
    ) -> Result<usize, ChannelError> {
        self.channels.send(handle, bytes)
    }

    /// Takes up to `max` bytes that the guest wrote to the channel.
    pub fn channel_receive(
        &mut self,
        handle: ChannelHandle,
        max: usize,
    ) -> Result<Vec<u8>, ChannelError> {
        self.channels.receive(handle, max)
    }

    /// Closes the channel from the host side. The guest reads every queued
    /// byte first, then its poll reports the close and it receives `S$HUP`.
    pub fn channel_hangup(&mut self, handle: ChannelHandle) -> Result<(), ChannelError> {
        self.channels.hangup(handle)
    }

    pub fn channel_diagnostics(&self) -> ChannelDiagnostics {
        self.channels.diagnostics()
    }
}

#[cfg(test)]
#[path = "serial_test.rs"]
mod tests;
