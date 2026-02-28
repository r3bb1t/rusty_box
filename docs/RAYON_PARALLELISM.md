# Rayon Data Parallelism Integration

## Overview

The `data_parallelism` Cargo feature enables optional rayon-based parallel iteration
for bulk data operations. The feature requires `std` and is disabled by default.

```toml
# Enable rayon parallelism
cargo build --features "std,data_parallelism"

# Default (no rayon)
cargo build --features "std"

# no_std (rayon not available)
cargo build --no-default-features --features "bx_full"
```

## Where Rayon Is Applied

### TLB Operations (`cpu/tlb.rs`)

| Operation | Elements | Description |
|-----------|----------|-------------|
| `flush()` | 2048-3072 | Full TLB invalidation (CR3/CR4 write) |
| `flush_non_global()` | 2048-3072 | Non-global invalidation with fold/reduce for mask accumulator |
| `invlpg()` split_large path | 2048-3072 | Large-page INVLPG scan with fold/reduce |

### ICache Operations (`cpu/icache.rs`)

| Operation | Elements | Description |
|-----------|----------|-------------|
| `flush_all()` | 16,384 | Full icache + page_split_index invalidation |
| `invalidate_all()` | 16,384 | All entry invalidation |
| `handle_smc_scan()` | 8,192 | Self-modifying code detection scan |
| `handle_smc()` | 8,192 | SMC page split scan |
| `flush_page()` | 8,192 | Page split index scan |
| `invalidate_page()` | 8,192 | Page split index scan |
| `reset_write_stamps()` | 1,048,576 | Page write stamp bulk clear (4MB) |

### VGA Rendering (`gui/shared_display.rs`)

| Operation | Elements | Description |
|-----------|----------|-------------|
| `render_text_to_framebuffer()` | 25 rows × 80 cols | Row-parallel text-to-RGBA rendering |

### Memory Init (`memory/memory_stub.rs`)

| Operation | Elements | Description |
|-----------|----------|-------------|
| ROM fill 0xFF | ~4MB | One-time ROM area initialization |

## Where Rayon Does NOT Help

### CPU Execution Loop (`cpu/cpu.rs`)

The CPU loop is **fundamentally sequential** — each x86 instruction depends on the
state from the previous instruction (registers, flags, memory). You cannot execute
instruction N+1 until instruction N completes. Rayon's data parallelism doesn't
apply here.

### Instruction Prefetch/Decode (`cpu/cpu.rs` get_icache_entry)

Instruction decoding requires knowing where each instruction starts, which depends
on the length of the previous instruction. This creates a serial dependency chain
that cannot be parallelized.

### The only way to parallelize CPU execution would be SMP (multiple CPU cores),
which is a much larger architectural change.

## Performance Measurements

### Headless Mode (300M instructions, DLX Linux 1.3.89 boot)

| Metric | Without Rayon | With Rayon | Delta |
|--------|-------------|----------|-------|
| Run 1 | 2.24 MIPS (133.9s) | 2.22 MIPS (135.3s) | -0.9% |
| Run 2 | 2.20 MIPS (136.1s) | 2.25 MIPS (133.1s) | +2.3% |
| **Average** | **2.22 MIPS** | **2.24 MIPS** | **~0%** |

**Conclusion**: No measurable improvement in headless mode. This is expected because:
1. The CPU execution loop (sequential) dominates runtime (~95%)
2. TLB/ICache flushes are infrequent (hundreds of times in 300M instructions)
3. VGA rendering is not active in headless mode
4. Memory init is a one-time cost

### Expected GUI Mode Impact

In GUI mode, `render_text_to_framebuffer()` is called every frame (15-60 FPS).
With 25 rows parallelized across 8+ threads, frame rendering should be 3-6x faster.
This would be visible as smoother GUI response, especially at higher refresh rates.

## Architecture Notes

- All rayon code paths are gated behind `#[cfg(feature = "data_parallelism")]`
- Sequential fallbacks exist for all operations (identical logic, no rayon imports)
- `no_std` builds are unaffected — rayon is never compiled
- TLB `flush_non_global()` and `invlpg()` use rayon `fold()/reduce()` to accumulate
  `lpf_mask` across threads without shared mutable state
- VGA rendering uses `par_chunks_mut()` to give each thread its own row slice,
  avoiding framebuffer contention

## When Rayon Matters

Rayon will have significant impact when:
1. **SMP emulation** is added (multiple emulated CPUs on separate threads)
2. **Large memory operations** become common (e.g., DMA transfers, frame buffer blits)
3. **VGA graphics mode** rendering is implemented (pixel-level parallelism over 640x480+)
