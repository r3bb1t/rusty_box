#![allow(dead_code)]
//! CRC-32 port of Bochs's `crc.cc`.
//!
//! Source credit: Rob Warnock <rpw3@sgi.com>, FDDI CRC-32 in BE/BE byte/bit
//! order. Polynomial: AUTODIN II / Ethernet / FDDI (0x04c11db7).
//!
//! Bochs computed the 256-entry table lazily on first call. We compute it at
//! compile time via `const fn`, which eliminates the runtime lock and lets us
//! drop the `spin` dependency.

/// AUTODIN II / Ethernet / FDDI polynomial.
const CRC32_POLY: u32 = 0x04c11db7;

/// Build the auxiliary table for parallel byte-at-a-time CRC-32.
/// `const fn` — evaluated at compile time, table baked into .rodata.
const fn build_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i: usize = 0;
    while i < 256 {
        // Bochs: `c = i << 24` — seed with byte in the top 8 bits.
        let mut c: u32 = (i as u32) << 24;
        let mut j = 0;
        while j < 8 {
            // Bochs: `c = c & 0x80000000 ? (c << 1) ^ CRC32_POLY : (c << 1)`.
            c = if c & 0x8000_0000 != 0 {
                (c << 1) ^ CRC32_POLY
            } else {
                c << 1
            };
            j += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

/// Precomputed CRC-32 table, baked into the binary.
static CRC32_TABLE: [u32; 256] = build_crc32_table();

/// Access the CRC-32 table. Kept as a function to preserve the old call shape.
#[inline]
pub fn crc32_table() -> &'static [u32; 256] {
    &CRC32_TABLE
}

/// CRC-32 over `buf`, per FDDI / AUTODIN II / Ethernet.
///
/// Matches Bochs `crc32(buf, len)` exactly:
/// - preload shift register to `0xFFFF_FFFF`
/// - `crc = (crc << 8) ^ table[(crc >> 24) ^ byte]` per byte
/// - transmit complement (`~crc`)
pub fn crc32(buf: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in buf {
        let idx = ((crc >> 24) as u8 ^ byte) as usize;
        crc = (crc << 8) ^ CRC32_TABLE[idx];
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity: table[0] must be 0 and table[1] must be the polynomial shifted
    /// appropriately. Bochs uses `crc32_table[1]` as its "initialized" sentinel.
    #[test]
    fn table_well_formed() {
        assert_eq!(CRC32_TABLE[0], 0);
        assert_ne!(CRC32_TABLE[1], 0, "table[1] is Bochs's init sentinel");
    }

    /// "123456789" is the canonical CRC-32 test vector.
    /// AUTODIN II (MSB-first, non-reflected) checksum of "123456789" is 0xFC891918.
    #[test]
    fn canonical_vector() {
        assert_eq!(crc32(b"123456789"), 0xFC891918);
    }

    /// Empty input should return the complement of the initial register (0).
    #[test]
    fn empty_input() {
        assert_eq!(crc32(b""), 0x0000_0000);
    }
}
