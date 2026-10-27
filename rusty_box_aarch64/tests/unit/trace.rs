//! The page write stamps give every page of a machine's RAM a stamp of its
//! own, from the board's default 256 MiB up to Bochs's 4 GiB of pages; and a
//! trace is marked as one that may begin a counter poll only when its
//! `B.cond` tests an ordering, the conditions `poll.rs`'s walk accepts.

use super::{classify, predecode, Slot, TraceCache, MAX_STAMPS, MIN_STAMPS};

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

#[test]
fn only_a_poll_branching_on_an_ordering_is_marked() {
    for cond in 0..16u32 {
        // mrs x0, cntvct_el0; cmp x19, x0; b.<cond> 1b
        let words = [0xD53B_E040, 0xEB00_027F, 0x54FF_FFC0 | cond];
        let mut cache = TraceCache::new();
        for word in words {
            cache.pool.push(Slot {
                op: predecode(classify(word), word),
                word,
            });
        }
        // Arm ARM `ConditionHolds`: HS, LO, HI, LS, GE, LT, GT and LE order
        // the compare's operands; EQ, NE, MI, PL, VS, VC, AL and NV do not.
        let ordering = matches!(cond, 2 | 3 | 8..=13);
        assert_eq!(cache.may_poll(0, words.len()), ordering, "condition {cond}");
    }
}
