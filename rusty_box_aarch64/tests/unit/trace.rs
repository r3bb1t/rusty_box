//! The page write stamps give every page of a machine's RAM a stamp of its
//! own, from the board's default 256 MiB up to Bochs's 4 GiB of pages.

use super::{TraceCache, MAX_STAMPS, MIN_STAMPS};

const RAM_BASE: u64 = 0x4000_0000;

#[test]
fn every_page_of_ram_has_a_stamp_of_its_own() {
    for (bytes, stamps) in [
        (64u64 << 20, MIN_STAMPS),
        (256 << 20, MIN_STAMPS),
        ((256 << 20) + 4096, 2 * MIN_STAMPS),
        (1 << 30, 4 * MIN_STAMPS),
        (16 << 30, MAX_STAMPS),
    ] {
        let mut cache = TraceCache::new();
        cache.cover(bytes);
        assert_eq!(cache.stamp_count(), stamps, "{bytes:#x} bytes");
        let pages = (bytes / 4096).min(stamps as u64);
        let first = cache.stamp_index(RAM_BASE);
        let last = cache.stamp_index(RAM_BASE + 4096 * (pages - 1));
        assert_eq!(last, (first + pages as usize - 1) % stamps, "{bytes:#x} bytes: pages numbered apart");
        // A page as far past the first as the table is long shares its stamp.
        assert_eq!(cache.stamp_index(RAM_BASE + 4096 * stamps as u64), first);
    }
}
