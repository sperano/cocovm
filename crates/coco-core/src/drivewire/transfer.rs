//! Sector I/O: header decoding and the READ/WRITE family's actual image
//! access, invoked once a full READ/READEX header or WRITE body has arrived
//! at [`DWServer::execute_read`]/[`DWServer::execute_write`] from
//! `drivewire::protocol`'s [`State`](super::protocol::State) machine.

use super::protocol::State;
use super::{DWServer, HEADER_LEN, SECTOR_SIZE, checksum_of, error};

impl DWServer {
    /// Decode a READ/READEX/WRITE header: byte 0 is the drive, and bytes 1..4
    /// contain the 24-bit big-endian LSN. In HDB-DOS mode both are derived
    /// from the LSN alone.
    fn decode_header(&self, header: &[u8]) -> (usize, u64) {
        let wire_drive = header[0] as usize;
        let lsn = (u64::from(header[1]) << 16) | (u64::from(header[2]) << 8) | u64::from(header[3]);
        if self.hdbdos {
            let drive = (lsn / super::HDBDOS_SECTORS_PER_DISK) as usize;
            let local_lsn = lsn % super::HDBDOS_SECTORS_PER_DISK;
            (drive, local_lsn)
        } else {
            (wire_drive, lsn)
        }
    }

    /// Read the 256-byte sector at `lsn` from `drive`'s mounted image.
    /// Returns [`error::NOT_READY`] if unmounted or out of range, and
    /// [`error::READ`] past the image's end or on I/O failure.
    fn read_sector(&mut self, drive: usize, lsn: u64) -> Result<[u8; SECTOR_SIZE], u8> {
        let image = self
            .drives
            .get_mut(drive)
            .and_then(|d| d.as_mut())
            .ok_or(error::NOT_READY)?;
        let mut sector = [0u8; SECTOR_SIZE];
        image
            .read_at(lsn * SECTOR_SIZE as u64, &mut sector)
            .map_err(|_| error::READ)?;
        Ok(sector)
    }

    /// Write `sector` to `drive` at `lsn`. Returns [`error::NOT_READY`] if
    /// unmounted or out of range, [`error::WRITE`] on I/O failure, or
    /// [`error::OK`] after marking the image dirty.
    fn write_sector(&mut self, drive: usize, lsn: u64, sector: &[u8]) -> u8 {
        let Some(image) = self.drives.get_mut(drive).and_then(|d| d.as_mut()) else {
            return error::NOT_READY;
        };
        match image.write_at(lsn * SECTOR_SIZE as u64, sector) {
            Ok(()) => error::OK,
            Err(_) => error::WRITE,
        }
    }

    /// Execute a completed READ/REREAD/READEX/REREADEX header: `ex` selects
    /// the READEX-family wire behaviour (see [`State`](super::protocol::State)).
    pub(super) fn execute_read(&mut self, ex: bool, header: &[u8]) {
        let (drive, lsn) = self.decode_header(header);
        let job = self
            .drives
            .get(drive)
            .and_then(Option::as_ref)
            .and_then(|image| image.read_job(lsn * SECTOR_SIZE as u64));
        if let Some(job) = job {
            self.state = State::AwaitHostRead { ex };
            self.begin_host_request(drive, job);
            return;
        }
        let result = self.read_sector(drive, lsn);
        self.finish_read(ex, drive, result);
    }

    pub(super) fn finish_read(
        &mut self,
        ex: bool,
        drive: usize,
        result: Result<[u8; SECTOR_SIZE], u8>,
    ) {
        if result.is_ok() {
            self.sectors_read += 1;
            self.drive_ops[drive] += 1;
        }
        if ex {
            let (data, pending_error) = match result {
                Ok(sector) => (sector, error::OK),
                Err(status) => ([0u8; SECTOR_SIZE], status),
            };
            let expected = checksum_of(&data);
            self.reply.extend(data);
            self.state = State::AwaitReadExChecksum {
                expected,
                pending_error,
                buf: Vec::with_capacity(2),
            };
        } else {
            match result {
                Ok(sector) => {
                    self.reply.push_back(error::OK);
                    let checksum = checksum_of(&sector);
                    self.reply.extend(sector);
                    self.reply.push_back((checksum >> 8) as u8);
                    self.reply.push_back((checksum & 0xFF) as u8);
                }
                Err(status) => self.reply.push_back(status),
            }
        }
    }

    /// Execute a completed WRITE/REWRITE body (header + 256 data bytes +
    /// 2-byte checksum, [`super::WRITE_BODY_LEN`] bytes total).
    pub(super) fn execute_write(&mut self, body: &[u8]) {
        let header = &body[0..HEADER_LEN];
        let sector = &body[HEADER_LEN..HEADER_LEN + SECTOR_SIZE];
        let received = (u16::from(body[HEADER_LEN + SECTOR_SIZE]) << 8)
            | u16::from(body[HEADER_LEN + SECTOR_SIZE + 1]);
        if received != checksum_of(sector) {
            self.reply.push_back(error::CRC);
            return;
        }
        let (drive, lsn) = self.decode_header(header);
        let job = self
            .drives
            .get(drive)
            .and_then(Option::as_ref)
            .and_then(|image| image.write_job(lsn * SECTOR_SIZE as u64, sector));
        if let Some(job) = job {
            // A cancelled in-flight write can still reach the host. Conservatively
            // mark it dirty before handing ownership to the worker.
            self.dirty[drive] = true;
            self.state = State::AwaitHostWrite;
            self.begin_host_request(drive, job);
            return;
        }
        let status = self.write_sector(drive, lsn, sector);
        self.finish_write(drive, status);
    }

    pub(super) fn finish_write(&mut self, drive: usize, status: u8) {
        if status == error::OK {
            self.sectors_written += 1;
            self.dirty[drive] = true;
            self.drive_ops[drive] += 1;
        }
        self.reply.push_back(status);
    }
}
