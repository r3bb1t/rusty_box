//! MSF 7.00 (LLVM, "The MSF File Format"): a superblock, then fixed-size
//! blocks. The superblock names the block holding the directory's block
//! list; the directory holds the stream count, each stream's size (0xFFFFFFFF
//! for a nil stream) and each stream's blocks.

use super::{reserved, short, PdbError};
use crate::read::{Bytes, Cursor};

/// The block sizes MSF 7.00 files use.
const BLOCK_SIZES: [u32; 4] = [512, 1024, 2048, 4096];
/// A nil stream's size.
const NIL_STREAM: u32 = 0xFFFF_FFFF;
/// Stream indices are 16-bit wherever a PDB names one.
const MAX_STREAMS: u32 = 1 << 16;
/// The largest directory read: 64 MiB lists over 16 million blocks.
const MAX_DIRECTORY_BYTES: u32 = 1 << 26;

/// One stream's size and blocks.
struct Stream {
    size: Option<u32>,
    blocks: Vec<u32>,
}

/// An MSF file's streams.
pub struct MsfStreams<'a> {
    bytes: Bytes<'a>,
    block_size: u32,
    block_count: u32,
    streams: Vec<Stream>,
}

/// `size` divided by `block`, rounded up (`block` is never 0).
fn blocks_for(size: u32, block: u32) -> u32 {
    size.div_ceil(block.max(1))
}

/// Which of a file's blocks have an owner. MSF gives a block one owner —
/// the block map, a directory block, or one block of one stream — so a
/// block named twice is refused: honoured, one block named N times reads
/// as N blocks, and a small file expands without bound.
struct BlockOwners {
    words: Vec<u64>,
    /// Whole blocks in the file.
    blocks: u32,
}

impl BlockOwners {
    fn new(file_len: usize, block_size: u32) -> Result<BlockOwners, PdbError> {
        let blocks = u32::try_from(file_len.checked_div(block_size as usize).unwrap_or(0)).unwrap_or(u32::MAX);
        let word_count = blocks.div_ceil(64) as usize;
        let mut words = Vec::new();
        words.try_reserve_exact(word_count).map_err(|_| PdbError::Allocation {
            what: "block owners",
            bytes: (word_count as u64).saturating_mul(8),
        })?;
        words.resize(word_count, 0);
        Ok(BlockOwners { words, blocks })
    }

    /// Records `block`'s owner; a block past the file or already owned is
    /// refused.
    fn claim(&mut self, block: u32) -> Result<(), PdbError> {
        let outside = PdbError::BadBlock { block, count: self.blocks };
        if block >= self.blocks {
            return Err(outside);
        }
        let word = self.words.get_mut((block / 64) as usize).ok_or(outside)?;
        let bit = 1u64 << (block % 64);
        if *word & bit != 0 {
            return Err(PdbError::SharedBlock { block });
        }
        *word |= bit;
        Ok(())
    }
}

impl<'a> MsfStreams<'a> {
    pub(super) fn open(file: &'a [u8]) -> Result<MsfStreams<'a>, PdbError> {
        let bytes = Bytes::new(file);
        let what = "MSF superblock";
        let block_size = bytes.u32_le(32).map_err(|source| short(what, source))?;
        if !BLOCK_SIZES.contains(&block_size) {
            return Err(PdbError::BadBlockSize { size: block_size });
        }
        let block_count = bytes.u32_le(40).map_err(|source| short(what, source))?;
        let directory_bytes = bytes.u32_le(44).map_err(|source| short(what, source))?;
        let block_map = bytes.u32_le(52).map_err(|source| short(what, source))?;
        if directory_bytes > MAX_DIRECTORY_BYTES {
            return Err(PdbError::TooMany { what: "directory bytes", count: u64::from(directory_bytes) });
        }
        let mut msf = MsfStreams { bytes, block_size, block_count, streams: Vec::new() };
        let mut owners = BlockOwners::new(file.len(), block_size)?;
        // The block map lists the directory's blocks; it fits one block.
        let directory_blocks = blocks_for(directory_bytes, block_size);
        owners.claim(block_map)?;
        let map = msf.block(block_map)?;
        // Whole blocks are copied, then the tail past the directory cut.
        let whole_blocks = u64::from(directory_blocks).saturating_mul(u64::from(block_size));
        let mut directory = reserved(whole_blocks, "MSF directory")?;
        let mut map_cursor = Cursor::new(Bytes::new(map), 0);
        for _ in 0..directory_blocks {
            let block = map_cursor.u32_le().map_err(|source| short("MSF block map", source))?;
            owners.claim(block)?;
            directory.extend_from_slice(msf.block(block)?);
        }
        directory.truncate(directory_bytes as usize);
        msf.streams = Self::directory(&directory, block_size, &mut owners)?;
        Ok(msf)
    }

    /// The directory's stream sizes and block lists; every listed block is
    /// claimed in `owners`.
    fn directory(directory: &[u8], block_size: u32, owners: &mut BlockOwners) -> Result<Vec<Stream>, PdbError> {
        let mut cursor = Cursor::new(Bytes::new(directory), 0);
        let what = "MSF directory";
        let count = cursor.u32_le().map_err(|source| short(what, source))?;
        if count > MAX_STREAMS {
            return Err(PdbError::TooMany { what: "streams", count: u64::from(count) });
        }
        let mut sizes = Vec::new();
        sizes
            .try_reserve_exact(count as usize)
            .map_err(|_| PdbError::Allocation { what: "stream sizes", bytes: u64::from(count) })?;
        for _ in 0..count {
            let size = cursor.u32_le().map_err(|source| short(what, source))?;
            sizes.push(if size == NIL_STREAM { None } else { Some(size) });
        }
        let mut streams = Vec::new();
        streams
            .try_reserve_exact(count as usize)
            .map_err(|_| PdbError::Allocation { what: "streams", bytes: u64::from(count) })?;
        for size in sizes {
            let wanted = blocks_for(size.unwrap_or(0), block_size);
            // Each block number takes 4 directory bytes, so a count past
            // what is left of the directory cannot be real.
            if (wanted as usize) > cursor.remaining() / 4 {
                return Err(PdbError::TooMany { what: "stream blocks", count: u64::from(wanted) });
            }
            let mut blocks = Vec::new();
            blocks
                .try_reserve_exact(wanted as usize)
                .map_err(|_| PdbError::Allocation { what: "stream blocks", bytes: u64::from(wanted) })?;
            for _ in 0..wanted {
                let block = cursor.u32_le().map_err(|source| short(what, source))?;
                owners.claim(block)?;
                blocks.push(block);
            }
            streams.push(Stream { size, blocks });
        }
        Ok(streams)
    }

    /// Block `block`'s bytes.
    fn block(&self, block: u32) -> Result<&'a [u8], PdbError> {
        if block >= self.block_count {
            return Err(PdbError::BadBlock { block, count: self.block_count });
        }
        let at = (block as usize)
            .checked_mul(self.block_size as usize)
            .ok_or(PdbError::BadBlock { block, count: self.block_count })?;
        self.bytes
            .slice(at, self.block_size as usize)
            .map_err(|source| short("MSF block", source))
    }

    pub(super) fn count(&self) -> u32 {
        u32::try_from(self.streams.len()).unwrap_or(u32::MAX)
    }

    pub(super) fn read(&self, index: u32) -> Result<Option<Vec<u8>>, PdbError> {
        let stream = self.streams.get(index as usize).ok_or(PdbError::NoStream { index })?;
        let Some(size) = stream.size else {
            return Ok(None);
        };
        let mut out = reserved(u64::from(size), "stream")?;
        let mut left = size as usize;
        for block in &stream.blocks {
            let bytes = self.block(*block)?;
            let take = left.min(bytes.len());
            out.extend_from_slice(bytes.get(..take).unwrap_or(bytes));
            left = left.saturating_sub(take);
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdb::MSF_MAGIC;

    const BLOCK: usize = 512;
    const FILE_BLOCKS: u32 = 8;
    /// The block map's block in every file `tiny_msf` builds.
    const MAP_BLOCK: u32 = 3;

    /// One stream of a test directory.
    struct TestStream {
        size: u32,
        blocks: &'static [u32],
    }

    /// The directory bytes listing `streams`.
    fn directory_of(streams: &[TestStream]) -> Vec<u8> {
        let mut out = (streams.len() as u32).to_le_bytes().to_vec();
        for stream in streams {
            out.extend_from_slice(&stream.size.to_le_bytes());
        }
        for stream in streams {
            for block in stream.blocks {
                out.extend_from_slice(&block.to_le_bytes());
            }
        }
        out
    }

    /// An 8-block MSF file whose block map (block 3) lists
    /// `directory_blocks`, with `directory` written across them in order.
    fn tiny_msf(directory_blocks: &[u32], directory: &[u8]) -> Vec<u8> {
        let mut file = vec![0u8; FILE_BLOCKS as usize * BLOCK];
        file[..32].copy_from_slice(MSF_MAGIC);
        file[32..36].copy_from_slice(&(BLOCK as u32).to_le_bytes());
        file[36..40].copy_from_slice(&1u32.to_le_bytes());
        file[40..44].copy_from_slice(&FILE_BLOCKS.to_le_bytes());
        file[44..48].copy_from_slice(&(directory.len() as u32).to_le_bytes());
        file[52..56].copy_from_slice(&MAP_BLOCK.to_le_bytes());
        for (slot, block) in directory_blocks.iter().enumerate() {
            let at = MAP_BLOCK as usize * BLOCK + slot * 4;
            file[at..at + 4].copy_from_slice(&block.to_le_bytes());
        }
        for (piece, block) in directory.chunks(BLOCK).zip(directory_blocks) {
            let at = *block as usize * BLOCK;
            file[at..at + piece.len()].copy_from_slice(piece);
        }
        file
    }

    #[test]
    fn distinct_blocks_read_back_in_order() {
        let directory = directory_of(&[TestStream { size: 600, blocks: &[6, 5] }]);
        let mut file = tiny_msf(&[4], &directory);
        file[6 * BLOCK..7 * BLOCK].fill(0xAB);
        file[5 * BLOCK..6 * BLOCK].fill(0xCD);
        let msf = MsfStreams::open(&file).unwrap();
        let stream = msf.read(0).unwrap().unwrap();
        assert_eq!(stream.len(), 600);
        assert!(stream[..BLOCK].iter().all(|byte| *byte == 0xAB));
        assert!(stream[BLOCK..].iter().all(|byte| *byte == 0xCD));
    }

    #[test]
    fn a_stream_naming_one_block_twice_is_refused() {
        // Honoured, block 5 named N times reads as N blocks: a small file
        // expands without bound.
        let directory = directory_of(&[TestStream { size: 1024, blocks: &[5, 5] }]);
        let file = tiny_msf(&[4], &directory);
        assert_eq!(MsfStreams::open(&file).err(), Some(PdbError::SharedBlock { block: 5 }));
    }

    #[test]
    fn a_block_map_naming_one_block_twice_is_refused() {
        let mut directory = directory_of(&[]);
        directory.resize(2 * BLOCK, 0);
        let file = tiny_msf(&[4, 4], &directory);
        assert_eq!(MsfStreams::open(&file).err(), Some(PdbError::SharedBlock { block: 4 }));
    }

    #[test]
    fn a_stream_cannot_own_the_directory_or_the_block_map() {
        let directory = directory_of(&[TestStream { size: 512, blocks: &[4] }]);
        let file = tiny_msf(&[4], &directory);
        assert_eq!(MsfStreams::open(&file).err(), Some(PdbError::SharedBlock { block: 4 }));
        let directory = directory_of(&[TestStream { size: 512, blocks: &[MAP_BLOCK] }]);
        let file = tiny_msf(&[4], &directory);
        assert_eq!(MsfStreams::open(&file).err(), Some(PdbError::SharedBlock { block: MAP_BLOCK }));
    }

    #[test]
    fn a_stream_block_past_the_file_is_refused_at_open() {
        let directory = directory_of(&[TestStream { size: 512, blocks: &[9] }]);
        let file = tiny_msf(&[4], &directory);
        assert_eq!(MsfStreams::open(&file).err(), Some(PdbError::BadBlock { block: 9, count: FILE_BLOCKS }));
    }
}
