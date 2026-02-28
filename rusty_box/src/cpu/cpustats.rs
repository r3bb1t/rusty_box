#[derive(Debug, Default)]
pub struct BxCpuStatistics {
    // icache statistics
    pub(crate) i_cache_lookups: u64,
    pub(crate) i_cache_prefetch: u64,
    pub(crate) i_cache_misses: u64,

    // tlb lookup statistics
    pub(crate) tlb_lookups: u64,
    pub(crate) tlb_execute_lookups: u64,
    pub(crate) tlb_write_lookups: u64,
    pub(crate) tlb_misses: u64,
    pub(crate) tlb_execute_misses: u64,
    pub(crate) tlb_write_misses: u64,

    // tlb flush statistics
    pub(crate) tlb_global_flushes: u64,
    pub(crate) tlb_non_global_flushes: u64,

    // stack prefetch statistics
    pub(crate) stack_prefetch: u64,

    // self modifying code statistics
    pub(crate) smc: u64,
}
