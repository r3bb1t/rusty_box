#[derive(Debug, Default)]
pub struct BxCpuStatistics {
    // icache statistics
    pub i_cache_lookups: u64,
    pub i_cache_prefetch: u64,
    pub i_cache_misses: u64,

    // tlb lookup statistics
    pub tlb_lookups: u64,
    pub tlb_execute_lookups: u64,
    pub tlb_write_lookups: u64,
    pub tlb_misses: u64,
    pub tlb_execute_misses: u64,
    pub tlb_write_misses: u64,

    // tlb flush statistics
    pub tlb_global_flushes: u64,
    pub tlb_non_global_flushes: u64,

    // stack prefetch statistics
    pub stack_prefetch: u64,

    // self modifying code statistics
    pub smc: u64,
}
