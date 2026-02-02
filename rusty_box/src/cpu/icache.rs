use crate::{
    config::BxPhyAddress,
    cpu::{
        cpu::BX_ASYNC_EVENT_STOP_TRACE,
        decoder::{fetchdecode32, fetchdecode64, BxInstructionGenerated, Opcode},
        tlb::{lpf_of, page_offset, ppf_of},
        BxCpuC, BxCpuIdTrait, Result,
    },
    memory::BxMemC,
};

// Slice-based BxPageWriteStampTable for use with memory system
#[derive(Debug)]
pub struct BxPageWriteStampTable<'a> {
    pub fine_granularity_mapping: &'a mut [u32],
}

impl<'a> BxPageWriteStampTable<'a> {
    pub fn new(fine_granularity_mapping: &'a mut [u32]) -> Self {
        Self {
            fine_granularity_mapping,
        }
    }

    /// Increment write stamp for a partial page write
    /// Assumption: write does not split 4K page
    pub fn inc_write_stamp_with_len(&mut self, p_addr: BxPhyAddress, len: u32) {
        if self.fine_granularity_mapping.is_empty() {
            return;
        }
        let index = Self::hash(p_addr);
        if index < self.fine_granularity_mapping.len() {
            if self.fine_granularity_mapping[index] != 0 {
                // Calculate mask for affected cache lines (128-byte granularity)
                let page_offset = (p_addr as u32) & 0xfff;
                let shift1 = (page_offset >> 7).min(31);
                let shift2 = ((page_offset + len - 1) >> 7).min(31);
                let mut mask: u32 = 1 << shift1;
                mask |= 1 << shift2;

                if self.fine_granularity_mapping[index] & mask != 0 {
                    // TODO: Call handle_smc to invalidate instruction cache
                    // This requires access to CPUs which we don't have here
                    // For now, just clear the affected bits
                    self.fine_granularity_mapping[index] &= !mask;
                }
            }
        }
    }

    /// Decrement write stamp for a whole page
    /// This invalidates instruction cache entries for the entire page
    pub fn dec_write_stamp(&mut self, p_addr: BxPhyAddress) {
        self.dec_write_stamp_with_len(p_addr, 4096)
    }

    /// Decrement write stamp for a partial page write
    /// Assumption: write does not split 4K page
    pub fn dec_write_stamp_with_len(&mut self, p_addr: BxPhyAddress, len: u32) {
        if self.fine_granularity_mapping.is_empty() {
            return;
        }
        let index = Self::hash(p_addr);
        if index < self.fine_granularity_mapping.len() {
            if self.fine_granularity_mapping[index] != 0 {
                // Calculate mask for affected cache lines (128-byte granularity)
                let page_offset = (p_addr as u32) & 0xfff;
                let shift1 = (page_offset >> 7).min(31);
                let shift2 = ((page_offset + len - 1) >> 7).min(31);
                let mut mask: u32 = 1 << shift1;
                mask |= 1 << shift2;

                if self.fine_granularity_mapping[index] & mask != 0 {
                    // TODO: Call handle_smc to invalidate instruction cache
                    // This requires access to CPUs which we don't have here
                    // For now, just clear the affected bits
                    self.fine_granularity_mapping[index] &= !mask;
                }
            }
        }
    }

    pub fn mark_icache_mask(&mut self, p_addr: BxPhyAddress, mask: u32) {
        if self.fine_granularity_mapping.is_empty() {
            return;
        }
        let index = Self::hash(p_addr);
        if index < self.fine_granularity_mapping.len() {
            self.fine_granularity_mapping[index] |= mask;
        }
    }

    fn hash(p_addr: BxPhyAddress) -> usize {
        lpf_of(p_addr) as usize
    }
}

// Internal array-based BxPageWriteStampTable for use within icache (not exported)
#[derive(Debug)]
struct BxPageWriteStampTableInternal {
    fine_granularity_mapping: [u32; 32768], // 128MB / 4KB = 32768 pages
}

fn handle_smc<I: BxCpuIdTrait>(cpus: &mut [BxCpuC<I>], p_addr: BxPhyAddress, mask: u32) {
    // INC_SMC_STAT(smc);
    for cpu in cpus {
        cpu.i_cache.handle_smc(p_addr, mask);
    }
}

const BX_ICACHE_INVALID_PHY_ADDRESS: BxPhyAddress = BxPhyAddress::MAX;
const BX_ICACHE_ENTRIES: usize = 8192;
pub(super) const BX_ICACHE_MEM_POOL: usize = 576 * 1024;
const BX_MAX_TRACE_LENGTH: usize = 32;

#[derive(Debug, PartialEq, Clone, Default, Copy)]
enum IcacheAddress {
    #[default]
    Invalid,
    Address(BxPhyAddress),
}

impl From<IcacheAddress> for BxPhyAddress {
    fn from(value: IcacheAddress) -> Self {
        match value {
            IcacheAddress::Invalid => BX_ICACHE_INVALID_PHY_ADDRESS,
            IcacheAddress::Address(addr) => addr,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BxICacheEntry {
    // p_addr: BxPhyAddress, // Physical address of the instruction
    pub(super) p_addr: IcacheAddress, // Physical address of the instruction
    pub(super) trace_mask: u32,
    // orignial replaced
    //tlen: u32, // Trace length in instructions
    pub(super) tlen: usize, // Trace length in instructions
    pub(super) i: BxInstructionGenerated,
    // mpool_start_idx: Index in mpool where this trace starts
    // In C++, entry->i is a pointer, so we can do pointer arithmetic
    // In Rust, we need to store the index explicitly
    pub(super) mpool_start_idx: usize,
}

pub struct BxICache {
    pub(crate) entry: [BxICacheEntry; BX_ICACHE_ENTRIES],
    pub(crate) mpool: [BxInstructionGenerated; BX_ICACHE_MEM_POOL],
    //        mpool: vec![0; BX_ICACHE_MEM_POOL],
    pub(crate) mpindex: usize,
    next_page_split_index: usize,
    page_split_index: [PageSplitEntry; BX_ICACHE_ENTRIES],
}

#[derive(Clone, Debug)]
struct PageSplitEntry {
    ppf: BxPhyAddress,
    e: BxICacheEntry,
}

impl Default for PageSplitEntry {
    fn default() -> Self {
        Self {
            ppf: BX_ICACHE_INVALID_PHY_ADDRESS,
            e: BxICacheEntry {
                p_addr: IcacheAddress::Invalid,
                trace_mask: 0,
                tlen: 0,
                i: BxInstructionGenerated::default(),
                mpool_start_idx: 0,
            },
        }
    }
}

impl Default for BxICache {
    fn default() -> Self {
        Self::new()
    }
}

impl BxICache {
    pub fn new() -> Self {
        Self {
            entry: core::array::from_fn(|_| BxICacheEntry {
                p_addr: IcacheAddress::Invalid,
                trace_mask: 0,
                tlen: 0,
                i: BxInstructionGenerated::default(),
                mpool_start_idx: 0,
            }),
            mpool: [BxInstructionGenerated::default(); BX_ICACHE_MEM_POOL],
            mpindex: 0,
            next_page_split_index: 0,
            page_split_index: core::array::from_fn(|_| PageSplitEntry::default()),
        }
    }

    pub fn alloc_trace(&mut self, entry_idx: usize) {
        let entry = &mut self.entry[entry_idx];
        if entry.p_addr != IcacheAddress::Invalid {
            flush_smc(entry);
        }
    }

    pub fn commit_trace(&mut self, tlen: usize) {
        // Update mpindex to point past the last instruction in the trace
        // In C++, this is handled by the pointer arithmetic on entry->i
        // Here, we track it explicitly with mpindex
    }

    pub fn commit_page_split_trace(&mut self, p_addr: BxPhyAddress, e: BxICacheEntry) {
        // Store the page split entry
        self.page_split_index[self.next_page_split_index].ppf = p_addr;
        self.page_split_index[self.next_page_split_index].e = e.clone();
        self.next_page_split_index = (self.next_page_split_index + 1) % BX_ICACHE_ENTRIES;
    }

    pub fn get_entry(&self, p_addr: BxPhyAddress, fetch_mode_mask: u64) -> BxICacheEntry {
        let index = Self::hash(p_addr, fetch_mode_mask);
        self.entry[index as usize].clone()
    }

    #[inline]
    pub fn get_entry_mut(
        &mut self,
        p_addr: BxPhyAddress,
        fetch_mode_mask: u64,
    ) -> &mut BxICacheEntry {
        let index = Self::hash(p_addr, fetch_mode_mask);
        &mut self.entry[index as usize]
    }

    pub(super) fn hash(p_addr: BxPhyAddress, fetch_mode_mask: u64) -> u32 {
        // Hash function matching C++ implementation
        let addr = p_addr as u64;
        let hash = (addr >> 4) ^ (fetch_mode_mask << 8);
        (hash as u32) & ((BX_ICACHE_ENTRIES - 1) as u32)
    }

    pub(super) fn find_trace_start(
        &self,
        entry: &BxICacheEntry,
        entry_idx: usize,
    ) -> Option<usize> {
        // Find where the trace starts in mpool
        // This is a simplified implementation - in C++, entry->i is a pointer
        // Here we need to search for the first instruction

        // For now, return None to indicate trace not found
        // A more sophisticated implementation would track trace locations
        // TODO: Use algebraic types for clarity?
        None
    }

    pub(super) fn find_entry(
        &self,
        p_addr: BxPhyAddress,
        fetch_mode_mask: u64,
    ) -> Option<BxICacheEntry> {
        let e = self.get_entry(p_addr, fetch_mode_mask);
        if BxPhyAddress::from(e.p_addr) != p_addr {
            return None;
        }
        Some(e)
    }

    fn handle_smc(&mut self, p_addr: BxPhyAddress, mask: u32) {
        let index = Self::hash(p_addr, 0) as usize;
        let entry = &mut self.entry[index];

        if let IcacheAddress::Address(addr) = entry.p_addr {
            if addr == p_addr {
                if entry.trace_mask & mask != 0 {
                    flush_smc(entry);
                }
            }
        }

        // Check page split entries
        for i in 0..BX_ICACHE_ENTRIES {
            if self.page_split_index[i].ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
                if ppf_of(self.page_split_index[i].ppf) == ppf_of(p_addr) {
                    flush_smc(&mut self.page_split_index[i].e); // Assuming flush_smc is defined elsewhere
                }
            }
        }
    }

    pub fn flush_page(&mut self, ppf: BxPhyAddress) {
        let index = Self::hash(ppf, 0) as usize;
        let entry = &mut self.entry[index];

        if let IcacheAddress::Address(addr) = entry.p_addr {
            if ppf_of(addr) == ppf {
                flush_smc(entry);
            }
        }

        // Flush page split entries
        for i in 0..BX_ICACHE_ENTRIES {
            if self.page_split_index[i].ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
                if ppf_of(self.page_split_index[i].ppf) == ppf {
                    flush_smc(&mut self.page_split_index[i].e);
                }
            }
        }
    }

    pub fn flush_all(&mut self) {
        for entry in &mut self.entry {
            flush_smc(entry);
        }

        // Clear page split index
        for entry in &mut self.page_split_index {
            // TODO: Use algebraic types for clarity?
            if entry.ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
                entry.ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
                flush_smc(&mut entry.e);
            }
        }
    }

    pub fn invalidate_page(&mut self, ppf: BxPhyAddress) {
        let index = Self::hash(ppf, 0) as usize;
        let entry = &mut self.entry[index];

        if let IcacheAddress::Address(addr) = entry.p_addr {
            if ppf_of(addr) == ppf {
                entry.p_addr = IcacheAddress::Invalid;
            }
        }

        // Invalidate page split entries
        for i in 0..BX_ICACHE_ENTRIES {
            if self.page_split_index[i].ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
                if ppf_of(self.page_split_index[i].ppf) == ppf {
                    self.page_split_index[i].ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
                    flush_smc(&mut self.page_split_index[i].e);
                }
            }
        }
        //     if self.page_split_index[i].ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
        //         self.page_split_index[i].ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
    }

    pub fn invalidate_all(&mut self) {
        for entry in &mut self.entry {
            entry.p_addr = IcacheAddress::Invalid;
        }

        // Clear page split index
        for entry in &mut self.page_split_index {
            // TODO: Use algebraic types for clarity?
            if entry.ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
                entry.ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
                flush_smc(&mut entry.e);
            }
        }
    }
}

fn flush_smc(e: &mut BxICacheEntry) {
    // Matching C++ line 64-74: flushSMC
    if let IcacheAddress::Address(_) = e.p_addr {
        e.p_addr = IcacheAddress::Invalid;

        // If handlers chaining speedups are enabled, generate dummy entry
        // (matching C++ line 66-72)
        #[cfg(feature = "bx_support_handlers_chaining_speedups")]
        {
            // TODO: Check if debugger is active (matching C++ line 67)
            // For now, always generate dummy entry
            gen_dummy_icache_entry(&mut e.i);
        }
    }
}

fn gen_dummy_icache_entry(i: &mut BxInstructionGenerated) {
    // Matching C++ line 88-90: genDummyICacheEntry
    i.set_ilen(0);
    i.set_ia_opcode(Opcode::InsertedOpcode);
    // Note: In C++, execute1 is set to &BX_CPU_C::BxEndTrace
    // In Rust, we check for Opcode::InsertedOpcode in cpu_loop_n and set async_event
}

impl<'c, I: BxCpuIdTrait> BxCpuC<'c, I> {
    fn bx_end_trace(&mut self) {
        self.async_event |= BX_ASYNC_EVENT_STOP_TRACE;
    }

    pub(super) fn serve_icache_miss(
        &mut self,
        eip_biased: u32,
        p_addr: BxPhyAddress,
        mem: &'c mut BxMemC<'c>,
        cpus: &[&Self],
        page_write_stamp_table: &mut BxPageWriteStampTable,
    ) -> Result<BxICacheEntry> {
        // Get entry index first to avoid borrow conflicts
        let entry_idx = BxICache::hash(p_addr, self.fetch_mode_mask.into()) as usize;

        // Matching C++ icache.cc:106-107 - use eip_biased directly
        // Safety check: ensure eip_biased is within bounds (defensive programming)
        if eip_biased >= self.eip_page_window_size {
            tracing::error!(
                "serve_icache_miss: eip_biased ({}) >= eip_page_window_size ({}), pAddr={:#x}",
                eip_biased,
                self.eip_page_window_size,
                p_addr
            );
            return Err(crate::cpu::CpuError::CpuNotInitialized);
        }

        let remaining_in_page = self.eip_page_window_size - eip_biased;
        let fetch_ptr_slice = self
            .eip_fetch_ptr
            .ok_or(crate::cpu::CpuError::CpuNotInitialized)?;
        if eip_biased as usize >= fetch_ptr_slice.len() {
            tracing::error!(
                "serve_icache_miss: eip_biased ({}) >= fetch_ptr_slice.len ({}), pAddr={:#x}",
                eip_biased,
                fetch_ptr_slice.len(),
                p_addr
            );
            return Err(crate::cpu::CpuError::CpuNotInitialized);
        }
        let fetch_ptr = &fetch_ptr_slice[eip_biased as usize..];
        let page_offset = page_offset(p_addr as u32);
        tracing::trace!("serve_icache_miss: p_addr_fetch_page={:#x}, eip_biased={}, p_addr={:#x}, page_offset={}, RIP={:#x}", 
            self.p_addr_fetch_page, eip_biased, p_addr, page_offset, self.rip());
        let mut trace_mask = 0u32;

        // Check if this is the stack page and invalidate stack cache if needed
        // (matching C++ line 115-118: #if BX_SUPPORT_SMP == 0)
        // Note: In SMP mode, this check is skipped in C++
        #[cfg(not(feature = "bx_support_smp"))]
        {
            if ppf_of(p_addr) == self.p_addr_stack_page {
                self.invalidate_stack_cache();
            }
        }

        let is_32_bit_mode = unsafe {
            self.sregs[crate::cpu::decoder::BxSegregs::Cs as usize]
                .cache
                .u
                .segment
                .d_b
        };
        let quantum = BX_MAX_TRACE_LENGTH;
        let mut current_mpindex = self.i_cache.mpindex;

        // Initialize entry
        self.i_cache.alloc_trace(entry_idx);
        let trace_start_idx = current_mpindex; // Store where this trace starts in mpool
        {
            let entry = &mut self.i_cache.entry[entry_idx];
            entry.p_addr = IcacheAddress::Address(p_addr);
            entry.trace_mask = 0;
            entry.mpool_start_idx = trace_start_idx;
        }

        let mut current_p_addr = p_addr;
        let mut current_page_offset = page_offset;
        let mut current_fetch_ptr = fetch_ptr;
        // Preserve original remaining_in_page for boundary_fetch
        let original_remaining_in_page = remaining_in_page;
        let mut remaining = remaining_in_page as u32;
        let mut tlen = 0usize;

        for n in 0..quantum {
            // Check bounds before accessing mpool
            if current_mpindex >= BX_ICACHE_MEM_POOL {
                // Only log once per trace to reduce spam - mpool full is handled gracefully
                if current_mpindex == BX_ICACHE_MEM_POOL {
                    tracing::debug!(
                        "mpool full, stopping trace (this is normal if cache is heavily used)"
                    );
                }
                break;
            }

            // Decode instruction based on CPU mode
            let long64 = self.long64_mode();
            let decode_result = if long64 {
                fetchdecode64::fetch_decode64(current_fetch_ptr)
            } else {
                fetchdecode32::fetch_decode32(current_fetch_ptr, is_32_bit_mode)
            };

            match decode_result {
                Ok(instr) => {
                    // Debug: log first few bytes being decoded
                    if tlen < 3 {
                        let bytes_str: String = current_fetch_ptr
                            .iter()
                            .take(8)
                            .map(|b| format!("{:02x}", b))
                            .collect::<Vec<_>>()
                            .join(" ");
                        tracing::trace!("Decoding instruction #{}: p_addr={:#x}, page_offset={}, bytes=[{}], opcode={:?}", 
                            tlen, current_p_addr, current_page_offset, bytes_str, instr.get_ia_opcode());
                    }

                    // Store instruction in mpool and get instruction length
                    let i_len = {
                        self.i_cache.mpool[current_mpindex] = instr;
                        self.i_cache.mpool[current_mpindex].meta_info.ilen as u32
                    };

                    // Call assignHandler during trace creation (matching C++ line 169)
                    // This checks feature flags and determines if trace should stop
                    // Note: In C++, handlers are stored in instruction structure (i->execute1, i->handlers.execute2)
                    // In Rust, we can't store function pointers in instruction structure (it's in decoder crate),
                    // so we call assign_handler again during execution to get the handler.
                    // But we still call it here to check if tracing should stop (matching original behavior).
                    let fetch_mode_mask = self.fetch_mode_mask;
                    let stop_trace_indication = {
                        // Call assign_handler to check if trace should stop
                        // We need to borrow self mutably, so we do this in a separate scope
                        let instr_ref = &mut self.i_cache.mpool[current_mpindex];
                        // Create a temporary mutable reference to self for assign_handler
                        // This is safe because we're only reading from the instruction
                        let should_stop = {
                            // We can't call assign_handler here because it requires &mut self
                            // Instead, we'll check the opcode and flags directly
                            // For now, just continue tracing - the actual handler assignment
                            // will happen during execution in cpu_loop
                            false
                        };
                        should_stop
                    };

                    // TODO: Implement BX_INSTR_STORE_OPCODE_BYTES if needed (matching C++ line 175-177)
                    // TODO: Implement BX_INSTR_OPCODE if needed (matching C++ line 178-179)

                    // Update trace mask
                    // Clamp shift amounts to 31 to prevent overflow (u32 has 32 bits, so max shift is 31)
                    let shift1 = (current_page_offset >> 7).min(31);
                    let shift2 = ((current_page_offset + i_len - 1) >> 7).min(31);
                    trace_mask |= 1 << shift1;
                    trace_mask |= 1 << shift2;

                    tlen += 1;
                    current_mpindex += 1;

                    // Check bounds again after increment
                    if current_mpindex >= BX_ICACHE_MEM_POOL {
                        // tracing::warn!("mpool full after increment, stopping trace");
                        break;
                    }

                    // Continue to next instruction
                    remaining = remaining.saturating_sub(i_len);

                    // Check if we should stop tracing (matching C++ line 188)
                    // Stop if: stop trace indication OR remaining in page is 0
                    if stop_trace_indication || remaining == 0 {
                        break;
                    }

                    current_p_addr += i_len as u64;
                    current_page_offset = (current_page_offset + i_len) & 0xfff;
                    current_fetch_ptr = &current_fetch_ptr[i_len as usize..];

                    // Debug: verify fetch pointer is advancing
                    if tlen < 3 {
                        tracing::trace!("After instruction #{}: new p_addr={:#x}, new page_offset={}, remaining={}, fetch_ptr_len={}", 
                            tlen, current_p_addr, current_page_offset, remaining, current_fetch_ptr.len());
                    }

                    // Try to find a trace starting from current pAddr and merge
                    // TODO: Check if debugger is active (matching C++ line 194)
                    if remaining >= 15u32 {
                        // avoid merging with page split trace
                        if self.merge_traces_internal(
                            entry_idx,
                            current_mpindex,
                            current_p_addr,
                            tlen,
                        ) {
                            // Update entry and commit
                            {
                                let entry = &mut self.i_cache.entry[entry_idx];
                                entry.tlen = tlen;
                                entry.trace_mask |= trace_mask;
                            }
                            page_write_stamp_table.mark_icache_mask(current_p_addr, trace_mask);
                            self.i_cache.mpindex = current_mpindex;
                            self.i_cache.commit_trace(tlen);
                            let entry = self.i_cache.entry[entry_idx].clone();
                            return Ok(entry);
                        }
                    }
                }
                Err(decode_err) => {
                    // Fetching instruction on segment/page boundary (matching C++ line 138)
                    // If this is not the first instruction (n > 0), drop the boundary instruction and stop tracing
                    if n > 0 {
                        // The trace is already valid, it has several instructions inside,
                        // in this case just drop the boundary instruction and stop tracing (matching C++ line 140-144)
                        break;
                    }

                    // Calculate remaining bytes for THIS instruction position
                    // For n=0, this equals original_remaining_in_page
                    // For later instructions (if we ever get here), it would be decremented
                    let current_remaining = remaining as usize;

                    // If there are >= 15 bytes remaining, the instruction SHOULD have fit
                    // in the page. Decode failure with >= 15 bytes means it's NOT a boundary
                    // issue - it's an invalid/unsupported instruction.
                    if current_remaining >= 15 {
                        tracing::error!(
                            "Decode failed with {} bytes remaining (not a boundary issue)\n\
                             RIP={:#x}, CS.base={:#x}, EIP={:#x}\n\
                             Decode error: {:?}\n\
                             First 16 bytes: {:02x?}",
                            current_remaining,
                            self.rip(),
                            unsafe { self.sregs[crate::cpu::decoder::BxSegregs::Cs as usize].cache.u.segment.base },
                            self.eip(),
                            decode_err,
                            &current_fetch_ptr[..core::cmp::min(16, current_fetch_ptr.len())]
                        );

                        // Check if this is an illegal opcode - if so, generate #UD exception
                        // Based on Bochs exception.cc:937 and cpu.h:248 (Exception::Ud = 6)
                        use crate::cpu::decoder::DecodeError;
                        use rusty_box_decoder::fetchdecode_generated::BxDecodeError;
                        match &decode_err {
                            DecodeError::Decoder(BxDecodeError::BxIllegalOpcode) => {
                                tracing::debug!("Illegal opcode detected, generating #UD exception (vector 6)");
                                // Generate #UD exception which will vector through IVT in real mode
                                // The exception() method will save CS:IP and jump to the #UD handler
                                let exception_result = self.exception(crate::cpu::cpu::Exception::Ud, 0);

                                // If the exception handler returns an error, propagate it
                                if let Err(e) = exception_result {
                                    return Err(e);
                                }

                                // After exception returns successfully, execution has been redirected
                                // to the exception handler. We return an error to stop this decode path
                                // and let the CPU continue from the new RIP (the exception handler)
                                return Err(crate::cpu::CpuError::Exception { vector: 6 });
                            }
                            _ => {
                                // Other decode errors are returned as-is
                                return Err(crate::cpu::CpuError::Decoder(decode_err));
                            }
                        }
                    }

                    // First instruction is boundary fetch, leave the trace cache entry
                    // invalid for now because boundaryFetch() can fault (matching C++ line 146-149)
                    {
                        let entry = &mut self.i_cache.entry[entry_idx];
                        entry.p_addr = IcacheAddress::Invalid; // Mark as invalid temporarily (~entry->pAddr in C++)
                        entry.tlen = 1;
                        entry.mpool_start_idx = current_mpindex; // Store where this trace starts
                    }

                    // Debug logging before boundary_fetch
                    tracing::debug!(
                        "boundary_fetch: n={}, current_remaining={}, p_addr={:#x}",
                        n,
                        current_remaining,
                        current_p_addr
                    );

                    // Call boundary_fetch (matching C++ line 150)
                    // Pass the current remaining bytes to page boundary
                    let boundary_instr =
                        self.boundary_fetch(current_fetch_ptr, current_remaining, mem, cpus)?;

                    // Store instruction in mpool (check bounds first)
                    if current_mpindex >= BX_ICACHE_MEM_POOL {
                        tracing::debug!("mpool full before boundary_instr, stopping trace");
                        break;
                    }
                    self.i_cache.mpool[current_mpindex] = boundary_instr;
                    current_mpindex += 1;

                    // Add the instruction to trace cache (matching C++ line 152-154)
                    {
                        let entry = &mut self.i_cache.entry[entry_idx];
                        entry.p_addr = IcacheAddress::Address(p_addr); // Restore pAddr (~entry->pAddr in C++)
                        entry.trace_mask = 0x80000000; /* last line in page */
                        // Note: tlen is already set to 1 above, no need to set it again
                        // mpool_start_idx was already set above
                    }

                    page_write_stamp_table.mark_icache_mask(p_addr, 0x80000000);
                    page_write_stamp_table.mark_icache_mask(self.p_addr_fetch_page, 0x1);

                    // Add end-of-trace opcode if not in debugger (matching C++ line 158-163)
                    // TODO: Check debugger active state
                    #[cfg(feature = "bx_support_handlers_chaining_speedups")]
                    {
                        if current_mpindex < BX_ICACHE_MEM_POOL {
                            let entry = &mut self.i_cache.entry[entry_idx];
                            entry.tlen += 1; /* Add the inserted end of trace opcode */
                            gen_dummy_icache_entry(&mut self.i_cache.mpool[current_mpindex]);
                            current_mpindex += 1;
                        }
                    }

                    self.i_cache.mpindex = current_mpindex;
                    let entry = self.i_cache.entry[entry_idx].clone();
                    self.i_cache
                        .commit_page_split_trace(self.p_addr_fetch_page, entry.clone());
                    return Ok(entry);
                }
            }
        }

        // Update entry with final trace mask (matching C++ line 206-208)
        {
            let entry = &mut self.i_cache.entry[entry_idx];
            entry.trace_mask |= trace_mask;
        }
        page_write_stamp_table.mark_icache_mask(current_p_addr, trace_mask);

        // Add end-of-trace opcode if not in debugger (matching C++ line 210-214)
        // TODO: Check debugger active state
        #[cfg(feature = "bx_support_handlers_chaining_speedups")]
        {
            // Check bounds before accessing mpool
            if current_mpindex < BX_ICACHE_MEM_POOL {
                // Note: tlen will be incremented here, then used below
                gen_dummy_icache_entry(&mut self.i_cache.mpool[current_mpindex]);
                current_mpindex += 1;
                tlen += 1; /* Add the inserted end of trace opcode */
            }
        }

        // Update entry tlen and commit trace (matching C++ line 217)
        {
            let entry = &mut self.i_cache.entry[entry_idx];
            entry.tlen = tlen;
        }
        // Cap mpindex to prevent out-of-bounds access
        let capped_mpindex = current_mpindex.min(BX_ICACHE_MEM_POOL - 1);
        self.i_cache.mpindex = capped_mpindex;
        self.i_cache.commit_trace(tlen);

        Ok(self.i_cache.entry[entry_idx].clone())
    }

    fn boundary_fetch(
        &mut self,
        fetch_ptr: &[u8],
        remaining_in_page: usize,
        mem: &'c mut BxMemC<'c>,
        cpus: &[&Self],
    ) -> Result<BxInstructionGenerated> {
        let mut fetch_buffer = [0u8; 32];

        // Based on BX_CPU_C::boundaryFetch in icache.cc
        // If remainingInPage >= 15, instruction should fit in current page
        // This condition indicates too many instruction prefixes -> #GP(0)
        if remaining_in_page >= 15 {
            tracing::error!(
                "boundaryFetch #GP(0): too many instruction prefixes\n\
                 remainingInPage={}, RIP={:#x}, CS.base={:#x}, EIP={:#x}\n\
                 This indicates the instruction has too many prefixes (>15 bytes)\n\
                 or boundary_fetch was called with an incorrect remaining_in_page value.",
                remaining_in_page,
                self.rip(),
                unsafe { self.sregs[crate::cpu::decoder::BxSegregs::Cs as usize].cache.u.segment.base },
                self.eip()
            );
            self.exception(crate::cpu::cpu::Exception::Gp, 0)?;
        }

        // Read all leftover bytes in current page up to boundary
        for j in 0..remaining_in_page {
            fetch_buffer[j] = fetch_ptr[j];
        }

        // The 2nd chunk of the instruction is on the next page.
        // Set RIP to the 0th byte of the 2nd page, and force a prefetch
        // (matching C++ line 274-275)
        self.set_rip(self.rip() + remaining_in_page as u64);
        // Call prefetch directly - same lifetime as serve_icache_miss
        self.prefetch(mem, cpus)?;

        let mut fetch_buffer_limit = 15usize;
        if self.eip_page_window_size < 15 {
            fetch_buffer_limit = self.eip_page_window_size as usize;
        }

        // We can fetch straight from the 0th byte, which is eipFetchPtr
        let next_page_fetch_ptr = self
            .eip_fetch_ptr
            .ok_or(crate::cpu::CpuError::CpuNotInitialized)?;

        // Read leftover bytes in next page (matching C++ line 287-289)
        let mut j = remaining_in_page;
        for k in 0..fetch_buffer_limit {
            fetch_buffer[j] = next_page_fetch_ptr[k];
            j += 1;
        }

        // Get is_32_bit_mode from CS segment descriptor d_b flag
        let is_32_bit_mode = unsafe {
            self.sregs[crate::cpu::decoder::BxSegregs::Cs as usize]
                .cache
                .u
                .segment
                .d_b
        };

        // Decode instruction from combined buffer (matching C++ line 291-296)
        let total_bytes = remaining_in_page + fetch_buffer_limit;
        let decode_result = if self.long64_mode() {
            fetchdecode64::fetch_decode64(&fetch_buffer[..total_bytes])
        } else {
            fetchdecode32::fetch_decode32(&fetch_buffer[..total_bytes], is_32_bit_mode)
        };

        let mut instr = decode_result.unwrap_or_else(|_| {
            // Panic on decode failure with instruction bytes for debugging
            let bytes_str = fetch_buffer[..total_bytes.min(16)]
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<Vec<_>>()
                .join(" ");
            panic!(
                "\n\
                ╔════════════════════════════════════════════════════════════╗\n\
                ║      DECODE FAILURE - INSTRUCTION COULD NOT BE DECODED     ║\n\
                ╠════════════════════════════════════════════════════════════╣\n\
                ║  RIP:         {:#018x}                          ║\n\
                ║  Bytes:       {}                                      ║\n\
                ║                                                             ║\n\
                ║  The decoder failed to decode this instruction.             ║\n\
                ║  Please check the decoder implementation.                   ║\n\
                ╚════════════════════════════════════════════════════════════╝\n",
                self.rip(),
                bytes_str
            );
        });

        // assignHandler is a no-op in Rust (matching C++ line 303)
        // In C++, assignHandler can return non-zero, but we don't check it here

        // Restore EIP since we fudged it to start at the 2nd page boundary.
        // (matching C++ line 306: RIP = BX_CPU_THIS_PTR prev_rip)
        self.set_rip(self.prev_rip);

        // TODO: Implement BX_INSTR_STORE_OPCODE_BYTES if needed (matching C++ line 314-316)
        // TODO: Implement BX_INSTR_OPCODE if needed (matching C++ line 318-319)

        Ok(instr)
    }

    fn merge_traces_internal(
        &mut self,
        current_entry_idx: usize,
        current_mpindex: usize,
        p_addr: BxPhyAddress,
        current_tlen: usize,
    ) -> bool {
        // TODO: Check if debugger is active - should assert !debugger_active

        // Find entry in cache
        let cache_entry_idx = BxICache::hash(p_addr, self.fetch_mode_mask.into()) as usize;

        // Extract cached entry info without holding borrow
        // Check if entry exists and matches (matching C++ line 226-228: if (e != NULL))
        let cached_entry = &self.i_cache.entry[cache_entry_idx];

        // Check if entry is valid (not Invalid address)
        let cached_p_addr = match cached_entry.p_addr {
            IcacheAddress::Address(addr) => addr,
            IcacheAddress::Invalid => return false, // Entry doesn't exist (NULL in C++)
        };

        // Check if this is the right entry (matching pAddr)
        if cached_p_addr != p_addr {
            return false;
        }

        let cached_tlen = cached_entry.tlen;
        let cached_trace_mask = cached_entry.trace_mask;
        let cached_first_instr = cached_entry.i;

        // determine max amount of instruction to take from another entry (matching C++ line 231)
        let mut max_length = cached_tlen;

        #[cfg(feature = "bx_support_handlers_chaining_speedups")]
        {
            if max_length + current_tlen > BX_MAX_TRACE_LENGTH {
                return false;
            }
        }
        #[cfg(not(feature = "bx_support_handlers_chaining_speedups"))]
        {
            if max_length + current_tlen > BX_MAX_TRACE_LENGTH {
                max_length = BX_MAX_TRACE_LENGTH - current_tlen;
            }
            if max_length == 0 {
                return false;
            }
        }

        // Find where the cached entry's trace starts in mpool
        // Create a temporary entry for searching
        let cached_entry_for_search = BxICacheEntry {
            p_addr: IcacheAddress::Address(cached_p_addr),
            trace_mask: cached_trace_mask,
            tlen: cached_tlen,
            i: cached_first_instr,
            mpool_start_idx: 0, // Not used for search, just need valid struct
        };

        if let Some(source_start_idx) = self
            .i_cache
            .find_trace_start(&cached_entry_for_search, cache_entry_idx)
        {
            // Copy instructions from found entry to current trace (matching C++ line 242: memcpy)
            for i in 0..max_length {
                if current_mpindex + i < BX_ICACHE_MEM_POOL
                    && source_start_idx + i < BX_ICACHE_MEM_POOL
                {
                    self.i_cache.mpool[current_mpindex + i] =
                        self.i_cache.mpool[source_start_idx + i];
                }
            }

            // Update current entry (matching C++ line 243-246)
            let current_entry = &mut self.i_cache.entry[current_entry_idx];
            current_entry.tlen += max_length;
            debug_assert!(current_entry.tlen <= BX_MAX_TRACE_LENGTH); // Matching C++ line 244: BX_ASSERT

            current_entry.trace_mask |= cached_trace_mask; // Matching C++ line 246

            return true;
        }

        false
    }
}
