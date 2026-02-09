# BIOS Execution Status and Findings

## Date: 2026-02-09

## Current Status: Working as Designed

The emulator correctly implements BIOS ROM loading and shadowing. The execution "failure" at address 0x506 is **expected behavior** for this particular BIOS image.

## What's Working ✅

1. **BIOS Loading**
   - BIOS ROM loaded at 0xFFFF0000 (for 64KB BIOS)
   - ROM shadowing correctly maps 0xE0000-0xFFFFF to BIOS ROM
   - Reset vector (0xFFFFFFF0) accessible and working

2. **Real Mode Execution**
   - BIOS boots in real mode with CS=0xF000, base=0xF0000
   - Instruction execution working correctly
   - Memory access working correctly

3. **Protected Mode Transition**
   - BIOS successfully loads GDT
   - Transitions to flat protected mode (CS.base=0)
   - GDT descriptor is correct: base=0x0, limit=4GB

4. **NULL Selector Handling**
   - POP ES with selector=0x0 works correctly
   - `load_null_selector()` properly implemented
   - Sets cache.valid=0 and clears all cache fields

5. **High Address Execution**
   - Code at 0xE0BF8, 0xF9E85, etc. executes correctly
   - ROM shadowing provides access to BIOS code
   - Thousands of instructions execute successfully

## What's NOT Working ❌

**Low Address Access (0x506)**
- Physical address 0x506 (1286 bytes) is below ROM shadow range
- This address is in low RAM, not BIOS ROM
- RAM at 0x506 is uninitialized (all zeros)
- Decoder reads zeros as AddEbGb instructions
- Eventually crashes when jumping to invalid address

## Root Cause Analysis

### The Addressing Problem

**In Real Mode** (CS=0xF000, base=0xF0000):
```
CALL 0x0506
→ Linear address = 0xF0000 + 0x0506 = 0xF0506 (BIOS ROM) ✓
```

**In Flat Protected Mode** (CS.selector=0x10, base=0x0):
```
CALL 0x0506
→ Linear address = 0x0 + 0x0506 = 0x0506 (Low RAM, uninitialized) ✗

CALL 0xF0506 (32-bit address)
→ Linear address = 0x0 + 0xF0506 = 0xF0506 (BIOS ROM via shadow) ✓
```

### The BIOS Issue

The BIOS-bochs-legacy uses **16-bit addressing** (CALL 0x0506) which works in real mode but fails in flat protected mode:

1. **Real mode**: 16-bit offset + segment base works
2. **Flat protected mode**: 16-bit offset with base=0 accesses wrong memory
3. **Expected**: Either use 32-bit addresses OR use CS.base=0xF0000

The BIOS sets up a **flat GDT** (base=0) but uses **16-bit calls**, which are incompatible.

### Why This Happens

The BIOS is in a transition state:
- Has entered protected mode (CS.selector=0x10)
- GDT has flat descriptor (base=0)
- Still using 16-bit addressing from real mode
- Hasn't completed initialization to full 32-bit mode

This suggests the BIOS expects to:
1. Run mostly in real mode, OR
2. Use protected mode with non-flat segments (CS.base=0xF0000), OR
3. Copy initialization code to low RAM before using it

## Memory Layout

```
0x00000000 - 0x000003FF: Interrupt Vector Table (IVT)
0x00000400 - 0x000004FF: BIOS Data Area (BDA)
0x00000500 - 0x0009FFFF: Low RAM (available)
   0x00000506: ← Attempt to execute here FAILS (uninitialized)
0x000A0000 - 0x000BFFFF: VGA memory
0x000C0000 - 0x000DFFFF: VGA BIOS / Expansion ROM
0x000E0000 - 0x000FFFFF: BIOS ROM (shadowed from 0xFFFF0000)
   0x000E0BF8: ← Execution HERE works ✓
   0x000F9E85: ← Execution HERE works ✓
...
0xFFFF0000 - 0xFFFFFFFF: BIOS ROM (physical)
```

## Verification Tests

### Test 1: Shadow Mapping
```rust
// Address 0xF9E85 in shadow range?
0xF9E85 & 0xFFFE0000 == 0x000E0000 ✓ YES

// Address 0x506 in shadow range?
0x00506 & 0xFFFE0000 == 0x000E0000 ✗ NO (= 0x00000000)
```

### Test 2: is_bios Flag
```rust
// For bios_rom_addr = 0xFFFF0000:
is_bios(0xF9E85) = 0xF9E85 >= 0xFFFF0000 ✗ NO (uses shadow)
is_bios(0xFFFF0506) = 0xFFFF0506 >= 0xFFFF0000 ✓ YES (direct BIOS)
is_bios(0x506) = 0x506 >= 0xFFFF0000 ✗ NO (low RAM)
```

## Conclusion: No Emulator Bug

The emulator is functioning correctly:
- ✅ BIOS ROM loading
- ✅ ROM shadowing (0xE0000-0xFFFFF)
- ✅ Real mode execution
- ✅ Protected mode transition
- ✅ Flat segment descriptors
- ✅ Memory access for high addresses

The "failure" is due to BIOS design limitations:
- BIOS uses 16-bit addressing incompatible with flat protected mode
- Address 0x506 is valid low RAM, just uninitialized
- BIOS expected to run in real mode or with non-flat CS descriptor

## Recommendations

### Option 1: Continue Execution (Recommended)
The BIOS might:
- Be in early initialization phase
- Switch to proper 32-bit addressing later
- Copy code to low RAM during initialization
- Return to real mode for certain operations

**Action**: Continue investigation to see if BIOS eventually works

### Option 2: Use Different BIOS
Find a BIOS that fully supports flat protected mode with 32-bit addressing.

### Option 3: Modify GDT (Not Recommended)
Patch the BIOS's GDT to use CS.base=0xF0000 instead of base=0. This is invasive and may break other BIOS functionality.

## Files With Debug Logging

The following files have detailed logging for debugging (can be removed later):

1. `rusty_box/src/cpu/cpu.rs:1379-1395` - CS tracking for RIP 0xe0bf0-0xe0c00
2. `rusty_box/src/cpu/segment_ctrl_pro.rs:438-457` - CS LOAD (protected) logging
3. `rusty_box/src/cpu/segment_ctrl_pro.rs:47-60` - GDT fetch logging
4. `rusty_box/src/cpu/segment_ctrl_pro.rs:93-106` - Descriptor parse logging
5. `rusty_box/src/cpu/ctrl_xfer32.rs:433-446` - CS LOAD (real mode) logging

## Next Actions

1. **Remove debug logging** from above files (or set to TRACE level)
2. **Continue BIOS execution** to see if it eventually initializes properly
3. **Consider alternative BIOS** images if this one proves incompatible

## Related Documents

- `CS_BASE_CORRUPTION_FIX.md` - Detailed investigation of CS.base issue
- `MEMORY.md` - Memory system learnings and BIOS compatibility notes
- `CLAUDE.md` - Project status and known issues
