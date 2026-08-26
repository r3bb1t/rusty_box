//! Reading and writing saved machine state, without naming a host.
//!
//! A device's snapshot body is a sequence of little-endian scalars and byte
//! arrays. Nothing about that needs `std::io` — but this port wrote it against
//! `io::Write`/`io::Read` and gated the whole surface on `std`, which puts it
//! out of reach of a device crate that must build `no_std`. These traits are
//! the same shape with the host removed: `std` supplies an adapter, and so
//! could a UEFI firmware volume, an in-memory buffer, or a replay log.
//!
//! Errors carry `&'static str`, not formatted text. Every message a device
//! produces is a fixed statement about its own state — "PIT mode is out of
//! range" — so allocation buys nothing, and the layer above is free to add
//! context in whatever richer form it has.

/// Why a snapshot could not be read or written.
///
/// `#[non_exhaustive]` because this crosses a crate boundary and will grow;
/// callers that must react to a specific cause match the ones they know and
/// fall back on the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapError {
    /// The source ran out of bytes mid-record.
    Truncated,
    /// A section body was not fully consumed by the device that owns it.
    TrailingBytes,
    /// A device wrote more than the length it declared. The section is named
    /// because the writer is bounded per section and the tag identifies which.
    Overran { section: u32 },
    /// A device wrote less than the length it declared.
    UnderRan { section: u32 },
    /// A computed length does not fit, or exceeds the implementation bound.
    LengthOutOfRange,
    /// The bytes are structurally fine but describe a state the device cannot
    /// be in. The message is the device's own statement of what was wrong.
    Invalid(&'static str),
    /// The host's own I/O failed. Deliberately opaque: a `no_std` consumer has
    /// no `std::io::Error` to carry, and the layer that has one keeps it.
    Host,
}

impl core::fmt::Display for SnapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated => f.write_str("snapshot ended mid-record"),
            Self::TrailingBytes => f.write_str("snapshot section has trailing bytes"),
            Self::Overran { section } => {
                write!(f, "snapshot section {section} overran its declared length")
            }
            Self::UnderRan { section } => {
                write!(f, "snapshot section {section} under-ran its declared length")
            }
            Self::LengthOutOfRange => f.write_str("snapshot length is out of range"),
            Self::Invalid(what) => f.write_str(what),
            Self::Host => f.write_str("snapshot host I/O failed"),
        }
    }
}

pub type SnapResult<T = ()> = Result<T, SnapError>;

/// Somewhere a snapshot body can be written.
///
/// One required method; the scalars are provided, so an implementor supplies
/// bytes and inherits the encoding. Little-endian throughout, matching the
/// on-disk format this port already writes.
pub trait SnapWrite {
    fn write_bytes(&mut self, bytes: &[u8]) -> SnapResult;

    #[inline]
    fn write_u8(&mut self, value: u8) -> SnapResult {
        self.write_bytes(&[value])
    }

    /// A bool occupies one byte, `1` or `0` — the encoding already on disk, and
    /// the only two [`SnapRead::read_bool`] will accept back.
    #[inline]
    fn write_bool(&mut self, value: bool) -> SnapResult {
        self.write_u8(u8::from(value))
    }

    #[inline]
    fn write_u16(&mut self, value: u16) -> SnapResult {
        self.write_bytes(&value.to_le_bytes())
    }

    #[inline]
    fn write_u32(&mut self, value: u32) -> SnapResult {
        self.write_bytes(&value.to_le_bytes())
    }

    #[inline]
    fn write_u64(&mut self, value: u64) -> SnapResult {
        self.write_bytes(&value.to_le_bytes())
    }

    #[inline]
    fn write_i64(&mut self, value: i64) -> SnapResult {
        self.write_bytes(&value.to_le_bytes())
    }
}

/// Somewhere a snapshot body can be read from.
///
/// `read_bytes` fills the buffer or fails: a short read is a truncated
/// snapshot, never a partial success, because a device that got half its state
/// cannot tell which half.
pub trait SnapRead {
    fn read_bytes(&mut self, out: &mut [u8]) -> SnapResult;

    #[inline]
    fn read_u8(&mut self) -> SnapResult<u8> {
        let mut byte = [0u8; 1];
        self.read_bytes(&mut byte)?;
        Ok(byte[0])
    }

    /// Only `0` and `1` are accepted.
    ///
    /// A byte outside that is not a true-ish value, it is a stream that has
    /// desynchronised — the reader is at an offset the writer never wrote a
    /// bool to — and reading on would decode every later field from the wrong
    /// place. This is the format's rule, not an implementor's choice, so it
    /// lives in the default rather than in each reader.
    #[inline]
    fn read_bool(&mut self) -> SnapResult<bool> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(SnapError::Invalid("snapshot boolean is not canonical")),
        }
    }

    #[inline]
    fn read_u16(&mut self) -> SnapResult<u16> {
        let mut bytes = [0u8; 2];
        self.read_bytes(&mut bytes)?;
        Ok(u16::from_le_bytes(bytes))
    }

    #[inline]
    fn read_u32(&mut self) -> SnapResult<u32> {
        let mut bytes = [0u8; 4];
        self.read_bytes(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    #[inline]
    fn read_u64(&mut self) -> SnapResult<u64> {
        let mut bytes = [0u8; 8];
        self.read_bytes(&mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }

    #[inline]
    fn read_i64(&mut self) -> SnapResult<i64> {
        let mut bytes = [0u8; 8];
        self.read_bytes(&mut bytes)?;
        Ok(i64::from_le_bytes(bytes))
    }
}

/// A device that owns exactly one section of the snapshot stream.
///
/// The tag travels with the device rather than with the call site, so a body
/// can never be written under another device's identity, and the declared
/// length can never be computed from a different device than the one that
/// fills it — the whole section is derived from a single `&D`.
pub trait SnapshotSection {
    /// This device's identity in the stream.
    const TAG: u32;

    /// What a restore hands back for the machine to act on. `()` means the
    /// device's state is entirely its own: nothing outside it has to move.
    type Restored;

    /// The exact byte length [`Self::save`] will write. Validating the state it
    /// would serialize is part of the answer, so an unserializable device fails
    /// here rather than half-way through a section body.
    fn snapshot_len(&self) -> SnapResult<u64>;

    fn save<W: SnapWrite>(&self, writer: &mut W) -> SnapResult;

    fn restore<R: SnapRead>(&mut self, reader: &mut R) -> SnapResult<Self::Restored>;
}

/// Add `len` to `base`, refusing a total that cannot be represented or that
/// exceeds `bound`.
///
/// Section lengths are computed from guest-influenced sizes, so the arithmetic
/// is checked rather than trusted: a length that wrapped would declare a small
/// section and then write a large one.
#[inline]
pub fn checked_len_add(base: u64, len: u64, bound: u64) -> SnapResult<u64> {
    match base.checked_add(len) {
        Some(total) if total <= bound => Ok(total),
        _ => Err(SnapError::LengthOutOfRange),
    }
}

/// Multiply, with the same rule.
#[inline]
pub fn checked_len_mul(lhs: u64, rhs: u64, bound: u64) -> SnapResult<u64> {
    match lhs.checked_mul(rhs) {
        Some(total) if total <= bound => Ok(total),
        _ => Err(SnapError::LengthOutOfRange),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Buffer {
        bytes: [u8; 64],
        len: usize,
        read: usize,
    }

    impl Default for Buffer {
        fn default() -> Self {
            Self {
                bytes: [0; 64],
                len: 0,
                read: 0,
            }
        }
    }

    impl SnapWrite for Buffer {
        fn write_bytes(&mut self, bytes: &[u8]) -> SnapResult {
            let end = self.len + bytes.len();
            let room = self.bytes.get_mut(self.len..end).ok_or(SnapError::Host)?;
            room.copy_from_slice(bytes);
            self.len = end;
            Ok(())
        }
    }

    impl SnapRead for Buffer {
        fn read_bytes(&mut self, out: &mut [u8]) -> SnapResult {
            let end = self.read + out.len();
            let taken = self
                .bytes
                .get(self.read..end)
                .filter(|_| end <= self.len)
                .ok_or(SnapError::Truncated)?;
            out.copy_from_slice(taken);
            self.read = end;
            Ok(())
        }
    }

    /// Every scalar comes back as it went in, in the order it went in — the
    /// property a saved machine's whole state rests on.
    #[test]
    fn every_scalar_round_trips_little_endian() {
        let mut buffer = Buffer::default();
        buffer.write_u8(0xA5).unwrap();
        buffer.write_bool(true).unwrap();
        buffer.write_bool(false).unwrap();
        buffer.write_u16(0x1234).unwrap();
        buffer.write_u32(0xDEAD_BEEF).unwrap();
        buffer.write_u64(0x0123_4567_89AB_CDEF).unwrap();
        buffer.write_i64(-2).unwrap();
        buffer.write_bytes(&[1, 2, 3]).unwrap();

        assert_eq!(buffer.read_u8().unwrap(), 0xA5);
        assert!(buffer.read_bool().unwrap());
        assert!(!buffer.read_bool().unwrap());
        assert_eq!(buffer.read_u16().unwrap(), 0x1234);
        assert_eq!(buffer.read_u32().unwrap(), 0xDEAD_BEEF);
        assert_eq!(buffer.read_u64().unwrap(), 0x0123_4567_89AB_CDEF);
        assert_eq!(buffer.read_i64().unwrap(), -2);
        let mut tail = [0u8; 3];
        buffer.read_bytes(&mut tail).unwrap();
        assert_eq!(tail, [1, 2, 3]);
    }

    /// A byte that is neither 0 nor 1 means the reader is at an offset the
    /// writer never wrote a bool to, so every later field would decode from the
    /// wrong place. Rejecting it is what stops a desynchronised stream from
    /// being silently accepted as a machine.
    #[test]
    fn a_non_canonical_boolean_is_a_desynchronised_stream() {
        let mut buffer = Buffer::default();
        buffer.write_u8(2).unwrap();
        assert_eq!(
            buffer.read_bool(),
            Err(SnapError::Invalid("snapshot boolean is not canonical"))
        );
    }

    /// The byte order is the format's, not the host's. Asserting it explicitly
    /// is what keeps a snapshot written on one machine readable on another.
    #[test]
    fn scalars_are_written_little_endian_on_every_host() {
        let mut buffer = Buffer::default();
        buffer.write_u32(0x1122_3344).unwrap();
        assert_eq!(&buffer.bytes[..4], &[0x44, 0x33, 0x22, 0x11]);
    }

    /// A short read is a truncated snapshot, never a partial success: a device
    /// handed half its state cannot tell which half it got.
    #[test]
    fn a_short_read_fails_rather_than_returning_less() {
        let mut buffer = Buffer::default();
        buffer.write_u16(0xBEEF).unwrap();
        let mut too_much = [0u8; 4];
        assert_eq!(
            buffer.read_bytes(&mut too_much),
            Err(SnapError::Truncated)
        );
    }

    /// Lengths are computed from guest-influenced sizes, so they are checked.
    #[test]
    fn length_arithmetic_refuses_to_wrap_or_exceed_its_bound() {
        assert_eq!(checked_len_add(2, 3, 16), Ok(5));
        assert_eq!(checked_len_add(u64::MAX, 1, u64::MAX), Err(SnapError::LengthOutOfRange));
        assert_eq!(checked_len_add(10, 10, 16), Err(SnapError::LengthOutOfRange));
        assert_eq!(checked_len_mul(4, 4, 16), Ok(16));
        assert_eq!(checked_len_mul(u64::MAX, 2, u64::MAX), Err(SnapError::LengthOutOfRange));
        assert_eq!(checked_len_mul(5, 4, 16), Err(SnapError::LengthOutOfRange));
    }
}
