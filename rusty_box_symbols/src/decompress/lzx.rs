//! LZX as cabinet folders hold it: MS-PATCH "LZX DELTA Compression and
//! Decompression" §2 without its DELTA extensions, framed as MS-CAB frames
//! it and as libmspack's `lzxd.c` decodes it. Tier P (spec §15): the trees
//! are fixed arrays, the output buffer is the window (a match reaches only
//! into what the folder has produced), and nothing allocates.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use crate::read::{copy_prefix, Bytes};

/// MS-CAB §2.4: each data block, and so each LZX frame, holds 32 KiB.
pub const FRAME_BYTES: usize = 32_768;
/// MS-CAB's `tcompLZX` windows: 2^15 to 2^21 bytes.
const MIN_WINDOW_BITS: u32 = 15;
const MAX_WINDOW_BITS: u32 = 21;
/// MS-PATCH §2.1.4: position slots for windows 2^15 .. 2^21.
const POSITION_SLOTS: [usize; 7] = [30, 32, 34, 36, 38, 42, 50];
/// The main tree: 256 literals and 8 match headers per position slot.
const MAX_MAIN_SYMBOLS: usize = 256 + 8 * 50;
const LENGTH_SYMBOLS: usize = 249;
const PRETREE_SYMBOLS: usize = 20;
const ALIGNED_SYMBOLS: usize = 8;
/// MS-PATCH §2.5: path lengths are at most 16.
const MAX_CODE_BITS: usize = 16;
/// The shortest match.
const MIN_MATCH: usize = 2;
/// MS-PATCH §2.6: E8 translation stops after the first 1 GiB of output.
const E8_LIMIT: usize = 0x4000_0000;
/// MS-PATCH §2.1.4, "position slot" bases and footer bits, slots 0..50.
const BASE_POSITION: [u32; 50] = [
    0, 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 1536,
    2048, 3072, 4096, 6144, 8192, 12288, 16384, 24576, 32768, 49152, 65536, 98304, 131072, 196608,
    262144, 393216, 524288, 655360, 786432, 917504, 1048576, 1179648, 1310720, 1441792, 1572864,
    1703936, 1835008, 1966080,
];
const FOOTER_BITS: [u32; 50] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13, 14, 14, 15, 15, 16, 16, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
];

/// Why a stream did not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LzxError {
    #[error("the stream ends inside a block (bit {bit})")]
    Truncated { bit: usize },
    #[error("a window of 2^{bits} bytes is not one cabinets use")]
    BadWindow { bits: u32 },
    #[error("block type {kind} is not verbatim, aligned or uncompressed")]
    BadBlockType { kind: u32 },
    #[error("a block declares no bytes")]
    EmptyBlock,
    #[error("the {what} tree's path lengths are not a complete code")]
    BadTree { what: &'static str },
    #[error("pretree code {code} is not a path length")]
    BadPretreeCode { code: u16 },
    #[error("a run of path lengths passes the end of the {what} tree")]
    RunPastTree { what: &'static str },
    #[error("a match runs past the end of its block")]
    MatchPastBlock,
    #[error("a match reaches {distance} bytes back with only {available} written")]
    DistanceTooFar { distance: usize, available: usize },
    #[error("the output is full at {capacity} bytes")]
    OutputFull { capacity: usize },
}

/// Bits read most significant first from little-endian 16-bit words
/// (MS-PATCH §2.2.1).
struct Bits<'a> {
    input: Bytes<'a>,
    bit: usize,
}

impl<'a> Bits<'a> {
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn truncated(&self) -> LzxError {
        LzxError::Truncated { bit: self.bit }
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn bit(&mut self) -> Result<u32, LzxError> {
        let word_at = self.bit.wrapping_shr(4).checked_mul(2).ok_or(self.truncated())?;
        let word = self.input.u16_le(word_at).map_err(|_| self.truncated())?;
        let shift = 15u32.wrapping_sub((self.bit & 15) as u32);
        self.bit = self.bit.checked_add(1).ok_or(self.truncated())?;
        Ok(u32::from(word.wrapping_shr(shift) & 1))
    }

    /// `count` bits (at most 32), the first read the most significant.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn bits(&mut self, count: u32) -> Result<u32, LzxError> {
        let mut value = 0u32;
        for _ in 0..count.min(32) {
            value = value.wrapping_shl(1) | self.bit()?;
        }
        Ok(value)
    }

    /// The next 16-bit word boundary; a position on one stays.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn align_word(&mut self) -> Result<(), LzxError> {
        let padding = 16usize.wrapping_sub(self.bit & 15) & 15;
        self.bit = self.bit.checked_add(padding).ok_or(self.truncated())?;
        Ok(())
    }

    /// An uncompressed block's padding (MS-PATCH §2.3.3): 1 to 16 bits, so
    /// a position already on a word boundary skips a whole word.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn align_uncompressed(&mut self) -> Result<(), LzxError> {
        let padding = 16usize.wrapping_sub(self.bit & 15);
        self.bit = self.bit.checked_add(padding).ok_or(self.truncated())?;
        Ok(())
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn byte_position(&self) -> usize {
        self.bit.wrapping_shr(3)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn set_byte_position(&mut self, byte: usize) -> Result<(), LzxError> {
        self.bit = byte.checked_mul(8).ok_or(self.truncated())?;
        Ok(())
    }
}

/// Whether a tree has codes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Codes {
    Empty,
    Complete,
}

/// A canonical Huffman code over `N` symbols (codes assigned shortest
/// first, lowest symbol first, as MS-PATCH §2.5 builds them).
struct Code<const N: usize> {
    count: [u16; MAX_CODE_BITS + 1],
    symbol: [u16; N],
    codes: Codes,
    what: &'static str,
}

impl<const N: usize> Code<N> {
    /// The code of `lengths`, which must be complete or empty.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn build(lengths: &[u8], what: &'static str) -> Result<Code<N>, LzxError> {
        let mut code = Code {
            count: [0; MAX_CODE_BITS + 1],
            symbol: [0; N],
            codes: Codes::Empty,
            what,
        };
        for length in lengths {
            let slot = code.count.get_mut(usize::from(*length)).ok_or(LzxError::BadTree { what })?;
            *slot = slot.wrapping_add(1);
        }
        let zero_lengths = usize::from(code.count.first().copied().unwrap_or(0));
        if zero_lengths == lengths.len() {
            return Ok(code);
        }
        let mut unused: i32 = 1;
        for length in 1..=MAX_CODE_BITS {
            unused = unused
                .wrapping_mul(2)
                .wrapping_sub(i32::from(code.count.get(length).copied().unwrap_or(0)));
            if unused < 0 {
                return Err(LzxError::BadTree { what });
            }
        }
        if unused != 0 {
            return Err(LzxError::BadTree { what });
        }
        let mut offsets = [0u16; MAX_CODE_BITS + 2];
        for length in 1..=MAX_CODE_BITS {
            let start = offsets.get(length).copied().unwrap_or(0);
            let count = code.count.get(length).copied().unwrap_or(0);
            if let Some(next) = offsets.get_mut(length.wrapping_add(1)) {
                *next = start.wrapping_add(count);
            }
        }
        for (symbol, length) in lengths.iter().enumerate() {
            if *length == 0 {
                continue;
            }
            let offset = offsets.get_mut(usize::from(*length)).ok_or(LzxError::BadTree { what })?;
            let slot = code.symbol.get_mut(usize::from(*offset)).ok_or(LzxError::BadTree { what })?;
            *slot = u16::try_from(symbol).map_err(|_| LzxError::BadTree { what })?;
            *offset = offset.wrapping_add(1);
        }
        code.codes = Codes::Complete;
        Ok(code)
    }

    /// The next symbol, its code read most significant bit first.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, LzxError> {
        if self.codes == Codes::Empty {
            return Err(LzxError::BadTree { what: self.what });
        }
        let mut code = 0u32;
        let mut first = 0u32;
        let mut index = 0u32;
        for length in 1..=MAX_CODE_BITS {
            code |= bits.bit()?;
            let count = u32::from(self.count.get(length).copied().unwrap_or(0));
            if code < first.wrapping_add(count) {
                let at = index.wrapping_add(code.wrapping_sub(first));
                return self.symbol.get(at as usize).copied().ok_or(LzxError::BadTree { what: self.what });
            }
            index = index.wrapping_add(count);
            first = first.wrapping_add(count).wrapping_shl(1);
            code = code.wrapping_shl(1);
        }
        Err(LzxError::BadTree { what: self.what })
    }
}

/// Reads a pretree, then the path lengths of `lengths[start..end]` coded
/// with it as deltas from the previous lengths (MS-PATCH §2.2.2).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn read_lengths(bits: &mut Bits<'_>, lengths: &mut [u8], start: usize, end: usize, what: &'static str) -> Result<(), LzxError> {
    let mut pretree_lengths = [0u8; PRETREE_SYMBOLS];
    for length in &mut pretree_lengths {
        *length = bits.bits(4)?.to_le_bytes()[0];
    }
    let pretree = Code::<PRETREE_SYMBOLS>::build(&pretree_lengths, "pretree")?;
    let mut at = start;
    while at < end {
        let code = pretree.decode(bits)?;
        let previous = lengths.get(at).copied().ok_or(LzxError::RunPastTree { what })?;
        let run = match code {
            0..=16 => Run { length: delta(previous, code), times: 1 },
            17 => Run { length: 0, times: (bits.bits(4)? as usize).wrapping_add(4) },
            18 => Run { length: 0, times: (bits.bits(5)? as usize).wrapping_add(20) },
            19 => {
                let times = (bits.bits(1)? as usize).wrapping_add(4);
                let code = pretree.decode(bits)?;
                if code > 16 {
                    return Err(LzxError::BadPretreeCode { code });
                }
                Run { length: delta(previous, code), times }
            }
            other => return Err(LzxError::BadPretreeCode { code: other }),
        };
        let run_end = at.checked_add(run.times).filter(|run_end| *run_end <= end).ok_or(LzxError::RunPastTree { what })?;
        let slots = lengths.get_mut(at..run_end).ok_or(LzxError::RunPastTree { what })?;
        slots.fill(run.length);
        at = run_end;
    }
    Ok(())
}

/// A run of equal path lengths.
struct Run {
    length: u8,
    times: usize,
}

/// `(previous - code) mod 17`, the delta coding of MS-PATCH §2.2.2.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn delta(previous: u8, code: u16) -> u8 {
    let value = u16::from(previous).wrapping_add(17).wrapping_sub(code).wrapping_rem(17);
    value.to_le_bytes()[0]
}

/// The output and where the next byte goes.
struct Output<'o> {
    out: &'o mut [u8],
    at: usize,
}

impl Output<'_> {
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn put(&mut self, byte: u8) -> Result<(), LzxError> {
        let capacity = self.out.len();
        let slot = self.out.get_mut(self.at).ok_or(LzxError::OutputFull { capacity })?;
        *slot = byte;
        self.at = self.at.wrapping_add(1);
        Ok(())
    }

    /// Copies `length` bytes from `distance` back; a match may overlap what
    /// it writes, and may not reach before the folder's first byte.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn copy_match(&mut self, distance: usize, length: usize) -> Result<(), LzxError> {
        if distance == 0 || distance > self.at {
            return Err(LzxError::DistanceTooFar { distance, available: self.at });
        }
        for _ in 0..length {
            let byte = self
                .out
                .get(self.at.wrapping_sub(distance))
                .copied()
                .ok_or(LzxError::DistanceTooFar { distance, available: self.at })?;
            self.put(byte)?;
        }
        Ok(())
    }
}

/// What the current block is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Verbatim,
    Aligned,
    Uncompressed,
}

/// The block being decoded and how many of its bytes are still to come.
struct Block {
    kind: BlockKind,
    size: u32,
    remaining: u32,
}

/// What carries from block to block in a folder (MS-PATCH §2.2): the trees'
/// path lengths, which each block codes as deltas, and the repeat offsets.
struct Decoder {
    main_lengths: [u8; MAX_MAIN_SYMBOLS],
    length_lengths: [u8; LENGTH_SYMBOLS],
    repeats: [usize; 3],
    main_symbols: usize,
}

/// The trees of one verbatim or aligned block.
struct Trees {
    main: Code<MAX_MAIN_SYMBOLS>,
    length: Code<LENGTH_SYMBOLS>,
    aligned: Code<ALIGNED_SYMBOLS>,
}

impl Decoder {
    /// Reads a block header and, for a verbatim or aligned block, its trees.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn read_block(&mut self, bits: &mut Bits<'_>, trees: &mut Trees) -> Result<Block, LzxError> {
        let kind = bits.bits(3)?;
        let size = bits.bits(24)?;
        let kind = match kind {
            1 => BlockKind::Verbatim,
            2 => BlockKind::Aligned,
            3 => BlockKind::Uncompressed,
            other => return Err(LzxError::BadBlockType { kind: other }),
        };
        if size == 0 {
            return Err(LzxError::EmptyBlock);
        }
        match kind {
            BlockKind::Uncompressed => {
                bits.align_uncompressed()?;
                let at = bits.byte_position();
                for (index, repeat) in self.repeats.iter_mut().enumerate() {
                    let offset = at.checked_add(index.wrapping_mul(4)).ok_or(bits.truncated())?;
                    *repeat = bits.input.u32_le(offset).map_err(|_| bits.truncated())? as usize;
                }
                bits.set_byte_position(at.checked_add(12).ok_or(bits.truncated())?)?;
            }
            BlockKind::Verbatim | BlockKind::Aligned => {
                if kind == BlockKind::Aligned {
                    let mut aligned_lengths = [0u8; ALIGNED_SYMBOLS];
                    for length in &mut aligned_lengths {
                        *length = bits.bits(3)?.to_le_bytes()[0];
                    }
                    trees.aligned = Code::build(&aligned_lengths, "aligned offset")?;
                }
                read_lengths(bits, &mut self.main_lengths, 0, 256, "main")?;
                read_lengths(bits, &mut self.main_lengths, 256, self.main_symbols, "main")?;
                read_lengths(bits, &mut self.length_lengths, 0, LENGTH_SYMBOLS, "length")?;
                let main_lengths = self.main_lengths.get(..self.main_symbols).ok_or(LzxError::BadTree { what: "main" })?;
                trees.main = Code::build(main_lengths, "main")?;
                if trees.main.codes == Codes::Empty {
                    return Err(LzxError::BadTree { what: "main" });
                }
                trees.length = Code::build(&self.length_lengths, "length")?;
            }
        }
        Ok(Block { kind, size, remaining: size })
    }

    /// Decodes one literal or match of a verbatim or aligned block.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn element(&mut self, bits: &mut Bits<'_>, trees: &Trees, block: &mut Block, output: &mut Output<'_>) -> Result<(), LzxError> {
        let element = usize::from(trees.main.decode(bits)?);
        if element < 256 {
            output.put(element.to_le_bytes()[0])?;
            block.remaining = block.remaining.wrapping_sub(1);
            return Ok(());
        }
        let header = element.wrapping_sub(256);
        let mut length = (header & 7).wrapping_add(MIN_MATCH);
        if header & 7 == 7 {
            length = length.wrapping_add(usize::from(trees.length.decode(bits)?));
        }
        let slot = header.wrapping_shr(3);
        let [first, second, third] = self.repeats;
        let offset = match slot {
            0 => first,
            1 => {
                self.repeats = [second, first, third];
                second
            }
            2 => {
                self.repeats = [third, second, first];
                third
            }
            _ => {
                let footer = FOOTER_BITS.get(slot).copied().ok_or(LzxError::BadTree { what: "main" })?;
                let base = BASE_POSITION.get(slot).copied().ok_or(LzxError::BadTree { what: "main" })?;
                let extra = if block.kind == BlockKind::Aligned && footer >= 3 {
                    let verbatim = bits.bits(footer.wrapping_sub(3))?.wrapping_shl(3);
                    verbatim.wrapping_add(u32::from(trees.aligned.decode(bits)?))
                } else {
                    bits.bits(footer)?
                };
                let formatted = base.wrapping_add(extra) as usize;
                let offset = formatted.checked_sub(2).ok_or(LzxError::DistanceTooFar { distance: 0, available: output.at })?;
                self.repeats = [offset, first, second];
                offset
            }
        };
        let length_u32 = u32::try_from(length).map_err(|_| LzxError::MatchPastBlock)?;
        if length_u32 > block.remaining {
            return Err(LzxError::MatchPastBlock);
        }
        output.copy_match(offset, length)?;
        block.remaining = block.remaining.wrapping_sub(length_u32);
        Ok(())
    }
}

/// Decodes one cabinet folder's LZX stream, its data blocks concatenated,
/// until `out` is full; then undoes E8 call translation.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decompress(input: Bytes<'_>, out: &mut [u8], window_bits: u32) -> Result<(), LzxError> {
    let slots = window_bits
        .checked_sub(MIN_WINDOW_BITS)
        .filter(|_| window_bits <= MAX_WINDOW_BITS)
        .and_then(|index| POSITION_SLOTS.get(index as usize).copied())
        .ok_or(LzxError::BadWindow { bits: window_bits })?;
    let mut bits = Bits { input, bit: 0 };
    // MS-PATCH §2.2.1: one bit, then the translation size in 32 bits.
    let file_size = if bits.bit()? == 1 { bits.bits(32)? } else { 0 };
    let mut decoder = Decoder {
        main_lengths: [0; MAX_MAIN_SYMBOLS],
        length_lengths: [0; LENGTH_SYMBOLS],
        repeats: [1, 1, 1],
        main_symbols: 256usize.wrapping_add(slots.wrapping_mul(8)),
    };
    let mut trees = Trees {
        main: Code::build(&[], "main")?,
        length: Code::build(&[], "length")?,
        aligned: Code::build(&[], "aligned offset")?,
    };
    let total = out.len();
    let mut output = Output { out, at: 0 };
    let mut block = Block { kind: BlockKind::Uncompressed, size: 0, remaining: 0 };
    let mut frame_end = 0usize;
    while output.at < total {
        frame_end = frame_end.saturating_add(FRAME_BYTES).min(total);
        while output.at < frame_end {
            if block.remaining == 0 {
                // An odd-sized uncompressed block is followed by a pad byte;
                // the next block replaces this one, so the pad is taken once.
                if block.kind == BlockKind::Uncompressed && block.size & 1 == 1 {
                    bits.set_byte_position(bits.byte_position().saturating_add(1))?;
                }
                block = decoder.read_block(&mut bits, &mut trees)?;
            }
            match block.kind {
                BlockKind::Uncompressed => {
                    let want = (block.remaining as usize).min(frame_end.wrapping_sub(output.at));
                    let at = bits.byte_position();
                    let raw = bits.input.slice(at, want).map_err(|_| bits.truncated())?;
                    for byte in raw {
                        output.put(*byte)?;
                    }
                    bits.set_byte_position(at.wrapping_add(want))?;
                    block.remaining = block.remaining.wrapping_sub(want as u32);
                }
                BlockKind::Verbatim | BlockKind::Aligned => {
                    decoder.element(&mut bits, &trees, &mut block, &mut output)?;
                }
            }
        }
        // libmspack `lzxd_decompress`: the bitstream realigns after each
        // frame, except while an uncompressed block's bytes are being read
        // (which are byte-aligned and carry on across the frame).
        let in_raw_bytes = block.kind == BlockKind::Uncompressed && block.remaining > 0;
        if !in_raw_bytes {
            if block.kind == BlockKind::Uncompressed && block.remaining == 0 && block.size & 1 == 1 {
                bits.set_byte_position(bits.byte_position().saturating_add(1))?;
                block.size = 0;
            }
            bits.align_word()?;
        }
    }
    if file_size != 0 {
        translate_e8(output.out, file_size);
    }
    Ok(())
}

/// Undoes E8 call translation (MS-PATCH §2.6) in place: in each 32 KiB
/// frame of the first 1 GiB, every `E8` byte not in the frame's last ten is
/// followed by an absolute target, which becomes relative again.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn translate_e8(out: &mut [u8], file_size: u32) {
    let size = file_size as i32;
    for (frame_index, frame) in out.chunks_mut(FRAME_BYTES).enumerate() {
        let start = frame_index.saturating_mul(FRAME_BYTES);
        if start >= E8_LIMIT {
            return;
        }
        let end = frame.len().saturating_sub(10);
        let mut at = 0usize;
        while at < end {
            if frame.get(at).copied() != Some(0xE8) {
                at = at.wrapping_add(1);
                continue;
            }
            let current = start.wrapping_add(at) as i32;
            let operand = at.wrapping_add(1);
            let Some(bytes) = frame.get_mut(operand..operand.wrapping_add(4)) else {
                return;
            };
            let Ok(absolute) = Bytes::new(bytes).array::<4>(0).map(i32::from_le_bytes) else {
                return;
            };
            if absolute >= current.wrapping_neg() && absolute < size {
                let relative = if absolute >= 0 {
                    absolute.wrapping_sub(current)
                } else {
                    absolute.wrapping_add(size)
                };
                // `bytes` is the operand's four bytes, so all four are copied.
                if copy_prefix(bytes, &relative.to_le_bytes()) != 4 {
                    return;
                }
            }
            at = at.wrapping_add(5);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNCOMPRESSED_HELLO: [u8; 22] = [
        0x00, 0x30, 0x50, 0x00, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, b'h', b'e', b'l', b'l', b'o', 0,
    ];

    #[test]
    fn an_uncompressed_block_is_copied() {
        let mut out = [0u8; 5];
        decompress(Bytes::new(&UNCOMPRESSED_HELLO), &mut out, 15).unwrap();
        assert_eq!(&out, b"hello");
    }

    #[test]
    fn a_window_cabinets_do_not_use_is_refused() {
        let mut out = [0u8; 5];
        assert_eq!(decompress(Bytes::new(&UNCOMPRESSED_HELLO), &mut out, 22), Err(LzxError::BadWindow { bits: 22 }));
        assert_eq!(decompress(Bytes::new(&UNCOMPRESSED_HELLO), &mut out, 14), Err(LzxError::BadWindow { bits: 14 }));
    }

    #[test]
    fn a_reserved_block_type_or_a_short_stream_is_refused() {
        let mut out = [0u8; 5];
        assert_eq!(decompress(Bytes::new(&[0, 0, 0, 0]), &mut out, 15), Err(LzxError::BadBlockType { kind: 0 }));
        assert!(matches!(
            decompress(Bytes::new(&UNCOMPRESSED_HELLO[..18]), &mut out, 15),
            Err(LzxError::Truncated { .. })
        ));
    }

    /// Review focus 3: a back-reference before the start of the output.
    #[test]
    fn a_match_before_the_start_of_the_output_is_refused() {
        let mut out = [7u8; 8];
        let mut output = Output { out: &mut out, at: 2 };
        assert_eq!(output.copy_match(3, 2), Err(LzxError::DistanceTooFar { distance: 3, available: 2 }));
        assert_eq!(output.copy_match(2, 2), Ok(()));
        assert_eq!(output.at, 4);
    }

    #[test]
    fn e8_translation_is_undone_where_the_specification_says() {
        // An E8 at offset 1 holding absolute 0x10, one holding -1, one out of
        // range, and the last ten bytes, which are never translated.
        let mut frame = vec![0x90, 0xE8, 0x10, 0, 0, 0, 0xE8, 0xFF, 0xFF, 0xFF, 0xFF, 0xE8, 0, 0x20, 0, 0];
        frame.extend_from_slice(&[0xE8, 0x10, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut expected = frame.clone();
        // At position 1: 0x10 - 1.
        expected[2..6].copy_from_slice(&0x0Fi32.to_le_bytes());
        // At position 6: -1 + 0x1000.
        expected[7..11].copy_from_slice(&0xFFFi32.to_le_bytes());
        translate_e8(&mut frame, 0x1000);
        assert_eq!(frame, expected);
    }
}
