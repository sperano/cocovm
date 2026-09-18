//! DriveWire image backends; host file operations run on the session worker.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

use super::SECTOR_SIZE;
use super::host::{HostError, HostJob};

#[cfg(test)]
#[path = "image_test.rs"]
mod tests;

/// A DriveWire backing image: either an in-memory buffer (small and suitable
/// for tests) or a real file, accessed by
/// seeking rather than loaded whole. Mirrors [`crate::vhd::VHDImage`] with
/// one deliberate difference: a read whose sector lies fully or partly
/// beyond the image's current length is an *error* here (DriveWire has no
/// "sparse image" semantics — a read past the end means the client asked
/// for an LSN the image doesn't have), whereas a write at or beyond the end
/// silently extends the image, so a fresh, empty image file can become a
/// valid disk by formatting it. DECB `FORMAT`/NitrOS-9 `format` write
/// every sector of a new volume in ascending LSN order).
pub enum DWImage {
    Memory(Vec<u8>),
    File(File),
    /// Mounted host handle, accessed only by the host worker.
    AsyncFile(Arc<Mutex<File>>),
}

impl DWImage {
    pub(super) fn into_async(self) -> Self {
        match self {
            Self::File(file) => Self::AsyncFile(Arc::new(Mutex::new(file))),
            image => image,
        }
    }

    pub(super) fn read_job(&self, offset: u64) -> Option<HostJob> {
        let Self::AsyncFile(file) = self else {
            return None;
        };
        let file = Arc::clone(file);
        Some(Box::new(move |cancel| {
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled);
            }
            let mut file = file
                .lock()
                .map_err(|_| HostError::Io(io::ErrorKind::Other))?;
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled);
            }
            let mut bytes = vec![0; SECTOR_SIZE];
            file.seek(SeekFrom::Start(offset))
                .map_err(|e| HostError::Io(e.kind()))?;
            file.read_exact(&mut bytes)
                .map_err(|e| HostError::Io(e.kind()))?;
            Ok(bytes)
        }))
    }

    pub(super) fn write_job(&self, offset: u64, bytes: &[u8]) -> Option<HostJob> {
        let Self::AsyncFile(file) = self else {
            return None;
        };
        let file = Arc::clone(file);
        let bytes = bytes.to_vec();
        Some(Box::new(move |cancel| {
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled);
            }
            let mut file = file
                .lock()
                .map_err(|_| HostError::Io(io::ErrorKind::Other))?;
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled);
            }
            file.seek(SeekFrom::Start(offset))
                .map_err(|e| HostError::Io(e.kind()))?;
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled);
            }
            file.write_all(&bytes)
                .map_err(|e| HostError::Io(e.kind()))?;
            Ok(Vec::new())
        }))
    }

    /// Current length of the backing image in bytes.
    fn len(&self) -> io::Result<u64> {
        match self {
            DWImage::Memory(bytes) => Ok(bytes.len() as u64),
            DWImage::File(file) => Ok(file.metadata()?.len()),
            DWImage::AsyncFile(_) => Err(io::ErrorKind::WouldBlock.into()),
        }
    }

    /// Read exactly `buf.len()` bytes at `offset`; errors (mapped by the
    /// caller to [`error::READ`]) past the image's end or on I/O failure.
    pub(crate) fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let len = self.len()?;
        if offset.saturating_add(buf.len() as u64) > len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "DriveWire read past end of image",
            ));
        }
        match self {
            DWImage::Memory(bytes) => {
                let start = offset as usize;
                buf.copy_from_slice(&bytes[start..start + buf.len()]);
                Ok(())
            }
            DWImage::File(file) => {
                file.seek(SeekFrom::Start(offset))?;
                file.read_exact(buf)
            }
            DWImage::AsyncFile(_) => Err(io::ErrorKind::WouldBlock.into()),
        }
    }

    /// Write `buf` at `offset`, growing the image (zero-filling any gap)
    /// if `offset + buf.len()` exceeds the current length.
    pub(crate) fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        match self {
            DWImage::Memory(bytes) => {
                let end = offset as usize + buf.len();
                if bytes.len() < end {
                    bytes.resize(end, 0);
                }
                bytes[offset as usize..end].copy_from_slice(buf);
                Ok(())
            }
            DWImage::File(file) => {
                file.seek(SeekFrom::Start(offset))?;
                file.write_all(buf)
            }
            DWImage::AsyncFile(_) => Err(io::ErrorKind::WouldBlock.into()),
        }
    }

    /// The image's raw bytes; `Some` only for the in-memory variant, `None`
    /// for a file-backed image.
    pub fn as_memory(&self) -> Option<&[u8]> {
        match self {
            DWImage::Memory(bytes) => Some(bytes),
            DWImage::File(_) | DWImage::AsyncFile(_) => None,
        }
    }
}
