//! Sector I/O: header decoding and the READ/WRITE family's actual image
//! access, invoked once a full READ/READEX header or WRITE body has arrived
//! at [`DwServer::execute_read`]/[`DwServer::execute_write`] from
//! `drivewire::protocol`'s [`State`](super::protocol::State) machine.

use super::protocol::State;
use super::{DwServer, HEADER_LEN, SECTOR_SIZE, checksum_of, error};

impl DwServer {
    /// Decode a READ/READEX/WRITE header: byte 0 is the drive number, bytes
    /// 1..4 are the 24-bit big-endian LSN. In HDB-DOS mode the wire drive
    /// byte is ignored and both drive and local LSN are derived from the
    /// LSN alone (see [`super::HDBDOS_SECTORS_PER_DISK`]).
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
    /// [`error::NOT_READY`] if the drive index is out of range or
    /// unmounted; [`error::READ`] if the LSN is beyond the image's current
    /// length or the host read failed.
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

    /// Write `sector` to `drive` at `lsn`. [`error::NOT_READY`] if the
    /// drive is unmounted/out of range, [`error::WRITE`] on a host I/O
    /// error, else [`error::OK`] (which also marks the drive dirty and
    /// bumps [`DwServer::sectors_written`]).
    fn write_sector(&mut self, drive: usize, lsn: u64, sector: &[u8]) -> u8 {
        let Some(image) = self.drives.get_mut(drive).and_then(|d| d.as_mut()) else {
            return error::NOT_READY;
        };
        match image.write_at(lsn * SECTOR_SIZE as u64, sector) {
            Ok(()) => {
                self.sectors_written += 1;
                if let Some(dirty) = self.dirty.get_mut(drive) {
                    *dirty = true;
                }
                error::OK
            }
            Err(_) => error::WRITE,
        }
    }

    /// Execute a completed READ/REREAD/READEX/REREADEX header: `ex`
    /// selects the READEX-family wire behaviour (see
    /// [`State`](super::protocol::State)).
    pub(super) fn execute_read(&mut self, ex: bool, header: &[u8]) {
        let (drive, lsn) = self.decode_header(header);
        if ex {
            let (data, pending_error) = match self.read_sector(drive, lsn) {
                Ok(sector) => {
                    self.sectors_read += 1;
                    (sector, error::OK)
                }
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
            match self.read_sector(drive, lsn) {
                Ok(sector) => {
                    self.sectors_read += 1;
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
        let status = self.write_sector(drive, lsn, sector);
        self.reply.push_back(status);
    }
}
