//! A hard disk's image file, as Bochs `iodev/hdimage/hdimage.cc` opens one:
//! flat, written in place; or volatile (`volatile_image_t`), the image opened
//! read-only and every sector written kept in a journal file that is removed
//! when the drive closes.

use std::collections::btree_map;
use std::collections::hash_map::RandomState;
use std::collections::BTreeMap;
use std::fs::{File, Metadata, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// The bytes of one sector of a hard disk image.
pub(crate) const SECTOR: usize = 512;
const SECTOR_BYTES: u64 = SECTOR as u64;
/// Bochs hdimage.h `VOLATILE_REDOLOG_EXTENSION` (`".XXXXXX"`): a journal is
/// its template, a dot, and six characters `mkstemp` picks.
const JOURNAL_SUFFIX_CHARS: usize = 6;
const JOURNAL_SUFFIX_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
/// The names tried before a journal is given up. glibc's `mkstemp` tries up
/// to `TMP_MAX` names; Bochs's own `bx_mkstemp` (osdep.cc), used where the C
/// library has none, tries one and truncates any file that already has it.
/// Every try here makes the file new, so no other file is ever opened, or
/// emptied, as the journal (D23).
const JOURNAL_NAME_TRIES: u32 = 100;

/// Where a volatile disk keeps the sectors written to it (Bochs's `journal`
/// option for the `volatile` mode): beside the image, named after it, or
/// named after the template given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Journal {
    BesideImage,
    Named(PathBuf),
}

impl Journal {
    /// The file the journal of the image at `image` is named after. Bochs
    /// hdimage.cc `volatile_image_t::volatile_image_t` takes an empty
    /// `journal` or `none` as no name at all, and `volatile_image_t::open`
    /// then names the redolog after the image.
    fn template<'a>(&'a self, image: &'a Path) -> &'a Path {
        match self {
            Journal::Named(name) if !name.as_os_str().is_empty() && name.as_os_str() != "none" => name,
            Journal::Named(_) | Journal::BesideImage => image,
        }
    }
}

/// When an image was last modified, as its file said when the drive opened
/// it: the time Bochs hdimage.cc `coherency_check` compares, read through
/// `device_image_t::get_timestamp`, before it lets a saved redolog over an
/// image. [`ImageModified::UNKNOWN`] where the file keeps no such time, as
/// Bochs's timestamp 0 is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImageModified {
    pub(crate) seconds: u64,
    pub(crate) nanos: u32,
}

impl ImageModified {
    pub(crate) const UNKNOWN: ImageModified = ImageModified { seconds: 0, nanos: 0 };

    fn of(metadata: &Metadata) -> ImageModified {
        match metadata.modified().map(|time| time.duration_since(UNIX_EPOCH)) {
            Ok(Ok(since)) => ImageModified { seconds: since.as_secs(), nanos: since.subsec_nanos() },
            // A platform that keeps no modification time, or one before the
            // epoch: nothing to compare, as with Bochs's timestamp 0.
            Ok(Err(_)) | Err(_) => ImageModified::UNKNOWN,
        }
    }
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

/// Bochs hdimage.cc `volatile_image_t`: the image read-only (`ro_disk`), and
/// a journal (its `redolog`) holding every sector written. A sector is read
/// from the journal when it was written, else from the image. The journal's
/// name is removed as soon as it is made where the platform allows it, else
/// when the drive closes, so nothing of it outlives the drive.
#[derive(Debug)]
pub(crate) struct VolatileImage {
    base: File,
    size: u64,
    /// The image's modification time when it was opened.
    modified: ImageModified,
    /// The file every journal of this image is named after: a refill's
    /// journal is made beside the first.
    template: PathBuf,
    journal: JournalFile,
    /// Each written sector's slot in the journal, by its LBA.
    slots: BTreeMap<u64, u64>,
}

const fn assert_send_sync<T: Send + Sync>() {}
/// Shared by `&self` across threads from a snapshot's save; derived, never
/// `unsafe impl` (CLAUDE.md "Send by derivation").
const _: () = assert_send_sync::<VolatileImage>();
const _: () = assert_send_sync::<VolatileDisk>();

impl VolatileImage {
    /// Bochs hdimage.cc `volatile_image_t::open`: the image read-only, and a
    /// journal made new beside `journal`'s template. A failure names the file
    /// it is about, as Bochs's `BX_PANIC("Can't create volatile redolog '%s'")`
    /// does.
    fn open(path: &Path, journal: &Journal) -> io::Result<VolatileImage> {
        let base = File::open(path).map_err(|error| naming("image", path, error))?;
        let metadata = base.metadata().map_err(|error| naming("image", path, error))?;
        let template = journal.template(path).to_path_buf();
        let journal = JournalFile::make(&template).map_err(|error| naming("journal", &template, error))?;
        Ok(VolatileImage {
            base,
            size: metadata.len(),
            modified: ImageModified::of(&metadata),
            template,
            journal,
            slots: BTreeMap::new(),
        })
    }

    /// The name the journal was made under.
    pub(crate) fn journal_path(&self) -> &Path {
        &self.journal.path
    }

    /// The image's modification time when it was opened.
    pub(crate) fn image_modified(&self) -> ImageModified {
        self.modified
    }

    /// Bochs hdimage.cc `volatile_image_t::read`: the journal's copy, else the
    /// image's.
    pub(crate) fn read(&mut self, lba: u64, sector: &mut [u8]) -> io::Result<()> {
        let offset = checked_offset(self.size, lba)?;
        match self.slots.get(&lba) {
            Some(slot) => read_at(&self.journal.file, sector_offset(*slot)?, sector),
            None => read_at(&self.base, offset, sector),
        }
    }

    /// Bochs hdimage.cc `volatile_image_t::write`: into the journal, never the
    /// image.
    pub(crate) fn write(&mut self, lba: u64, sector: &[u8]) -> io::Result<()> {
        checked_offset(self.size, lba)?;
        write_journal(&self.journal.file, &mut self.slots, lba, sector)
    }

    /// How many sectors the journal holds.
    pub(crate) fn written_count(&self) -> usize {
        self.slots.len()
    }

    /// Every sector written, in sector order, each read from the journal as it
    /// is reached: what a snapshot keeps, with one sector in memory at a time.
    /// Reads by position, so any number of threads may walk it at once.
    pub(crate) fn written(&self) -> WrittenSectors<'_> {
        WrittenSectors {
            journal: &self.journal.file,
            slots: self.slots.iter(),
        }
    }

    /// A new, empty journal for this image, beside the one it has: what a
    /// snapshot's sectors are restored into. This image keeps its own
    /// journal until [`Self::adopt`] takes the new one.
    pub(crate) fn begin_refill(&self) -> io::Result<JournalRefill> {
        let journal = JournalFile::make(&self.template).map_err(|error| naming("journal", &self.template, error))?;
        Ok(JournalRefill {
            journal,
            slots: BTreeMap::new(),
            size: self.size,
        })
    }

    /// The journal holds exactly what `refill` was given from now on; the one
    /// it had is closed, and with it goes whatever of its name was left.
    pub(crate) fn adopt(&mut self, refill: JournalRefill) {
        self.journal = refill.journal;
        self.slots = refill.slots;
    }
}

/// A journal being filled from a snapshot ([`VolatileImage::begin_refill`]).
/// Dropped instead of adopted, it is gone, and the image's journal is as it
/// was.
#[derive(Debug)]
pub(crate) struct JournalRefill {
    journal: JournalFile,
    slots: BTreeMap<u64, u64>,
    size: u64,
}

impl JournalRefill {
    pub(crate) fn write(&mut self, lba: u64, sector: &[u8]) -> io::Result<()> {
        checked_offset(self.size, lba)?;
        write_journal(&self.journal.file, &mut self.slots, lba, sector)
    }
}

/// The sectors a volatile image holds over its image, in sector order
/// ([`VolatileImage::written`]).
#[derive(Debug)]
pub(crate) struct WrittenSectors<'a> {
    journal: &'a File,
    slots: btree_map::Iter<'a, u64, u64>,
}

impl Iterator for WrittenSectors<'_> {
    type Item = io::Result<WrittenSector>;

    fn next(&mut self) -> Option<io::Result<WrittenSector>> {
        let (lba, slot) = self.slots.next()?;
        let mut bytes = [0; SECTOR];
        Some(
            sector_offset(*slot)
                .and_then(|offset| read_at(self.journal, offset, &mut bytes))
                .map(|()| WrittenSector { lba: *lba, bytes }),
        )
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slots.size_hint()
    }
}

/// A volatile disk opened and not attached to a machine, or taken back from
/// one: the image read-only, and its journal, which lasts as long as this
/// does.
///
/// A front end that builds a new machine for a restart gives each the same
/// one, so the guest's writes last until power-off, as Bochs's redolog lasts
/// across a reset: hdimage.cc `volatile_image_t::close` runs only when Bochs
/// exits.
#[derive(Debug)]
pub struct VolatileDisk {
    pub(crate) image: VolatileImage,
    /// The image's path as it was given, the drive's identity in a snapshot.
    pub(crate) path: String,
}

impl VolatileDisk {
    /// The image at `path`, opened read-only, and a journal made new as
    /// `journal` says (Bochs hdimage.cc `volatile_image_t::open`).
    pub fn open(path: &str, journal: &Journal) -> io::Result<VolatileDisk> {
        let image = VolatileImage::open(Path::new(path), journal)?;
        // Bochs hdimage.cc volatile_image_t::open's line.
        tracing::info!(
            "'volatile' disk opened: ro-file is '{}', redolog is '{}'",
            path,
            image.journal_path().display()
        );
        Ok(VolatileDisk {
            image,
            path: path.to_owned(),
        })
    }

    /// The name the journal was made under.
    pub fn journal_path(&self) -> &Path {
        self.image.journal_path()
    }
}

/// The byte offset of sector `lba` in an image of `size` bytes, when the
/// image holds it.
fn checked_offset(size: u64, lba: u64) -> io::Result<u64> {
    let offset = sector_offset(lba)?;
    match offset.checked_add(SECTOR_BYTES) {
        Some(end) if end <= size => Ok(offset),
        Some(_) | None => Err(io::Error::new(io::ErrorKind::InvalidInput, "sector past the end of the image")),
    }
}

/// `sector` written to `journal` as sector `lba`: over its slot when it has
/// one, else into the next free slot.
fn write_journal(journal: &File, slots: &mut BTreeMap<u64, u64>, lba: u64, sector: &[u8]) -> io::Result<()> {
    if sector.len() != SECTOR {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "a write is one sector"));
    }
    let next = u64::try_from(slots.len()).map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "journal slot count"))?;
    let slot = *slots.get(&lba).unwrap_or(&next);
    write_at(journal, sector_offset(slot)?, sector)?;
    slots.insert(lba, slot);
    Ok(())
}

/// Whether a journal's file still has its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JournalName {
    /// Removed as soon as the file was made: nothing of it outlives the
    /// drive, a crash included.
    Removed,
    /// The platform would not remove the name of an open file, so it goes
    /// when the drive closes, as Bochs hdimage.cc `volatile_image_t::close`
    /// removes it on Windows.
    RemoveAtClose,
}

/// A journal's file and the name it was made under.
#[derive(Debug)]
struct JournalFile {
    file: File,
    path: PathBuf,
    name: JournalName,
}

impl JournalFile {
    /// A journal made new beside `template`, its name removed at once where
    /// the platform allows it, else at close. Bochs unlinks at once where the
    /// platform lets an open file lose its name and at close elsewhere; a
    /// refusal here is that elsewhere, not a reason to refuse the drive.
    fn make(template: &Path) -> io::Result<JournalFile> {
        let made = make_journal(template)?;
        let name = match std::fs::remove_file(&made.path) {
            Ok(()) => JournalName::Removed,
            Err(error) => {
                tracing::warn!("journal {} keeps its name until the drive closes: {error}", made.path.display());
                JournalName::RemoveAtClose
            }
        };
        Ok(JournalFile { file: made.file, path: made.path, name })
    }
}

impl Drop for JournalFile {
    fn drop(&mut self) {
        match self.name {
            JournalName::Removed => {}
            JournalName::RemoveAtClose => match std::fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(error) => tracing::error!("journal {} is left behind: {error}", self.path.display()),
            },
        }
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

    /// A volatile image over `path`, its journal placed as `journal` says.
    fn volatile(path: &Path, journal: &Journal) -> DiskImage {
        DiskImage::Volatile(VolatileDisk::open(path.to_str().expect("utf-8"), journal).expect("open").image)
    }

    /// Bochs hdimage.cc `volatile_image_t::volatile_image_t`: an empty
    /// `journal` and `none` name no journal of their own.
    #[test]
    fn a_journal_named_empty_or_none_goes_beside_the_image() {
        let scratch = Scratch::new("unnamed-journal");
        let path = scratch.image(2);
        for name in ["", "none"] {
            let disk = VolatileDisk::open(path.to_str().expect("utf-8"), &Journal::Named(PathBuf::from(name)))
                .expect("open");
            let made = disk.journal_path().file_name().and_then(|made| made.to_str()).expect("a name").to_owned();
            assert!(made.starts_with("disk.img."), "{name:?} names no journal of its own: {made}");
        }
    }

    /// A journal whose name the platform would not remove while it was open
    /// loses it when the drive closes, as Bochs hdimage.cc
    /// `volatile_image_t::close` removes it on Windows.
    #[test]
    fn a_journal_that_kept_its_name_loses_it_when_the_drive_closes() {
        let scratch = Scratch::new("named-at-close");
        let named = scratch.dir.join("still-named");
        std::fs::write(&named, b"").expect("file");
        let journal = JournalFile {
            file: File::open(&named).expect("open"),
            path: named.clone(),
            name: JournalName::RemoveAtClose,
        };
        drop(journal);
        assert!(!named.exists(), "the drive's close removes the name the journal kept");
    }

    #[test]
    fn a_refilled_journal_takes_over_only_when_adopted() {
        let scratch = Scratch::new("refill");
        let path = scratch.image(4);
        let mut image = volatile(&path, &Journal::BesideImage);
        let DiskImage::Volatile(own) = &mut image else { panic!("a volatile image") };
        own.write(1, &sector_of(0xAA)).expect("write");
        let mut abandoned = own.begin_refill().expect("refill");
        abandoned.write(2, &sector_of(0xBB)).expect("write");
        assert!(abandoned.write(4, &sector_of(0xBB)).is_err(), "a sector past the image");
        drop(abandoned);
        assert_eq!(read(&mut image, 1), sector_of(0xAA), "an abandoned refill changes nothing");
        assert_eq!(read(&mut image, 2), sector_of(2));

        let DiskImage::Volatile(own) = &mut image else { panic!("a volatile image") };
        let mut refill = own.begin_refill().expect("refill");
        refill.write(2, &sector_of(0xBB)).expect("write");
        own.adopt(refill);
        assert_eq!(own.written_count(), 1);
        assert_eq!(read(&mut image, 1), sector_of(1), "a sector the refill lacks reads from the image again");
        assert_eq!(read(&mut image, 2), sector_of(0xBB));
        drop(image);
        assert!(scratch.others(&path).is_empty(), "neither journal outlives the drive");
    }

    /// What Bochs hdimage.cc `coherency_check` compares: the image's
    /// modification time as the drive found it at open.
    #[test]
    fn an_image_records_when_it_was_modified() {
        let scratch = Scratch::new("modified");
        let path = scratch.image(2);
        // A multiple of 100 ns, the finest time NTFS keeps.
        let when = std::time::UNIX_EPOCH + std::time::Duration::new(1_700_000_000, 123_456_700);
        File::options().write(true).open(&path).expect("open").set_modified(when).expect("touch");
        let disk = VolatileDisk::open(path.to_str().expect("utf-8"), &Journal::BesideImage).expect("open");
        assert_eq!(
            disk.image.image_modified(),
            ImageModified {
                seconds: 1_700_000_000,
                nanos: 123_456_700
            }
        );
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
        let mut image = volatile(&path, &Journal::BesideImage);
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
        let image = path.to_str().expect("utf-8");
        let named = VolatileDisk::open(image, &Journal::Named(template.clone())).expect("open");
        let name = named.journal_path().file_name().and_then(|name| name.to_str()).expect("a name").to_owned();
        assert!(name.starts_with("work."), "{name}");
        assert_eq!(name.len(), "work.".len() + 6, "Bochs's six-character mkstemp suffix: {name}");
        assert!(name["work.".len()..].chars().all(|character| character.is_ascii_alphanumeric()));
        let beside = VolatileDisk::open(image, &Journal::BesideImage).expect("open");
        let beside_name = beside.journal_path().file_name().and_then(|name| name.to_str()).expect("a name").to_owned();
        assert!(beside_name.starts_with("disk.img."), "{beside_name}");
        drop(named);
    }

    #[test]
    fn a_sector_past_the_end_is_refused() {
        let scratch = Scratch::new("end");
        let path = scratch.image(2);
        let mut image = volatile(&path, &Journal::BesideImage);
        let mut sector = [0; SECTOR];
        assert!(image.read_sector(2, &mut sector).is_err());
        assert!(image.write_sector(2, &sector).is_err());
        assert!(image.write_sector(u64::MAX, &sector).is_err());
    }

    #[test]
    fn the_written_sectors_are_listed_in_sector_order() {
        let scratch = Scratch::new("written");
        let path = scratch.image(4);
        let mut image = volatile(&path, &Journal::BesideImage);
        let DiskImage::Volatile(own) = &mut image else { panic!("a volatile image") };
        own.write(3, &sector_of(0xBB)).expect("write");
        own.write(0, &sector_of(0xAA)).expect("write");
        let written = own.written().collect::<io::Result<Vec<WrittenSector>>>().expect("listed");
        assert_eq!(written.iter().map(|sector| sector.lba).collect::<Vec<u64>>(), [0, 3], "in sector order");
        assert_eq!(written[0].bytes, sector_of(0xAA));
        assert_eq!(written[1].bytes, sector_of(0xBB));
    }

    #[test]
    fn a_journal_that_cannot_be_made_names_its_template() {
        let scratch = Scratch::new("named-error");
        let path = scratch.image(2);
        let template = scratch.dir.join("no-such-folder").join("journal");
        let error = VolatileDisk::open(path.to_str().expect("utf-8"), &Journal::Named(template.clone()))
            .expect_err("no folder");
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
        let mut image = volatile(&path, &Journal::BesideImage);
        let DiskImage::Volatile(own) = &mut image else { panic!("a volatile image") };
        own.write(0, &sector_of(0xAA)).expect("write");
        own.write(2, &sector_of(0xBB)).expect("write");
        own.write(3, &sector_of(0xCC)).expect("write");
        let shared: &VolatileImage = own;
        std::thread::scope(|scope| {
            for _ in 0..READERS {
                scope.spawn(move || {
                    for _ in 0..ROUNDS {
                        let written = shared.written().collect::<io::Result<Vec<WrittenSector>>>().expect("listed");
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
