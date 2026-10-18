//! A hard disk's image file, as Bochs `iodev/hdimage/hdimage.cc` opens one:
//! flat, written in place; or volatile (`volatile_image_t`), the image opened
//! read-only and every sector written kept in a journal file that is removed
//! when the drive closes.

use std::collections::hash_map::RandomState;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;
use std::path::{Path, PathBuf};

/// The bytes of one sector of a hard disk image.
pub(crate) const SECTOR: usize = 512;
const SECTOR_BYTES: u64 = SECTOR as u64;
/// Bochs hdimage.h `VOLATILE_REDOLOG_EXTENSION` (`".XXXXXX"`): a journal is
/// its template, a dot, and six characters `mkstemp` picks.
const JOURNAL_SUFFIX_CHARS: usize = 6;
const JOURNAL_SUFFIX_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
/// The names tried before a journal is given up: `mkstemp`'s own retries.
const JOURNAL_NAME_TRIES: u32 = 100;

/// Where a volatile disk keeps the sectors written to it (Bochs's `journal`
/// option for the `volatile` mode): beside the image, named after it, or
/// named after the template given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Journal {
    BesideImage,
    Named(PathBuf),
}

/// One sector a volatile disk holds over its image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WrittenSector {
    pub(crate) lba: u64,
    pub(crate) bytes: [u8; SECTOR],
}

/// A hard disk's image: written in place, or volatile.
#[derive(Debug)]
pub(crate) enum DiskImage {
    Flat(File),
    Volatile(VolatileImage),
}

impl DiskImage {
    /// The image at `path`, opened for reading and writing.
    pub(crate) fn open_flat(path: &Path) -> io::Result<DiskImage> {
        Ok(DiskImage::Flat(File::options().read(true).write(true).open(path)?))
    }

    /// The image at `path`, opened read-only, its writes kept in a journal
    /// placed as `journal` says (Bochs `volatile_image_t::open`).
    pub(crate) fn open_volatile(path: &Path, journal: &Journal) -> io::Result<DiskImage> {
        Ok(DiskImage::Volatile(VolatileImage::open(path, journal)?))
    }

    /// The image's bytes.
    pub(crate) fn size(&self) -> io::Result<u64> {
        match self {
            DiskImage::Flat(file) => Ok(file.metadata()?.len()),
            DiskImage::Volatile(volatile) => Ok(volatile.size),
        }
    }

    /// The file the image's identity is taken from: the image itself.
    pub(crate) fn image_file(&self) -> &File {
        match self {
            DiskImage::Flat(file) => file,
            DiskImage::Volatile(volatile) => &volatile.base,
        }
    }

    pub(crate) fn read_sector(&mut self, lba: u64, sector: &mut [u8]) -> io::Result<()> {
        match self {
            DiskImage::Flat(file) => read_at(file, sector_offset(lba)?, sector),
            DiskImage::Volatile(volatile) => volatile.read(lba, sector),
        }
    }

    pub(crate) fn write_sector(&mut self, lba: u64, sector: &[u8]) -> io::Result<()> {
        match self {
            DiskImage::Flat(file) => write_at(file, sector_offset(lba)?, sector),
            DiskImage::Volatile(volatile) => volatile.write(lba, sector),
        }
    }

    /// Hands a flat image's writes to the host; a volatile one's journal is
    /// thrown away, so it keeps them where they are.
    pub(crate) fn flush(&mut self) -> io::Result<()> {
        match self {
            DiskImage::Flat(file) => file.flush(),
            DiskImage::Volatile(_) => Ok(()),
        }
    }
}

fn sector_offset(lba: u64) -> io::Result<u64> {
    lba.checked_mul(SECTOR_BYTES)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "sector past the end of the image"))
}

/// `buf.len()` bytes of `file` at `offset`, or an error.
///
/// Positional, so the handle's cursor is never relied on: a `seek` then a
/// `read` is two steps another holder of the handle can split, and a
/// [`VolatileImage`] is shared by `&self` from a snapshot's save.
#[cfg(unix)]
fn read_at(file: &File, offset: u64, buf: &mut [u8]) -> io::Result<()> {
    file.read_exact_at(buf, offset)
}

/// `buf.len()` bytes of `file` at `offset`, or an error. See the Unix twin
/// on why it is positional; `seek_read` moves the cursor as a side effect,
/// and nothing here reads it.
#[cfg(windows)]
fn read_at(file: &File, offset: u64, buf: &mut [u8]) -> io::Result<()> {
    let mut at = offset;
    let mut rest = buf;
    while !rest.is_empty() {
        match file.seek_read(rest, at) {
            Ok(0) => {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "sector past the end of the file"));
            }
            Ok(moved) => {
                let Some(tail) = rest.get_mut(moved..) else {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "read past its buffer"));
                };
                rest = tail;
                at = at
                    .checked_add(u64::try_from(moved).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "offset past the end of the file"))?;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// `buf` written to `file` at `offset`, whole, or an error. Positional for
/// the reason [`read_at`] is.
#[cfg(unix)]
fn write_at(file: &File, offset: u64, buf: &[u8]) -> io::Result<()> {
    file.write_all_at(buf, offset)
}

/// `buf` written to `file` at `offset`, whole, or an error. Positional for
/// the reason [`read_at`] is.
#[cfg(windows)]
fn write_at(file: &File, offset: u64, buf: &[u8]) -> io::Result<()> {
    let mut at = offset;
    let mut rest = buf;
    while !rest.is_empty() {
        match file.seek_write(rest, at) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "the file took no bytes")),
            Ok(moved) => {
                let Some(tail) = rest.get(moved..) else {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "wrote past its buffer"));
                };
                rest = tail;
                at = at
                    .checked_add(u64::try_from(moved).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "offset past the end of the file"))?;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Bochs `volatile_image_t`: the image read-only (`ro_disk`), and a journal
/// (its `redolog`) holding every sector written. A sector is read from the
/// journal when it was written, else from the image. The journal's name is
/// removed as soon as it is made, so nothing of it outlives the drive.
#[derive(Debug)]
pub(crate) struct VolatileImage {
    base: File,
    size: u64,
    journal: File,
    journal_path: PathBuf,
    /// Each written sector's slot in the journal, by its LBA.
    slots: BTreeMap<u64, u64>,
}

const fn assert_send_sync<T: Send + Sync>() {}
/// Shared by `&self` across threads from a snapshot's save; derived, never
/// `unsafe impl` (CLAUDE.md "Send by derivation").
const _: () = assert_send_sync::<VolatileImage>();

impl VolatileImage {
    /// Bochs `volatile_image_t::open`: the image read-only, and a journal made
    /// new beside `journal`'s template. A failure names the file it is about,
    /// as Bochs's `BX_PANIC("Can't create volatile redolog '%s'")` does.
    fn open(path: &Path, journal: &Journal) -> io::Result<VolatileImage> {
        let base = File::open(path).map_err(|error| naming("image", path, error))?;
        let size = base.metadata().map_err(|error| naming("image", path, error))?.len();
        let template = match journal {
            Journal::BesideImage => path,
            Journal::Named(template) => template.as_path(),
        };
        let made = make_journal(template).map_err(|error| naming("journal", template, error))?;
        // Bochs unlinks the journal at once where the platform lets an open
        // file lose its name; every host this port runs on does.
        std::fs::remove_file(&made.path)?;
        Ok(VolatileImage { base, size, journal: made.file, journal_path: made.path, slots: BTreeMap::new() })
    }

    /// The name the journal was made under.
    pub(crate) fn journal_path(&self) -> &Path {
        &self.journal_path
    }

    /// The byte offset of sector `lba`, when the image holds it.
    fn checked_offset(&self, lba: u64) -> io::Result<u64> {
        let offset = sector_offset(lba)?;
        match offset.checked_add(SECTOR_BYTES) {
            Some(end) if end <= self.size => Ok(offset),
            Some(_) | None => Err(io::Error::new(io::ErrorKind::InvalidInput, "sector past the end of the image")),
        }
    }

    /// Bochs `volatile_image_t::read`: the journal's copy, else the image's.
    pub(crate) fn read(&mut self, lba: u64, sector: &mut [u8]) -> io::Result<()> {
        let offset = self.checked_offset(lba)?;
        match self.slots.get(&lba) {
            Some(slot) => read_at(&self.journal, sector_offset(*slot)?, sector),
            None => read_at(&self.base, offset, sector),
        }
    }

    /// Bochs `volatile_image_t::write`: into the journal, never the image.
    pub(crate) fn write(&mut self, lba: u64, sector: &[u8]) -> io::Result<()> {
        self.checked_offset(lba)?;
        if sector.len() != SECTOR {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a write is one sector"));
        }
        let next = u64::try_from(self.slots.len())
            .map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "journal slot count"))?;
        let slot = *self.slots.get(&lba).unwrap_or(&next);
        write_at(&self.journal, sector_offset(slot)?, sector)?;
        self.slots.insert(lba, slot);
        Ok(())
    }

    /// How many sectors the journal holds.
    pub(crate) fn written_count(&self) -> usize {
        self.slots.len()
    }

    /// Every sector written, in sector order: what a snapshot keeps. Reads
    /// the journal by position, so any number of threads may call it at once.
    pub(crate) fn written(&self) -> io::Result<Vec<WrittenSector>> {
        let mut written = Vec::new();
        written.try_reserve(self.slots.len()).map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "written sectors"))?;
        for (lba, slot) in &self.slots {
            let mut bytes = [0; SECTOR];
            read_at(&self.journal, sector_offset(*slot)?, &mut bytes)?;
            written.push(WrittenSector { lba: *lba, bytes });
        }
        Ok(written)
    }

    /// The journal holds exactly `sectors` from now: what a snapshot restores.
    pub(crate) fn replace_written(&mut self, sectors: &[WrittenSector]) -> io::Result<()> {
        self.journal.set_len(0)?;
        self.slots.clear();
        for sector in sectors {
            self.write(sector.lba, &sector.bytes)?;
        }
        Ok(())
    }
}

/// `error`, of the same kind, saying which file (`what`, at `shown`) it is about.
fn naming(what: &str, shown: &Path, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{what} {}: {error}", shown.display()))
}

/// A journal just made, and the name it was made under.
struct MadeJournal {
    file: File,
    path: PathBuf,
}

/// Bochs `mkstemp(redolog_temp)`: `template` + `.` + six characters, made
/// new so no other file is ever opened as the journal.
fn make_journal(template: &Path) -> io::Result<MadeJournal> {
    let state = RandomState::new();
    for attempt in 0..JOURNAL_NAME_TRIES {
        let mut hasher = state.build_hasher();
        hasher.write_u32(attempt);
        let mut seed = hasher.finish();
        let mut name = template.as_os_str().to_owned();
        name.push(".");
        let mut suffix = String::with_capacity(JOURNAL_SUFFIX_CHARS);
        for _ in 0..JOURNAL_SUFFIX_CHARS {
            let pick = usize::try_from(seed % JOURNAL_SUFFIX_ALPHABET.len() as u64).unwrap_or(0);
            suffix.push(char::from(*JOURNAL_SUFFIX_ALPHABET.get(pick).unwrap_or(&b'X')));
            seed /= JOURNAL_SUFFIX_ALPHABET.len() as u64;
        }
        name.push(suffix);
        let path = PathBuf::from(name);
        match OpenOptions::new().read(true).write(true).create_new(true).open(&path) {
            Ok(file) => return Ok(MadeJournal { file, path }),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free journal name beside the template"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder of its own, removed when the test ends.
    struct Scratch {
        dir: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!("rusty-box-hdimage-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("scratch folder");
            Scratch { dir }
        }

        /// An image of `sectors` sectors, sector `n` filled with byte `n`.
        fn image(&self, sectors: u8) -> PathBuf {
            let path = self.dir.join("disk.img");
            let mut bytes = Vec::new();
            for sector in 0..sectors {
                bytes.extend_from_slice(&[sector; SECTOR]);
            }
            std::fs::write(&path, bytes).expect("image");
            path
        }

        /// Every file in the folder but the image.
        fn others(&self, image: &Path) -> Vec<PathBuf> {
            let mut found = Vec::new();
            for entry in std::fs::read_dir(&self.dir).expect("list") {
                let path = entry.expect("entry").path();
                if path != image {
                    found.push(path);
                }
            }
            found
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Err(error) = std::fs::remove_dir_all(&self.dir) {
                eprintln!("scratch folder {} stays: {error}", self.dir.display());
            }
        }
    }

    fn sector_of(byte: u8) -> [u8; SECTOR] {
        [byte; SECTOR]
    }

    fn read(image: &mut DiskImage, lba: u64) -> [u8; SECTOR] {
        let mut sector = [0; SECTOR];
        image.read_sector(lba, &mut sector).expect("read");
        sector
    }

    #[test]
    fn a_flat_image_is_written_in_place() {
        let scratch = Scratch::new("flat");
        let path = scratch.image(4);
        let mut image = DiskImage::open_flat(&path).expect("open");
        assert_eq!(image.size().expect("size"), 4 * SECTOR as u64);
        image.write_sector(2, &sector_of(0xAA)).expect("write");
        assert_eq!(read(&mut image, 2), sector_of(0xAA));
        drop(image);
        assert_eq!(std::fs::read(&path).expect("image")[2 * SECTOR], 0xAA);
    }

    #[test]
    fn a_volatile_image_reads_its_writes_and_leaves_the_file_alone() {
        let scratch = Scratch::new("volatile");
        let path = scratch.image(4);
        let mut image = DiskImage::open_volatile(&path, &Journal::BesideImage).expect("open");
        assert_eq!(image.size().expect("size"), 4 * SECTOR as u64);
        assert_eq!(read(&mut image, 1), sector_of(1), "an unwritten sector reads from the image");
        image.write_sector(1, &sector_of(0xAA)).expect("write");
        image.write_sector(3, &sector_of(0xBB)).expect("write");
        image.write_sector(1, &sector_of(0xCC)).expect("write again");
        assert_eq!(read(&mut image, 1), sector_of(0xCC), "the latest write wins");
        assert_eq!(read(&mut image, 3), sector_of(0xBB));
        assert_eq!(read(&mut image, 2), sector_of(2));
        drop(image);
        let bytes = std::fs::read(&path).expect("image");
        assert_eq!(bytes[SECTOR], 1, "the image is never written");
        assert_eq!(bytes[3 * SECTOR], 3);
        assert!(scratch.others(&path).is_empty(), "the journal is gone once the drive closes");
    }

    #[test]
    fn a_volatile_journal_is_named_after_its_template() {
        let scratch = Scratch::new("journal");
        let path = scratch.image(2);
        let template = scratch.dir.join("work");
        let image = DiskImage::open_volatile(&path, &Journal::Named(template.clone())).expect("open");
        let DiskImage::Volatile(volatile) = &image else { panic!("a volatile image") };
        let journal = volatile.journal_path().to_path_buf();
        let name = journal.file_name().and_then(|name| name.to_str()).expect("a name").to_owned();
        assert!(name.starts_with("work."), "{name}");
        assert_eq!(name.len(), "work.".len() + 6, "Bochs's six-character mkstemp suffix: {name}");
        assert!(name["work.".len()..].chars().all(|character| character.is_ascii_alphanumeric()));
        let beside = DiskImage::open_volatile(&path, &Journal::BesideImage).expect("open");
        let DiskImage::Volatile(beside) = &beside else { panic!("a volatile image") };
        let beside_name = beside.journal_path().file_name().and_then(|name| name.to_str()).expect("a name").to_owned();
        assert!(beside_name.starts_with("disk.img."), "{beside_name}");
        drop(image);
    }

    #[test]
    fn a_sector_past_the_end_is_refused() {
        let scratch = Scratch::new("end");
        let path = scratch.image(2);
        let mut image = DiskImage::open_volatile(&path, &Journal::BesideImage).expect("open");
        let mut sector = [0; SECTOR];
        assert!(image.read_sector(2, &mut sector).is_err());
        assert!(image.write_sector(2, &sector).is_err());
        assert!(image.write_sector(u64::MAX, &sector).is_err());
    }

    #[test]
    fn the_written_sectors_are_listed_and_replaced_whole() {
        let scratch = Scratch::new("written");
        let path = scratch.image(4);
        let mut image = DiskImage::open_volatile(&path, &Journal::BesideImage).expect("open");
        let DiskImage::Volatile(volatile) = &mut image else { panic!("a volatile image") };
        volatile.write(3, &sector_of(0xBB)).expect("write");
        volatile.write(0, &sector_of(0xAA)).expect("write");
        let written = volatile.written().expect("listed");
        assert_eq!(written.iter().map(|sector| sector.lba).collect::<Vec<u64>>(), [0, 3], "in sector order");
        assert_eq!(written[1].bytes, sector_of(0xBB));
        volatile.replace_written(&[WrittenSector { lba: 2, bytes: sector_of(0xEE) }]).expect("replaced");
        assert_eq!(read(&mut image, 2), sector_of(0xEE));
        assert_eq!(read(&mut image, 0), sector_of(0), "a sector the new set lacks reads from the image again");
        assert_eq!(read(&mut image, 3), sector_of(3));
    }

    #[test]
    fn a_journal_that_cannot_be_made_names_its_template() {
        let scratch = Scratch::new("named-error");
        let path = scratch.image(2);
        let template = scratch.dir.join("no-such-folder").join("journal");
        let error = DiskImage::open_volatile(&path, &Journal::Named(template.clone())).expect_err("no folder");
        assert!(error.to_string().contains(&template.display().to_string()), "{error}");
    }

    /// Compiles only while `VolatileImage: Sync` (the scope shares one
    /// `&VolatileImage`), and passes only while no reader depends on the
    /// journal's cursor.
    #[test]
    fn a_volatile_image_is_read_from_several_threads_at_once() {
        const READERS: usize = 4;
        const ROUNDS: usize = 64;
        let scratch = Scratch::new("threads");
        let path = scratch.image(4);
        let mut image = DiskImage::open_volatile(&path, &Journal::BesideImage).expect("open");
        let DiskImage::Volatile(volatile) = &mut image else { panic!("a volatile image") };
        volatile.write(0, &sector_of(0xAA)).expect("write");
        volatile.write(2, &sector_of(0xBB)).expect("write");
        volatile.write(3, &sector_of(0xCC)).expect("write");
        let volatile: &VolatileImage = volatile;
        std::thread::scope(|scope| {
            for _ in 0..READERS {
                scope.spawn(move || {
                    for _ in 0..ROUNDS {
                        let written = volatile.written().expect("listed");
                        assert_eq!(written.iter().map(|sector| sector.lba).collect::<Vec<u64>>(), [0, 2, 3]);
                        assert_eq!(written[0].bytes, sector_of(0xAA));
                        assert_eq!(written[1].bytes, sector_of(0xBB));
                        assert_eq!(written[2].bytes, sector_of(0xCC));
                    }
                });
            }
        });
    }
}
