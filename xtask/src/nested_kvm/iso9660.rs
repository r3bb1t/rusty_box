//! The files of an ISO 9660 image (ECMA-119) by their Rock Ridge names:
//! the `NM` entries (IEEE P1282) in each directory record's System Use
//! field, read as the System Use Sharing Protocol (IEEE P1281) lays them
//! out, `CE` continuation areas followed. A record with no `NM` is named by
//! its ISO 9660 identifier, less its `;` version and a trailing `.`.

/// ECMA-119 6.1.2: the volume descriptors are 2048-byte sectors from
/// sector 16 on, whatever the volume's logical block size.
const DESCRIPTOR_SECTOR: usize = 2048;
const FIRST_DESCRIPTOR: usize = 16;
/// ECMA-119 8.1: a descriptor's type byte and standard identifier.
const PRIMARY_DESCRIPTOR: u8 = 1;
const SET_TERMINATOR: u8 = 255;
const STANDARD_IDENTIFIER: &[u8] = b"CD001";
/// ECMA-119 8.4: where the primary descriptor holds the logical block size
/// (both-endian, little-endian half first) and the root's directory record.
const BLOCK_SIZE_AT: usize = 128;
const ROOT_RECORD_AT: usize = 156;
const ROOT_RECORD_LEN: usize = 34;

/// ECMA-119 9.1: a directory record's fixed part, up to its identifier.
const RECORD_FIXED_LEN: usize = 33;
const RECORD_EXTENT_AT: usize = 2;
const RECORD_DATA_LENGTH_AT: usize = 10;
const RECORD_FLAGS_AT: usize = 25;
const RECORD_IDENTIFIER_LEN_AT: usize = 32;
/// ECMA-119 9.1.6: file flags.
const FLAG_DIRECTORY: u8 = 0x02;
const FLAG_MULTI_EXTENT: u8 = 0x80;
/// ECMA-119 9.1.11: the identifiers of a directory's own record and its
/// parent's.
const SELF_IDENTIFIER: &[u8] = &[0];
const PARENT_IDENTIFIER: &[u8] = &[1];

/// IEEE P1281 5.3: the `SP` entry opening the System Use field of the
/// root's own record, its check bytes, and where it holds the bytes every
/// other System Use field skips.
const SP_LEN: usize = 7;
const SP_CHECK: &[u8] = &[0xBE, 0xEF];
const SP_SKIP_AT: usize = 6;
/// IEEE P1281 4: an entry's header (signature, length, version).
const ENTRY_HEADER_LEN: usize = 4;
/// IEEE P1281 5.1: a `CE` entry's continuation block, offset and length,
/// each both-endian, little-endian half first.
const CE_LEN: usize = 28;
const CE_BLOCK_AT: usize = 4;
const CE_OFFSET_AT: usize = 12;
const CE_LENGTH_AT: usize = 20;
/// IEEE P1282 4.1.4: an `NM` entry's flags and where its name part starts;
/// a part flagged CURRENT or PARENT names `.` or `..` and carries no text.
const NM_FLAGS_AT: usize = 4;
const NM_NAME_AT: usize = 5;
const NM_CURRENT: u8 = 0x02;
const NM_PARENT: u8 = 0x04;
/// The continuation areas one System Use field may chain through; a longer
/// chain is a loop.
const MAX_CONTINUATIONS: usize = 64;

/// An ISO 9660 image in memory.
pub struct IsoImage<'a> {
    bytes: &'a [u8],
    block_size: u64,
    root: Entry,
    susp: Susp,
}

/// Whether the volume's directory records carry SUSP entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Susp {
    /// No `SP` entry in the root's own record: the identifiers are the names.
    Absent,
    /// Each System Use field's entries start this many bytes in.
    Present { skip: usize },
}

/// One extent of a file: its first logical block and its length in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Extent {
    block: u32,
    length: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    File,
    Directory,
}

/// A file or directory, its extents in order (a multi-extent file has one
/// record, and one extent, per piece).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    name: String,
    kind: EntryKind,
    extents: Vec<Extent>,
}

/// The fields of one directory record the reader uses.
struct Record<'r> {
    extent: Extent,
    flags: u8,
    identifier: &'r [u8],
    system_use: &'r [u8],
}

impl<'a> IsoImage<'a> {
    /// The image in `bytes`, by its primary volume descriptor.
    pub fn open(bytes: &'a [u8]) -> Result<Self, String> {
        let mut sector = FIRST_DESCRIPTOR;
        let primary = loop {
            let start = sector * DESCRIPTOR_SECTOR;
            let descriptor = bytes
                .get(start..start + DESCRIPTOR_SECTOR)
                .ok_or("the image ends before its primary volume descriptor")?;
            if &descriptor[1..6] != STANDARD_IDENTIFIER {
                return Err(format!("sector {sector} is not a volume descriptor"));
            }
            match descriptor[0] {
                PRIMARY_DESCRIPTOR => break descriptor,
                SET_TERMINATOR => {
                    return Err("the volume descriptor set has no primary descriptor".into())
                }
                _ => sector += 1,
            }
        };
        let block_size = u16::from_le_bytes(field(primary, BLOCK_SIZE_AT, "the block size")?);
        if !block_size.is_power_of_two() || block_size < 512 {
            return Err(format!(
                "logical block size {block_size} is not one ECMA-119 allows"
            ));
        }
        let root = parse_record(&primary[ROOT_RECORD_AT..ROOT_RECORD_AT + ROOT_RECORD_LEN])?;
        let mut image = Self {
            bytes,
            block_size: u64::from(block_size),
            root: Entry {
                name: String::new(),
                kind: EntryKind::Directory,
                extents: vec![root.extent],
            },
            susp: Susp::Absent,
        };
        image.susp = image.root_susp()?;
        Ok(image)
    }

    /// The contents of the file at `path`, `/`-separated from the root.
    pub fn file(&self, path: &str) -> Result<Vec<u8>, String> {
        let mut current = self.root.clone();
        for component in path.split('/').filter(|c| !c.is_empty()) {
            match current.kind {
                EntryKind::Directory => {}
                EntryKind::File => {
                    return Err(format!("{path}: {} is not a directory", current.name))
                }
            }
            current = self
                .entries(&current)?
                .into_iter()
                .find(|entry| entry.name == component)
                .ok_or_else(|| format!("{path}: no {component} in the image"))?;
        }
        match current.kind {
            EntryKind::File => {
                let mut contents = Vec::new();
                for extent in &current.extents {
                    contents.extend_from_slice(self.extent(*extent)?);
                }
                Ok(contents)
            }
            EntryKind::Directory => Err(format!("{path} is a directory")),
        }
    }

    /// Whether the root's own record opens with an `SP` entry, and the
    /// skip it gives.
    fn root_susp(&self) -> Result<Susp, String> {
        let directory = self.directory_bytes(&self.root)?;
        let length = usize::from(*directory.first().ok_or("the root directory is empty")?);
        let own = parse_record(
            directory
                .get(..length)
                .ok_or("the root's own record runs past its directory")?,
        )?;
        let su = own.system_use;
        Ok(
            if su.len() >= SP_LEN && &su[..2] == b"SP" && &su[4..6] == SP_CHECK {
                Susp::Present {
                    skip: usize::from(su[SP_SKIP_AT]),
                }
            } else {
                Susp::Absent
            },
        )
    }

    /// The bytes of one extent.
    fn extent(&self, extent: Extent) -> Result<&'a [u8], String> {
        self.span(
            u64::from(extent.block) * self.block_size,
            u64::from(extent.length),
        )
    }

    /// `length` bytes at byte `offset` of the image.
    fn span(&self, offset: u64, length: u64) -> Result<&'a [u8], String> {
        let outside = || format!("{length} bytes at {offset} run past the image");
        let start = usize::try_from(offset).map_err(|_| outside())?;
        let end = offset
            .checked_add(length)
            .and_then(|end| usize::try_from(end).ok())
            .ok_or_else(outside)?;
        self.bytes.get(start..end).ok_or_else(outside)
    }

    /// A directory's records, as one run of bytes.
    fn directory_bytes(&self, directory: &Entry) -> Result<&'a [u8], String> {
        match directory.extents.as_slice() {
            [extent] => self.extent(*extent),
            _ => Err(format!(
                "directory {} has {} extents, not one",
                directory.name,
                directory.extents.len()
            )),
        }
    }

    /// The files and directories in `directory`, its own and its parent's
    /// records left out. A record never crosses a logical block: a zero
    /// length byte pads the rest of a block.
    fn entries(&self, directory: &Entry) -> Result<Vec<Entry>, String> {
        let data = self.directory_bytes(directory)?;
        let block = usize::try_from(self.block_size).map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        let mut continuing: Option<Entry> = None;
        let mut at = 0;
        while at < data.len() {
            let length = usize::from(data[at]);
            if length == 0 {
                at = (at / block + 1) * block;
                continue;
            }
            let record = parse_record(data.get(at..at + length).ok_or_else(|| {
                format!("a record in directory {} runs past it", directory.name)
            })?)?;
            at += length;
            if record.identifier == SELF_IDENTIFIER || record.identifier == PARENT_IDENTIFIER {
                continue;
            }
            match continuing.as_mut() {
                Some(entry) => entry.extents.push(record.extent),
                None => {
                    continuing = Some(Entry {
                        name: self.name(&record)?,
                        kind: if record.flags & FLAG_DIRECTORY != 0 {
                            EntryKind::Directory
                        } else {
                            EntryKind::File
                        },
                        extents: vec![record.extent],
                    })
                }
            }
            if record.flags & FLAG_MULTI_EXTENT == 0 {
                entries.extend(continuing.take());
            }
        }
        match continuing {
            None => Ok(entries),
            Some(entry) => Err(format!(
                "{} in directory {} has no final extent",
                entry.name, directory.name
            )),
        }
    }

    /// A record's Rock Ridge name, else its ISO 9660 identifier less its
    /// version.
    fn name(&self, record: &Record<'_>) -> Result<String, String> {
        let rock_ridge = match self.susp {
            Susp::Absent => None,
            Susp::Present { skip } => {
                self.rock_ridge_name(record.system_use.get(skip..).unwrap_or_default())?
            }
        };
        match rock_ridge {
            Some(name) => Ok(name),
            None => iso_name(record.identifier),
        }
    }

    /// The name the `NM` entries of a System Use field spell, its parts
    /// joined in order through every continuation area; `None` when there
    /// is no `NM`.
    fn rock_ridge_name(&self, system_use: &'a [u8]) -> Result<Option<String>, String> {
        let mut name: Option<Vec<u8>> = None;
        let mut area = system_use;
        for _ in 0..MAX_CONTINUATIONS {
            let mut continuation = None;
            let mut at = 0;
            while at + ENTRY_HEADER_LEN <= area.len() {
                let length = usize::from(area[at + 2]);
                // A length too short for a header, or past the area, is the
                // padding that ends it.
                if length < ENTRY_HEADER_LEN || at + length > area.len() {
                    break;
                }
                let entry = &area[at..at + length];
                match &entry[..2] {
                    b"NM" => {
                        let flags = *entry.get(NM_FLAGS_AT).ok_or("an NM entry has no flags")?;
                        if flags & (NM_CURRENT | NM_PARENT) == 0 {
                            name.get_or_insert_with(Vec::new)
                                .extend_from_slice(&entry[NM_NAME_AT..]);
                        }
                    }
                    b"CE" => {
                        if length < CE_LEN {
                            return Err(format!("a CE entry of {length} bytes"));
                        }
                        let block = u32::from_le_bytes(field(entry, CE_BLOCK_AT, "CE block")?);
                        let offset = u32::from_le_bytes(field(entry, CE_OFFSET_AT, "CE offset")?);
                        let size = u32::from_le_bytes(field(entry, CE_LENGTH_AT, "CE length")?);
                        continuation = Some(self.span(
                            u64::from(block) * self.block_size + u64::from(offset),
                            u64::from(size),
                        )?);
                    }
                    b"ST" => break,
                    _ => {}
                }
                at += length;
            }
            match continuation {
                Some(next) => area = next,
                None => {
                    return name
                        .map(|bytes| {
                            String::from_utf8(bytes)
                                .map_err(|e| format!("a Rock Ridge name is not UTF-8: {e}"))
                        })
                        .transpose()
                }
            }
        }
        Err(format!(
            "a System Use field chains through more than {MAX_CONTINUATIONS} continuation areas"
        ))
    }
}

/// The fields of the directory record that is all of `record`.
fn parse_record(record: &[u8]) -> Result<Record<'_>, String> {
    if record.len() <= RECORD_FIXED_LEN {
        return Err(format!("a directory record of {} bytes", record.len()));
    }
    let identifier_end = RECORD_FIXED_LEN + usize::from(record[RECORD_IDENTIFIER_LEN_AT]);
    let identifier = record
        .get(RECORD_FIXED_LEN..identifier_end)
        .ok_or("a directory record's identifier runs past it")?;
    // ECMA-119 9.1.12: a pad byte follows an identifier of even length.
    let system_use_at = identifier_end + usize::from(identifier.len().is_multiple_of(2));
    Ok(Record {
        extent: Extent {
            block: u32::from_le_bytes(field(record, RECORD_EXTENT_AT, "the extent")?),
            length: u32::from_le_bytes(field(record, RECORD_DATA_LENGTH_AT, "the data length")?),
        },
        flags: record[RECORD_FLAGS_AT],
        identifier,
        system_use: record.get(system_use_at..).unwrap_or_default(),
    })
}

/// An ISO 9660 file identifier as a name: `NAME.EXT;1` is `NAME.EXT`, and
/// `NAME.;1` is `NAME`.
fn iso_name(identifier: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(identifier)
        .map_err(|e| format!("an ISO 9660 identifier is not text: {e}"))?;
    let unversioned = match text.find(';') {
        Some(at) => &text[..at],
        None => text,
    };
    Ok(unversioned
        .strip_suffix('.')
        .unwrap_or(unversioned)
        .to_owned())
}

/// The `N` bytes at `at` of `bytes`.
fn field<const N: usize>(bytes: &[u8], at: usize, what: &str) -> Result<[u8; N], String> {
    bytes
        .get(at..at + N)
        .and_then(|slice| <[u8; N]>::try_from(slice).ok())
        .ok_or_else(|| format!("{what} runs past its record"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: usize = 2048;

    fn both_endian_u32(value: u32) -> [u8; 8] {
        let mut out = [0; 8];
        out[..4].copy_from_slice(&value.to_le_bytes());
        out[4..].copy_from_slice(&value.to_be_bytes());
        out
    }

    /// A directory record, padded to an even length.
    fn record(block: u32, size: u32, flags: u8, identifier: &[u8], system_use: &[u8]) -> Vec<u8> {
        let pad = usize::from(identifier.len().is_multiple_of(2));
        let unpadded = RECORD_FIXED_LEN + identifier.len() + pad + system_use.len();
        let length = unpadded + unpadded % 2;
        let mut r = vec![0u8; length];
        r[0] = u8::try_from(length).expect("a record fits its length byte");
        r[2..10].copy_from_slice(&both_endian_u32(block));
        r[10..18].copy_from_slice(&both_endian_u32(size));
        r[25] = flags;
        r[28..30].copy_from_slice(&1u16.to_le_bytes());
        r[30..32].copy_from_slice(&1u16.to_be_bytes());
        r[32] = u8::try_from(identifier.len()).expect("an identifier fits");
        r[33..33 + identifier.len()].copy_from_slice(identifier);
        let su = 33 + identifier.len() + pad;
        r[su..su + system_use.len()].copy_from_slice(system_use);
        r
    }

    fn nm(flags: u8, name: &[u8]) -> Vec<u8> {
        let mut entry = vec![
            b'N',
            b'M',
            u8::try_from(5 + name.len()).expect("fits"),
            1,
            flags,
        ];
        entry.extend_from_slice(name);
        entry
    }

    fn ce(block: u32, offset: u32, length: u32) -> Vec<u8> {
        let mut entry = vec![b'C', b'E', 28, 1];
        entry.extend_from_slice(&both_endian_u32(block));
        entry.extend_from_slice(&both_endian_u32(offset));
        entry.extend_from_slice(&both_endian_u32(length));
        entry
    }

    fn put(image: &mut [u8], at: usize, bytes: &[u8]) {
        image[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// Records laid one after another from `at`.
    fn put_records(image: &mut [u8], mut at: usize, records: &[Vec<u8>]) {
        for r in records {
            put(image, at, r);
            at += r.len();
        }
    }

    const KERNEL: &[u8] = b"an arm64 kernel, as far as this test cares";
    const README: &[u8] = b"read me";
    const SPLIT_TAIL: &[u8] = b"tail!";

    /// Sectors: 16 primary descriptor, 17 terminator, 18 root, 19-20
    /// `boot` (two blocks), 21 a continuation area, 22 the kernel, 23 the
    /// readme, 24-25 a two-extent file.
    fn sample_image() -> Vec<u8> {
        let mut image = vec![0u8; 26 * BLOCK];
        let pvd = 16 * BLOCK;
        put(&mut image, pvd, &[PRIMARY_DESCRIPTOR]);
        put(&mut image, pvd + 1, STANDARD_IDENTIFIER);
        put(&mut image, pvd + 6, &[1]);
        put(&mut image, pvd + BLOCK_SIZE_AT, &2048u16.to_le_bytes());
        put(&mut image, pvd + BLOCK_SIZE_AT + 2, &2048u16.to_be_bytes());
        let root = record(18, 2048, FLAG_DIRECTORY, SELF_IDENTIFIER, &[]);
        assert_eq!(root.len(), ROOT_RECORD_LEN);
        put(&mut image, pvd + ROOT_RECORD_AT, &root);
        put(&mut image, 17 * BLOCK, &[SET_TERMINATOR]);
        put(&mut image, 17 * BLOCK + 1, STANDARD_IDENTIFIER);

        let mut sp = vec![b'S', b'P', 7, 1, 0xBE, 0xEF, 0];
        sp.extend(nm(NM_CURRENT, b""));
        put_records(
            &mut image,
            18 * BLOCK,
            &[
                record(18, 2048, FLAG_DIRECTORY, SELF_IDENTIFIER, &sp),
                record(18, 2048, FLAG_DIRECTORY, PARENT_IDENTIFIER, &[]),
                record(19, 4096, FLAG_DIRECTORY, b"BOOT", &nm(0, b"boot")),
            ],
        );

        let tail = nm(0, b"virt");
        let mut kernel_su = nm(0x01, b"vmlinuz-");
        kernel_su.extend(ce(21, 100, u32::try_from(tail.len()).expect("fits")));
        put(&mut image, 21 * BLOCK + 100, &tail);
        put_records(
            &mut image,
            19 * BLOCK,
            &[
                record(19, 4096, FLAG_DIRECTORY, SELF_IDENTIFIER, &[]),
                record(18, 2048, FLAG_DIRECTORY, PARENT_IDENTIFIER, &[]),
                record(22, KERNEL.len() as u32, 0, b"VMLINUZ_.;1", &kernel_su),
            ],
        );
        // The second block of `boot`: the first block's tail is zero, which
        // ends its records.
        put_records(
            &mut image,
            20 * BLOCK,
            &[
                record(23, README.len() as u32, 0, b"README.TXT;1", &[]),
                record(
                    24,
                    2048,
                    FLAG_MULTI_EXTENT,
                    b"SPLIT.BIN;1",
                    &nm(0, b"split.bin"),
                ),
                record(25, SPLIT_TAIL.len() as u32, 0, b"SPLIT.BIN;1", &[]),
            ],
        );
        put(&mut image, 22 * BLOCK, KERNEL);
        put(&mut image, 23 * BLOCK, README);
        image[24 * BLOCK..25 * BLOCK].fill(b'x');
        put(&mut image, 25 * BLOCK, SPLIT_TAIL);
        image
    }

    #[test]
    fn a_name_split_across_a_continuation_area_is_found() {
        let image = sample_image();
        let iso = IsoImage::open(&image).expect("open");
        assert_eq!(iso.susp, Susp::Present { skip: 0 });
        assert_eq!(iso.file("boot/vmlinuz-virt").expect("read"), KERNEL);
        assert_eq!(iso.file("/boot/vmlinuz-virt").expect("read"), KERNEL);
    }

    #[test]
    fn a_record_without_nm_is_named_by_its_identifier_less_its_version() {
        let image = sample_image();
        let iso = IsoImage::open(&image).expect("open");
        assert_eq!(iso.file("boot/README.TXT").expect("read"), README);
    }

    #[test]
    fn a_multi_extent_file_reads_every_extent_in_order() {
        let image = sample_image();
        let iso = IsoImage::open(&image).expect("open");
        let split = iso.file("boot/split.bin").expect("read");
        assert_eq!(split.len(), 2048 + SPLIT_TAIL.len());
        assert!(split[..2048].iter().all(|&b| b == b'x'));
        assert_eq!(&split[2048..], SPLIT_TAIL);
    }

    #[test]
    fn missing_files_and_directories_are_refused() {
        let image = sample_image();
        let iso = IsoImage::open(&image).expect("open");
        assert!(iso.file("boot/vmlinuz-lts").is_err());
        assert!(iso.file("boot").is_err());
        assert!(iso.file("boot/README.TXT/x").is_err());
        // The identifier is not the name when an NM gives one.
        assert!(iso.file("BOOT/VMLINUZ_.").is_err());
    }

    #[test]
    fn an_image_without_a_primary_descriptor_is_refused() {
        let mut image = sample_image();
        image[16 * BLOCK] = 2;
        assert!(IsoImage::open(&image).is_err());
        assert!(IsoImage::open(&image[..10 * BLOCK]).is_err());
    }

    #[test]
    fn identifiers_lose_their_version_and_empty_extension() {
        assert_eq!(iso_name(b"README.TXT;1").expect("name"), "README.TXT");
        assert_eq!(iso_name(b"MAKEFILE.;1").expect("name"), "MAKEFILE");
        assert_eq!(iso_name(b"BOOT").expect("name"), "BOOT");
    }
}
