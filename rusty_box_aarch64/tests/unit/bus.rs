//! A RAM store writes exactly its width, little-endian, or nothing at all.

use super::{le_value, store_le, AccessWidth};

const WIDTHS: [AccessWidth; 4] = [
    AccessWidth::Byte,
    AccessWidth::Half,
    AccessWidth::Word,
    AccessWidth::Dword,
];
const VALUE: u64 = 0x1122_3344_5566_7788;

#[test]
fn a_store_writes_its_width_little_endian() {
    for width in WIDTHS {
        let n = width.bytes() as usize;
        for offset in [0, 3, 16 - n] {
            let mut bytes = [0xAAu8; 16];
            assert_eq!(
                store_le(&mut bytes, offset, width, VALUE),
                Some(()),
                "{width:?} at {offset}"
            );
            assert_eq!(
                &bytes[offset..offset + n],
                &VALUE.to_le_bytes()[..n],
                "{width:?} at {offset}"
            );
            assert!(
                bytes[..offset].iter().all(|&b| b == 0xAA),
                "{width:?} at {offset} wrote before itself"
            );
            assert!(
                bytes[offset + n..].iter().all(|&b| b == 0xAA),
                "{width:?} at {offset} wrote past itself"
            );
            assert_eq!(
                le_value(&bytes[offset..offset + n]),
                VALUE & width.value_mask(),
                "{width:?} at {offset}"
            );
        }
    }
}

#[test]
fn a_store_that_does_not_fit_writes_nothing() {
    for width in WIDTHS {
        let n = width.bytes() as usize;
        // Running past the end by one byte, and starting past it.
        for offset in [16 - n + 1, 16, 17, usize::MAX] {
            let mut bytes = [0xAAu8; 16];
            assert_eq!(
                store_le(&mut bytes, offset, width, VALUE),
                None,
                "{width:?} at {offset}"
            );
            assert!(
                bytes.iter().all(|&b| b == 0xAA),
                "{width:?} at {offset} wrote"
            );
        }
    }
}
