//! Zstandard frames (RFC 8878), as MSFZ chunks and `ELFCOMPRESS_ZSTD`
//! sections hold them: raw, RLE and compressed blocks; Huffman-coded
//! literals in one or four streams; FSE-coded sequences in all four table
//! modes; repeat offsets; content checksums. Dictionaries are refused (no
//! symbol-file writer uses one), and so are windows over 128 MiB.
//!
//! Tier P (spec §15): every table is a fixed array in the decoder's state,
//! the output buffer is the window (a match reaches back only into what this
//! frame has written), and one block's literals go into a scratch buffer the
//! caller allocated. Nothing allocates, and in this crate's release test
//! build `#[no_panic]` proves no path panics (`read`'s module doc says why
//! only there). Every loop is bounded by the input (each step
//! consumes bits) or by the output (each step writes bytes).

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use crate::read::{copy_prefix, Bytes};

/// RFC 8878 §3.1.1: `0xFD2FB528`, little-endian.
const MAGIC: u32 = 0xFD2F_B528;
/// §3.1.2: skippable frames use `0x184D2A50` to `0x184D2A5F`.
const SKIPPABLE_MAGIC: u32 = 0x184D_2A50;
const SKIPPABLE_MASK: u32 = 0xFFFF_FFF0;
/// The largest window accepted: the RFC's decoder recommendation (§3.1.1.1.2:
/// "at least 8 MB", most decoders refuse past 128 MiB) and far beyond what
/// MSFZ chunks or ELF sections need.
pub const MAX_WINDOW: u64 = 1 << 27;
/// §3.1.1.2.4: a block decompresses to at most 128 KiB.
pub const MAX_BLOCK: usize = 1 << 17;
/// The scratch a caller provides: one block's literals.
pub const SCRATCH_BYTES: usize = MAX_BLOCK;
/// §4.2.1: Huffman codes are at most 11 bits.
const MAX_HUFFMAN_BITS: u32 = 11;
const HUFFMAN_TABLE_SIZE: usize = 1 << MAX_HUFFMAN_BITS;
/// §3.1.1.3.2.2: the largest accuracy logs.
const LL_MAX_LOG: u32 = 9;
const ML_MAX_LOG: u32 = 9;
const OF_MAX_LOG: u32 = 8;
/// §4.2.1.2: Huffman weights are FSE-coded with accuracy at most 6.
const WEIGHT_MAX_LOG: u32 = 6;
const MAX_LL_SYMBOL: usize = 35;
const MAX_ML_SYMBOL: usize = 52;
const MAX_OF_SYMBOL: usize = 31;
/// At most 255 weights are stored; the 256th is implied.
const MAX_WEIGHTS: usize = 255;

/// §3.1.1.3.2.2: the predefined distributions and their accuracy logs.
const LL_DEFAULT: [i16; 36] = [
    4, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 2, 1, 1, 1, 1, 1,
    -1, -1, -1, -1,
];
const LL_DEFAULT_LOG: u32 = 6;
const ML_DEFAULT: [i16; 53] = [
    1, 4, 3, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1, -1, -1,
];
const ML_DEFAULT_LOG: u32 = 6;
const OF_DEFAULT: [i16; 29] = [
    1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1,
];
const OF_DEFAULT_LOG: u32 = 5;

/// §3.1.1.3.2.1.1: literal-length codes.
const LL_BASE: [u32; 36] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28, 32, 40, 48, 64,
    128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536,
];
const LL_BITS: [u32; 36] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 4, 6, 7, 8, 9, 10, 11,
    12, 13, 14, 15, 16,
];
/// §3.1.1.3.2.1.1: match-length codes.
const ML_BASE: [u32; 53] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
    28, 29, 30, 31, 32, 33, 34, 35, 37, 39, 41, 43, 47, 51, 59, 67, 83, 99, 131, 259, 515, 1027,
    2051, 4099, 8195, 16387, 32771, 65539,
];
const ML_BITS: [u32; 53] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 2, 2, 3, 3, 4, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
];

/// XXH64's primes (xxHash specification).
const PRIME64_1: u64 = 0x9E37_79B1_85EB_CA87;
const PRIME64_2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const PRIME64_3: u64 = 0x1656_67B1_9E37_79F9;
const PRIME64_4: u64 = 0x85EB_CA77_C2B2_AE63;
const PRIME64_5: u64 = 0x27D4_EB2F_1656_67C5;

/// Why a stream did not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ZstdError {
    #[error("the input ends at byte {at}, inside a frame")]
    Truncated { at: usize },
    #[error("{found:#010x} is not a zstd frame's magic")]
    BadMagic { found: u32 },
    #[error("a reserved bit is set")]
    ReservedBit,
    #[error("the frame needs dictionary {id}, and dictionaries are not supported")]
    Dictionary { id: u32 },
    #[error("the frame's window of {size} bytes is over the 128 MiB accepted")]
    WindowTooLarge { size: u64 },
    #[error("block type 3 is reserved")]
    ReservedBlockType,
    #[error("a block of {size} bytes is over the block maximum")]
    BlockTooLarge { size: usize },
    #[error("the {what} is corrupt")]
    Corrupt { what: &'static str },
    #[error("a block repeats the previous {what} table, and there is none")]
    NoPreviousTable { what: &'static str },
    #[error("a match reaches {offset} bytes back with only {available} written in this frame")]
    OffsetTooFar { offset: usize, available: usize },
    #[error("the output is full at {capacity} bytes")]
    OutputFull { capacity: usize },
    #[error("the content checksum {found:#010x} is not the data's {computed:#010x}")]
    Checksum { found: u32, computed: u32 },
    #[error("the frame declares {declared} bytes and holds {produced}")]
    ContentSize { declared: u64, produced: u64 },
    #[error("the scratch buffer is {given} bytes, {needed} are needed")]
    ScratchTooSmall { needed: usize, given: usize },
}

/// `ZstdError::Corrupt` for the named part.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn corrupt(what: &'static str) -> ZstdError {
    ZstdError::Corrupt { what }
}

/// The index of the highest set bit (`value` must not be 0; 0 gives 0).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn highbit(value: u32) -> u32 {
    31u32.saturating_sub(value.leading_zeros())
}

/// Bits read forwards, least significant first (FSE table descriptions,
/// §4.1.1). Peeking past the end reads zeros; consuming past it fails.
struct ForwardBits<'a> {
    input: Bytes<'a>,
    bit: usize,
}

impl<'a> ForwardBits<'a> {
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn bit_at(&self, index: usize) -> u32 {
        let byte = self.input.u8(index.wrapping_shr(3)).unwrap_or(0);
        u32::from(byte.wrapping_shr((index & 7) as u32) & 1)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn peek(&self, count: u32) -> u32 {
        let mut value = 0u32;
        for index in 0..count.min(32) {
            value |= self.bit_at(self.bit.wrapping_add(index as usize)).wrapping_shl(index);
        }
        value
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn consume(&mut self, count: u32) -> Result<(), ZstdError> {
        let next = self.bit.checked_add(count as usize).ok_or(corrupt("FSE table description"))?;
        if next > self.input.len().saturating_mul(8) {
            return Err(ZstdError::Truncated { at: self.input.len() });
        }
        self.bit = next;
        Ok(())
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn read(&mut self, count: u32) -> Result<u32, ZstdError> {
        let value = self.peek(count);
        self.consume(count)?;
        Ok(value)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn bytes_used(&self) -> usize {
        self.bit.saturating_add(7).wrapping_shr(3)
    }
}

/// Bits read backwards from a stream's end (§4.1, "Bitstream"): the last
/// byte's highest set bit marks the start, and each read takes the next
/// lower bits as one little-endian number. Reading past the start yields
/// zeros and marks the stream overflowed, which every user checks.
struct BackwardBits<'a> {
    input: Bytes<'a>,
    total: usize,
    consumed: usize,
}

impl<'a> BackwardBits<'a> {
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn new(input: Bytes<'a>) -> Result<Self, ZstdError> {
        let last_at = input.len().checked_sub(1).ok_or(corrupt("empty bitstream"))?;
        let last = input.u8(last_at).map_err(|_| corrupt("empty bitstream"))?;
        if last == 0 {
            return Err(corrupt("bitstream end marker"));
        }
        let total = last_at
            .checked_mul(8)
            .and_then(|bits| bits.checked_add(highbit(u32::from(last)) as usize))
            .ok_or(corrupt("bitstream size"))?;
        Ok(BackwardBits { input, total, consumed: 0 })
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn bit_at(&self, index: usize) -> u32 {
        let byte = self.input.u8(index.wrapping_shr(3)).unwrap_or(0);
        u32::from(byte.wrapping_shr((index & 7) as u32) & 1)
    }

    /// The next `count` bits (at most 32) without consuming them; bits
    /// before the stream's start read as zero.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn peek(&self, count: u32) -> u32 {
        let top = self.total.saturating_sub(self.consumed);
        let mut value = 0u32;
        for index in 0..count.min(32) {
            // Bit `index` of the result is stream bit `top - count + index`.
            let below_top = (count.wrapping_sub(index)) as usize;
            if let Some(at) = top.checked_sub(below_top) {
                value |= self.bit_at(at).wrapping_shl(index);
            }
        }
        if self.consumed > self.total {
            return 0;
        }
        value
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn consume(&mut self, count: u32) {
        self.consumed = self.consumed.saturating_add(count as usize);
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn read(&mut self, count: u32) -> u32 {
        let value = self.peek(count);
        self.consume(count);
        value
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn overflowed(&self) -> bool {
        self.consumed > self.total
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn finished(&self) -> bool {
        self.consumed == self.total
    }
}

/// One FSE decoding-table cell (§4.1.1): the symbol it decodes, how many
/// bits the next state needs, and the base those bits are added to.
#[derive(Clone, Copy)]
struct FseEntry {
    symbol: u8,
    bits: u8,
    base: u16,
}

const EMPTY_ENTRY: FseEntry = FseEntry { symbol: 0, bits: 0, base: 0 };

/// Whether a table has been defined in this frame (§3.1.1.3.2.1: "Repeat"
/// mode reuses the previous block's).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Defined {
    Yes,
    No,
}

/// An FSE decoding table of at most `N` cells.
struct FseTable<const N: usize> {
    cells: [FseEntry; N],
    log: u32,
    defined: Defined,
}

impl<const N: usize> FseTable<N> {
    const fn empty() -> Self {
        FseTable {
            cells: [EMPTY_ENTRY; N],
            log: 0,
            defined: Defined::No,
        }
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn cell(&self, state: u32) -> Result<FseEntry, ZstdError> {
        self.cells.get(state as usize).copied().ok_or(corrupt("FSE state"))
    }

    /// The table every state of which decodes `symbol` (RLE mode).
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn rle(&mut self, symbol: u8) {
        if let Some(cell) = self.cells.first_mut() {
            *cell = FseEntry { symbol, bits: 0, base: 0 };
        }
        self.log = 0;
        self.defined = Defined::Yes;
    }

    /// The decoding table of normalised counts `counts` at accuracy `log`
    /// (§4.1.1, "FSE Table Description" and its decoding-table build, as
    /// the reference `FSE_buildDTable`).
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn build(&mut self, counts: &[i16], log: u32) -> Result<(), ZstdError> {
        let size = 1usize.checked_shl(log).filter(|size| *size <= N).ok_or(corrupt("FSE accuracy"))?;
        let mut next_state = [0u16; 256];
        let mut high = size.wrapping_sub(1);
        for (symbol, count) in counts.iter().enumerate() {
            let slot = next_state.get_mut(symbol).ok_or(corrupt("FSE symbol"))?;
            if *count == -1 {
                let cell = self.cells.get_mut(high).ok_or(corrupt("FSE counts"))?;
                cell.symbol = u8::try_from(symbol).map_err(|_| corrupt("FSE symbol"))?;
                high = high.checked_sub(1).ok_or(corrupt("FSE counts"))?;
                *slot = 1;
            } else {
                *slot = u16::try_from(*count).map_err(|_| corrupt("FSE counts"))?;
            }
        }
        let step = size.wrapping_shr(1).wrapping_add(size.wrapping_shr(3)).wrapping_add(3);
        let mask = size.wrapping_sub(1);
        let mut position = 0usize;
        for (symbol, count) in counts.iter().enumerate() {
            for _ in 0..(*count).max(0) {
                let cell = self.cells.get_mut(position).ok_or(corrupt("FSE counts"))?;
                cell.symbol = u8::try_from(symbol).map_err(|_| corrupt("FSE symbol"))?;
                // An odd step visits every cell, so a free one is found
                // within `size` steps.
                let mut placed = false;
                for _ in 0..size {
                    position = position.wrapping_add(step) & mask;
                    if position <= high {
                        placed = true;
                        break;
                    }
                }
                if !placed {
                    return Err(corrupt("FSE counts"));
                }
            }
        }
        if position != 0 {
            return Err(corrupt("FSE counts"));
        }
        for index in 0..size {
            let cell = self.cells.get_mut(index).ok_or(corrupt("FSE counts"))?;
            let next = next_state.get_mut(usize::from(cell.symbol)).ok_or(corrupt("FSE symbol"))?;
            let state = u32::from(*next);
            *next = next.wrapping_add(1);
            let bits = log.checked_sub(highbit(state)).ok_or(corrupt("FSE counts"))?;
            let base = state
                .checked_shl(bits)
                .and_then(|shifted| shifted.checked_sub(size as u32))
                .ok_or(corrupt("FSE counts"))?;
            cell.bits = u8::try_from(bits).map_err(|_| corrupt("FSE counts"))?;
            cell.base = u16::try_from(base).map_err(|_| corrupt("FSE counts"))?;
        }
        self.log = log;
        self.defined = Defined::Yes;
        Ok(())
    }
}

/// Reads an FSE table description (§4.1.1) of accuracy at most `max_log`
/// and symbols up to `max_symbol` into `table`; returns the bytes it used.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn read_fse_description<const N: usize>(
    input: Bytes<'_>,
    max_log: u32,
    max_symbol: usize,
    table: &mut FseTable<N>,
) -> Result<usize, ZstdError> {
    let mut bits = ForwardBits { input, bit: 0 };
    let log = bits.read(4)?.wrapping_add(5);
    if log > max_log {
        return Err(corrupt("FSE accuracy"));
    }
    let mut counts = [0i16; 256];
    let mut remaining: i32 = 1i32.wrapping_shl(log).wrapping_add(1);
    let mut threshold: i32 = 1i32.wrapping_shl(log);
    let mut width = log.wrapping_add(1);
    let mut symbol = 0usize;
    let mut previous_zero = false;
    while remaining > 1 && symbol <= max_symbol {
        if previous_zero {
            // Two-bit repeat flags: each adds that many zero counts; 3 means
            // "three, and another flag follows".
            loop {
                let flag = bits.read(2)?;
                symbol = symbol.wrapping_add(flag as usize);
                if symbol > max_symbol {
                    return Err(corrupt("FSE zero run"));
                }
                if flag != 3 {
                    break;
                }
            }
        }
        let largest = threshold.wrapping_mul(2).wrapping_sub(1).wrapping_sub(remaining);
        let peeked = bits.peek(width) as i32;
        let low = peeked & threshold.wrapping_sub(1);
        let raw = if low < largest {
            bits.consume(width.wrapping_sub(1))?;
            low
        } else {
            bits.consume(width)?;
            let full = peeked & threshold.wrapping_mul(2).wrapping_sub(1);
            if full >= threshold {
                full.wrapping_sub(largest)
            } else {
                full
            }
        };
        let count = raw.wrapping_sub(1);
        remaining = remaining.wrapping_sub(count.wrapping_abs());
        let slot = counts.get_mut(symbol).ok_or(corrupt("FSE symbol"))?;
        *slot = i16::try_from(count).map_err(|_| corrupt("FSE counts"))?;
        symbol = symbol.wrapping_add(1);
        previous_zero = count == 0;
        while remaining < threshold && threshold > 1 {
            width = width.wrapping_sub(1);
            threshold = threshold.wrapping_shr(1);
        }
    }
    if remaining != 1 {
        return Err(corrupt("FSE counts"));
    }
    table.build(counts.get(..symbol).unwrap_or(&[]), log)?;
    Ok(bits.bytes_used())
}

/// One Huffman decoding-table cell: the symbol and its code length.
#[derive(Clone, Copy)]
struct HuffmanEntry {
    symbol: u8,
    bits: u8,
}

/// A Huffman decoding table indexed by the next `max_bits` bits.
struct HuffmanTable {
    cells: [HuffmanEntry; HUFFMAN_TABLE_SIZE],
    max_bits: u32,
    defined: Defined,
}

impl HuffmanTable {
    const fn empty() -> Self {
        HuffmanTable {
            cells: [HuffmanEntry { symbol: 0, bits: 0 }; HUFFMAN_TABLE_SIZE],
            max_bits: 0,
            defined: Defined::No,
        }
    }

    /// Reads a Huffman tree description (§4.2.1) and builds the table;
    /// returns the bytes it used.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn read(&mut self, input: Bytes<'_>) -> Result<usize, ZstdError> {
        let header = input.u8(0).map_err(|_| ZstdError::Truncated { at: 0 })?;
        let mut weights = [0u8; 256];
        let count;
        let used;
        if header < 128 {
            let size = usize::from(header);
            let compressed = input.sub(1, size).map_err(|_| ZstdError::Truncated { at: 1 })?;
            used = size.wrapping_add(1);
            let mut table = FseTable::<64>::empty();
            let description = read_fse_description(compressed, WEIGHT_MAX_LOG, MAX_WEIGHTS, &mut table)?;
            let stream = compressed.tail(description).map_err(|_| corrupt("Huffman weights"))?;
            count = decode_weights(&table, stream, &mut weights)?;
        } else {
            count = usize::from(header).wrapping_sub(127);
            let packed = input
                .sub(1, count.wrapping_add(1).wrapping_shr(1))
                .map_err(|_| ZstdError::Truncated { at: 1 })?;
            used = packed.len().wrapping_add(1);
            for (index, slot) in weights.iter_mut().take(count).enumerate() {
                let byte = packed.u8(index.wrapping_shr(1)).map_err(|_| corrupt("Huffman weights"))?;
                *slot = if index & 1 == 0 { byte.wrapping_shr(4) } else { byte & 0x0F };
            }
        }
        self.build(&mut weights, count)?;
        Ok(used)
    }

    /// The table from the first `count` weights, the last weight implied
    /// (§4.2.1.3).
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn build(&mut self, weights: &mut [u8; 256], count: usize) -> Result<(), ZstdError> {
        let mut total = 0u32;
        for weight in weights.iter().take(count) {
            if u32::from(*weight) > MAX_HUFFMAN_BITS {
                return Err(corrupt("Huffman weight"));
            }
            if *weight > 0 {
                total = total.wrapping_add(1u32.wrapping_shl(u32::from(*weight).wrapping_sub(1)));
            }
        }
        if total == 0 {
            return Err(corrupt("Huffman weights"));
        }
        let max_bits = highbit(total).wrapping_add(1);
        if max_bits > MAX_HUFFMAN_BITS {
            return Err(corrupt("Huffman weights"));
        }
        let rest = 1u32.wrapping_shl(max_bits).wrapping_sub(total);
        if !rest.is_power_of_two() {
            return Err(corrupt("Huffman weights"));
        }
        let last = weights.get_mut(count).ok_or(corrupt("Huffman weights"))?;
        *last = u8::try_from(highbit(rest).wrapping_add(1)).map_err(|_| corrupt("Huffman weights"))?;
        let symbols = weights.get(..=count).ok_or(corrupt("Huffman weights"))?;
        let mut ones = 0usize;
        for weight in symbols {
            if *weight == 1 {
                ones = ones.wrapping_add(1);
            }
        }
        if ones < 2 || ones & 1 == 1 {
            return Err(corrupt("Huffman weights"));
        }
        let mut position = 0usize;
        for weight in 1..=max_bits {
            let cells = 1usize.wrapping_shl(weight.wrapping_sub(1));
            let bits = u8::try_from(max_bits.wrapping_add(1).wrapping_sub(weight)).map_err(|_| corrupt("Huffman weights"))?;
            let mut symbol = 0u8;
            for symbol_weight in symbols {
                if u32::from(*symbol_weight) == weight {
                    for _ in 0..cells {
                        let cell = self.cells.get_mut(position).ok_or(corrupt("Huffman weights"))?;
                        *cell = HuffmanEntry { symbol, bits };
                        position = position.wrapping_add(1);
                    }
                }
                // At most 256 symbols: the last one's increment is unused.
                symbol = symbol.wrapping_add(1);
            }
        }
        self.max_bits = max_bits;
        self.defined = Defined::Yes;
        Ok(())
    }

    /// Decodes `out.len()` literals from one stream, which must end exactly.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn decode_stream(&self, stream: Bytes<'_>, out: &mut [u8]) -> Result<(), ZstdError> {
        let mut bits = BackwardBits::new(stream)?;
        for slot in out.iter_mut() {
            let cell = self
                .cells
                .get(bits.peek(self.max_bits) as usize)
                .copied()
                .ok_or(corrupt("Huffman stream"))?;
            bits.consume(u32::from(cell.bits));
            if bits.overflowed() {
                return Err(corrupt("Huffman stream"));
            }
            *slot = cell.symbol;
        }
        if !bits.finished() {
            return Err(corrupt("Huffman stream"));
        }
        Ok(())
    }
}

/// The weights of an FSE-coded tree description: two interleaved states
/// until the bitstream runs out (reference `FSE_decompress_usingDTable`).
/// Returns how many weights it decoded.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn decode_weights(table: &FseTable<64>, stream: Bytes<'_>, weights: &mut [u8; 256]) -> Result<usize, ZstdError> {
    let mut bits = BackwardBits::new(stream)?;
    let mut first = bits.read(table.log);
    let mut second = bits.read(table.log);
    let mut count = 0usize;
    loop {
        for turn in 0..2 {
            let state = if turn == 0 { &mut first } else { &mut second };
            let cell = table.cell(*state)?;
            if count >= MAX_WEIGHTS {
                return Err(corrupt("Huffman weights"));
            }
            let slot = weights.get_mut(count).ok_or(corrupt("Huffman weights"))?;
            *slot = cell.symbol;
            count = count.wrapping_add(1);
            *state = u32::from(cell.base).wrapping_add(bits.read(u32::from(cell.bits)));
            if bits.overflowed() {
                // The other state's symbol is the last one.
                let other = if turn == 0 { second } else { first };
                if count >= MAX_WEIGHTS {
                    return Err(corrupt("Huffman weights"));
                }
                let slot = weights.get_mut(count).ok_or(corrupt("Huffman weights"))?;
                *slot = table.cell(other)?.symbol;
                return Ok(count.wrapping_add(1));
            }
        }
    }
}

/// One table's mode in a sequences section (§3.1.1.3.2.1).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn read_table<const N: usize>(
    mode: u8,
    input: Bytes<'_>,
    table: &mut FseTable<N>,
    kind: TableKind,
) -> Result<usize, ZstdError> {
    match mode {
        0 => {
            table.build(kind.default_counts, kind.default_log)?;
            Ok(0)
        }
        1 => {
            let symbol = input.u8(0).map_err(|_| ZstdError::Truncated { at: 0 })?;
            if usize::from(symbol) > kind.max_symbol {
                return Err(corrupt("RLE sequence code"));
            }
            table.rle(symbol);
            Ok(1)
        }
        2 => read_fse_description(input, kind.max_log, kind.max_symbol, table),
        _ => match table.defined {
            Defined::Yes => Ok(0),
            Defined::No => Err(ZstdError::NoPreviousTable { what: kind.name }),
        },
    }
}

/// What distinguishes the three sequence tables.
#[derive(Clone, Copy)]
struct TableKind {
    name: &'static str,
    default_counts: &'static [i16],
    default_log: u32,
    max_log: u32,
    max_symbol: usize,
}

const LITERAL_LENGTHS: TableKind = TableKind {
    name: "literal-length",
    default_counts: &LL_DEFAULT,
    default_log: LL_DEFAULT_LOG,
    max_log: LL_MAX_LOG,
    max_symbol: MAX_LL_SYMBOL,
};
const OFFSETS: TableKind = TableKind {
    name: "offset",
    default_counts: &OF_DEFAULT,
    default_log: OF_DEFAULT_LOG,
    max_log: OF_MAX_LOG,
    max_symbol: MAX_OF_SYMBOL,
};
const MATCH_LENGTHS: TableKind = TableKind {
    name: "match-length",
    default_counts: &ML_DEFAULT,
    default_log: ML_DEFAULT_LOG,
    max_log: ML_MAX_LOG,
    max_symbol: MAX_ML_SYMBOL,
};

/// Everything a frame carries from one block to the next.
struct FrameState {
    huffman: HuffmanTable,
    literal_lengths: FseTable<512>,
    offsets: FseTable<256>,
    match_lengths: FseTable<512>,
    /// §3.1.1.5: the three repeat offsets, starting 1, 4, 8.
    repeats: [usize; 3],
}

impl FrameState {
    const fn new() -> Self {
        FrameState {
            huffman: HuffmanTable::empty(),
            literal_lengths: FseTable::empty(),
            offsets: FseTable::empty(),
            match_lengths: FseTable::empty(),
            repeats: [1, 4, 8],
        }
    }
}

/// The output, where this frame started in it, and where the next byte goes.
struct Output<'o> {
    out: &'o mut [u8],
    frame_start: usize,
    at: usize,
}

impl Output<'_> {
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn put(&mut self, byte: u8) -> Result<(), ZstdError> {
        let capacity = self.out.len();
        let slot = self.out.get_mut(self.at).ok_or(ZstdError::OutputFull { capacity })?;
        *slot = byte;
        self.at = self.at.wrapping_add(1);
        Ok(())
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn put_all(&mut self, bytes: &[u8]) -> Result<(), ZstdError> {
        for byte in bytes {
            self.put(*byte)?;
        }
        Ok(())
    }

    /// Copies `length` bytes from `offset` back; a match may overlap what it
    /// writes, and may not reach before this frame's start.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn copy_match(&mut self, offset: usize, length: usize) -> Result<(), ZstdError> {
        let available = self.at.wrapping_sub(self.frame_start);
        if offset == 0 || offset > available {
            return Err(ZstdError::OffsetTooFar { offset, available });
        }
        for _ in 0..length {
            let byte = self
                .out
                .get(self.at.wrapping_sub(offset))
                .copied()
                .ok_or(ZstdError::OffsetTooFar { offset, available })?;
            self.put(byte)?;
        }
        Ok(())
    }
}

/// Where a block's literals ended up: their count, and the bytes of the
/// block the literals section used.
struct Literals {
    count: usize,
    used: usize,
}

/// The literals section of a compressed block (§3.1.1.3.1), decoded into
/// `scratch`.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn read_literals(block: Bytes<'_>, scratch: &mut [u8], state: &mut FrameState) -> Result<Literals, ZstdError> {
    let truncated = ZstdError::Truncated { at: 0 };
    let first = block.u8(0).map_err(|_| truncated)?;
    let kind = first & 3;
    let size_format = first.wrapping_shr(2) & 3;
    let byte = |at: usize| block.u8(at).map(u32::from).map_err(|_| truncated);
    if kind < 2 {
        // Raw (0) or RLE (1): a 5-, 12- or 20-bit size.
        let header = match size_format {
            0 | 2 => SizeHeader { regenerated: u32::from(first.wrapping_shr(3)), compressed: 0, header: 1 },
            1 => SizeHeader {
                regenerated: u32::from(first.wrapping_shr(4)) | byte(1)?.wrapping_shl(4),
                compressed: 0,
                header: 2,
            },
            _ => SizeHeader {
                regenerated: u32::from(first.wrapping_shr(4)) | byte(1)?.wrapping_shl(4) | byte(2)?.wrapping_shl(12),
                compressed: 0,
                header: 3,
            },
        };
        let count = header.regenerated as usize;
        let target = scratch.get_mut(..count).ok_or(ZstdError::BlockTooLarge { size: count })?;
        if kind == 0 {
            let raw = block.slice(header.header, count).map_err(|_| truncated)?;
            if copy_prefix(target, raw) != count {
                return Err(corrupt("raw literals"));
            }
            return Ok(Literals { count, used: header.header.wrapping_add(count) });
        }
        let repeated = block.u8(header.header).map_err(|_| truncated)?;
        target.fill(repeated);
        return Ok(Literals { count, used: header.header.wrapping_add(1) });
    }
    // Compressed (2) or treeless (3): sizes of 10, 14 or 18 bits.
    let header = match size_format {
        0 | 1 => {
            let bits = u32::from(first) | byte(1)?.wrapping_shl(8) | byte(2)?.wrapping_shl(16);
            SizeHeader { regenerated: bits.wrapping_shr(4) & 0x3FF, compressed: bits.wrapping_shr(14) & 0x3FF, header: 3 }
        }
        2 => {
            let bits = u32::from(first) | byte(1)?.wrapping_shl(8) | byte(2)?.wrapping_shl(16) | byte(3)?.wrapping_shl(24);
            SizeHeader { regenerated: bits.wrapping_shr(4) & 0x3FFF, compressed: bits.wrapping_shr(18) & 0x3FFF, header: 4 }
        }
        _ => {
            let low = u64::from(first)
                | u64::from(byte(1)?).wrapping_shl(8)
                | u64::from(byte(2)?).wrapping_shl(16)
                | u64::from(byte(3)?).wrapping_shl(24)
                | u64::from(byte(4)?).wrapping_shl(32);
            SizeHeader {
                regenerated: (low.wrapping_shr(4) & 0x3_FFFF) as u32,
                compressed: (low.wrapping_shr(22) & 0x3_FFFF) as u32,
                header: 5,
            }
        }
    };
    let count = header.regenerated as usize;
    if count > MAX_BLOCK {
        return Err(ZstdError::BlockTooLarge { size: count });
    }
    let compressed_len = header.compressed as usize;
    let body = block.sub(header.header, compressed_len).map_err(|_| truncated)?;
    let tree_used = if kind == 2 {
        state.huffman.read(body)?
    } else {
        match state.huffman.defined {
            Defined::Yes => 0,
            Defined::No => return Err(ZstdError::NoPreviousTable { what: "Huffman" }),
        }
    };
    let streams = body.tail(tree_used).map_err(|_| corrupt("literals"))?;
    let target = scratch.get_mut(..count).ok_or(ZstdError::BlockTooLarge { size: count })?;
    if size_format == 0 {
        state.huffman.decode_stream(streams, target)?;
    } else {
        decode_four_streams(&state.huffman, streams, target)?;
    }
    Ok(Literals {
        count,
        used: header.header.wrapping_add(compressed_len),
    })
}

/// A literals-section header's sizes and its own length.
struct SizeHeader {
    regenerated: u32,
    compressed: u32,
    header: usize,
}

/// Four Huffman streams behind a six-byte jump table (§3.1.1.3.1.6); each of
/// the first three regenerates a quarter (rounded up), the last the rest.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn decode_four_streams(table: &HuffmanTable, streams: Bytes<'_>, out: &mut [u8]) -> Result<(), ZstdError> {
    let sizes = [
        usize::from(streams.u16_le(0).map_err(|_| corrupt("jump table"))?),
        usize::from(streams.u16_le(2).map_err(|_| corrupt("jump table"))?),
        usize::from(streams.u16_le(4).map_err(|_| corrupt("jump table"))?),
    ];
    let segment = out.len().wrapping_add(3).wrapping_shr(2);
    let mut input_at = 6usize;
    let mut output_at = 0usize;
    for index in 0..4 {
        let input_len = match sizes.get(index) {
            Some(size) => *size,
            None => streams.len().checked_sub(input_at).ok_or(corrupt("jump table"))?,
        };
        let output_len = if index < 3 {
            segment
        } else {
            out.len().checked_sub(output_at).ok_or(corrupt("jump table"))?
        };
        let stream = streams.sub(input_at, input_len).map_err(|_| corrupt("jump table"))?;
        let end = output_at.checked_add(output_len).ok_or(corrupt("jump table"))?;
        let target = out.get_mut(output_at..end).ok_or(corrupt("jump table"))?;
        table.decode_stream(stream, target)?;
        input_at = input_at.wrapping_add(input_len);
        output_at = end;
    }
    Ok(())
}

/// The sequences section (§3.1.1.3.2) executed against `output`, with the
/// block's literals in `literals`.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn execute_sequences(
    section: Bytes<'_>,
    literals: &[u8],
    state: &mut FrameState,
    output: &mut Output<'_>,
) -> Result<(), ZstdError> {
    let truncated = ZstdError::Truncated { at: 0 };
    let first = section.u8(0).map_err(|_| truncated)?;
    let byte = |at: usize| section.u8(at).map(usize::from).map_err(|_| truncated);
    let header = match first {
        0 => {
            if section.len() != 1 {
                return Err(corrupt("sequences section"));
            }
            return output.put_all(literals);
        }
        1..=127 => SequenceCount { count: usize::from(first), used: 1 },
        128..=254 => SequenceCount {
            count: usize::from(first.wrapping_sub(128)).wrapping_shl(8) | byte(1)?,
            used: 2,
        },
        255 => SequenceCount {
            count: (byte(1)? | byte(2)?.wrapping_shl(8)).wrapping_add(0x7F00),
            used: 3,
        },
    };
    let modes = section.u8(header.used).map_err(|_| truncated)?;
    if modes & 3 != 0 {
        return Err(ZstdError::ReservedBit);
    }
    let mut at = header.used.wrapping_add(1);
    let tail = |at: usize| section.tail(at).map_err(|_| truncated);
    at = at.wrapping_add(read_table(modes.wrapping_shr(6), tail(at)?, &mut state.literal_lengths, LITERAL_LENGTHS)?);
    at = at.wrapping_add(read_table(modes.wrapping_shr(4) & 3, tail(at)?, &mut state.offsets, OFFSETS)?);
    at = at.wrapping_add(read_table(modes.wrapping_shr(2) & 3, tail(at)?, &mut state.match_lengths, MATCH_LENGTHS)?);
    let mut bits = BackwardBits::new(tail(at)?)?;
    let mut ll_state = bits.read(state.literal_lengths.log);
    let mut of_state = bits.read(state.offsets.log);
    let mut ml_state = bits.read(state.match_lengths.log);
    let mut literal_at = 0usize;
    for index in 0..header.count {
        let ll = state.literal_lengths.cell(ll_state)?;
        let of = state.offsets.cell(of_state)?;
        let ml = state.match_lengths.cell(ml_state)?;
        // §3.1.1.3.2.1.1: the offset's extra bits first, then the match
        // length's, then the literal length's.
        let offset_code = u32::from(of.symbol);
        if offset_code > MAX_OF_SYMBOL as u32 {
            return Err(corrupt("offset code"));
        }
        let offset_value = 1u64.wrapping_shl(offset_code).wrapping_add(u64::from(bits.read(offset_code)));
        let match_length = code_value(&ML_BASE, &ML_BITS, usize::from(ml.symbol), &mut bits, "match-length code")?;
        let literal_length = code_value(&LL_BASE, &LL_BITS, usize::from(ll.symbol), &mut bits, "literal-length code")?;
        let offset = resolve_offset(&mut state.repeats, offset_value, literal_length)?;
        if index.wrapping_add(1) < header.count {
            // §3.1.1.3.2.2: literal length, then match length, then offset.
            ll_state = u32::from(ll.base).wrapping_add(bits.read(u32::from(ll.bits)));
            ml_state = u32::from(ml.base).wrapping_add(bits.read(u32::from(ml.bits)));
            of_state = u32::from(of.base).wrapping_add(bits.read(u32::from(of.bits)));
        }
        if bits.overflowed() {
            return Err(corrupt("sequences bitstream"));
        }
        let literal_end = literal_at.checked_add(literal_length).ok_or(corrupt("literal length"))?;
        let copied = literals.get(literal_at..literal_end).ok_or(corrupt("literal length"))?;
        output.put_all(copied)?;
        literal_at = literal_end;
        output.copy_match(offset, match_length)?;
    }
    if !bits.finished() {
        return Err(corrupt("sequences bitstream"));
    }
    output.put_all(literals.get(literal_at..).unwrap_or(&[]))
}

/// A sequence count and the header bytes it took.
struct SequenceCount {
    count: usize,
    used: usize,
}

/// A length code's value: its base plus the extra bits that follow it.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn code_value(
    bases: &[u32],
    extra: &[u32],
    code: usize,
    bits: &mut BackwardBits<'_>,
    what: &'static str,
) -> Result<usize, ZstdError> {
    match (bases.get(code), extra.get(code)) {
        (Some(base), Some(count)) => Ok(base.wrapping_add(bits.read(*count)) as usize),
        (Some(_) | None, Some(_) | None) => Err(corrupt(what)),
    }
}

/// The match offset `offset_value` names, updating the repeat offsets
/// (§3.1.1.5).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn resolve_offset(repeats: &mut [usize; 3], offset_value: u64, literal_length: usize) -> Result<usize, ZstdError> {
    let [first, second, third] = *repeats;
    if offset_value > 3 {
        let offset = usize::try_from(offset_value.wrapping_sub(3)).map_err(|_| corrupt("offset"))?;
        *repeats = [offset, first, second];
        return Ok(offset);
    }
    // A literal length of 0 shifts the repeat codes by one.
    let code = if literal_length == 0 { offset_value.wrapping_add(1) } else { offset_value };
    let offset = match code {
        1 => first,
        2 => {
            *repeats = [second, first, third];
            second
        }
        3 => {
            *repeats = [third, first, second];
            third
        }
        _ => {
            let offset = first.checked_sub(1).filter(|offset| *offset != 0).ok_or(corrupt("repeat offset"))?;
            *repeats = [offset, first, second];
            offset
        }
    };
    Ok(offset)
}

/// A frame header's fields (§3.1.1.1).
struct FrameHeader {
    window: u64,
    content_size: Option<u64>,
    checksum: Checksum,
    used: usize,
}

/// Whether a frame ends with a content checksum.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Checksum {
    Present,
    Absent,
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn read_frame_header(frame: Bytes<'_>) -> Result<FrameHeader, ZstdError> {
    let truncated = ZstdError::Truncated { at: 4 };
    let descriptor = frame.u8(4).map_err(|_| truncated)?;
    let size_flag = descriptor.wrapping_shr(6);
    let single_segment = descriptor & 0x20 != 0;
    if descriptor & 0x08 != 0 {
        return Err(ZstdError::ReservedBit);
    }
    let checksum = if descriptor & 0x04 != 0 { Checksum::Present } else { Checksum::Absent };
    let mut at = 5usize;
    let mut window = 0u64;
    if !single_segment {
        let window_descriptor = frame.u8(at).map_err(|_| truncated)?;
        let log = 10u32.wrapping_add(u32::from(window_descriptor.wrapping_shr(3)));
        let base = 1u64.wrapping_shl(log);
        window = base.wrapping_add(base.wrapping_shr(3).wrapping_mul(u64::from(window_descriptor & 7)));
        at = at.wrapping_add(1);
    }
    let dictionary_bytes = match descriptor & 3 {
        0 => 0usize,
        1 => 1,
        2 => 2,
        _ => 4,
    };
    let mut id = 0u32;
    for index in 0..dictionary_bytes {
        let byte = frame.u8(at.wrapping_add(index)).map_err(|_| truncated)?;
        id |= u32::from(byte).wrapping_shl((index as u32).wrapping_mul(8));
    }
    if id != 0 {
        return Err(ZstdError::Dictionary { id });
    }
    at = at.wrapping_add(dictionary_bytes);
    let size_bytes = match size_flag {
        0 => usize::from(single_segment),
        1 => 2,
        2 => 4,
        _ => 8,
    };
    let content_size = if size_bytes == 0 {
        None
    } else {
        let mut value = 0u64;
        for index in 0..size_bytes {
            let byte = frame.u8(at.wrapping_add(index)).map_err(|_| truncated)?;
            value |= u64::from(byte).wrapping_shl((index as u32).wrapping_mul(8));
        }
        // §3.1.1.1.4: a two-byte size is stored minus 256.
        Some(if size_bytes == 2 { value.wrapping_add(256) } else { value })
    };
    at = at.wrapping_add(size_bytes);
    if single_segment {
        window = content_size.unwrap_or(0);
    }
    if window > MAX_WINDOW {
        return Err(ZstdError::WindowTooLarge { size: window });
    }
    Ok(FrameHeader { window, content_size, checksum, used: at })
}

/// Decodes one frame starting at `frame`'s first byte; returns the bytes of
/// input it used.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn decode_frame(frame: Bytes<'_>, output: &mut Output<'_>, scratch: &mut [u8]) -> Result<usize, ZstdError> {
    let header = read_frame_header(frame)?;
    let block_maximum = usize::try_from(header.window).map_or(MAX_BLOCK, |window| window.min(MAX_BLOCK));
    let mut state = FrameState::new();
    let mut at = header.used;
    loop {
        let truncated = ZstdError::Truncated { at };
        let raw = frame.array::<3>(at).map_err(|_| truncated)?;
        let [low, middle, high] = raw;
        let block_header = u32::from(low) | u32::from(middle).wrapping_shl(8) | u32::from(high).wrapping_shl(16);
        let last = block_header & 1 == 1;
        let kind = block_header.wrapping_shr(1) & 3;
        let size = block_header.wrapping_shr(3) as usize;
        at = at.wrapping_add(3);
        match kind {
            0 => {
                if size > MAX_BLOCK {
                    return Err(ZstdError::BlockTooLarge { size });
                }
                output.put_all(frame.slice(at, size).map_err(|_| truncated)?)?;
                at = at.wrapping_add(size);
            }
            1 => {
                if size > MAX_BLOCK {
                    return Err(ZstdError::BlockTooLarge { size });
                }
                let byte = frame.u8(at).map_err(|_| truncated)?;
                for _ in 0..size {
                    output.put(byte)?;
                }
                at = at.wrapping_add(1);
            }
            2 => {
                if size > block_maximum {
                    return Err(ZstdError::BlockTooLarge { size });
                }
                let block = frame.sub(at, size).map_err(|_| truncated)?;
                let literals = read_literals(block, scratch, &mut state)?;
                let section = block.tail(literals.used).map_err(|_| corrupt("sequences section"))?;
                let held = scratch.get(..literals.count).ok_or(corrupt("literals"))?;
                execute_sequences(section, held, &mut state, output)?;
                at = at.wrapping_add(size);
            }
            _ => return Err(ZstdError::ReservedBlockType),
        }
        if last {
            break;
        }
    }
    let produced = output.at.wrapping_sub(output.frame_start);
    if let Some(declared) = header.content_size {
        if declared != produced as u64 {
            return Err(ZstdError::ContentSize { declared, produced: produced as u64 });
        }
    }
    if header.checksum == Checksum::Present {
        let found = frame.u32_le(at).map_err(|_| ZstdError::Truncated { at })?;
        let content = output.out.get(output.frame_start..output.at).unwrap_or(&[]);
        let computed = (xxh64(content, 0) & 0xFFFF_FFFF) as u32;
        if found != computed {
            return Err(ZstdError::Checksum { found, computed });
        }
        at = at.wrapping_add(4);
    }
    Ok(at)
}

/// Decodes every frame of `input` into `out`, skipping skippable frames;
/// returns the bytes written. `scratch` must hold [`SCRATCH_BYTES`].
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decompress_frames(input: Bytes<'_>, out: &mut [u8], scratch: &mut [u8]) -> Result<usize, ZstdError> {
    if scratch.len() < SCRATCH_BYTES {
        return Err(ZstdError::ScratchTooSmall { needed: SCRATCH_BYTES, given: scratch.len() });
    }
    let mut output = Output { out, frame_start: 0, at: 0 };
    let mut at = 0usize;
    while at < input.len() {
        let rest = input.tail(at).map_err(|_| ZstdError::Truncated { at })?;
        let magic = rest.u32_le(0).map_err(|_| ZstdError::Truncated { at })?;
        if magic & SKIPPABLE_MASK == SKIPPABLE_MAGIC {
            let size = rest.u32_le(4).map_err(|_| ZstdError::Truncated { at })? as usize;
            let skipped = size.checked_add(8).ok_or(ZstdError::Truncated { at })?;
            if skipped > rest.len() {
                return Err(ZstdError::Truncated { at });
            }
            at = at.wrapping_add(skipped);
            continue;
        }
        if magic != MAGIC {
            return Err(ZstdError::BadMagic { found: magic });
        }
        output.frame_start = output.at;
        let used = decode_frame(rest, &mut output, scratch)?;
        at = at.checked_add(used).ok_or(ZstdError::Truncated { at })?;
    }
    Ok(output.at)
}

/// XXH64's round: one 8-byte lane into an accumulator.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn round(accumulator: u64, lane: u64) -> u64 {
    accumulator
        .wrapping_add(lane.wrapping_mul(PRIME64_2))
        .rotate_left(31)
        .wrapping_mul(PRIME64_1)
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn merge(hash: u64, accumulator: u64) -> u64 {
    (hash ^ round(0, accumulator)).wrapping_mul(PRIME64_1).wrapping_add(PRIME64_4)
}

/// XXH64 (the xxHash specification), whose low 32 bits are a zstd frame's
/// content checksum (§3.1.1, seed 0).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
#[must_use]
pub fn xxh64(data: &[u8], seed: u64) -> u64 {
    // Every lane is an exact 8-byte chunk, so the read cannot fail.
    let lane = |bytes: &[u8]| Bytes::new(bytes).u64_le(0).unwrap_or(0);
    let mut stripes = data.chunks_exact(32);
    let mut hash = if data.len() >= 32 {
        let mut accumulators = [
            seed.wrapping_add(PRIME64_1).wrapping_add(PRIME64_2),
            seed.wrapping_add(PRIME64_2),
            seed,
            seed.wrapping_sub(PRIME64_1),
        ];
        for stripe in stripes.by_ref() {
            let stripe = Bytes::new(stripe);
            for (index, accumulator) in accumulators.iter_mut().enumerate() {
                *accumulator = round(*accumulator, stripe.u64_le(index.wrapping_mul(8)).unwrap_or(0));
            }
        }
        let [a, b, c, d] = accumulators;
        let mut hash = a
            .rotate_left(1)
            .wrapping_add(b.rotate_left(7))
            .wrapping_add(c.rotate_left(12))
            .wrapping_add(d.rotate_left(18));
        for accumulator in accumulators {
            hash = merge(hash, accumulator);
        }
        hash
    } else {
        seed.wrapping_add(PRIME64_5)
    };
    hash = hash.wrapping_add(data.len() as u64);
    let rest = stripes.remainder();
    let mut words = rest.chunks_exact(8);
    for word in words.by_ref() {
        hash ^= round(0, lane(word));
        hash = hash.rotate_left(27).wrapping_mul(PRIME64_1).wrapping_add(PRIME64_4);
    }
    let mut tail = words.remainder();
    if tail.len() >= 4 {
        let half = Bytes::new(tail).u32_le(0).unwrap_or(0);
        hash ^= u64::from(half).wrapping_mul(PRIME64_1);
        hash = hash.rotate_left(23).wrapping_mul(PRIME64_2).wrapping_add(PRIME64_3);
        tail = tail.get(4..).unwrap_or(&[]);
    }
    for byte in tail {
        hash ^= u64::from(*byte).wrapping_mul(PRIME64_5);
        hash = hash.rotate_left(11).wrapping_mul(PRIME64_1);
    }
    hash ^= hash.wrapping_shr(33);
    hash = hash.wrapping_mul(PRIME64_2);
    hash ^= hash.wrapping_shr(29);
    hash = hash.wrapping_mul(PRIME64_3);
    hash ^ hash.wrapping_shr(32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_symbols_fixtures::compress::{sample_binary, sample_text, zstd_frame, Checksum, ZstdOptions};

    const MAGIC_BYTES: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

    /// A single-segment frame with a one-byte content size and one block.
    fn frame(content_size: u8, block_header: [u8; 3], block: &[u8]) -> Vec<u8> {
        let mut frame = MAGIC_BYTES.to_vec();
        frame.push(0x20);
        frame.push(content_size);
        frame.extend_from_slice(&block_header);
        frame.extend_from_slice(block);
        frame
    }

    /// Literals `abc`, then one sequence: 3 literals, offset code
    /// `offset_code`, match length 3; the bitstream `bitstream`. The frame
    /// declares a 1 KiB window (descriptor 0) and no content size: a
    /// single-segment frame's window is its content size, 6 bytes here,
    /// which a 10-byte compressed block may not exceed.
    fn one_sequence(offset_code: u8, bitstream: u8) -> Vec<u8> {
        let block = [0x18, b'a', b'b', b'c', 0x01, 0x54, 0x03, offset_code, 0x00, bitstream];
        let mut frame = MAGIC_BYTES.to_vec();
        frame.extend_from_slice(&[0x00, 0x00]);
        // Last block, compressed (type 2), 10 bytes: 1 | 2 << 1 | 10 << 3.
        frame.extend_from_slice(&[0x55, 0x00, 0x00]);
        frame.extend_from_slice(&block);
        frame
    }

    fn decode(input: &[u8], capacity: usize) -> Result<Vec<u8>, ZstdError> {
        let mut out = vec![0u8; capacity];
        let mut scratch = vec![0u8; SCRATCH_BYTES];
        let written = decompress_frames(Bytes::new(input), &mut out, &mut scratch)?;
        out.truncate(written);
        Ok(out)
    }

    #[test]
    fn xxh64_matches_its_published_values() {
        assert_eq!(xxh64(b"", 0), 0xEF46_DB37_51D8_E999);
        assert_eq!(xxh64(b"abc", 0), 0x44BC_2CF5_AD77_0999);
    }

    #[test]
    fn raw_and_rle_blocks_decode() {
        // Last block, raw, 5 bytes: 1 | 0 << 1 | 5 << 3 = 0x29.
        assert_eq!(decode(&frame(5, [0x29, 0, 0], b"hello"), 16).unwrap(), b"hello");
        // Last block, RLE, 4 bytes: 1 | 1 << 1 | 4 << 3 = 0x23.
        assert_eq!(decode(&frame(4, [0x23, 0, 0], b"z"), 16).unwrap(), b"zzzz");
    }

    #[test]
    fn a_skippable_frame_is_skipped() {
        let mut input = vec![0x50, 0x2A, 0x4D, 0x18, 3, 0, 0, 0, 1, 2, 3];
        input.extend_from_slice(&frame(5, [0x29, 0, 0], b"hello"));
        assert_eq!(decode(&input, 16).unwrap(), b"hello");
    }

    #[test]
    fn a_hand_built_sequence_copies_its_match() {
        assert_eq!(decode(&one_sequence(0x02, 0x06), 16).unwrap(), b"abcabc");
    }

    /// Review focus 3: a back-reference before the start of the output.
    #[test]
    fn a_match_before_the_start_of_the_output_is_refused() {
        assert_eq!(
            decode(&one_sequence(0x03, 0x08), 16),
            Err(ZstdError::OffsetTooFar { offset: 5, available: 3 })
        );
    }

    #[test]
    fn a_dictionary_or_an_oversized_window_is_refused() {
        let mut with_dictionary = MAGIC_BYTES.to_vec();
        with_dictionary.extend_from_slice(&[0x21, 0x07, 0x05, 0x29, 0, 0]);
        with_dictionary.extend_from_slice(b"hello");
        assert_eq!(decode(&with_dictionary, 16), Err(ZstdError::Dictionary { id: 7 }));
        // Not single-segment, no content size; window log 10 + 18 = 28.
        let mut huge_window = MAGIC_BYTES.to_vec();
        huge_window.extend_from_slice(&[0x00, 18 << 3, 0x29, 0, 0]);
        huge_window.extend_from_slice(b"hello");
        assert_eq!(decode(&huge_window, 16), Err(ZstdError::WindowTooLarge { size: 1 << 28 }));
    }

    #[test]
    fn frames_from_the_reference_encoder_decode_exactly() {
        let inputs = [
            Vec::new(),
            sample_text(1),
            sample_text(300_000),
            sample_binary(400_000, 3),
            vec![0u8; 200_000],
        ];
        let settings = [
            ZstdOptions { level: 1, checksum: Checksum::Without, window_log: 0 },
            ZstdOptions { level: 3, checksum: Checksum::With, window_log: 0 },
            ZstdOptions { level: 9, checksum: Checksum::With, window_log: 0 },
            ZstdOptions { level: 19, checksum: Checksum::With, window_log: 0 },
            ZstdOptions { level: 3, checksum: Checksum::With, window_log: 17 },
        ];
        for data in &inputs {
            for options in settings {
                let compressed = zstd_frame(data, options);
                assert_eq!(&decode(&compressed, data.len()).unwrap(), data, "{options:?}, {} bytes", data.len());
            }
        }
    }

    #[test]
    fn concatenated_frames_decode_one_after_the_other() {
        let options = ZstdOptions { level: 3, checksum: Checksum::With, window_log: 0 };
        let first = sample_text(50_000);
        let second = sample_binary(30_000, 9);
        let mut input = zstd_frame(&first, options);
        input.extend_from_slice(&zstd_frame(&second, options));
        let mut expected = first.clone();
        expected.extend_from_slice(&second);
        assert_eq!(decode(&input, expected.len()).unwrap(), expected);
    }

    #[test]
    fn a_wrong_checksum_or_content_size_is_refused() {
        let data = sample_text(10_000);
        let mut compressed = zstd_frame(&data, ZstdOptions { level: 3, checksum: Checksum::With, window_log: 0 });
        let last = compressed.len() - 1;
        compressed[last] ^= 0xFF;
        assert!(matches!(decode(&compressed, data.len()), Err(ZstdError::Checksum { .. })));
        assert_eq!(decode(&frame(6, [0x29, 0, 0], b"hello"), 16), Err(ZstdError::ContentSize { declared: 6, produced: 5 }));
    }

    #[test]
    fn an_output_too_small_or_input_cut_short_is_refused() {
        let data = sample_text(10_000);
        let compressed = zstd_frame(&data, ZstdOptions { level: 3, checksum: Checksum::With, window_log: 0 });
        assert!(matches!(decode(&compressed, 9_999), Err(ZstdError::OutputFull { .. })));
        assert!(decode(&compressed[..compressed.len() / 2], data.len()).is_err());
        let mut out = [0u8; 16];
        let mut scratch = [0u8; 16];
        assert_eq!(
            decompress_frames(Bytes::new(&frame(5, [0x29, 0, 0], b"hello")), &mut out, &mut scratch),
            Err(ZstdError::ScratchTooSmall { needed: SCRATCH_BYTES, given: 16 })
        );
    }
}
