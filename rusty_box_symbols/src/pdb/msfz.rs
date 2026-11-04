//! MSFZ ("MSFZ Container Specification"; `ms-pdb-msfz` 0.1.10 `reader.rs`):
//! an 80-byte header, a chunk table (each chunk compressed with zstd or
//! raw DEFLATE, or stored), and a stream directory listing each stream's
//! fragments. A fragment is a slice of one decompressed chunk, or bytes
//! stored in the file at a 48-bit offset. Chunks are decompressed once and
//! kept.

use super::{reserved, short, PdbError};
use crate::decompress::{decompress_to_vec, Codec};
use crate::read::{Bytes, Cursor};

/// `MSFZ_FILE_VERSION_V0`.
const VERSION: u64 = 0;
const COMPRESSION_NONE: u32 = 0;
const COMPRESSION_ZSTD: u32 = 1;
const COMPRESSION_DEFLATE: u32 = 2;
const NIL_STREAM: u32 = 0xFFFF_FFFF;
const CHUNK_ENTRY_BYTES: usize = 20;
/// Bit 31 of a fragment location's high word marks a compressed fragment.
const COMPRESSED_FRAGMENT: u32 = 1 << 31;
/// A stored fragment's file offset has 48 bits.
const STORED_OFFSET_MASK: u64 = (1 << 48) - 1;
const MAX_STREAMS: u32 = 1 << 16;
/// ms-pdb writes chunks of a few MiB; past this a chunk table is not real.
const MAX_CHUNKS: u32 = 1 << 20;
/// The largest chunk or directory decompressed.
const MAX_CHUNK_BYTES: u64 = 1 << 28;
/// The largest stream assembled.
const MAX_STREAM_BYTES: u64 = 1 << 31;

/// One chunk-table entry.
#[derive(Clone, Copy)]
struct Chunk {
    file_offset: u64,
    compression: u32,
    compressed: u32,
    uncompressed: u32,
}

/// One fragment of a stream.
#[derive(Clone, Copy)]
enum Fragment {
    InChunk { size: u32, chunk: u32, offset: u32 },
    Stored { size: u32, file_offset: u64 },
}

/// A stream's fragments, or a nil stream.
enum Stream {
    Nil,
    Fragments(Vec<Fragment>),
}

/// Where a fragment's bytes live.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Space {
    Chunk(u32),
    File,
}

/// One fragment's bytes, for the overlap check.
struct Span {
    space: Space,
    start: u64,
    end: u64,
    stream: u32,
    fragment: u32,
}

impl Span {
    /// The bytes `fragment`, number `index` of `stream`, names.
    fn of(fragment: Fragment, stream: u32, index: u32) -> Span {
        match fragment {
            Fragment::InChunk { size, chunk, offset } => Span {
                space: Space::Chunk(chunk),
                start: u64::from(offset),
                end: u64::from(offset).saturating_add(u64::from(size)),
                stream,
                fragment: index,
            },
            Fragment::Stored { size, file_offset } => Span {
                space: Space::File,
                start: file_offset,
                end: file_offset.saturating_add(u64::from(size)),
                stream,
                fragment: index,
            },
        }
    }
}

/// Refuses two fragments that share bytes. MSFZ gives a byte one owner;
/// honoured, one range named N times reads as N ranges, and a small file
/// expands without bound.
fn disjoint(spans: &mut [Span]) -> Result<(), PdbError> {
    spans.sort_by(|a, b| a.space.cmp(&b.space).then(a.start.cmp(&b.start)));
    // Sorted by start, a span overlapping any earlier one in its space
    // overlaps the one just before it.
    for pair in spans.windows(2) {
        if let [before, after] = pair {
            if after.space == before.space && after.start < before.end {
                return Err(PdbError::OverlappingFragments { stream: after.stream, fragment: after.fragment });
            }
        }
    }
    Ok(())
}

/// An MSFZ file's streams.
pub struct MsfzStreams<'a> {
    bytes: Bytes<'a>,
    chunks: Vec<Chunk>,
    decompressed: Vec<Option<Vec<u8>>>,
    streams: Vec<Stream>,
}

/// The codec of an MSFZ compression code.
fn codec(code: u32) -> Result<Option<Codec>, PdbError> {
    match code {
        COMPRESSION_NONE => Ok(None),
        COMPRESSION_ZSTD => Ok(Some(Codec::Zstd)),
        COMPRESSION_DEFLATE => Ok(Some(Codec::RawDeflate)),
        other => Err(PdbError::UnknownCompression { code: other }),
    }
}

/// `len` bytes of `file` at `at`, decompressed by `code` to `size` bytes.
fn unpack(file: Bytes<'_>, at: u64, len: u32, code: u32, size: u32) -> Result<Vec<u8>, PdbError> {
    let at = usize::try_from(at).map_err(|_| PdbError::TooMany { what: "file offset", count: at })?;
    let raw = file.slice(at, len as usize).map_err(|source| short("MSFZ chunk", source))?;
    match codec(code)? {
        Some(codec) => Ok(decompress_to_vec(codec, raw, u64::from(size), MAX_CHUNK_BYTES)?),
        None => {
            if len != size {
                return Err(PdbError::StoredChunkSize { stored: len, declared: size });
            }
            let mut out = reserved(u64::from(len), "MSFZ chunk")?;
            out.extend_from_slice(raw);
            Ok(out)
        }
    }
}

impl<'a> MsfzStreams<'a> {
    pub(super) fn open(file: &'a [u8]) -> Result<MsfzStreams<'a>, PdbError> {
        let bytes = Bytes::new(file);
        let what = "MSFZ header";
        let mut header = Cursor::new(bytes, 32);
        let version = header.u64_le().map_err(|source| short(what, source))?;
        if version != VERSION {
            return Err(PdbError::BadVersion { version });
        }
        let directory_offset = header.u64_le().map_err(|source| short(what, source))?;
        let chunk_table_offset = header.u64_le().map_err(|source| short(what, source))?;
        let stream_count = header.u32_le().map_err(|source| short(what, source))?;
        let directory_compression = header.u32_le().map_err(|source| short(what, source))?;
        let directory_compressed = header.u32_le().map_err(|source| short(what, source))?;
        let directory_size = header.u32_le().map_err(|source| short(what, source))?;
        let chunk_count = header.u32_le().map_err(|source| short(what, source))?;
        let chunk_table_size = header.u32_le().map_err(|source| short(what, source))?;
        if stream_count > MAX_STREAMS || stream_count == 0 {
            return Err(PdbError::TooMany { what: "streams", count: u64::from(stream_count) });
        }
        let table_bytes = u64::from(chunk_count).saturating_mul(CHUNK_ENTRY_BYTES as u64);
        if chunk_count > MAX_CHUNKS || u64::from(chunk_table_size) != table_bytes {
            return Err(PdbError::TooMany { what: "chunks", count: u64::from(chunk_count) });
        }
        let table_at = usize::try_from(chunk_table_offset)
            .map_err(|_| PdbError::TooMany { what: "chunk table offset", count: chunk_table_offset })?;
        let table = bytes
            .sub(table_at, chunk_table_size as usize)
            .map_err(|source| short("MSFZ chunk table", source))?;
        let mut chunks = Vec::new();
        chunks
            .try_reserve_exact(chunk_count as usize)
            .map_err(|_| PdbError::Allocation { what: "chunks", bytes: u64::from(chunk_count) })?;
        let mut table_cursor = Cursor::new(table, 0);
        for _ in 0..chunk_count {
            let file_offset = table_cursor.u64_le().map_err(|source| short("MSFZ chunk table", source))?;
            let compression = table_cursor.u32_le().map_err(|source| short("MSFZ chunk table", source))?;
            let compressed = table_cursor.u32_le().map_err(|source| short("MSFZ chunk table", source))?;
            let uncompressed = table_cursor.u32_le().map_err(|source| short("MSFZ chunk table", source))?;
            chunks.push(Chunk { file_offset, compression, compressed, uncompressed });
        }
        let directory = unpack(bytes, directory_offset, directory_compressed, directory_compression, directory_size)?;
        let streams = Self::directory(&directory, stream_count, &chunks)?;
        let mut decompressed = Vec::new();
        decompressed
            .try_reserve_exact(chunks.len())
            .map_err(|_| PdbError::Allocation { what: "chunk cache", bytes: u64::from(chunk_count) })?;
        decompressed.resize_with(chunks.len(), || None);
        Ok(MsfzStreams { bytes, chunks, decompressed, streams })
    }

    /// The directory: per stream, fragment sizes and locations until a
    /// size of 0; a first size of 0xFFFFFFFF is a nil stream. Each fragment
    /// takes 12 directory bytes, so the walk ends with the directory. An
    /// in-chunk fragment must end inside its chunk, and no two fragments
    /// may share bytes.
    fn directory(directory: &[u8], count: u32, chunks: &[Chunk]) -> Result<Vec<Stream>, PdbError> {
        let mut cursor = Cursor::new(Bytes::new(directory), 0);
        let what = "MSFZ stream directory";
        let mut spans: Vec<Span> = Vec::new();
        let mut streams = Vec::new();
        streams
            .try_reserve_exact(count as usize)
            .map_err(|_| PdbError::Allocation { what: "streams", bytes: u64::from(count) })?;
        for stream in 0..count {
            let mut size = cursor.u32_le().map_err(|source| short(what, source))?;
            if size == NIL_STREAM {
                streams.push(Stream::Nil);
                continue;
            }
            let mut fragments = Vec::new();
            let mut index = 0u32;
            while size != 0 {
                if size == NIL_STREAM {
                    return Err(PdbError::BadFragment { stream, fragment: index });
                }
                let low = cursor.u32_le().map_err(|source| short(what, source))?;
                let high = cursor.u32_le().map_err(|source| short(what, source))?;
                let fragment = if high & COMPRESSED_FRAGMENT != 0 {
                    let chunk = high & !COMPRESSED_FRAGMENT;
                    let entry = chunks.get(chunk as usize).ok_or(PdbError::BadFragment { stream, fragment: index })?;
                    if u64::from(low).saturating_add(u64::from(size)) > u64::from(entry.uncompressed) {
                        return Err(PdbError::BadFragment { stream, fragment: index });
                    }
                    Fragment::InChunk { size, chunk, offset: low }
                } else {
                    let file_offset = (u64::from(high).wrapping_shl(32) | u64::from(low)) & STORED_OFFSET_MASK;
                    Fragment::Stored { size, file_offset }
                };
                fragments
                    .try_reserve(1)
                    .map_err(|_| PdbError::Allocation { what: "fragments", bytes: u64::from(index) })?;
                fragments.push(fragment);
                spans.try_reserve(1).map_err(|_| PdbError::Allocation { what: "fragment spans", bytes: u64::from(index) })?;
                spans.push(Span::of(fragment, stream, index));
                index = index.wrapping_add(1);
                size = cursor.u32_le().map_err(|source| short(what, source))?;
            }
            streams.push(Stream::Fragments(fragments));
        }
        disjoint(&mut spans)?;
        Ok(streams)
    }

    pub(super) fn count(&self) -> u32 {
        u32::try_from(self.streams.len()).unwrap_or(u32::MAX)
    }

    /// Chunk `chunk`, decompressed (once).
    fn chunk(&mut self, chunk: u32) -> Result<&[u8], PdbError> {
        let entry = *self.chunks.get(chunk as usize).ok_or(PdbError::NoStream { index: chunk })?;
        let slot = self.decompressed.get_mut(chunk as usize).ok_or(PdbError::NoStream { index: chunk })?;
        if slot.is_none() {
            *slot = Some(unpack(self.bytes, entry.file_offset, entry.compressed, entry.compression, entry.uncompressed)?);
        }
        Ok(slot.as_deref().unwrap_or(&[]))
    }

    pub(super) fn read(&mut self, index: u32) -> Result<Option<Vec<u8>>, PdbError> {
        let fragments = match self.streams.get(index as usize).ok_or(PdbError::NoStream { index })? {
            Stream::Nil => return Ok(None),
            Stream::Fragments(fragments) => fragments.clone(),
        };
        let total = fragments.iter().fold(0u64, |total, fragment| {
            total.saturating_add(u64::from(match fragment {
                Fragment::InChunk { size, .. } | Fragment::Stored { size, .. } => *size,
            }))
        });
        if total > MAX_STREAM_BYTES {
            return Err(PdbError::TooMany { what: "stream bytes", count: total });
        }
        let mut out = reserved(total, "stream")?;
        for (number, fragment) in fragments.iter().enumerate() {
            let bad = PdbError::BadFragment { stream: index, fragment: u32::try_from(number).unwrap_or(u32::MAX) };
            match *fragment {
                Fragment::InChunk { size, chunk, offset } => {
                    let data = self.chunk(chunk)?;
                    let end = (offset as usize).checked_add(size as usize).ok_or(bad)?;
                    out.extend_from_slice(data.get(offset as usize..end).ok_or(bad)?);
                }
                Fragment::Stored { size, file_offset } => {
                    let at = usize::try_from(file_offset).map_err(|_| bad)?;
                    out.extend_from_slice(self.bytes.slice(at, size as usize).map_err(|_| bad)?);
                }
            }
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdb::MSFZ_MAGIC;

    /// Where `tiny_msfz` puts its 32 payload bytes; they are also chunk 0,
    /// stored uncompressed.
    const PAYLOAD_AT: u32 = 80;
    const PAYLOAD_LEN: u32 = 32;

    /// One fragment as the directory encodes it.
    struct TestFragment {
        size: u32,
        low: u32,
        high: u32,
    }

    fn in_chunk(size: u32, offset: u32) -> TestFragment {
        TestFragment { size, low: offset, high: COMPRESSED_FRAGMENT }
    }

    fn stored(size: u32, file_offset: u32) -> TestFragment {
        TestFragment { size, low: file_offset, high: 0 }
    }

    fn payload() -> Vec<u8> {
        (0..PAYLOAD_LEN as u8).collect()
    }

    /// An MSFZ file with one stored chunk (the payload) and an uncompressed
    /// directory listing `streams`.
    fn tiny_msfz(streams: &[&[TestFragment]]) -> Vec<u8> {
        let mut directory = Vec::new();
        for fragments in streams {
            for fragment in *fragments {
                directory.extend_from_slice(&fragment.size.to_le_bytes());
                directory.extend_from_slice(&fragment.low.to_le_bytes());
                directory.extend_from_slice(&fragment.high.to_le_bytes());
            }
            directory.extend_from_slice(&0u32.to_le_bytes());
        }
        let table_at = PAYLOAD_AT + PAYLOAD_LEN;
        let directory_at = table_at + CHUNK_ENTRY_BYTES as u32;
        let mut file = MSFZ_MAGIC.to_vec();
        file.extend_from_slice(&VERSION.to_le_bytes());
        file.extend_from_slice(&u64::from(directory_at).to_le_bytes());
        file.extend_from_slice(&u64::from(table_at).to_le_bytes());
        file.extend_from_slice(&(streams.len() as u32).to_le_bytes());
        file.extend_from_slice(&COMPRESSION_NONE.to_le_bytes());
        file.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        file.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        file.extend_from_slice(&1u32.to_le_bytes());
        file.extend_from_slice(&(CHUNK_ENTRY_BYTES as u32).to_le_bytes());
        assert_eq!(file.len(), PAYLOAD_AT as usize);
        file.extend_from_slice(&payload());
        file.extend_from_slice(&u64::from(PAYLOAD_AT).to_le_bytes());
        file.extend_from_slice(&COMPRESSION_NONE.to_le_bytes());
        file.extend_from_slice(&PAYLOAD_LEN.to_le_bytes());
        file.extend_from_slice(&PAYLOAD_LEN.to_le_bytes());
        file.extend_from_slice(&directory);
        file
    }

    #[test]
    fn disjoint_fragments_read_back() {
        let file = tiny_msfz(&[&[in_chunk(8, 0), in_chunk(8, 8)], &[stored(4, PAYLOAD_AT + 16)]]);
        let mut msfz = MsfzStreams::open(&file).unwrap();
        assert_eq!(msfz.read(0).unwrap(), Some(payload()[..16].to_vec()));
        assert_eq!(msfz.read(1).unwrap(), Some(payload()[16..20].to_vec()));
    }

    #[test]
    fn fragments_reusing_chunk_bytes_are_refused() {
        // Honoured, one chunk range named N times reads as N ranges: a
        // small file expands without bound.
        let file = tiny_msfz(&[&[in_chunk(16, 0), in_chunk(16, 8)]]);
        assert_eq!(MsfzStreams::open(&file).err(), Some(PdbError::OverlappingFragments { stream: 0, fragment: 1 }));
    }

    #[test]
    fn two_streams_sharing_chunk_bytes_are_refused() {
        let file = tiny_msfz(&[&[in_chunk(8, 0)], &[in_chunk(8, 4)]]);
        assert_eq!(MsfzStreams::open(&file).err(), Some(PdbError::OverlappingFragments { stream: 1, fragment: 0 }));
    }

    #[test]
    fn stored_fragments_reusing_file_bytes_are_refused() {
        let file = tiny_msfz(&[&[stored(8, PAYLOAD_AT), stored(8, PAYLOAD_AT + 4)]]);
        assert_eq!(MsfzStreams::open(&file).err(), Some(PdbError::OverlappingFragments { stream: 0, fragment: 1 }));
    }

    #[test]
    fn a_fragment_running_past_its_chunk_is_refused_at_open() {
        let file = tiny_msfz(&[&[in_chunk(16, 24)]]);
        assert_eq!(MsfzStreams::open(&file).err(), Some(PdbError::BadFragment { stream: 0, fragment: 0 }));
    }
}
