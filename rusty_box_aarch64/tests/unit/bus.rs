//! A RAM store writes exactly its width, little-endian.

use super::{le_value, store_le};

#[test]
fn a_store_writes_its_width_little_endian() {
    const VALUE: u64 = 0x1122_3344_5566_7788;
    for width in [1usize, 2, 4, 8] {
        let mut bytes = [0xAAu8; 8];
        store_le(&mut bytes[..width], VALUE);
        assert_eq!(&bytes[..width], &VALUE.to_le_bytes()[..width], "width {width}");
        assert!(bytes[width..].iter().all(|&b| b == 0xAA), "width {width} wrote past itself");
        assert_eq!(le_value(&bytes[..width]), VALUE & (u64::MAX >> (64 - 8 * width)), "width {width}");
    }
}
