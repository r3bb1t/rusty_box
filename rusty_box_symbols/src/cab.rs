//! Cabinets as symbol servers serve them (`.pd_`, MS-CAB): one folder, one
//! file, stored, MSZIP or LZX. A cabinet that spans files, holds several,
//! or uses Quantum is refused by name. The header, folder, file and data
//! records are decoded by tier-P functions; [`extract_single`] (tier A)
//! checks every declared size against the caller's cap before allocating
//! the folder's output.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use crate::decompress::inflate::{inflate, InflateError};
use crate::decompress::lzx::{self, LzxError};
use crate::read::{Bytes, Cursor, ReadError};

/// `MSCF`.
pub const CAB_MAGIC: [u8; 4] = *b"MSCF";
/// MS-CAB §2.1: `cfhdrPREV_CABINET | cfhdrNEXT_CABINET`.
const SPANNING_FLAGS: u16 = 0x0003;
/// MS-CAB §2.1: `cfhdrRESERVE_PRESENT`.
const RESERVE_FLAG: u16 = 0x0004;
/// MS-CAB §2.4: a data block holds at most 32 KiB of output.
const MAX_BLOCK_OUTPUT: u16 = 32_768;
/// A file name longer than this is not a symbol file's (MS-CAB caps it at
/// 256 bytes with the NUL).
const MAX_NAME_BYTES: usize = 256;

/// How the folder is compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CabCompression {
    Stored,
    MsZip,
    Lzx { window_bits: u32 },
}

/// The one file a symbol cabinet holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extracted {
    pub name: String,
    pub compression: CabCompression,
    pub bytes: Vec<u8>,
}

/// Why a cabinet did not give its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CabError {
    #[error("{found:02x?} is not a cabinet's signature")]
    BadMagic { found: [u8; 4] },
    #[error("the cabinet is cut short in its {what}: {source}")]
    Truncated {
        what: &'static str,
        #[source]
        source: ReadError,
    },
    #[error("the cabinet holds {what}, which a symbol file does not")]
    Unsupported { what: &'static str },
    #[error("compression type {code:#06x} is not stored, MSZIP or LZX")]
    UnknownCompression { code: u16 },
    #[error("data block {block}'s checksum {found:#010x} is not its contents' {computed:#010x}")]
    Checksum { block: usize, found: u32, computed: u32 },
    #[error("the folder declares {declared} bytes, over the cap of {cap}")]
    TooLarge { declared: u64, cap: u64 },
    #[error("data block {block} declares {declared} bytes of output and gives {produced}")]
    WrongSize { block: usize, declared: u16, produced: usize },
    #[error("data block {block} has no MSZIP signature")]
    NoMsZipSignature { block: usize },
    #[error("the file lies outside its folder")]
    FileOutsideFolder,
    #[error("no memory for {bytes} bytes")]
    Allocation { bytes: u64 },
    #[error("an MSZIP block does not decode: {0}")]
    Inflate(#[from] InflateError),
    #[error("the LZX stream does not decode: {0}")]
    Lzx(#[from] LzxError),
}

/// The fields of `CFHEADER` a single-file cabinet needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CabHeader {
    pub files_offset: u32,
    pub folders: u16,
    pub files: u16,
    pub flags: u16,
    /// Where the first `CFFOLDER` starts.
    pub folders_at: usize,
    /// The reserve bytes after each `CFFOLDER` and each `CFDATA` header.
    pub folder_reserve: u8,
    pub data_reserve: u8,
}

/// `CFFOLDER`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FolderEntry {
    data_offset: u32,
    data_blocks: u16,
    compression: u16,
}

/// One `CFDATA`: its checksum, sizes and data.
struct DataBlock<'a> {
    checksum: u32,
    compressed: u16,
    uncompressed: u16,
    data: &'a [u8],
    reserve: &'a [u8],
    next: usize,
}

/// `Truncated` for the named part.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn cut(what: &'static str, source: ReadError) -> CabError {
    CabError::Truncated { what, source }
}

/// MS-CAB's checksum (libmspack `cabd_checksum`): little-endian words XORed
/// into `seed`, then the 1 to 3 bytes left, first byte highest.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
#[must_use]
pub fn checksum(bytes: &[u8], seed: u32) -> u32 {
    let mut sum = seed;
    let mut words = bytes.chunks_exact(4);
    for word in words.by_ref() {
        // Every chunk is exactly four bytes, so the read cannot fail.
        sum ^= Bytes::new(word).u32_le(0).unwrap_or(0);
    }
    let mut tail = 0u32;
    for byte in words.remainder() {
        tail = tail.wrapping_shl(8) | u32::from(*byte);
    }
    sum ^ tail
}

/// [`checksum`] of `first` followed by `second` as one byte stream, without
/// joining them in memory.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn checksum_stream(first: &[u8], second: &[u8]) -> u32 {
    let mut sum = 0u32;
    let mut word = [0u8; 4];
    let mut filled = 0usize;
    for byte in first.iter().chain(second) {
        if let Some(slot) = word.get_mut(filled) {
            *slot = *byte;
        }
        filled = filled.wrapping_add(1);
        if filled == 4 {
            sum ^= u32::from_le_bytes(word);
            filled = 0;
        }
    }
    let mut tail = 0u32;
    for byte in word.iter().take(filled) {
        tail = tail.wrapping_shl(8) | u32::from(*byte);
    }
    sum ^ tail
}

/// `CFHEADER` (MS-CAB §2.1), with the reserve sizes when present.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_header(cabinet: Bytes<'_>) -> Result<CabHeader, CabError> {
    let magic = cabinet.array::<4>(0).map_err(|source| cut("header", source))?;
    if magic != CAB_MAGIC {
        return Err(CabError::BadMagic { found: magic });
    }
    let files_offset = cabinet.u32_le(16).map_err(|source| cut("header", source))?;
    let folders = cabinet.u16_le(26).map_err(|source| cut("header", source))?;
    let files = cabinet.u16_le(28).map_err(|source| cut("header", source))?;
    let flags = cabinet.u16_le(30).map_err(|source| cut("header", source))?;
    let mut folders_at = 36usize;
    let mut folder_reserve = 0u8;
    let mut data_reserve = 0u8;
    if flags & RESERVE_FLAG != 0 {
        let header_reserve = cabinet.u16_le(36).map_err(|source| cut("header", source))?;
        folder_reserve = cabinet.u8(38).map_err(|source| cut("header", source))?;
        data_reserve = cabinet.u8(39).map_err(|source| cut("header", source))?;
        folders_at = 40usize.wrapping_add(usize::from(header_reserve));
    }
    Ok(CabHeader {
        files_offset,
        folders,
        files,
        flags,
        folders_at,
        folder_reserve,
        data_reserve,
    })
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn decode_folder(cabinet: Bytes<'_>, at: usize) -> Result<FolderEntry, CabError> {
    let mut cursor = Cursor::new(cabinet, at);
    Ok(FolderEntry {
        data_offset: cursor.u32_le().map_err(|source| cut("folder", source))?,
        data_blocks: cursor.u16_le().map_err(|source| cut("folder", source))?,
        compression: cursor.u16_le().map_err(|source| cut("folder", source))?,
    })
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn decode_data_block(cabinet: Bytes<'_>, at: usize, reserve: u8) -> Result<DataBlock<'_>, CabError> {
    let mut cursor = Cursor::new(cabinet, at);
    let checksum = cursor.u32_le().map_err(|source| cut("data block", source))?;
    let compressed = cursor.u16_le().map_err(|source| cut("data block", source))?;
    let uncompressed = cursor.u16_le().map_err(|source| cut("data block", source))?;
    let reserve = cursor.bytes(usize::from(reserve)).map_err(|source| cut("data block", source))?;
    let data = cursor.bytes(usize::from(compressed)).map_err(|source| cut("data block", source))?;
    Ok(DataBlock {
        checksum,
        compressed,
        uncompressed,
        data,
        reserve,
        next: cursor.position(),
    })
}

/// Whether `bytes` start like a cabinet.
#[must_use]
pub fn is_cabinet(bytes: &[u8]) -> bool {
    bytes.starts_with(&CAB_MAGIC)
}

/// The one file of a symbol server's cabinet. Every size is checked against
/// `cap` before the folder's output is allocated.
pub fn extract_single(bytes: &[u8], cap: u64) -> Result<Extracted, CabError> {
    let cabinet = Bytes::new(bytes);
    let header = decode_header(cabinet)?;
    if header.flags & SPANNING_FLAGS != 0 {
        return Err(CabError::Unsupported { what: "a set spanning several cabinets" });
    }
    if header.folders != 1 {
        return Err(CabError::Unsupported { what: "more than one folder" });
    }
    if header.files != 1 {
        return Err(CabError::Unsupported { what: "more than one file" });
    }
    let folder = decode_folder(cabinet, header.folders_at)?;
    let compression = match folder.compression & 0x000F {
        0 => CabCompression::Stored,
        1 => CabCompression::MsZip,
        2 => return Err(CabError::Unsupported { what: "Quantum compression" }),
        3 => CabCompression::Lzx {
            window_bits: u32::from(folder.compression.wrapping_shr(8) & 0x1F),
        },
        _ => return Err(CabError::UnknownCompression { code: folder.compression }),
    };
    let mut file = Cursor::new(cabinet, header.files_offset as usize);
    let file_size = file.u32_le().map_err(|source| cut("file entry", source))?;
    let file_offset = file.u32_le().map_err(|source| cut("file entry", source))?;
    file.skip(8).map_err(|source| cut("file entry", source))?;
    let name = file.cstr(MAX_NAME_BYTES).map_err(|source| cut("file name", source))?;
    let name = String::from_utf8_lossy(name).into_owned();

    // First pass: every block's header, checksum and declared sizes.
    let mut total: u64 = 0;
    let mut compressed_total: u64 = 0;
    let mut at = folder.data_offset as usize;
    for block in 0..usize::from(folder.data_blocks) {
        let data = decode_data_block(cabinet, at, header.data_reserve)?;
        if data.uncompressed > MAX_BLOCK_OUTPUT {
            return Err(CabError::WrongSize { block, declared: data.uncompressed, produced: 0 });
        }
        if data.checksum != 0 {
            // The `cab` crate's rule (`folder.rs`): the reserve bytes, then
            // the data, as one stream, XORed with the two sizes as one word.
            // Symbol servers' cabinets carry no reserve.
            let sizes = u32::from(data.compressed) | u32::from(data.uncompressed).wrapping_shl(16);
            let computed = checksum_stream(data.reserve, data.data) ^ sizes;
            if computed != data.checksum {
                return Err(CabError::Checksum { block, found: data.checksum, computed });
            }
        }
        total = total.saturating_add(u64::from(data.uncompressed));
        compressed_total = compressed_total.saturating_add(u64::from(data.compressed));
        at = data.next;
    }
    if total > cap {
        return Err(CabError::TooLarge { declared: total, cap });
    }
    let file_end = u64::from(file_offset).saturating_add(u64::from(file_size));
    if file_end > total {
        return Err(CabError::FileOutsideFolder);
    }
    let len = usize::try_from(total).map_err(|_| CabError::Allocation { bytes: total })?;
    let mut out = Vec::new();
    out.try_reserve_exact(len).map_err(|_| CabError::Allocation { bytes: total })?;
    out.resize(len, 0);

    // Second pass: decode.
    match compression {
        CabCompression::Stored | CabCompression::MsZip => {
            let mut at = folder.data_offset as usize;
            let mut produced = 0usize;
            for block in 0..usize::from(folder.data_blocks) {
                let data = decode_data_block(cabinet, at, header.data_reserve)?;
                let declared = usize::from(data.uncompressed);
                let end = produced.checked_add(declared).ok_or(CabError::FileOutsideFolder)?;
                if compression == CabCompression::Stored {
                    let target = out.get_mut(produced..end).ok_or(CabError::FileOutsideFolder)?;
                    if data.data.len() != declared {
                        return Err(CabError::WrongSize { block, declared: data.uncompressed, produced: data.data.len() });
                    }
                    for (to, from) in target.iter_mut().zip(data.data) {
                        *to = *from;
                    }
                } else {
                    let body = data.data.strip_prefix(b"CK").ok_or(CabError::NoMsZipSignature { block })?;
                    let window = out.get_mut(..end).ok_or(CabError::FileOutsideFolder)?;
                    let done = inflate(Bytes::new(body), window, produced)?;
                    if done.end != end {
                        return Err(CabError::WrongSize { block, declared: data.uncompressed, produced: done.end.wrapping_sub(produced) });
                    }
                }
                produced = end;
                at = data.next;
            }
        }
        CabCompression::Lzx { window_bits } => {
            let concatenated_len = usize::try_from(compressed_total).map_err(|_| CabError::Allocation { bytes: compressed_total })?;
            let mut concatenated = Vec::new();
            concatenated
                .try_reserve_exact(concatenated_len)
                .map_err(|_| CabError::Allocation { bytes: compressed_total })?;
            let mut at = folder.data_offset as usize;
            for _ in 0..folder.data_blocks {
                let data = decode_data_block(cabinet, at, header.data_reserve)?;
                concatenated.extend_from_slice(data.data);
                at = data.next;
            }
            lzx::decompress(Bytes::new(&concatenated), &mut out, window_bits)?;
        }
    }
    let start = file_offset as usize;
    let end = usize::try_from(file_end).map_err(|_| CabError::FileOutsideFolder)?;
    let bytes = if start == 0 && end == out.len() {
        out
    } else {
        out.get(start..end).ok_or(CabError::FileOutsideFolder)?.to_vec()
    };
    Ok(Extracted { name, compression, bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_symbols_fixtures::cab::{cabinet, lzx_input, two_file_cabinet, CabinetKind, LZX15_CABINET, LZX21_CABINET};
    use rusty_box_symbols_fixtures::compress::sample_text;

    /// MS-CAB's checksum, against the values the `cab` crate's tests take
    /// from the specification's example cabinet.
    #[test]
    fn the_checksum_matches_the_specifications_examples() {
        assert_eq!(checksum(b"\x0e\0\x0e\0Hello, world!\n", 0), 0x7F2E_1A4C);
        assert_eq!(checksum(b"\x1d\0\x1d\0Hello, world!\nSee you later!\n", 0), 0x3509_541A);
        assert_eq!(checksum(b"", 0), 0);
    }

    #[test]
    fn stored_and_mszip_cabinets_open() {
        let contents = sample_text(100_000);
        for kind in [CabinetKind::Stored, CabinetKind::MsZip] {
            let bytes = cabinet("ntkrnlmp.pdb", &contents, kind);
            assert!(is_cabinet(&bytes));
            let extracted = extract_single(&bytes, 1 << 30).unwrap();
            assert_eq!(extracted.name, "ntkrnlmp.pdb", "{kind:?}");
            assert_eq!(extracted.bytes, contents, "{kind:?}");
        }
    }

    /// A committed cabinet and the window it was made with.
    struct LzxCabinet {
        bytes: &'static [u8],
        window_bits: u32,
    }

    /// The committed `makecab` cabinets: LZX at both ends of the window range.
    #[test]
    fn lzx_cabinets_from_makecab_open() {
        let expected = lzx_input();
        let cases = [
            LzxCabinet { bytes: LZX15_CABINET, window_bits: 15 },
            LzxCabinet { bytes: LZX21_CABINET, window_bits: 21 },
        ];
        for case in cases {
            let extracted = extract_single(case.bytes, 1 << 30).unwrap();
            assert_eq!(extracted.compression, CabCompression::Lzx { window_bits: case.window_bits });
            assert_eq!(extracted.name, "lzx-input.bin");
            assert!(extracted.bytes == expected, "window {}: the output differs", case.window_bits);
        }
    }

    #[test]
    fn a_damaged_data_block_fails_its_checksum() {
        let mut bytes = cabinet("x.pdb", &sample_text(1000), CabinetKind::MsZip);
        let last = bytes.len() - 1;
        bytes[last] ^= 0x55;
        assert!(matches!(extract_single(&bytes, 1 << 30), Err(CabError::Checksum { block: 0, .. })));
    }

    #[test]
    fn a_cabinet_of_several_files_is_not_a_symbol_file() {
        assert_eq!(
            extract_single(&two_file_cabinet(), 1 << 30).map(|extracted| extracted.name),
            Err(CabError::Unsupported { what: "more than one file" })
        );
    }

    #[test]
    fn a_folder_larger_than_the_cap_is_refused_before_it_is_decoded() {
        let bytes = cabinet("x.pdb", &sample_text(100_000), CabinetKind::MsZip);
        assert_eq!(
            extract_single(&bytes, 50_000).map(|extracted| extracted.name),
            Err(CabError::TooLarge { declared: 100_000, cap: 50_000 })
        );
    }

    #[test]
    fn bytes_that_are_not_a_cabinet_are_refused() {
        assert!(!is_cabinet(b"Microsoft C/C++ MSF 7.00\r\n"));
        assert_eq!(
            extract_single(b"MSCX\0\0\0\0", 1 << 30).map(|extracted| extracted.name),
            Err(CabError::BadMagic { found: *b"MSCX" })
        );
        let bytes = cabinet("x.pdb", b"short", CabinetKind::Stored);
        assert!(matches!(extract_single(&bytes[..40], 1 << 30), Err(CabError::Truncated { .. })));
    }
}
