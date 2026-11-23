//! Shared x86 code-write bookkeeping (Bochs icache.h / icache.cc).
//! PC memory retains its stamp hash; dense RAM uses caller-supplied stamps.
use crate::config::BxPhyAddress;
use crate::cpu::icache::{PendingSmc, SMC_PENDING_CAP, smc_cache_line_mask};

#[derive(Debug)]
pub(crate) struct CodeWrites {
    pending: [PendingSmc; SMC_PENDING_CAP],
    len: usize,
    next: u64,
    overflow: u64,
}

impl CodeWrites {
    pub(crate) fn new() -> Self {
        Self {
            pending: [PendingSmc::default(); SMC_PENDING_CAP],
            len: 0,
            next: 0,
            overflow: 0,
        }
    }

    #[inline(always)]
    pub(crate) fn write(&mut self, stamps: &mut [u32], index: usize, addr: BxPhyAddress, len: u32) {
        if stamps[index] == 0 {
            return;
        }
        self.write_mask(stamps, index, addr, smc_cache_line_mask(addr, len));
    }

    #[inline(always)]
    pub(crate) fn write_mask(
        &mut self,
        stamps: &mut [u32],
        index: usize,
        addr: BxPhyAddress,
        mask: u32,
    ) {
        if stamps[index] & mask == 0 {
            return;
        }
        stamps[index] &= !mask;
        self.push(addr, mask);
    }

    /// Code hits are rare; keep queue publication out of ordinary RAM stores.
    #[cold]
    #[inline(never)]
    fn push(&mut self, addr: BxPhyAddress, mask: u32) {
        if self.len == SMC_PENDING_CAP {
            // Readers below this watermark may have missed discarded
            // events. Readers at it can still apply the new suffix.
            self.overflow = self.next;
            self.len = 0;
        }
        self.pending[self.len] = PendingSmc { p_addr: addr, mask };
        self.len += 1;
        self.next += 1;
    }

    #[inline]
    pub(crate) fn next(&self) -> u64 {
        self.next
    }

    #[inline]
    pub(crate) fn since(&self, since: u64) -> (bool, &[PendingSmc]) {
        let base = self.next - self.len as u64;
        let start = since.saturating_sub(base) as usize;
        (
            since < self.overflow,
            &self.pending[start.min(self.len)..self.len],
        )
    }

    pub(crate) fn clear(&mut self) {
        self.len = 0;
    }

    pub(crate) fn has_pending(&self) -> bool {
        self.len != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_log_keeps_new_writes_available_to_a_caught_up_reader() {
        let mut writes = CodeWrites::new();
        let mut stamp = [0];
        for page in 0..=2 * SMC_PENDING_CAP {
            let caught_up = writes.next();
            let addr = BxPhyAddress::new((page as u64) << 12);
            stamp[0] = 1;
            writes.write_mask(&mut stamp, 0, addr, 1);
            assert_eq!(stamp[0], 0);
            let (flush_all, unseen) = writes.since(caught_up);
            assert!(!flush_all);
            assert_eq!(unseen.len(), 1);
            assert_eq!(unseen[0].p_addr, addr);
        }

        let newest_batch = (2 * SMC_PENDING_CAP) as u64;
        assert!(writes.since(newest_batch - 1).0);
        assert!(!writes.since(newest_batch).0);
        writes.clear(); // all consumers have now applied the new suffix
        let (flush_all, unseen) = writes.since(writes.next());
        assert!(!flush_all);
        assert!(unseen.is_empty());
    }
}
