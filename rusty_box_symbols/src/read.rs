//! The one way this crate reads bytes it did not make. Every read checks its
//! bounds and returns a [`ReadError`] naming where it failed; nothing
//! indexes, slices or adds without a check. Every public function here is a
//! tier-P function (spec §15): it allocates nothing, and in this crate's
//! release test build `#[no_panic]` makes the link fail if any path through
//! it can reach a panic. Every other build leaves the attribute out: a debug
//! build because the proof needs the optimiser, and a crate that depends on
//! this one because the proof holds only where a function is compiled in its
//! own crate (a generic caller instantiated elsewhere inlines it without the
//! facts that prove its callees do not unwind, and its link fails).

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

/// Why a read failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// `len` bytes at `at` reach past the end of the input.
    #[error("{len} bytes at offset {at:#x} reach past the end of the input")]
    Truncated { at: usize, len: usize },
    /// The LEB128 number at `at` does not fit 64 bits.
    #[error("the LEB128 number at offset {at:#x} does not fit 64 bits")]
    LebOverflow { at: usize },
    /// No NUL within `max` bytes from `at`.
    #[error("no NUL within {max} bytes of offset {at:#x}")]
    Unterminated { at: usize, max: usize },
}

/// A LEB128 number and the offset just past it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct Leb<T> {
    pub value: T,
    pub next: usize,
}

/// A borrowed input whose every read is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bytes<'a> {
    bytes: &'a [u8],
}

/// LEB128 numbers longer than this many bytes cannot fit 64 bits
/// (`ceil(64 / 7)`).
const MAX_LEB_BYTES: u32 = 10;

impl<'a> Bytes<'a> {
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Bytes<'a> {
        Bytes { bytes }
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.bytes.len()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }

    #[must_use]
    pub const fn as_slice(self) -> &'a [u8] {
        self.bytes
    }

    /// `len` bytes at `at`.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn slice(self, at: usize, len: usize) -> Result<&'a [u8], ReadError> {
        let truncated = ReadError::Truncated { at, len };
        let end = at.checked_add(len).ok_or(truncated)?;
        self.bytes.get(at..end).ok_or(truncated)
    }

    /// `len` bytes at `at`, as an input of their own (offsets restart at 0).
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn sub(self, at: usize, len: usize) -> Result<Bytes<'a>, ReadError> {
        self.slice(at, len).map(Bytes::new)
    }

    /// Everything from `at` to the end; `at == len` gives an empty input.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn tail(self, at: usize) -> Result<Bytes<'a>, ReadError> {
        self.bytes
            .get(at..)
            .map(Bytes::new)
            .ok_or(ReadError::Truncated { at, len: 0 })
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn array<const N: usize>(self, at: usize) -> Result<[u8; N], ReadError> {
        let truncated = ReadError::Truncated { at, len: N };
        let raw = self.slice(at, N)?;
        <[u8; N]>::try_from(raw).map_err(|_| truncated)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn u8(self, at: usize) -> Result<u8, ReadError> {
        self.bytes
            .get(at)
            .copied()
            .ok_or(ReadError::Truncated { at, len: 1 })
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn u16_le(self, at: usize) -> Result<u16, ReadError> {
        self.array::<2>(at).map(u16::from_le_bytes)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn u32_le(self, at: usize) -> Result<u32, ReadError> {
        self.array::<4>(at).map(u32::from_le_bytes)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn u64_le(self, at: usize) -> Result<u64, ReadError> {
        self.array::<8>(at).map(u64::from_le_bytes)
    }

    /// The bytes from `at` up to (not including) the first NUL, which must
    /// lie within `max` bytes.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn cstr(self, at: usize, max: usize) -> Result<&'a [u8], ReadError> {
        let rest = self
            .bytes
            .get(at..)
            .filter(|rest| !rest.is_empty())
            .ok_or(ReadError::Truncated { at, len: 1 })?;
        let window = rest.get(..max).unwrap_or(rest);
        let end = window
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(ReadError::Unterminated { at, max })?;
        window.get(..end).ok_or(ReadError::Unterminated { at, max })
    }

    /// An unsigned LEB128 number (DWARF 5 §7.6). A tenth byte may carry
    /// only bit 63; anything more does not fit.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn uleb128(self, at: usize) -> Result<Leb<u64>, ReadError> {
        let mut value = 0u64;
        let mut cursor = at;
        for index in 0..MAX_LEB_BYTES {
            let byte = self.u8(cursor)?;
            cursor = cursor
                .checked_add(1)
                .ok_or(ReadError::Truncated { at: cursor, len: 1 })?;
            let shift = index.wrapping_mul(7);
            if index == MAX_LEB_BYTES.wrapping_sub(1) && byte > 1 {
                return Err(ReadError::LebOverflow { at });
            }
            value |= u64::from(byte & 0x7F).wrapping_shl(shift);
            if byte & 0x80 == 0 {
                return Ok(Leb { value, next: cursor });
            }
        }
        Err(ReadError::LebOverflow { at })
    }

    /// A signed LEB128 number (DWARF 5 §7.6). A tenth byte may only extend
    /// the sign (0x00 or 0x7F).
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn sleb128(self, at: usize) -> Result<Leb<i64>, ReadError> {
        let mut value = 0i64;
        let mut cursor = at;
        for index in 0..MAX_LEB_BYTES {
            let byte = self.u8(cursor)?;
            cursor = cursor
                .checked_add(1)
                .ok_or(ReadError::Truncated { at: cursor, len: 1 })?;
            let shift = index.wrapping_mul(7);
            if index == MAX_LEB_BYTES.wrapping_sub(1) && byte != 0x00 && byte != 0x7F {
                return Err(ReadError::LebOverflow { at });
            }
            value |= i64::from(byte & 0x7F).wrapping_shl(shift);
            if byte & 0x80 == 0 {
                let used = shift.wrapping_add(7);
                if used < 64 && byte & 0x40 != 0 {
                    value |= (!0i64).wrapping_shl(used);
                }
                return Ok(Leb { value, next: cursor });
            }
        }
        Err(ReadError::LebOverflow { at })
    }
}

/// A position in a [`Bytes`] that moves forward as it reads. A read that
/// fails leaves the position where it was.
#[derive(Clone, Copy, Debug)]
pub struct Cursor<'a> {
    bytes: Bytes<'a>,
    at: usize,
}

impl<'a> Cursor<'a> {
    #[must_use]
    pub const fn new(bytes: Bytes<'a>, at: usize) -> Cursor<'a> {
        Cursor { bytes, at }
    }

    #[must_use]
    pub const fn position(&self) -> usize {
        self.at
    }

    /// Bytes left after the position; none when it is past the end.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }

    /// The position `len` bytes on, or why there is none.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn advanced(&self, len: usize) -> Result<usize, ReadError> {
        let next = self
            .at
            .checked_add(len)
            .ok_or(ReadError::Truncated { at: self.at, len })?;
        if next > self.bytes.len() {
            return Err(ReadError::Truncated { at: self.at, len });
        }
        Ok(next)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn bytes(&mut self, len: usize) -> Result<&'a [u8], ReadError> {
        let out = self.bytes.slice(self.at, len)?;
        self.at = self.advanced(len)?;
        Ok(out)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn skip(&mut self, len: usize) -> Result<(), ReadError> {
        self.at = self.advanced(len)?;
        Ok(())
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], ReadError> {
        let out = self.bytes.array::<N>(self.at)?;
        self.at = self.advanced(N)?;
        Ok(out)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn u8(&mut self) -> Result<u8, ReadError> {
        self.array::<1>().map(|[byte]| byte)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn u16_le(&mut self) -> Result<u16, ReadError> {
        self.array::<2>().map(u16::from_le_bytes)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn u32_le(&mut self) -> Result<u32, ReadError> {
        self.array::<4>().map(u32::from_le_bytes)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn u64_le(&mut self) -> Result<u64, ReadError> {
        self.array::<8>().map(u64::from_le_bytes)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn uleb128(&mut self) -> Result<u64, ReadError> {
        let leb = self.bytes.uleb128(self.at)?;
        self.at = leb.next;
        Ok(leb.value)
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn sleb128(&mut self) -> Result<i64, ReadError> {
        let leb = self.bytes.sleb128(self.at)?;
        self.at = leb.next;
        Ok(leb.value)
    }

    /// The bytes before the next NUL (within `max`), stepping past the NUL.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn cstr(&mut self, max: usize) -> Result<&'a [u8], ReadError> {
        let text = self.bytes.cstr(self.at, max)?;
        let with_nul = text
            .len()
            .checked_add(1)
            .ok_or(ReadError::Unterminated { at: self.at, max })?;
        self.at = self.advanced(with_nul)?;
        Ok(text)
    }

    /// Moves to the next multiple of `to` (a power of two); a position
    /// already on one stays.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    pub fn align(&mut self, to: usize) -> Result<(), ReadError> {
        let mask = to.wrapping_sub(1);
        let padding = to.wrapping_sub(self.at & mask) & mask;
        self.at = self.advanced(padding)?;
        Ok(())
    }
}

/// Copies as much of `source` as fits into `destination` and returns how
/// many bytes it copied: the one way readers move bytes between buffers,
/// since `copy_from_slice` panics when the lengths differ.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn copy_prefix(destination: &mut [u8], source: &[u8]) -> usize {
    let mut copied = 0usize;
    for to in destination.iter_mut() {
        let Some(from) = source.get(copied) else {
            break;
        };
        *to = *from;
        copied = copied.wrapping_add(1);
    }
    copied
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A LEB128 encoding and the value it holds.
    struct UlebCase {
        bytes: &'static [u8],
        value: u64,
    }

    struct SlebCase {
        bytes: &'static [u8],
        value: i64,
    }

    #[test]
    fn fixed_width_reads_check_their_bounds() {
        let bytes = Bytes::new(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09]);
        assert_eq!(bytes.u8(0), Ok(1));
        assert_eq!(bytes.u16_le(1), Ok(0x0302));
        assert_eq!(bytes.u32_le(1), Ok(0x0504_0302));
        assert_eq!(bytes.u64_le(1), Ok(0x0908_0706_0504_0302));
        assert_eq!(bytes.u64_le(2), Err(ReadError::Truncated { at: 2, len: 8 }));
        assert_eq!(bytes.u8(9), Err(ReadError::Truncated { at: 9, len: 1 }));
        assert_eq!(bytes.u32_le(usize::MAX), Err(ReadError::Truncated { at: usize::MAX, len: 4 }));
    }

    #[test]
    fn slices_and_sub_ranges_check_their_bounds() {
        let bytes = Bytes::new(b"abcdef");
        assert_eq!(bytes.slice(1, 3), Ok(&b"bcd"[..]));
        assert_eq!(bytes.slice(6, 0), Ok(&b""[..]));
        assert_eq!(bytes.slice(4, 3), Err(ReadError::Truncated { at: 4, len: 3 }));
        assert_eq!(bytes.slice(1, usize::MAX), Err(ReadError::Truncated { at: 1, len: usize::MAX }));
        let sub = bytes.sub(2, 3).unwrap();
        assert_eq!(sub.as_slice(), b"cde");
        assert_eq!(sub.u8(3), Err(ReadError::Truncated { at: 3, len: 1 }));
        assert_eq!(bytes.tail(6).map(Bytes::len), Ok(0));
        assert_eq!(bytes.tail(7).map(Bytes::len), Err(ReadError::Truncated { at: 7, len: 0 }));
        assert_eq!(bytes.array::<2>(4), Ok(*b"ef"));
    }

    #[test]
    fn a_c_string_ends_at_its_nul_or_is_refused() {
        let bytes = Bytes::new(b"name\0rest");
        assert_eq!(bytes.cstr(0, 64), Ok(&b"name"[..]));
        assert_eq!(bytes.cstr(5, 64), Err(ReadError::Unterminated { at: 5, max: 64 }));
        assert_eq!(bytes.cstr(0, 3), Err(ReadError::Unterminated { at: 0, max: 3 }));
        assert_eq!(bytes.cstr(10, 64), Err(ReadError::Truncated { at: 10, len: 1 }));
    }

    #[test]
    fn unsigned_leb128_reads_the_standards_examples() {
        let cases = [
            UlebCase { bytes: &[0x02], value: 2 },
            UlebCase { bytes: &[0x7F], value: 127 },
            UlebCase { bytes: &[0x80, 0x01], value: 128 },
            UlebCase { bytes: &[0x81, 0x01], value: 129 },
            UlebCase { bytes: &[0x82, 0x01], value: 130 },
            UlebCase { bytes: &[0xB9, 0x64], value: 12857 },
            UlebCase {
                bytes: &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01],
                value: u64::MAX,
            },
        ];
        for case in cases {
            let leb = Bytes::new(case.bytes).uleb128(0).unwrap();
            assert_eq!(leb.value, case.value);
            assert_eq!(leb.next, case.bytes.len());
        }
    }

    #[test]
    fn signed_leb128_reads_the_standards_examples() {
        let cases = [
            SlebCase { bytes: &[0x02], value: 2 },
            SlebCase { bytes: &[0x7E], value: -2 },
            SlebCase { bytes: &[0xFF, 0x00], value: 127 },
            SlebCase { bytes: &[0x81, 0x7F], value: -127 },
            SlebCase { bytes: &[0x80, 0x01], value: 128 },
            SlebCase { bytes: &[0x80, 0x7F], value: -128 },
            SlebCase { bytes: &[0x81, 0x01], value: 129 },
            SlebCase { bytes: &[0xFF, 0x7E], value: -129 },
            SlebCase {
                bytes: &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x7F],
                value: i64::MIN,
            },
        ];
        for case in cases {
            let leb = Bytes::new(case.bytes).sleb128(0).unwrap();
            assert_eq!(leb.value, case.value);
            assert_eq!(leb.next, case.bytes.len());
        }
    }

    #[test]
    fn a_leb128_too_long_for_64_bits_or_cut_short_is_refused() {
        let too_big = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02];
        assert_eq!(Bytes::new(&too_big).uleb128(0), Err(ReadError::LebOverflow { at: 0 }));
        let endless = [0x80; 32];
        assert_eq!(Bytes::new(&endless).uleb128(0), Err(ReadError::LebOverflow { at: 0 }));
        assert_eq!(Bytes::new(&endless).sleb128(0), Err(ReadError::LebOverflow { at: 0 }));
        assert_eq!(Bytes::new(&[0x80, 0x80]).uleb128(0), Err(ReadError::Truncated { at: 2, len: 1 }));
    }

    #[test]
    fn a_cursor_moves_only_when_a_read_succeeds() {
        let data = [0x34, 0x12, 0x78, 0x56, 0x34, 0x12, b'h', b'i', 0, 0x85, 0x01, 0xAA];
        let mut cursor = Cursor::new(Bytes::new(&data), 0);
        assert_eq!(cursor.u16_le(), Ok(0x1234));
        assert_eq!(cursor.u32_le(), Ok(0x1234_5678));
        assert_eq!(cursor.cstr(16), Ok(&b"hi"[..]));
        assert_eq!(cursor.position(), 9);
        assert_eq!(cursor.uleb128(), Ok(0x85));
        assert_eq!(cursor.u32_le(), Err(ReadError::Truncated { at: 11, len: 4 }));
        assert_eq!(cursor.position(), 11, "a failed read leaves the cursor where it was");
        assert_eq!(cursor.remaining(), 1);
        assert_eq!(cursor.align(4), Ok(()));
        assert_eq!(cursor.position(), 12);
        assert_eq!(cursor.align(4), Ok(()));
        assert_eq!(cursor.u8(), Err(ReadError::Truncated { at: 12, len: 1 }));
        let mut short = Cursor::new(Bytes::new(&[1, 2, 3]), 1);
        assert_eq!(short.align(4), Err(ReadError::Truncated { at: 1, len: 3 }));
        assert_eq!(short.skip(2), Ok(()));
        assert_eq!(short.skip(1), Err(ReadError::Truncated { at: 3, len: 1 }));
    }

    #[test]
    fn a_copy_takes_what_fits() {
        let mut small = [0u8; 2];
        assert_eq!(copy_prefix(&mut small, b"abc"), 2);
        assert_eq!(&small, b"ab");
        let mut large = [9u8; 4];
        assert_eq!(copy_prefix(&mut large, b"xy"), 2);
        assert_eq!(&large, b"xy\x09\x09");
    }
}
