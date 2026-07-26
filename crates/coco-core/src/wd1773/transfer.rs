//! Byte-paced data transfer handling: the data register ($FF4B) read/write
//! side effects, the DRQ-interval advance loop, and the Write Track (format)
//! MFM stream parser. See [`WD1773::write_data`] and
//! [`WD1773::tick`](super::WD1773::tick), which drives [`advance_transfer`]
//! via [`WD1773::advance_transfer`].

use crate::fdc::JvcDisk;

use super::{
    AWAITING_HOST_CYCLES, CRC_TRAILER_CYCLES, DRQ_INTERVAL_CYCLES, FormatState, Op, Transfer,
    TransferKind, WD1773,
};

/// MFM control-byte constants recognized by the Write Track (format) stream
/// parser ([`feed_write_track_byte`]). Everything else in the stream is
/// literal ID/data payload or gap filler (spec).
mod mfm {
    /// Sync/preamble marker (`A1` on the wire; also presets CRC on real
    /// hardware, irrelevant here). At least one run of these precedes every
    /// address mark.
    pub const SYNC: u8 = 0xF5;
    /// Index-AM preamble (`C2` on the wire). DSKINI never emits an index
    /// address mark; treated as filler.
    pub const INDEX_AM_PREAMBLE: u8 = 0xF6;
    /// "Write CRC": one host byte causes the chip to emit two CRC bytes —
    /// a field terminator from the host's point of view.
    pub const WRITE_CRC: u8 = 0xF7;
    /// ID address mark: the next [`ID_FIELD_LEN`] literal bytes are
    /// track/side/sector/size.
    pub const ID_AM: u8 = 0xFE;
    /// Data address mark (normal data).
    pub const DATA_AM: u8 = 0xFB;
    /// Deleted-data address mark — treated identically to [`DATA_AM`] here.
    pub const DELETED_DATA_AM: u8 = 0xF8;
    /// Bytes in an ID field: track, side, sector, size code.
    pub const ID_FIELD_LEN: usize = 4;
}

impl WD1773 {
    /// Write the data register ($FF4B). Side effect: clears DRQ, and — mid a
    /// Write Sector/Write Track transfer — supplies the next byte, advancing
    /// (and possibly completing) the transfer.
    pub fn write_data(&mut self, val: u8, mut disk: Option<&mut JvcDisk>, side: u8) {
        self.data = val;
        self.drq = false;
        let Op::Transfer(mut t) = std::mem::replace(&mut self.op, Op::Idle) else {
            return;
        };
        if !matches!(t.kind, TransferKind::WriteSector | TransferKind::WriteTrack) {
            self.op = Op::Transfer(t);
            return;
        }
        if t.index < t.total {
            match t.kind {
                TransferKind::WriteSector => {
                    if let Some(d) = disk.as_deref_mut() {
                        d.write_byte(t.offset + t.index, val);
                    }
                }
                TransferKind::WriteTrack if t.format_enabled => {
                    feed_write_track_byte(&mut t, val, disk.as_deref_mut(), side);
                }
                // FM Write Track (format_enabled == false): discard, matching
                // the pre-parser behavior — FM parsing is unimplemented.
                _ => {}
            }
            t.index += 1;
        }
        if t.index >= t.total {
            self.finish_transfer(t, disk, side);
        } else {
            t.remaining = DRQ_INTERVAL_CYCLES;
            self.op = Op::Transfer(t);
        }
    }

    /// A DRQ interval elapsed mid-transfer: deliver the next read byte, or (for
    /// writes) request one and wait for [`WD1773::write_data`].
    pub(super) fn advance_transfer(&mut self, disk: Option<&mut JvcDisk>, side: u8) {
        let Op::Transfer(t) = std::mem::replace(&mut self.op, Op::Idle) else {
            unreachable!("advance_transfer only called from the Op::Transfer arm");
        };
        match t.kind {
            TransferKind::ReadSector | TransferKind::ReadAddress => {
                self.advance_read_transfer(t, disk, side)
            }
            TransferKind::WriteSector | TransferKind::WriteTrack => self.advance_write_transfer(t),
        }
    }

    /// [`advance_transfer`](Self::advance_transfer)'s Read Sector/Read
    /// Address half: deliver the next staged byte via DRQ, or — once `total`
    /// bytes are delivered — finish the transfer after the CRC trailer delay.
    fn advance_read_transfer(&mut self, mut t: Transfer, disk: Option<&mut JvcDisk>, side: u8) {
        if t.index >= t.total {
            // The CRC trailer elapsed after the final data byte; a
            // still-unread final byte is a genuine overrun.
            if self.drq {
                self.status_lost_data = true;
            }
            self.finish_transfer(t, disk, side);
            return;
        }
        // Spec: "if the previous byte was never taken, set LOST
        // DATA but keep going (do not stall)." Exempt only the
        // very first byte of a fresh command — see `first_byte`.
        if self.drq && !t.first_byte {
            self.status_lost_data = true;
        }
        // The WD1773 has no side input: head (side) select is the
        // external DSKREG bit, and the controller reads the data field
        // off whatever side the head sits over *when the field streams*
        // — after the ID-address-mark search ([`FIRST_BYTE_LATENCY_CYCLES`]),
        // not when the command was written. OS-9's RBF driver relies on
        // this: it issues the Read Sector command, *then* flips DSKREG to
        // the next side, before the (halting) DATAREG read. So resolve a
        // Read Sector's bytes from the live side at first delivery, not at
        // dispatch. (Multiple-sector continuations already re-resolve in
        // `finish_transfer`; this covers the first/only sector.)
        if t.first_byte
            && t.kind == TransferKind::ReadSector
            && let Some(d) = disk.as_deref()
            && let Some(offset) = d.sector_offset(self.physical_track, side, self.sector)
        {
            t.offset = offset;
            t.buf = d.read_bytes(offset, t.total).to_vec();
        }
        self.data = t.buf[t.index];
        self.drq = true;
        t.index += 1;
        t.first_byte = false;
        // After the final data byte, INTRQ waits out the CRC trailer
        // (see [`CRC_TRAILER_CYCLES`]) so the host can collect the
        // byte before completion clears halt-enable and fires NMI.
        t.remaining = if t.index >= t.total { CRC_TRAILER_CYCLES } else { DRQ_INTERVAL_CYCLES };
        self.op = Op::Transfer(t);
    }

    /// [`advance_transfer`](Self::advance_transfer)'s Write Sector/Write
    /// Track half: request the next byte and wait — [`WD1773::write_data`]
    /// drives the transfer forward from here, so there's no natural timeout.
    fn advance_write_transfer(&mut self, mut t: Transfer) {
        self.drq = true;
        t.remaining = AWAITING_HOST_CYCLES;
        self.op = Op::Transfer(t);
    }

    /// A sector/ID-field/format run finished. For a multiple-sector Type II
    /// command, roll onto the next sector (RNF once one runs past the end of
    /// the track); otherwise complete with INTRQ.
    fn finish_transfer(&mut self, t: Transfer, disk: Option<&mut JvcDisk>, side: u8) {
        if t.multiple {
            let next_sector = self.sector.wrapping_add(1);
            if let Some(d) = disk
                && let Some(offset) = d.sector_offset(self.physical_track, side, next_sector)
            {
                self.sector = next_sector;
                let total = d.sector_size();
                let buf = match t.kind {
                    TransferKind::ReadSector => d.read_bytes(offset, total).to_vec(),
                    _ => Vec::new(),
                };
                self.op = Op::Transfer(Transfer {
                    kind: t.kind,
                    remaining: DRQ_INTERVAL_CYCLES,
                    index: 0,
                    total,
                    multiple: true,
                    offset,
                    buf,
                    // Not exempted: this continues the same multiple-sector
                    // transfer, so a still-unread last byte of the previous
                    // sector is a genuine overrun (spec's LOST DATA case).
                    first_byte: false,
                    // Only Read/Write Sector ever set `multiple`, so this
                    // continuation never applies to Write Track.
                    format_state: FormatState::Gap,
                    last_id_field: None,
                    format_enabled: true,
                });
                return;
            }
            self.status_record_not_found = true;
        }
        self.busy = false;
        self.intrq = true;
        self.op = Op::Idle;
    }
}

/// Feed one Write Track (format) byte through the mark-triggered parser,
/// advancing `t.format_state`. Recognizes the MFM control bytes in `mod mfm`;
/// everything else is either gap filler (`Gap`/`Sync` states) or literal
/// ID/data payload (`IdField`/`DataField` states — captured verbatim, never
/// interpreted as a mark, per spec). On a completed data field (the `$F7`
/// that ends `DataFieldTerm`), writes the buffered payload into `disk` at
/// `t.last_id_field`'s (track, sector, size_code) and `hw_side` — the
/// hardware side select, not the stream's own (discarded) literal side byte,
/// since the WD1773 never derives side from the ID field on Write Track
/// (spec). Does nothing if `disk` is `None` or no ID field has completed yet.
fn feed_write_track_byte(t: &mut Transfer, val: u8, disk: Option<&mut JvcDisk>, hw_side: u8) {
    let state = std::mem::replace(&mut t.format_state, FormatState::Gap);
    t.format_state = match state {
        FormatState::Gap => step_gap(val),
        FormatState::Sync => step_sync(val, t, disk.as_deref()),
        FormatState::IdField(buf) => step_id_field(buf, val),
        FormatState::IdFieldTerm { track, sector, size_code } => {
            step_id_field_term(track, sector, size_code, val, t)
        }
        FormatState::DataField(buf, target_len) => step_data_field(buf, target_len, val),
        FormatState::DataFieldTerm(buf) => step_data_field_term(buf, val, disk, hw_side, t.last_id_field),
    };
}

/// `Gap` state: waiting for a sync run.
fn step_gap(val: u8) -> FormatState {
    if val == mfm::SYNC { FormatState::Sync } else { FormatState::Gap }
}

/// `Sync` state: at least one sync byte seen; the next non-sync byte is the
/// address mark. `IdField`, `DataField`/`DeletedDataField` are recognized;
/// the index-AM preamble and any other unexpected byte are treated as
/// filler.
fn step_sync(val: u8, t: &Transfer, disk: Option<&JvcDisk>) -> FormatState {
    match val {
        mfm::SYNC => FormatState::Sync,
        mfm::ID_AM => FormatState::IdField(Vec::new()),
        mfm::DATA_AM | mfm::DELETED_DATA_AM => {
            let target_len = t
                .last_id_field
                .map(|(_, _, size_code)| 128usize << size_code)
                .or_else(|| disk.map(JvcDisk::sector_size))
                .unwrap_or(256);
            FormatState::DataField(Vec::new(), target_len)
        }
        // Index-AM preamble: unused by DSKINI, but a real mark — filler.
        mfm::INDEX_AM_PREAMBLE => FormatState::Gap,
        // Any other unexpected byte: also filler.
        _ => FormatState::Gap,
    }
}

/// `IdField` state: gathering the 4 literal ID bytes (track, side, sector,
/// size code). `buf[1]` (side) is captured but deliberately discarded once
/// the field completes — see [`feed_write_track_byte`]'s doc comment.
fn step_id_field(mut buf: Vec<u8>, val: u8) -> FormatState {
    buf.push(val);
    if buf.len() == mfm::ID_FIELD_LEN {
        FormatState::IdFieldTerm { track: buf[0], sector: buf[2], size_code: buf[3] }
    } else {
        FormatState::IdField(buf)
    }
}

/// `IdFieldTerm` state: the 4 ID bytes are gathered; consuming (ignored)
/// bytes until the write-CRC terminator, which latches `t.last_id_field`.
fn step_id_field_term(track: u8, sector: u8, size_code: u8, val: u8, t: &mut Transfer) -> FormatState {
    if val == mfm::WRITE_CRC {
        t.last_id_field = Some((track, sector, size_code));
        FormatState::Gap
    } else {
        FormatState::IdFieldTerm { track, sector, size_code }
    }
}

/// `DataField` state: gathering the sector's data payload.
fn step_data_field(mut buf: Vec<u8>, target_len: usize, val: u8) -> FormatState {
    buf.push(val);
    if buf.len() == target_len { FormatState::DataFieldTerm(buf) } else { FormatState::DataField(buf, target_len) }
}

/// `DataFieldTerm` state: the payload is fully gathered; consuming (ignored)
/// bytes until the write-CRC terminator, which writes the buffered payload
/// into `disk` at the most recently completed ID field's (track, sector,
/// size_code) and `hw_side`.
fn step_data_field_term(
    buf: Vec<u8>,
    val: u8,
    disk: Option<&mut JvcDisk>,
    hw_side: u8,
    last_id_field: Option<(u8, u8, u8)>,
) -> FormatState {
    if val == mfm::WRITE_CRC {
        if let (Some(d), Some((track, sector, size_code))) = (disk, last_id_field) {
            d.format_sector(track, hw_side, sector, size_code, &buf);
        }
        FormatState::Gap
    } else {
        FormatState::DataFieldTerm(buf)
    }
}
