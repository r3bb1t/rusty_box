//! Decompressors for the containers symbol files come in: DEFLATE (CAB
//! MSZIP blocks, MSFZ chunks, zlib-compressed ELF sections), Zstandard
//! (MSFZ chunks, zstd-compressed ELF sections) and LZX (CAB folders). Each
//! core is tier P: it writes into an
//! output buffer its caller allocated and stops at that buffer's end.
//! [`decompress_to_vec`] is the tier-A wrapper: it sizes the buffer from the
//! size the container declares, and refuses, before allocating, a size over
//! the caller's cap or one the input could not expand to.

pub mod inflate;
pub mod lzx;
pub mod zstd;

use crate::read::Bytes;
use inflate::{inflate, inflate_zlib, InflateError};

/// The format of a compressed stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// RFC 1951, no wrapper (MSFZ chunks).
    RawDeflate,
    /// RFC 1950 (ELF `SHF_COMPRESSED` with `ELFCOMPRESS_ZLIB`, `.zdebug`).
    Zlib,
    /// RFC 8878 (MSFZ chunks, `ELFCOMPRESS_ZSTD`).
    Zstd,
}

impl Codec {
    /// The most output one input byte can produce. DEFLATE's best case is a
    /// 258-byte match coded in two bits, so 1032 bytes per input byte (the
    /// zlib FAQ's "maximum compression factor").
    #[must_use]
    pub fn max_ratio(self) -> u64 {
        match self {
            Codec::RawDeflate | Codec::Zlib => 1032,
            // An RLE block: four bytes (header and byte) to 128 KiB.
            Codec::Zstd => 32768,
        }
    }
}

/// Why a compressed stream did not become its declared bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DecompressError {
    #[error("the stream declares {declared} bytes, over the cap of {cap}")]
    TooLarge { declared: u64, cap: u64 },
    #[error("{input} compressed bytes cannot expand to the {declared} declared (at most {ratio} per byte)")]
    Bomb { declared: u64, input: u64, ratio: u64 },
    #[error("no memory for {bytes} bytes of output")]
    Allocation { bytes: u64 },
    #[error(transparent)]
    Inflate(#[from] InflateError),
    #[error(transparent)]
    Zstd(#[from] zstd::ZstdError),
    #[error("the stream decoded to {produced} bytes, not the {declared} declared")]
    WrongSize { declared: u64, produced: u64 },
}

/// The `declared` bytes `input` decompresses to. The size is checked
/// against `cap` and against what `input` could expand to before anything
/// is allocated, and the output must be exactly that size.
pub fn decompress_to_vec(codec: Codec, input: &[u8], declared: u64, cap: u64) -> Result<Vec<u8>, DecompressError> {
    if declared > cap {
        return Err(DecompressError::TooLarge { declared, cap });
    }
    let input_len = u64::try_from(input.len()).unwrap_or(u64::MAX);
    let ratio = codec.max_ratio();
    if declared > input_len.saturating_mul(ratio) {
        return Err(DecompressError::Bomb {
            declared,
            input: input_len,
            ratio,
        });
    }
    let len = usize::try_from(declared).map_err(|_| DecompressError::Allocation { bytes: declared })?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|_| DecompressError::Allocation { bytes: declared })?;
    // Within the reserved capacity: no reallocation.
    out.resize(len, 0);
    let produced = match codec {
        Codec::RawDeflate => inflate(Bytes::new(input), &mut out, 0)?.end,
        Codec::Zlib => inflate_zlib(Bytes::new(input), &mut out)?,
        Codec::Zstd => {
            let mut scratch = Vec::new();
            scratch
                .try_reserve_exact(zstd::SCRATCH_BYTES)
                .map_err(|_| DecompressError::Allocation { bytes: zstd::SCRATCH_BYTES as u64 })?;
            scratch.resize(zstd::SCRATCH_BYTES, 0);
            zstd::decompress_frames(Bytes::new(input), &mut out, &mut scratch)?
        }
    };
    if produced != len {
        return Err(DecompressError::WrongSize {
            declared,
            produced: u64::try_from(produced).unwrap_or(u64::MAX),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_symbols_fixtures::compress::{sample_text, zlib};

    #[test]
    fn a_declared_size_is_checked_before_anything_is_allocated() {
        let data = sample_text(10_000);
        let stream = zlib(&data, 9);
        assert_eq!(decompress_to_vec(Codec::Zlib, &stream, 10_000, 1 << 20).unwrap(), data);
        assert_eq!(
            decompress_to_vec(Codec::Zlib, &stream, 10_000, 9_999),
            Err(DecompressError::TooLarge { declared: 10_000, cap: 9_999 })
        );
        assert!(matches!(
            decompress_to_vec(Codec::Zlib, &stream, 10_001, 1 << 20),
            Err(DecompressError::WrongSize { declared: 10_001, .. })
        ));
    }

    /// Review focus 2: a few bytes claiming gigabytes are refused before
    /// the output is allocated.
    #[test]
    fn a_stream_claiming_more_than_the_cap_is_refused_before_allocating() {
        let tiny = [0x78, 0x9C, 0x4B, 0x04, 0x00, 0x00, 0x62, 0x00, 0x62];
        assert_eq!(
            decompress_to_vec(Codec::Zlib, &tiny, 1 << 40, u64::MAX),
            Err(DecompressError::Bomb { declared: 1 << 40, input: 9, ratio: 1032 })
        );
    }

    #[test]
    fn a_zstd_stream_claiming_more_than_it_could_hold_is_refused() {
        let tiny = [0x28, 0xB5, 0x2F, 0xFD, 0x20, 0x04, 0x23, 0, 0, b'z'];
        assert_eq!(decompress_to_vec(Codec::Zstd, &tiny, 4, 1 << 20).unwrap(), b"zzzz");
        assert_eq!(
            decompress_to_vec(Codec::Zstd, &tiny, 1 << 40, u64::MAX),
            Err(DecompressError::Bomb { declared: 1 << 40, input: 10, ratio: 32768 })
        );
    }
}
