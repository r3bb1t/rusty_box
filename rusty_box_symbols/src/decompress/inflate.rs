//! DEFLATE (RFC 1951) and its zlib wrapper (RFC 1950), after Mark Adler's
//! `puff.c`: canonical Huffman codes decoded one bit at a time from tables
//! that are fixed-size arrays. Every function here is tier P (spec §15): it
//! allocates nothing and is proven panic-free at link time in this crate's
//! release test build (`read`'s module doc says why only there). The output
//! buffer is the window: a match may reach anywhere back into what is
//! already in it, history included, and nowhere before it.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use crate::read::Bytes;

/// The longest Huffman code DEFLATE allows.
const MAX_BITS: usize = 15;
/// RFC 1951 §3.2.7: at most 286 literal/length and 30 distance codes.
const MAX_LITLEN_CODES: usize = 286;
const MAX_DIST_CODES: usize = 30;
/// The fixed literal/length code has 288 symbols (286 and 287 never occur).
const FIXED_LITLEN_CODES: usize = 288;
const CODE_LENGTH_CODES: usize = 19;
const MAX_CODE_LENGTHS: usize = MAX_LITLEN_CODES + MAX_DIST_CODES;
/// The end-of-block symbol.
const END_OF_BLOCK: u16 = 256;
/// Adler-32's modulus (RFC 1950 §9): the largest prime below 2^16.
const ADLER_MODULUS: u32 = 65_521;

/// RFC 1951 §3.2.5: lengths 257..285.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// RFC 1951 §3.2.5: distances 0..29.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// RFC 1951 §3.2.7: the order code-length code lengths are stored in.
const CODE_LENGTH_ORDER: [usize; CODE_LENGTH_CODES] =
    [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// Why a stream did not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InflateError {
    #[error("the stream ends inside a block (bit {bit})")]
    Truncated { bit: usize },
    #[error("block type 3 is reserved")]
    ReservedBlockType,
    #[error("a stored block's length {len:#06x} does not match its complement {complement:#06x}")]
    StoredLength { len: u16, complement: u16 },
    #[error("a dynamic block declares {literals} literal/length and {distances} distance codes")]
    TooManyCodes { literals: usize, distances: usize },
    #[error("the {what} code is over-subscribed")]
    OverSubscribed { what: &'static str },
    #[error("the {what} code is incomplete")]
    Incomplete { what: &'static str },
    #[error("a code-length repeat has no previous length")]
    RepeatWithoutPrevious,
    #[error("code lengths run past the {count} declared")]
    LengthsOverrun { count: usize },
    #[error("the block has no end-of-block code")]
    NoEndOfBlock,
    #[error("no code matches the bits before bit {bit}")]
    NoSuchCode { bit: usize },
    #[error("symbol {symbol} is not a valid {what}")]
    BadSymbol { what: &'static str, symbol: u16 },
    #[error("a match reaches {distance} bytes back with only {available} written")]
    DistanceTooFar { distance: usize, available: usize },
    #[error("the output is full at {capacity} bytes")]
    OutputFull { capacity: usize },
    #[error("the history start {start} is past the output's {capacity} bytes")]
    BadStart { start: usize, capacity: usize },
    #[error("the zlib header {cmf:#04x} {flg:#04x} is not a deflate stream without a dictionary")]
    BadZlibHeader { cmf: u8, flg: u8 },
    #[error("the Adler-32 checksum {found:#010x} is not the data's {computed:#010x}")]
    BadAdler { found: u32, computed: u32 },
}

/// What a stream decoded to: the input bytes it used (its last byte counted
/// whole) and the output position after its last byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct Inflated {
    pub consumed: usize,
    pub end: usize,
}

/// Bits read least significant first (RFC 1951 §3.1.1).
struct Bits<'a> {
    input: Bytes<'a>,
    bit: usize,
}

impl<'a> Bits<'a> {
    fn new(input: Bytes<'a>) -> Self {
        Bits { input, bit: 0 }
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn truncated(&self) -> InflateError {
        InflateError::Truncated { bit: self.bit }
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn bit(&mut self) -> Result<u32, InflateError> {
        let byte = self.input.u8(self.bit.wrapping_shr(3)).map_err(|_| self.truncated())?;
        let value = u32::from(byte.wrapping_shr((self.bit & 7) as u32) & 1);
        self.bit = self.bit.checked_add(1).ok_or(self.truncated())?;
        Ok(value)
    }

    /// `count` bits (at most 16), the first read the least significant.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn bits(&mut self, count: u32) -> Result<u32, InflateError> {
        let mut value = 0u32;
        for index in 0..count.min(16) {
            value |= self.bit()?.wrapping_shl(index);
        }
        Ok(value)
    }

    /// The offset of the first byte not yet touched.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn byte_position(&self) -> usize {
        self.bit.saturating_add(7).wrapping_shr(3)
    }

    /// Skips to the next byte boundary (stored blocks start on one).
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn align_to_byte(&mut self) -> Result<(), InflateError> {
        self.bit = self.byte_position().checked_mul(8).ok_or(self.truncated())?;
        Ok(())
    }
}

/// A canonical Huffman code over `N` symbols: how many codes have each
/// length, and the symbols in code order.
struct Huffman<const N: usize> {
    count: [u16; MAX_BITS + 1],
    symbol: [u16; N],
}

/// A built code and how many code slots it leaves unused: 0 is complete;
/// more is incomplete, which DEFLATE allows only for a lone code.
struct Built<const N: usize> {
    code: Huffman<N>,
    unused_slots: i32,
    used_symbols: usize,
}

impl<const N: usize> Huffman<N> {
    /// The code whose lengths are `lengths` (puff's `construct`).
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn build(lengths: &[u8], what: &'static str) -> Result<Built<N>, InflateError> {
        let mut code = Huffman {
            count: [0; MAX_BITS + 1],
            symbol: [0; N],
        };
        for length in lengths {
            let slot = code
                .count
                .get_mut(usize::from(*length))
                .ok_or(InflateError::BadSymbol { what, symbol: u16::from(*length) })?;
            *slot = slot.wrapping_add(1);
        }
        let unused_lengths = usize::from(code.count.first().copied().unwrap_or(0));
        let used_symbols = lengths.len().saturating_sub(unused_lengths);
        if used_symbols == 0 {
            // No codes: complete, though any decode with it fails.
            return Ok(Built { code, unused_slots: 0, used_symbols });
        }
        let mut unused_slots: i32 = 1;
        for length in 1..=MAX_BITS {
            let count = i32::from(code.count.get(length).copied().unwrap_or(0));
            unused_slots = unused_slots.wrapping_mul(2).wrapping_sub(count);
            if unused_slots < 0 {
                return Err(InflateError::OverSubscribed { what });
            }
        }
        let mut offsets = [0u16; MAX_BITS + 2];
        for length in 1..=MAX_BITS {
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
            let offset = offsets
                .get_mut(usize::from(*length))
                .ok_or(InflateError::BadSymbol { what, symbol: u16::from(*length) })?;
            let slot = code
                .symbol
                .get_mut(usize::from(*offset))
                .ok_or(InflateError::OverSubscribed { what })?;
            *slot = u16::try_from(symbol).map_err(|_| InflateError::OverSubscribed { what })?;
            *offset = offset.wrapping_add(1);
        }
        Ok(Built { code, unused_slots, used_symbols })
    }

    /// The next symbol (puff's `decode`). The canonical order keeps
    /// `code >= first` at every step, and every value stays below 2^16.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, InflateError> {
        let mut code = 0u32;
        let mut first = 0u32;
        let mut index = 0u32;
        for length in 1..=MAX_BITS {
            code |= bits.bit()?;
            let count = u32::from(self.count.get(length).copied().unwrap_or(0));
            if code < first.wrapping_add(count) {
                let at = index.wrapping_add(code.wrapping_sub(first));
                return self
                    .symbol
                    .get(at as usize)
                    .copied()
                    .ok_or(InflateError::NoSuchCode { bit: bits.bit });
            }
            index = index.wrapping_add(count);
            first = first.wrapping_add(count).wrapping_shl(1);
            code = code.wrapping_shl(1);
        }
        Err(InflateError::NoSuchCode { bit: bits.bit })
    }
}

/// Requires `built` to be complete, or incomplete with a single code (RFC
/// 1951 permits one distance code of length 1; puff permits the same for
/// literal/length codes).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn usable<const N: usize>(built: Built<N>, what: &'static str) -> Result<Huffman<N>, InflateError> {
    if built.unused_slots > 0 && built.used_symbols != 1 {
        return Err(InflateError::Incomplete { what });
    }
    Ok(built.code)
}

/// The output and where the next byte goes.
struct Output<'o> {
    out: &'o mut [u8],
    at: usize,
}

impl Output<'_> {
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn put(&mut self, byte: u8) -> Result<(), InflateError> {
        let capacity = self.out.len();
        let slot = self.out.get_mut(self.at).ok_or(InflateError::OutputFull { capacity })?;
        *slot = byte;
        // `at` indexed a slot, so it is below `usize::MAX`.
        self.at = self.at.wrapping_add(1);
        Ok(())
    }

    /// Copies `length` bytes from `distance` back, a byte at a time, so a
    /// match may overlap the bytes it is writing.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn copy_match(&mut self, distance: usize, length: usize) -> Result<(), InflateError> {
        if distance == 0 || distance > self.at {
            return Err(InflateError::DistanceTooFar { distance, available: self.at });
        }
        for _ in 0..length {
            let byte = self
                .out
                .get(self.at.wrapping_sub(distance))
                .copied()
                .ok_or(InflateError::DistanceTooFar { distance, available: self.at })?;
            self.put(byte)?;
        }
        Ok(())
    }
}

/// A stored block (RFC 1951 §3.2.4): byte-aligned LEN, NLEN, then LEN bytes.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn stored(bits: &mut Bits<'_>, out: &mut Output<'_>) -> Result<(), InflateError> {
    bits.align_to_byte()?;
    let at = bits.byte_position();
    let len = bits.input.u16_le(at).map_err(|_| bits.truncated())?;
    let complement = bits.input.u16_le(at.wrapping_add(2)).map_err(|_| bits.truncated())?;
    if complement != !len {
        return Err(InflateError::StoredLength { len, complement });
    }
    let body_at = at.checked_add(4).ok_or(bits.truncated())?;
    let body = bits.input.slice(body_at, usize::from(len)).map_err(|_| bits.truncated())?;
    for byte in body {
        out.put(*byte)?;
    }
    let after = body_at.checked_add(usize::from(len)).ok_or(bits.truncated())?;
    bits.bit = after.checked_mul(8).ok_or(bits.truncated())?;
    Ok(())
}

/// The literal/length and distance symbols of one block, to its end code.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn codes(
    bits: &mut Bits<'_>,
    out: &mut Output<'_>,
    litlen: &Huffman<FIXED_LITLEN_CODES>,
    distance: &Huffman<MAX_DIST_CODES>,
) -> Result<(), InflateError> {
    loop {
        let symbol = litlen.decode(bits)?;
        if symbol < END_OF_BLOCK {
            out.put(symbol.to_le_bytes()[0])?;
            continue;
        }
        if symbol == END_OF_BLOCK {
            return Ok(());
        }
        let index = usize::from(symbol.wrapping_sub(257));
        let length_code = match (LENGTH_BASE.get(index), LENGTH_EXTRA.get(index)) {
            (Some(base), Some(extra)) => BaseAndExtra { base: *base, extra: *extra },
            (Some(_) | None, Some(_) | None) => {
                return Err(InflateError::BadSymbol { what: "literal/length", symbol })
            }
        };
        let length = length_code.value(bits)?;
        let distance_symbol = distance.decode(bits)?;
        let index = usize::from(distance_symbol);
        let distance_code = match (DIST_BASE.get(index), DIST_EXTRA.get(index)) {
            (Some(base), Some(extra)) => BaseAndExtra { base: *base, extra: *extra },
            (Some(_) | None, Some(_) | None) => {
                return Err(InflateError::BadSymbol { what: "distance", symbol: distance_symbol })
            }
        };
        let back = distance_code.value(bits)?;
        out.copy_match(back, length)?;
    }
}

/// A length or distance symbol's base value and its count of extra bits.
struct BaseAndExtra {
    base: u16,
    extra: u32,
}

impl BaseAndExtra {
    /// The base plus the extra bits that follow the symbol.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn value(&self, bits: &mut Bits<'_>) -> Result<usize, InflateError> {
        Ok(usize::from(self.base).wrapping_add(bits.bits(self.extra)? as usize))
    }
}

/// A block coded with the fixed codes of RFC 1951 §3.2.6.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn fixed(bits: &mut Bits<'_>, out: &mut Output<'_>) -> Result<(), InflateError> {
    let mut lengths = [8u8; FIXED_LITLEN_CODES];
    for (symbol, length) in lengths.iter_mut().enumerate() {
        *length = match symbol {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let litlen = Huffman::<FIXED_LITLEN_CODES>::build(&lengths, "fixed literal/length")?.code;
    // Thirty 5-bit codes leave two slots unused; RFC 1951 defines them so.
    let distance = Huffman::<MAX_DIST_CODES>::build(&[5u8; MAX_DIST_CODES], "fixed distance")?.code;
    codes(bits, out, &litlen, &distance)
}

/// A repeat in a dynamic block's code lengths.
struct Repeat {
    length: u8,
    times: usize,
}

/// A block whose codes are described at its start (RFC 1951 §3.2.7).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn dynamic(bits: &mut Bits<'_>, out: &mut Output<'_>) -> Result<(), InflateError> {
    let literals = (bits.bits(5)? as usize).wrapping_add(257);
    let distances = (bits.bits(5)? as usize).wrapping_add(1);
    let code_lengths = (bits.bits(4)? as usize).wrapping_add(4);
    if literals > MAX_LITLEN_CODES || distances > MAX_DIST_CODES {
        return Err(InflateError::TooManyCodes { literals, distances });
    }
    let mut length_lengths = [0u8; CODE_LENGTH_CODES];
    for order in CODE_LENGTH_ORDER.iter().take(code_lengths) {
        if let Some(slot) = length_lengths.get_mut(*order) {
            *slot = bits.bits(3)?.to_le_bytes()[0];
        }
    }
    let built = Huffman::<CODE_LENGTH_CODES>::build(&length_lengths, "code length")?;
    if built.unused_slots != 0 {
        return Err(InflateError::Incomplete { what: "code length" });
    }
    let length_code = built.code;
    let total = literals.wrapping_add(distances);
    let mut lengths = [0u8; MAX_CODE_LENGTHS];
    let mut index = 0usize;
    while index < total {
        let symbol = length_code.decode(bits)?;
        let repeat = match symbol {
            0..=15 => Repeat { length: symbol.to_le_bytes()[0], times: 1 },
            16 => {
                let previous = index
                    .checked_sub(1)
                    .and_then(|at| lengths.get(at))
                    .copied()
                    .ok_or(InflateError::RepeatWithoutPrevious)?;
                Repeat { length: previous, times: (bits.bits(2)? as usize).wrapping_add(3) }
            }
            17 => Repeat { length: 0, times: (bits.bits(3)? as usize).wrapping_add(3) },
            18 => Repeat { length: 0, times: (bits.bits(7)? as usize).wrapping_add(11) },
            other => return Err(InflateError::BadSymbol { what: "code length", symbol: other }),
        };
        for _ in 0..repeat.times {
            let slot = lengths
                .get_mut(index)
                .filter(|_| index < total)
                .ok_or(InflateError::LengthsOverrun { count: total })?;
            *slot = repeat.length;
            index = index.wrapping_add(1);
        }
    }
    if lengths.get(usize::from(END_OF_BLOCK)).copied().unwrap_or(0) == 0 {
        return Err(InflateError::NoEndOfBlock);
    }
    let literal_lengths = lengths.get(..literals).unwrap_or(&[]);
    let distance_lengths = lengths.get(literals..total).unwrap_or(&[]);
    let litlen = usable(
        Huffman::<FIXED_LITLEN_CODES>::build(literal_lengths, "literal/length")?,
        "literal/length",
    )?;
    let distance = usable(Huffman::<MAX_DIST_CODES>::build(distance_lengths, "distance")?, "distance")?;
    codes(bits, out, &litlen, &distance)
}

/// Decodes one raw DEFLATE stream into `out[start..]`; matches may reach
/// back into `out[..start]`. Stops after the block marked final.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn inflate(input: Bytes<'_>, out: &mut [u8], start: usize) -> Result<Inflated, InflateError> {
    let capacity = out.len();
    if start > capacity {
        return Err(InflateError::BadStart { start, capacity });
    }
    let mut bits = Bits::new(input);
    let mut output = Output { out, at: start };
    loop {
        let last = bits.bit()?;
        match bits.bits(2)? {
            0 => stored(&mut bits, &mut output)?,
            1 => fixed(&mut bits, &mut output)?,
            2 => dynamic(&mut bits, &mut output)?,
            _ => return Err(InflateError::ReservedBlockType),
        }
        if last == 1 {
            return Ok(Inflated {
                consumed: bits.byte_position(),
                end: output.at,
            });
        }
    }
}

/// Adler-32 (RFC 1950 §9).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
#[must_use]
pub fn adler32(data: &[u8]) -> u32 {
    let mut low = 1u32;
    let mut high = 0u32;
    for byte in data {
        low = low.wrapping_add(u32::from(*byte)).wrapping_rem(ADLER_MODULUS);
        high = high.wrapping_add(low).wrapping_rem(ADLER_MODULUS);
    }
    high.wrapping_shl(16) | low
}

/// A zlib stream (RFC 1950): a deflate header without a preset dictionary,
/// a raw DEFLATE stream, and the Adler-32 of the output, big-endian.
/// Returns the number of bytes written to `out`.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn inflate_zlib(input: Bytes<'_>, out: &mut [u8]) -> Result<usize, InflateError> {
    let cmf = input.u8(0).map_err(|_| InflateError::Truncated { bit: 0 })?;
    let flg = input.u8(1).map_err(|_| InflateError::Truncated { bit: 8 })?;
    let check = u16::from(cmf).wrapping_shl(8) | u16::from(flg);
    let deflate = cmf & 0x0F == 8 && cmf.wrapping_shr(4) <= 7;
    let dictionary = flg & 0x20 != 0;
    if !deflate || dictionary || check.wrapping_rem(31) != 0 {
        return Err(InflateError::BadZlibHeader { cmf, flg });
    }
    let body = input.tail(2).map_err(|_| InflateError::Truncated { bit: 16 })?;
    let done = inflate(body, out, 0)?;
    let trailer = body
        .array::<4>(done.consumed)
        .map_err(|_| InflateError::Truncated { bit: done.consumed.saturating_mul(8) })?;
    let found = u32::from_be_bytes(trailer);
    let computed = adler32(out.get(..done.end).unwrap_or(&[]));
    if found != computed {
        return Err(InflateError::BadAdler { found, computed });
    }
    Ok(done.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_symbols_fixtures::compress::{raw_deflate, sample_binary, sample_text, zlib};

    /// A fixed block whose only content is "copy 5 bytes from 5 back".
    const MATCH_FIVE_BACK: [u8; 3] = [0x03, 0x13, 0x00];

    #[test]
    fn a_stored_block_is_copied() {
        let stream = [0x01, 0x05, 0x00, 0xFA, 0xFF, b'h', b'e', b'l', b'l', b'o'];
        let mut out = [0u8; 5];
        let done = inflate(Bytes::new(&stream), &mut out, 0).unwrap();
        assert_eq!(done, Inflated { consumed: 10, end: 5 });
        assert_eq!(&out, b"hello");
    }

    #[test]
    fn a_fixed_block_from_zlib_decodes_with_its_checksum() {
        let stream = [0x78, 0x9C, 0x4B, 0x04, 0x00, 0x00, 0x62, 0x00, 0x62];
        let mut out = [0u8; 1];
        assert_eq!(inflate_zlib(Bytes::new(&stream), &mut out), Ok(1));
        assert_eq!(&out, b"a");
        assert_eq!(adler32(b"a"), 0x0062_0062);
    }

    #[test]
    fn a_match_may_reach_into_the_history_before_start() {
        let mut out = *b"hello\0\0\0\0\0";
        let done = inflate(Bytes::new(&MATCH_FIVE_BACK), &mut out, 5).unwrap();
        assert_eq!(done.end, 10);
        assert_eq!(&out, b"hellohello");
    }

    /// Review focus 3: a back-reference before the start of the output.
    #[test]
    fn a_match_before_the_start_of_the_output_is_refused() {
        let mut out = [0u8; 10];
        assert_eq!(
            inflate(Bytes::new(&MATCH_FIVE_BACK), &mut out, 0),
            Err(InflateError::DistanceTooFar { distance: 5, available: 0 })
        );
        assert_eq!(
            inflate(Bytes::new(&MATCH_FIVE_BACK), &mut out, 11),
            Err(InflateError::BadStart { start: 11, capacity: 10 })
        );
    }

    #[test]
    fn streams_from_the_reference_encoder_decode_exactly() {
        let inputs = [
            Vec::new(),
            sample_text(1),
            sample_text(70_000),
            sample_binary(200_000, 7),
            vec![0u8; 100_000],
        ];
        for data in &inputs {
            for level in [0, 1, 6, 9] {
                let mut out = vec![0u8; data.len()];
                let stream = raw_deflate(data, level);
                let done = inflate(Bytes::new(&stream), &mut out, 0).unwrap();
                assert_eq!(done.end, data.len(), "level {level}");
                assert_eq!(done.consumed, stream.len(), "level {level}");
                assert_eq!(&out, data, "level {level}");
                let wrapped = zlib(data, level);
                assert_eq!(inflate_zlib(Bytes::new(&wrapped), &mut out), Ok(data.len()));
                assert_eq!(&out, data, "zlib level {level}");
            }
        }
    }

    #[test]
    fn a_reserved_block_type_is_refused() {
        let mut out = [0u8; 4];
        assert_eq!(inflate(Bytes::new(&[0x07]), &mut out, 0), Err(InflateError::ReservedBlockType));
    }

    #[test]
    fn a_stored_length_that_does_not_match_its_complement_is_refused() {
        let mut out = [0u8; 8];
        assert_eq!(
            inflate(Bytes::new(&[0x01, 0x05, 0x00, 0x00, 0x00]), &mut out, 0),
            Err(InflateError::StoredLength { len: 5, complement: 0 })
        );
    }

    #[test]
    fn a_stream_cut_short_is_refused() {
        let data = sample_text(5000);
        let stream = raw_deflate(&data, 6);
        let mut out = vec![0u8; data.len()];
        let cut = &stream[..stream.len() / 2];
        assert!(matches!(inflate(Bytes::new(cut), &mut out, 0), Err(InflateError::Truncated { .. })));
    }

    #[test]
    fn an_output_too_small_is_refused() {
        let data = sample_text(5000);
        let stream = raw_deflate(&data, 6);
        let mut out = vec![0u8; 4999];
        assert_eq!(inflate(Bytes::new(&stream), &mut out, 0), Err(InflateError::OutputFull { capacity: 4999 }));
    }

    #[test]
    fn a_bad_zlib_header_or_checksum_is_refused() {
        let mut out = [0u8; 1];
        assert!(matches!(
            inflate_zlib(Bytes::new(&[0x78, 0x9D, 0x4B, 0x04, 0x00]), &mut out),
            Err(InflateError::BadZlibHeader { .. })
        ));
        let wrong_sum = [0x78, 0x9C, 0x4B, 0x04, 0x00, 0x00, 0x62, 0x00, 0x63];
        assert_eq!(
            inflate_zlib(Bytes::new(&wrong_sum), &mut out),
            Err(InflateError::BadAdler { found: 0x0062_0063, computed: 0x0062_0062 })
        );
    }
}
