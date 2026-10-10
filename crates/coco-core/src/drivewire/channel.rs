//! Virtual serial channels: per-channel open state, bounded queues in both
//! directions, and fair selection of the next poll reply. The wire encoding
//! of the `OP_SER*` family and the host-side `DWServer::channel_*` API live
//! in `drivewire::serial`.
//!
//! Semantics follow the pinned guest/server pair named by the guest
//! contract: NitrOS-9 `scdwv.asm`/`dwio.asm` (nitros9 `0c9940f`) and the
//! DriveWire 4 Java server's `DWVSerialPorts.java`/`DWVSerialPort.java`
//! (drivewire4 `4e57ffe`), checked against the DriveWire specification's
//! "Virtual Serial Channels" section (DriveWire `a795310`).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Virtual serial channels a poll reply can address. The reply's low nibble
/// carries `channel + 1`, so values 1–15 name channels 0–14 (specification,
/// "The SERREAD / POLL Transaction"; `dwio.asm` `mode00`). NitrOS-9 maps
/// `/TERM` to 0, `/N1`–`/N13` to 1–13, and `/MIDI` to 14.
pub const CHANNEL_COUNT: usize = 15;

/// Bytes each direction of one channel can hold. Host senders see a short
/// write when the guest-bound queue is full; guest writes past a full
/// host-bound queue are dropped and counted, because the wire has no flow
/// control for them.
pub const CHANNEL_BUFFER_BYTES: usize = 4096;

/// Largest count one block poll reply can advertise (an unsigned byte).
const MAX_BLOCK_ADVERTISEMENT: usize = u8::MAX as usize;

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

/// A fresh identity for the channel table's host side. Reset, restore, and
/// stop take a new one so handles from before the event go stale.
fn next_epoch() -> u64 {
    NEXT_EPOCH.fetch_add(1, Ordering::Relaxed)
}

/// Names one guest session on one channel. Operations through a handle fail
/// with [`ChannelError::Stale`] once the guest reopens the channel or the
/// session is reset or restored.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ChannelHandle {
    channel: u8,
    session: u32,
    epoch: u64,
}

impl ChannelHandle {
    pub fn channel(&self) -> u8 {
        self.channel
    }
}

/// Why a host-side channel operation was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelError {
    /// The handle names a session that has been replaced or reset.
    Stale,
    /// The guest has closed the channel.
    NotOpen,
    /// The host already hung up; the channel closes once the guest drains it.
    Closing,
}

/// A snapshot of one channel's state for host services and diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelInfo {
    /// The guest has an open path on the channel.
    pub open: bool,
    /// The host hung up and the close is waiting for queued data to drain.
    pub closing: bool,
    /// Bytes waiting for the guest to read.
    pub to_guest: usize,
    /// Bytes the guest wrote that the host has not received yet.
    pub from_guest: usize,
    /// The latest guest session, while the host may still use it.
    pub handle: Option<ChannelHandle>,
}

/// Bounded counters across all channels, for the status display.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ChannelDiagnostics {
    pub open: usize,
    pub to_guest: usize,
    pub from_guest: usize,
    /// Guest bytes discarded: written to a closed channel or a full queue.
    pub dropped_bytes: u64,
    /// Operations naming a channel outside `0..CHANNEL_COUNT`.
    pub unknown_channel_ops: u64,
    /// Block reads asking for more bytes than were queued.
    pub short_reads: u64,
}

/// The next `OP_SERREAD` reply, before wire encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PollReply {
    Idle,
    /// `count` bytes are queued; the guest fetches them with `OP_SERREADM`.
    Block {
        channel: u8,
        count: u8,
    },
    /// The host hung up and every queued byte has been read.
    Closed {
        channel: u8,
    },
}

#[derive(Default, Serialize, Deserialize)]
struct Channel {
    /// `SS.Open` calls not yet balanced by `SS.Close`. The edition 2 `scdwv`
    /// in the NitrOS-9 3.3.0 image sends one pair per path, and the Java
    /// server counts them the same way (`DWVSerialPort.open`/`close`).
    opens: u8,
    /// Incremented each time the channel goes from closed to open.
    session: u32,
    hangup: bool,
    to_guest: VecDeque<u8>,
    from_guest: VecDeque<u8>,
}

impl Channel {
    fn is_open(&self) -> bool {
        self.opens > 0
    }

    /// Ends the guest session. Unread guest output stays available to the
    /// host until the channel opens again; unread host output is discarded.
    fn close(&mut self) {
        self.opens = 0;
        self.hangup = false;
        self.to_guest.clear();
    }
}

/// Every virtual serial channel of one DriveWire session.
#[derive(Serialize, Deserialize)]
pub(super) struct Channels {
    slots: [Channel; CHANNEL_COUNT],
    /// Where the next poll starts looking, for round-robin fairness.
    poll_cursor: u8,
    #[serde(skip, default = "next_epoch")]
    epoch: u64,
    dropped_bytes: u64,
    unknown_channel_ops: u64,
    short_reads: u64,
}

impl Default for Channels {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| Channel::default()),
            poll_cursor: 0,
            epoch: next_epoch(),
            dropped_bytes: 0,
            unknown_channel_ops: 0,
            short_reads: 0,
        }
    }
}

impl Channels {
    /// The slot for a wire channel number, counting an unknown one.
    fn slot_mut(&mut self, channel: u8) -> Option<&mut Channel> {
        let slot = self.slots.get_mut(usize::from(channel));
        if slot.is_none() {
            self.unknown_channel_ops += 1;
        }
        slot
    }

    /// Validates a channel named by an operation that has no other effect.
    pub(super) fn touch(&mut self, channel: u8) {
        let _ = self.slot_mut(channel);
    }

    /// `SS.Open`: a guest path opened on the channel.
    pub(super) fn guest_open(&mut self, channel: u8) {
        let Some(slot) = self.slot_mut(channel) else {
            return;
        };
        if slot.opens == 0 {
            slot.session = slot.session.wrapping_add(1);
            slot.hangup = false;
            slot.to_guest.clear();
            slot.from_guest.clear();
        }
        slot.opens = slot.opens.saturating_add(1);
    }

    /// `SS.Close`: one guest path closed. A close after the host hangup was
    /// reported finds the channel closed already and has no effect.
    pub(super) fn guest_close(&mut self, channel: u8) {
        let Some(slot) = self.slot_mut(channel) else {
            return;
        };
        match slot.opens {
            0 => {}
            1 => slot.close(),
            _ => slot.opens -= 1,
        }
    }

    /// `OP_SERINIT`/`OP_SERTERM`: `scdwv` sends these when its descriptor is
    /// attached or detached, so no guest path can remain on the channel.
    pub(super) fn guest_detach(&mut self, channel: u8) {
        if let Some(slot) = self.slot_mut(channel) {
            slot.close();
        }
    }

    /// Guest output from `OP_SERWRITE`, `OP_SERWRITEM`, or a fast write.
    pub(super) fn guest_write(&mut self, channel: u8, bytes: &[u8]) {
        let Some(slot) = self.slot_mut(channel) else {
            return;
        };
        let accepted = if slot.is_open() {
            let room = CHANNEL_BUFFER_BYTES - slot.from_guest.len();
            let accepted = bytes.len().min(room);
            slot.from_guest.extend(&bytes[..accepted]);
            accepted
        } else {
            0
        };
        self.dropped_bytes += (bytes.len() - accepted) as u64;
    }

    /// `OP_SERREADM`: exactly `count` bytes, as the wire requires. A request
    /// for more than is queued is padded with zeros and counted.
    pub(super) fn guest_read(&mut self, channel: u8, count: u8) -> Vec<u8> {
        let count = usize::from(count);
        let mut bytes = match self.slot_mut(channel) {
            Some(slot) => {
                let take = count.min(slot.to_guest.len());
                slot.to_guest.drain(..take).collect()
            }
            None => Vec::new(),
        };
        if bytes.len() < count {
            self.short_reads += 1;
            bytes.resize(count, 0);
        }
        bytes
    }

    /// Picks the next channel with data or a pending close, starting after
    /// the channel served last so a busy channel cannot starve the others.
    pub(super) fn poll(&mut self) -> PollReply {
        let start = usize::from(self.poll_cursor);
        for offset in 0..CHANNEL_COUNT {
            let index = (start + offset) % CHANNEL_COUNT;
            if let Some(reply) = self.poll_slot(index) {
                self.poll_cursor = ((index + 1) % CHANNEL_COUNT) as u8;
                return reply;
            }
        }
        PollReply::Idle
    }

    fn poll_slot(&mut self, index: usize) -> Option<PollReply> {
        let slot = &mut self.slots[index];
        let channel = index as u8;
        if !slot.is_open() {
            return None;
        }
        if !slot.to_guest.is_empty() {
            let count = slot.to_guest.len().min(MAX_BLOCK_ADVERTISEMENT) as u8;
            return Some(PollReply::Block { channel, count });
        }
        if slot.hangup {
            slot.close();
            return Some(PollReply::Closed { channel });
        }
        None
    }

    /// Machine reset, protocol reset, `DWINIT`, or stop: the guest driver
    /// starts over, so every channel closes and old handles go stale.
    pub(super) fn reset(&mut self) {
        for slot in &mut self.slots {
            slot.close();
            slot.from_guest.clear();
        }
        self.poll_cursor = 0;
        self.epoch = next_epoch();
    }

    pub(super) fn clear_counters(&mut self) {
        self.dropped_bytes = 0;
        self.unknown_channel_ops = 0;
        self.short_reads = 0;
    }

    /// Snapshot restore: the host services behind open channels belong to
    /// the replaced session. Each open channel hangs up after the guest
    /// reads what the snapshot had queued; unread guest output is dropped.
    pub(super) fn after_restore(&mut self) {
        for slot in &mut self.slots {
            slot.from_guest.clear();
            slot.hangup = slot.is_open();
        }
        self.epoch = next_epoch();
    }

    pub(super) fn info(&self, channel: u8) -> Option<ChannelInfo> {
        let slot = self.slots.get(usize::from(channel))?;
        let handle = (slot.session != 0).then_some(ChannelHandle {
            channel,
            session: slot.session,
            epoch: self.epoch,
        });
        Some(ChannelInfo {
            open: slot.is_open(),
            closing: slot.hangup,
            to_guest: slot.to_guest.len(),
            from_guest: slot.from_guest.len(),
            handle,
        })
    }

    /// The handle's channel, if the handle still names its latest session.
    fn current_mut(&mut self, handle: ChannelHandle) -> Result<&mut Channel, ChannelError> {
        let slot = self
            .slots
            .get_mut(usize::from(handle.channel))
            .ok_or(ChannelError::Stale)?;
        if handle.epoch != self.epoch || handle.session != slot.session {
            return Err(ChannelError::Stale);
        }
        Ok(slot)
    }

    /// Queues host bytes for the guest and returns how many fit.
    pub(super) fn send(
        &mut self,
        handle: ChannelHandle,
        bytes: &[u8],
    ) -> Result<usize, ChannelError> {
        let slot = self.current_mut(handle)?;
        if !slot.is_open() {
            return Err(ChannelError::NotOpen);
        }
        if slot.hangup {
            return Err(ChannelError::Closing);
        }
        let room = CHANNEL_BUFFER_BYTES - slot.to_guest.len();
        let accepted = bytes.len().min(room);
        slot.to_guest.extend(&bytes[..accepted]);
        Ok(accepted)
    }

    /// Takes up to `max` bytes of guest output, including output written
    /// before the guest closed the channel.
    pub(super) fn receive(
        &mut self,
        handle: ChannelHandle,
        max: usize,
    ) -> Result<Vec<u8>, ChannelError> {
        let slot = self.current_mut(handle)?;
        let take = max.min(slot.from_guest.len());
        Ok(slot.from_guest.drain(..take).collect())
    }

    /// Closes the channel from the host side once queued output drains.
    pub(super) fn hangup(&mut self, handle: ChannelHandle) -> Result<(), ChannelError> {
        let slot = self.current_mut(handle)?;
        if !slot.is_open() {
            return Err(ChannelError::NotOpen);
        }
        slot.hangup = true;
        Ok(())
    }

    pub(super) fn diagnostics(&self) -> ChannelDiagnostics {
        let mut diagnostics = ChannelDiagnostics {
            dropped_bytes: self.dropped_bytes,
            unknown_channel_ops: self.unknown_channel_ops,
            short_reads: self.short_reads,
            ..ChannelDiagnostics::default()
        };
        for slot in &self.slots {
            diagnostics.open += usize::from(slot.is_open());
            diagnostics.to_guest += slot.to_guest.len();
            diagnostics.from_guest += slot.from_guest.len();
        }
        diagnostics
    }
}

#[cfg(test)]
#[path = "channel_test.rs"]
mod tests;
