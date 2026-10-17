//! A source file's bytes as lines of text, and whether they are the bytes
//! the build recorded a checksum of.

use md5::Digest;
use rusty_box_symbols::tables::{ChecksumKind, FileChecksum};

/// A source file's lines, without their line ends: line `n` is
/// `lines[n - 1]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceText {
    pub(crate) lines: Vec<String>,
}

/// Whether a file's bytes are those the build recorded a checksum of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceCheck {
    Matches,
    Differs,
    /// The build recorded none: nothing says whether the file is the one
    /// it compiled.
    NoChecksum,
}

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
const UTF16_LE_BOM: [u8; 2] = [0xFF, 0xFE];
const UTF16_BE_BOM: [u8; 2] = [0xFE, 0xFF];

/// Windows-1252's bytes 0x80 to 0x9F; the rest are Latin-1's. The five
/// bytes it leaves unassigned keep their own code (as the WHATWG decoder).
const WINDOWS_1252_80_TO_9F: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{2C6}', '\u{2030}',
    '\u{160}', '\u{2039}', '\u{152}', '\u{8D}', '\u{17D}', '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}',
    '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}', '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}',
    '\u{17E}', '\u{178}',
];

fn utf16<F: Fn([u8; 2]) -> u16>(bytes: &[u8], unit: F) -> String {
    let pairs = bytes.chunks_exact(2);
    let odd = !pairs.remainder().is_empty();
    let units = pairs.map(|pair| unit([pair.first().copied().unwrap_or(0), pair.get(1).copied().unwrap_or(0)]));
    let mut text: String = char::decode_utf16(units).map(|decoded| decoded.unwrap_or(char::REPLACEMENT_CHARACTER)).collect();
    if odd {
        text.push(char::REPLACEMENT_CHARACTER);
    }
    text
}

fn windows_1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| match byte {
            0x80..=0x9F => WINDOWS_1252_80_TO_9F.get(usize::from(byte - 0x80)).copied().unwrap_or(char::REPLACEMENT_CHARACTER),
            _ => char::from(*byte),
        })
        .collect()
}

/// The file as text: UTF-8 (a BOM dropped), UTF-16 in the order its BOM
/// gives, else Windows-1252, the code page a Windows build's source without
/// a BOM is most often in.
pub(crate) fn decode(bytes: &[u8]) -> SourceText {
    let text = if let Some(rest) = bytes.strip_prefix(&UTF8_BOM) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Some(rest) = bytes.strip_prefix(&UTF16_LE_BOM) {
        utf16(rest, u16::from_le_bytes)
    } else if let Some(rest) = bytes.strip_prefix(&UTF16_BE_BOM) {
        utf16(rest, u16::from_be_bytes)
    } else {
        match std::str::from_utf8(bytes) {
            Ok(text) => text.to_owned(),
            Err(_) => windows_1252(bytes),
        }
    };
    SourceText { lines: text.lines().map(str::to_owned).collect() }
}

/// Whether `bytes` are those `recorded` is a checksum of.
pub(crate) fn check(bytes: &[u8], recorded: Option<&FileChecksum>) -> SourceCheck {
    let Some(recorded) = recorded else {
        return SourceCheck::NoChecksum;
    };
    let same = match recorded.kind {
        ChecksumKind::Md5 => md5::Md5::digest(bytes).as_slice() == recorded.bytes.as_slice(),
        ChecksumKind::Sha1 => sha1::Sha1::digest(bytes).as_slice() == recorded.bytes.as_slice(),
        ChecksumKind::Sha256 => sha2::Sha256::digest(bytes).as_slice() == recorded.bytes.as_slice(),
    };
    if same {
        SourceCheck::Matches
    } else {
        SourceCheck::Differs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &SourceText) -> Vec<&str> {
        text.lines.iter().map(String::as_str).collect()
    }

    #[test]
    fn utf8_splits_into_lines_without_their_ends() {
        assert_eq!(lines(&decode(b"int a;\r\nint b;\nint c;")), ["int a;", "int b;", "int c;"]);
        assert_eq!(lines(&decode(b"a\n\nb\n")), ["a", "", "b"], "a last line end starts no line");
        assert_eq!(lines(&decode("\u{FEFF}// \u{e9}t\u{e9}\n".as_bytes())), ["// \u{e9}t\u{e9}"], "the BOM is not text");
        assert_eq!(lines(&decode(b"")), Vec::<&str>::new());
    }

    #[test]
    fn utf16_with_a_bom_is_read_in_its_byte_order() {
        let little = [0xFF, 0xFE, b'h', 0, b'i', 0, b'\r', 0, b'\n', 0, 0xAC, 0x20];
        assert_eq!(lines(&decode(&little)), ["hi", "\u{20AC}"]);
        let big = [0xFE, 0xFF, 0, b'o', 0, b'k', 0, b'\n', 0x20, 0xAC];
        assert_eq!(lines(&decode(&big)), ["ok", "\u{20AC}"]);
    }

    /// Bytes that are not UTF-8 are a Windows code page's: Windows-1252.
    #[test]
    fn other_bytes_are_windows_1252() {
        assert_eq!(lines(&decode(&[b'q', 0x93, b'x', 0x94, b' ', 0xE9, 0x80])), ["q\u{201C}x\u{201D} \u{e9}\u{20AC}"]);
        assert_eq!(lines(&decode(&[0x81, 0x8D])), ["\u{81}\u{8D}"], "its five unassigned bytes keep their code");
    }

    fn checksum(kind: ChecksumKind, hex: &str) -> FileChecksum {
        let bytes = (0..hex.len()).step_by(2).map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap()).collect();
        FileChecksum { kind, bytes }
    }

    #[test]
    fn each_kind_of_checksum_is_checked() {
        let md5 = checksum(ChecksumKind::Md5, "900150983cd24fb0d6963f7d28e17f72");
        let sha1 = checksum(ChecksumKind::Sha1, "a9993e364706816aba3e25717850c26c9cd0d89d");
        let sha256 = checksum(ChecksumKind::Sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        for recorded in [&md5, &sha1, &sha256] {
            assert_eq!(check(b"abc", Some(recorded)), SourceCheck::Matches, "{:?}", recorded.kind);
            assert_eq!(check(b"abd", Some(recorded)), SourceCheck::Differs, "{:?}", recorded.kind);
        }
        assert_eq!(check(b"abc", None), SourceCheck::NoChecksum);
    }
}
