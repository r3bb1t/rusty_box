//! The disk behind the board's virtio-blk device.
//!
//! Provenance (R7): QEMU's raw format over its `file` protocol driver
//! (`block/raw-format.c`, `block/file-posix.c`, `block/io.c`), as a drive
//! given with `-drive file=…,format=raw,if=none` sees an image:
//!
//! - its size is the image's length rounded up to whole 512-byte sectors
//!   (`bdrv_refresh_total_sectors`, the length from `raw_getlength`'s seek to
//!   the end, so a host block device has its own), and what lies past the
//!   end of the image reads as zeros (`handle_aiocb_rw` filling a short read)
//!   until a write extends it;
//! - a flush makes what was written durable (`raw_co_flush_to_disk`, an
//!   `fdatasync`);
//! - a drive given `readonly=on` refuses every write (`bdrv_co_write_req_prepare`
//!   returning `-EPERM`) and has nothing to flush (`bdrv_co_flush`).
//!
//! A disk is held in host memory, or (with `std`) is a host file read and
//! written in place. Either way it is the host's memory or the host's file,
//! hence `alloc`, as for the virtio-gpu's resources.

#[cfg(feature = "std")]
use alloc::vec;
use alloc::{boxed::Box, vec::Vec};

/// QEMU's `BDRV_SECTOR_SIZE`: the unit a disk's size and every request are
/// counted in.
pub const SECTOR_SIZE: u64 = 512;

/// The most zeros written to a file at once.
#[cfg(feature = "std")]
const ZERO_CHUNK: u64 = 1 << 20;

/// A position in a disk, in bytes from its start (R4: not a guest address).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DiskOffset(u64);

impl DiskOffset {
    /// The start of `sector`.
    pub(crate) fn of_sector(sector: u64) -> Self {
        Self(sector * SECTOR_SIZE)
    }

    pub(crate) fn get(self) -> u64 {
        self.0
    }

    /// `bytes` further on.
    pub(crate) fn after(self, bytes: u64) -> Self {
        Self(self.0 + bytes)
    }
}

/// A host read, write or flush failed, or a write met a read-only disk. The
/// guest is told `VIRTIO_BLK_S_IOERR`; with `std` the host finds a host error
/// with [`Disk::take_io_error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IoFailed;

/// Whether the guest may write a disk.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    ReadWrite,
    /// QEMU's `readonly=on`: the device says so (`VIRTIO_BLK_F_RO`) and
    /// refuses writes.
    ReadOnly,
}

/// The bytes a virtio-blk device reads and writes for its guest.
pub struct Disk {
    backing: Backing,
    access: Access,
    /// The most recent host error the guest was told of, not yet taken.
    #[cfg(feature = "std")]
    io_error: Option<std::io::Error>,
}

/// Where a disk's bytes are.
///
/// Closed (R5): every kind of disk a machine can be given.
enum Backing {
    /// Host memory, a whole number of sectors long.
    Memory(Box<[u8]>),
    /// A host file, and its length: QEMU reads a raw image's length when it
    /// opens it, and a write past the end extends it.
    #[cfg(feature = "std")]
    File { file: std::fs::File, len: u64 },
}

impl Disk {
    /// A disk held in host memory, starting as `image`; what the guest
    /// writes stays in it ([`Disk::memory_bytes`]). An image that is not a
    /// whole number of sectors is padded with zeros to the next one, which
    /// is what its last sector reads as on QEMU.
    pub fn memory(image: Vec<u8>) -> Self {
        let mut image = image;
        let len = (image.len() as u64).next_multiple_of(SECTOR_SIZE);
        image.resize(len as usize, 0);
        Self::with(Backing::Memory(image.into_boxed_slice()))
    }

    /// A disk that is a host file or block device, read and written in
    /// place. It must be open for writing unless the disk is made
    /// [`Disk::read_only`].
    ///
    /// # Errors
    ///
    /// The error finding the file's length.
    #[cfg(feature = "std")]
    pub fn file(file: std::fs::File) -> std::io::Result<Self> {
        use std::io::{Seek, SeekFrom};
        let mut file = file;
        let len = file.seek(SeekFrom::End(0))?;
        Ok(Self::with(Backing::File { file, len }))
    }

    fn with(backing: Backing) -> Self {
        Self {
            backing,
            access: Access::ReadWrite,
            #[cfg(feature = "std")]
            io_error: None,
        }
    }

    /// The same disk, which the guest may only read (QEMU `readonly=on`).
    pub fn read_only(self) -> Self {
        Self {
            access: Access::ReadOnly,
            ..self
        }
    }

    /// Whether the guest may write it.
    pub fn access(&self) -> Access {
        self.access
    }

    /// How many sectors the guest sees (QEMU `blk_get_geometry`).
    pub fn sectors(&self) -> u64 {
        match &self.backing {
            Backing::Memory(bytes) => bytes.len() as u64 / SECTOR_SIZE,
            #[cfg(feature = "std")]
            Backing::File { len, .. } => len.div_ceil(SECTOR_SIZE),
        }
    }

    /// A memory disk's bytes, with everything the guest has written; `None`
    /// for a file.
    pub fn memory_bytes(&self) -> Option<&[u8]> {
        match &self.backing {
            Backing::Memory(bytes) => Some(bytes),
            #[cfg(feature = "std")]
            Backing::File { .. } => None,
        }
    }

    /// Make what the guest has written durable, as QEMU does when it closes
    /// a drive (`bdrv_close` flushing it). A memory disk and a read-only one
    /// have nothing to do.
    ///
    /// # Errors
    ///
    /// The host's error syncing the file.
    #[cfg(feature = "std")]
    pub fn flush(&mut self) -> std::io::Result<()> {
        match (&self.backing, self.access) {
            (Backing::File { file, .. }, Access::ReadWrite) => file.sync_data(),
            (Backing::Memory(_), _) | (Backing::File { .. }, Access::ReadOnly) => Ok(()),
        }
    }

    /// The most recent host error a guest request met, which the guest was
    /// told of as an I/O error (QEMU's `BLOCK_IO_ERROR` event), taken so the
    /// next call reports only a newer one.
    #[cfg(feature = "std")]
    pub fn take_io_error(&mut self) -> Option<std::io::Error> {
        self.io_error.take()
    }

    /// Fill `out` from `at`. Past the end of a file reads as zeros.
    ///
    /// The device asks only for whole sectors inside [`Disk::sectors`]; a
    /// range outside a memory disk is refused rather than read.
    pub(crate) fn read_at(&mut self, at: DiskOffset, out: &mut [u8]) -> Result<(), IoFailed> {
        match &mut self.backing {
            Backing::Memory(bytes) => {
                let span = span(at, out.len() as u64, bytes.len()).ok_or(IoFailed)?;
                out.copy_from_slice(&bytes[span]);
                Ok(())
            }
            #[cfg(feature = "std")]
            Backing::File { file, .. } => {
                let result = host::read(file, at, out);
                self.record(result)
            }
        }
    }

    /// Store `data` at `at`. A write past the end of a file extends it.
    pub(crate) fn write_at(&mut self, at: DiskOffset, data: &[u8]) -> Result<(), IoFailed> {
        if self.access == Access::ReadOnly {
            return Err(IoFailed);
        }
        match &mut self.backing {
            Backing::Memory(bytes) => {
                let span = span(at, data.len() as u64, bytes.len()).ok_or(IoFailed)?;
                bytes[span].copy_from_slice(data);
                Ok(())
            }
            #[cfg(feature = "std")]
            Backing::File { file, len } => {
                let result = host::write(file, at, data);
                if result.is_ok() {
                    *len = (*len).max(at.get() + data.len() as u64);
                }
                self.record(result)
            }
        }
    }

    /// Zero `bytes` bytes from `at` (QEMU `blk_aio_pwrite_zeroes`).
    pub(crate) fn write_zeroes(&mut self, at: DiskOffset, bytes: u64) -> Result<(), IoFailed> {
        if self.access == Access::ReadOnly {
            return Err(IoFailed);
        }
        match &mut self.backing {
            Backing::Memory(image) => {
                let span = span(at, bytes, image.len()).ok_or(IoFailed)?;
                image[span].fill(0);
                Ok(())
            }
            #[cfg(feature = "std")]
            Backing::File { .. } => {
                let zeros = vec![0u8; bytes.min(ZERO_CHUNK) as usize];
                let mut done = 0;
                while done < bytes {
                    let n = (bytes - done).min(ZERO_CHUNK) as usize;
                    self.write_at(at.after(done), &zeros[..n])?;
                    done += n as u64;
                }
                Ok(())
            }
        }
    }

    /// Make every completed write durable (QEMU `blk_aio_flush`).
    #[cfg(feature = "std")]
    pub(crate) fn sync(&mut self) -> Result<(), IoFailed> {
        let result = self.flush();
        self.record(result)
    }

    /// Make every completed write durable: without `std` every disk is in
    /// memory, where a write is as durable as it gets.
    #[cfg(not(feature = "std"))]
    pub(crate) fn sync(&mut self) -> Result<(), IoFailed> {
        Ok(())
    }

    /// Keep a failed host operation's error for the host to take.
    #[cfg(feature = "std")]
    fn record(&mut self, result: std::io::Result<()>) -> Result<(), IoFailed> {
        result.map_err(|error| {
            self.io_error = Some(error);
            IoFailed
        })
    }
}

/// The bytes of a memory disk of `total` bytes that `len` bytes from `at`
/// cover, if they all lie in it.
fn span(at: DiskOffset, len: u64, total: usize) -> Option<core::ops::Range<usize>> {
    let start = usize::try_from(at.get()).ok()?;
    let end = start.checked_add(usize::try_from(len).ok()?)?;
    (end <= total).then_some(start..end)
}

/// Positioned reads and writes of a host file: one call each where the host
/// has one (`pread`/`pwrite`, `ReadFile`/`WriteFile` at an offset), a seek
/// and a read or write elsewhere.
#[cfg(feature = "std")]
mod host {
    use super::DiskOffset;
    use std::fs::File;
    use std::io::{ErrorKind, Result};

    /// Fill `out` from `at`, zeros past the end of the file.
    pub(super) fn read(file: &mut File, at: DiskOffset, out: &mut [u8]) -> Result<()> {
        let mut done = 0;
        while done < out.len() {
            match read_some(file, at.get() + done as u64, &mut out[done..]) {
                Ok(0) => {
                    out[done..].fill(0);
                    break;
                }
                Ok(n) => done += n,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Store all of `data` at `at`.
    pub(super) fn write(file: &mut File, at: DiskOffset, data: &[u8]) -> Result<()> {
        let mut done = 0;
        while done < data.len() {
            match write_some(file, at.get() + done as u64, &data[done..]) {
                Ok(0) => return Err(ErrorKind::WriteZero.into()),
                Ok(n) => done += n,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    #[cfg(unix)]
    fn read_some(file: &mut File, at: u64, out: &mut [u8]) -> Result<usize> {
        std::os::unix::fs::FileExt::read_at(file, out, at)
    }

    #[cfg(unix)]
    fn write_some(file: &mut File, at: u64, data: &[u8]) -> Result<usize> {
        std::os::unix::fs::FileExt::write_at(file, data, at)
    }

    #[cfg(windows)]
    fn read_some(file: &mut File, at: u64, out: &mut [u8]) -> Result<usize> {
        std::os::windows::fs::FileExt::seek_read(file, out, at)
    }

    #[cfg(windows)]
    fn write_some(file: &mut File, at: u64, data: &[u8]) -> Result<usize> {
        std::os::windows::fs::FileExt::seek_write(file, data, at)
    }

    #[cfg(not(any(unix, windows)))]
    fn read_some(file: &mut File, at: u64, out: &mut [u8]) -> Result<usize> {
        use std::io::{Read, Seek, SeekFrom};
        file.seek(SeekFrom::Start(at))?;
        file.read(out)
    }

    #[cfg(not(any(unix, windows)))]
    fn write_some(file: &mut File, at: u64, data: &[u8]) -> Result<usize> {
        use std::io::{Seek, SeekFrom, Write};
        file.seek(SeekFrom::Start(at))?;
        file.write(data)
    }
}
