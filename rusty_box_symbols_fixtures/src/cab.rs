//! Cabinets: stored and MSZIP ones written by the `cab` crate, and two LZX
//! ones made once by Windows' `makecab` and committed, since no Rust crate
//! writes LZX. Both LZX cabinets hold [`lzx_input`].

use std::io::{Cursor, Write};

use crate::compress::{sample_binary, sample_text};

/// What a written cabinet's folder holds.
#[derive(Clone, Copy, Debug)]
pub enum CabinetKind {
    Stored,
    MsZip,
}

/// A one-folder, one-file cabinet, as a symbol server's `.pd_` is.
pub fn cabinet(name: &str, contents: &[u8], kind: CabinetKind) -> Vec<u8> {
    let compression = match kind {
        CabinetKind::Stored => cab::CompressionType::None,
        CabinetKind::MsZip => cab::CompressionType::MsZip,
    };
    let mut builder = cab::CabinetBuilder::new();
    builder.add_folder(compression).add_file(name);
    let mut writer = builder.build(Cursor::new(Vec::new())).expect("cabinet");
    while let Some(mut file) = writer.next_file().expect("next file") {
        file.write_all(contents).expect("file contents");
    }
    writer.finish().expect("cabinet").into_inner()
}

/// A cabinet of two files, which no symbol server writes.
pub fn two_file_cabinet() -> Vec<u8> {
    let mut builder = cab::CabinetBuilder::new();
    let folder = builder.add_folder(cab::CompressionType::MsZip);
    folder.add_file("a.txt");
    folder.add_file("b.txt");
    let mut writer = builder.build(Cursor::new(Vec::new())).expect("cabinet");
    while let Some(mut file) = writer.next_file().expect("next file") {
        file.write_all(b"contents").expect("file contents");
    }
    writer.finish().expect("cabinet").into_inner()
}

/// The input both LZX cabinets hold: 150 000 bytes of text (long matches,
/// repeat offsets) then 100 000 noisy bytes (literals, and `E8` bytes that
/// exercise call translation). 250 000 bytes is eight 32 KiB frames.
pub fn lzx_input() -> Vec<u8> {
    let mut input = sample_text(150_000);
    input.extend_from_slice(&sample_binary(100_000, 5));
    input
}

/// `makecab /D CompressionType=LZX /D CompressionMemory=15` of `lzx_input()`.
pub const LZX15_CABINET: &[u8] = include_bytes!("../data/lzx15.cab");
/// The same with `CompressionMemory=21`, the window Microsoft's symbol
/// server's `.pd_` files use.
pub const LZX21_CABINET: &[u8] = include_bytes!("../data/lzx21.cab");

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes `lzx_input()` where `LZX_INPUT` says, for `makecab` to compress
    /// (Task 7 Step 2). Ignored: run only to rebuild the committed cabinets.
    #[test]
    #[ignore]
    fn write_lzx_input() {
        let path = std::env::var("LZX_INPUT").expect("set LZX_INPUT to the file to write");
        std::fs::write(path, lzx_input()).expect("write");
    }
}
