//! Compressed streams from the reference writers, and the inputs they
//! compress: text with repeats (so encoders emit matches) and noisy binary
//! (so they emit literals and long codes). Deterministic: a fixed xorshift.

use std::io::Write;

/// A raw DEFLATE stream (RFC 1951) of `data` at `level` (0 stores, 9 is
/// smallest).
pub fn raw_deflate(data: &[u8], level: u32) -> Vec<u8> {
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(level));
    encoder.write_all(data).expect("deflate");
    encoder.finish().expect("deflate")
}

/// A zlib stream (RFC 1950) of `data` at `level`.
pub fn zlib(data: &[u8], level: u32) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(level));
    encoder.write_all(data).expect("zlib");
    encoder.finish().expect("zlib")
}

/// Whether a zstd frame carries its content checksum.
#[derive(Clone, Copy, Debug)]
pub enum Checksum {
    With,
    Without,
}

/// How a test frame is written.
#[derive(Clone, Copy, Debug)]
pub struct ZstdOptions {
    pub level: i32,
    pub checksum: Checksum,
    /// log2 of the window; 0 leaves the level's default.
    pub window_log: u32,
}

/// One zstd frame of `data`, with its content size in the header.
pub fn zstd_frame(data: &[u8], options: ZstdOptions) -> Vec<u8> {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), options.level).expect("encoder");
    encoder
        .include_checksum(matches!(options.checksum, Checksum::With))
        .expect("checksum flag");
    encoder.include_contentsize(true).expect("content size flag");
    encoder.set_pledged_src_size(Some(data.len() as u64)).expect("pledged size");
    if options.window_log != 0 {
        encoder.window_log(options.window_log).expect("window log");
    }
    encoder.write_all(data).expect("zstd");
    encoder.finish().expect("zstd")
}

/// The next xorshift64 state.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// `len` bytes of words and numbers with frequent repeats.
pub fn sample_text(len: usize) -> Vec<u8> {
    const WORDS: [&str; 12] = [
        "KeBugCheckEx", "the ", "process ", "PsActiveProcessHead", "\n", "0x7FFE0000 ",
        "ntoskrnl ", "symbol ", "store ", "_EPROCESS", " = ", "module ",
    ];
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        let word = WORDS[(next(&mut state) % WORDS.len() as u64) as usize];
        out.extend_from_slice(word.as_bytes());
    }
    out.truncate(len);
    out
}

/// `len` noisy bytes, with a copy of an earlier run every 4 KiB so a
/// decoder still meets long matches.
pub fn sample_binary(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        if out.len() % 4096 == 4000 && out.len() > 300 {
            let from = out.len() - 300;
            for index in 0..96 {
                out.push(out[from + index]);
            }
        } else {
            out.push(next(&mut state) as u8);
        }
    }
    out.truncate(len);
    out
}
