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
        if self.len < SMC_PENDING_CAP {
            self.pending[self.len] = PendingSmc { p_addr: addr, mask };
            self.len += 1;
        } else {
            self.overflow = self.next + 1;
        }
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
