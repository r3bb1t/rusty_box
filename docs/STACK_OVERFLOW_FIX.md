# Stack Overflow Fix - 32 MB RAM Support

## Problem

Emulator crashed with `STATUS_STACK_OVERFLOW` when configured with 32 MB RAM, but worked fine with 4 MB. Thread had 1.5 GB stack space, yet still overflowed.

## Root Cause Analysis

### Investigation Process

1. **Created struct size checker** (`examples/check_struct_sizes.rs`)
   - Found: `Emulator<Corei7SkylakeX>` = **15.2 MB**
   - Found: `BxCpuC<Corei7SkylakeX>` = **15.2 MB** (almost entire Emulator size)
   - Found: `BxMemC` = 264 bytes (not the issue)

2. **Identified the culprit**: `BxICache::mpool` array
   - Located in: `rusty_box/src/cpu/icache.rs:149`
   - Size: `[BxInstructionGenerated; BX_ICACHE_MEM_POOL]`
   - Elements: 576 × 1024 = **589,824 instructions**
   - Total size: ~**15 MB** (26 bytes per instruction)

3. **Compared with original Bochs** (`cpp_orig/bochs/cpu/icache.h`)
   - Bochs also has: `bxInstruction_c mpool[BxICacheMemPool]` (576K entries)
   - BUT: Bochs uses `new BX_CPU_C(i)` → **heap allocation**
   - Rust: `Emulator::new()` returns by value → **stack allocation**

### Why Stack Overflow Happened

```
Stack allocation chain:
  Emulator::new() creates Emulator (15 MB) on stack
    └─> Contains BxCpuC (15 MB)
          └─> Contains BxICache
                └─> Contains mpool array (15 MB) ← CULPRIT!
```

Even with 1.5 GB thread stack:
1. **Emulator::new()** tries to create 15 MB Emulator on stack
2. Returns by value → temporary 15 MB struct on stack
3. With deep call chains → stack overflow
4. **Compiler** also crashed during build with `Box::new([...])` approach!

## Solution

### Two-Part Fix

#### Part 1: Vec Instead of Array (icache.rs)

**Before:**
```rust
pub struct BxICache {
    pub(crate) mpool: [BxInstructionGenerated; BX_ICACHE_MEM_POOL],  // 15 MB on stack!
    ...
}

impl BxICache {
    pub fn new() -> Self {
        Self {
            mpool: [BxInstructionGenerated::default(); BX_ICACHE_MEM_POOL],  // Stack allocation
            ...
        }
    }
}
```

**After:**
```rust
pub struct BxICache {
    /// Vec to avoid stack overflow - this is ~15 MB!
    pub(crate) mpool: Vec<BxInstructionGenerated>,  // Heap-allocated
    ...
}

impl BxICache {
    pub fn new() -> Self {
        Self {
            // vec![val; size] is efficient and heap-allocated
            mpool: vec![BxInstructionGenerated::default(); BX_ICACHE_MEM_POOL],
            ...
        }
    }
}
```

**Size reduction**: 15.2 MB → 1.4 MB (Emulator struct)

#### Part 2: Box<Emulator> Return Type (emulator.rs)

**Before:**
```rust
pub fn new(config: EmulatorConfig) -> Result<Self> {
    Ok(Self { ... })  // Returns 1.4 MB struct by value (stack allocation)
}
```

**After:**
```rust
/// Returns Box<Emulator> to avoid stack overflow (Emulator is ~1.4 MB).
/// This matches original Bochs behavior which uses `new BX_CPU_C(i)`.
pub fn new(config: EmulatorConfig) -> Result<Box<Self>> {
    Ok(Box::new(Self { ... }))  // Heap allocation (matches Bochs)
}
```

**Why Box instead of attempted Box<[T; N]>:**
- `Box::new([val; N])` creates array on stack first, then boxes
- Compiler crashed with `STATUS_STACK_BUFFER_OVERRUN` during const eval
- `vec![val; N]` directly allocates on heap → no compiler crash

### Impact

| Configuration | Before | After |
|---------------|--------|-------|
| **4 MB RAM** | ✅ Works | ✅ Works |
| **32 MB RAM** | ❌ Stack overflow | ✅ Works |
| **Emulator size** | 15.2 MB | 1.4 MB |
| **BxCpuC size** | 15.2 MB | 1.4 MB |
| **Allocation** | Stack | **Heap** |

## Files Modified

1. **rusty_box/src/cpu/icache.rs**
   - Changed `mpool: [BxInstructionGenerated; BX_ICACHE_MEM_POOL]` → `Vec<BxInstructionGenerated>`
   - Changed initialization from array to `vec![default; size]`

2. **rusty_box/src/emulator.rs**
   - Changed `pub fn new() -> Result<Self>` → `pub fn new() -> Result<Box<Self>>`
   - Wrapped return in `Box::new(...)`

3. **rusty_box/examples/dlxlinux.rs**
   - Restored 32 MB RAM configuration
   - Updated CMOS and display to show 32 MB

4. **rusty_box/examples/check_struct_sizes.rs** (diagnostic tool)
   - Created to measure struct sizes

## Testing

### Before Fix
```
$ cargo run --release --example dlxlinux
thread 'DLX Linux' (9260) has overflowed its stack
error: process didn't exit successfully (exit code: 0xc00000fd, STATUS_STACK_OVERFLOW)
```

### After Fix
```
$ cargo run --release --example dlxlinux
✓ BIOS loaded: 65536 bytes
✓ VGA BIOS loaded: 32768 bytes
║  Memory = 32 MB                                           ║
Instructions:        50,000,004
Final RIP:           0x97d
Status:              ✅ Success
```

## Lessons Learned

1. **Large arrays must be heap-allocated**
   - Arrays >1 MB should use Vec, not `[T; N]`
   - Even with huge thread stacks, local arrays cause overflow

2. **Match C++ heap allocation patterns**
   - C++ `new Foo()` → Rust `Box::new(Foo { ... })`
   - C++ stack size not a concern → Rust must be explicit about heap vs stack

3. **Beware const evaluation limits**
   - `Box::new([val; N])` can crash the compiler for large N
   - `vec![val; N]` is safer for large allocations

4. **Diagnostic tools are essential**
   - `std::mem::size_of::<T>()` quickly identifies bloat
   - Created reusable `check_struct_sizes` example

## Original Bochs Comparison

| Aspect | Bochs (C++) | Rusty Box (Before) | Rusty Box (After) |
|--------|-------------|-------------------|-------------------|
| Allocation | `new BX_CPU_C(i)` (heap) | `Emulator::new()` (stack) | `Box::new(Emulator)` (heap) |
| mpool array | `bxInstruction_c mpool[...]` | `[...; 589824]` | `Vec<...>` |
| Size | ~15 MB | ~15 MB | ~1.4 MB |
| Stack overflow | No | **Yes** with 32 MB | **No** |

## Future Improvements

1. **Consider further size reduction**
   - Emulator is still 1.4 MB (large for a struct)
   - Could Box other large components (entry array, page_split_index)

2. **Profile memory usage**
   - Verify heap allocation performance is acceptable
   - Monitor allocation patterns during BIOS boot

3. **Document large struct patterns**
   - Add size checks to CI
   - Warn when struct size exceeds threshold (e.g., 100 KB)

## Related Documentation

- `SESSION_2026-02-10_BIOS_FIX.md` - Earlier BIOS execution fixes
- `DECODER_FIX_AUDIT.md` - Decoder operand direction fix
- Original Bochs: `cpp_orig/bochs/main.cc:1345` (CPU allocation)
- Original Bochs: `cpp_orig/bochs/cpu/icache.h:125` (mpool array)
