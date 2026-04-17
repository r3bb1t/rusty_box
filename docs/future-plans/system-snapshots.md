# System Snapshots — Full State Save/Restore

## Problem

`cpu_snapshot()` / `restore_cpu_snapshot()` only covers registers. For deterministic replay and fuzzing, we need to save and restore the entire system: CPU, RAM, device state, timers. Without this, every emulation run from a checkpoint requires replaying from boot.

## Prior Art: Nyx Fuzzer

Nyx (USENIX Security '21) achieves thousands of snapshot restores per second for KVM VMs through:

1. **Dirty page tracking** — instead of copying all guest RAM, track which pages were modified since the snapshot. On restore, copy back only dirty pages. Nyx maintains a stack of dirty page addresses (not a bitmap scan) for O(dirty) restore instead of O(total).

2. **Device state serialization** — all device state (PIC, PIT, LAPIC, disk controllers) serialized to a flat buffer. Restore = memcpy.

3. **Incremental snapshots** (Nyx-Net, EuroSys '22) — take snapshot at point A, run to B, snapshot again. Restore to B without replaying A->B. Dirty tracking is relative to the most recent snapshot.

Papers:
- Schumilo et al., "Nyx: Greybox Hypervisor Fuzzing using Fast Snapshots and Affine Types", USENIX Security 2021
- Schumilo et al., "Nyx-Net: Network Fuzzing with Incremental Snapshots", EuroSys 2022

## Adaptation to rusty_box

Nyx relies on KVM hardware dirty bits. rusty_box is a software emulator that owns guest RAM directly, which is simpler in some ways:

### Phase 1: Naive Full Copy (correctness first)

Copy the entire guest RAM + CPU + device state. For 128MB guest, that's ~128MB per snapshot (~10ms on modern hardware). Good enough for analysis workloads, single-stepping, and low-frequency checkpointing.

**Components to snapshot:**

| Component | Size | Complexity |
|-----------|------|------------|
| CPU registers | ~200 bytes | Done (`CpuSnapshot`) |
| Guest RAM | guest_memory_size (typ. 128MB-4GB) | `memcpy` the `Vec<u8>` |
| PIC (8259A x2) | ~50 bytes | Serialize struct fields |
| PIT (8254) | ~100 bytes | 3 channel counters + state |
| CMOS/RTC | 128 bytes | Register array |
| Keyboard controller (8042) | ~200 bytes | Internal buffers + state |
| Serial ports (16550 x4) | ~400 bytes | FIFO buffers + registers |
| DMA (8237 x2) | ~200 bytes | Channel registers |
| LAPIC | ~1KB | Timer + register file |
| Hard drive controller | ~2KB | ATA register state + DMA buffers |
| PC system (timers) | ~500 bytes | Timer list, IPS, A20 state |
| TLB caches | ~64KB | DTLB + ITLB entries |
| Instruction cache | ~1MB | iCache entries |

Total non-RAM: ~1.1MB. Negligible compared to RAM.

**What is NOT snapshotted:**
- Instrumentation tracer `T` — user-owned, user manages
- GUI state — display-only, not emulation state
- Host file handles for disk images — reopened on restore or kept open

### Phase 2: Dirty Page Tracking (performance)

For fuzzing at >100 restores/sec with large RAM:

1. Add a `dirty_bitmap: BitVec` to `BxMemC` (1 bit per 4KB page = 32KB for 1GB guest).
2. Every `mem_write` in the emulation core sets the corresponding bit. This is one OR instruction per store — negligible overhead.
3. `save_snapshot()` copies only clean pages on first save (full copy), then records the bitmap.
4. `restore_snapshot()` iterates dirty bits, copies back only modified pages.
5. Reset the bitmap after restore.

Cost: O(dirty pages) per restore instead of O(total pages). If 1000 pages dirtied out of 256K pages (1GB), restore copies 4MB instead of 1GB — 250x faster.

### Phase 3: Incremental Snapshots

Stack of snapshots with per-level dirty tracking. Restore to any level. Useful for:
- Fuzzing stateful protocols (snapshot after handshake, fuzz payload)
- Bisecting execution (binary search for divergence point)
- Checkpoint/restart for long-running emulation

## Proposed API

```rust
/// Opaque handle to a saved system state.
/// Contains CPU registers, guest RAM (or dirty pages), device state, timer state.
pub struct SystemSnapshot { /* private */ }

impl<'a, I: BxCpuIdTrait, T: Instrumentation> Emulator<'a, I, T> {
    /// Save full system state. O(RAM size) for first snapshot,
    /// O(dirty pages) for subsequent snapshots with dirty tracking enabled.
    /// The instrumentation tracer `T` is NOT included — user manages that.
    pub fn save_system_snapshot(&self) -> SystemSnapshot;

    /// Restore full system state: CPU, memory, devices, timers.
    /// The instrumentation tracer `T` is NOT restored.
    pub fn restore_system_snapshot(&mut self, snap: &SystemSnapshot);

    /// Enable dirty page tracking for faster subsequent restores.
    /// Small per-write overhead (~1 OR instruction per memory store).
    pub fn enable_dirty_tracking(&mut self);

    /// Disable dirty page tracking.
    pub fn disable_dirty_tracking(&mut self);
}
```

## Implementation Order

1. Add `save()`/`restore()` to each device struct (PIC, PIT, CMOS, keyboard, serial, DMA, LAPIC, hard drive, PC system)
2. Implement `SystemSnapshot` as a struct holding `CpuSnapshot` + `Vec<u8>` (RAM copy) + device state blobs
3. Implement `save_system_snapshot` / `restore_system_snapshot` with full RAM copy
4. Add dirty bitmap to `BxMemC`, wire into memory write path
5. Optimize `restore_system_snapshot` to use dirty bitmap when available
6. Add incremental snapshot stack

## Design Decisions to Make

- Should `SystemSnapshot` be `Clone`? (expensive for large RAM — maybe `Arc` the RAM portion)
- Should snapshots be serializable to disk? (adds serde dependency or custom format)
- Should the TLB/iCache be invalidated on restore instead of saved? (simpler, slight perf hit on resume)
- Should `T` (tracer) be optionally included via a `T: Clone` bound?
