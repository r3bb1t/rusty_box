# Memory Mapping Returns Zeros After FAR JMP

## Date: 2026-02-11

## Status: UNDER INVESTIGATION

## Problem

After the FAR JMP instruction executes successfully and jumps to F000:E05B (linear address 0xFE05B), all memory reads return zeros instead of BIOS code.

## Evidence

### ✅ FAR JMP Works
```
🚀 JmpfAp HANDLER: ilen=5, Id=0xe05b, Iw2=0xf000
🚀 FAR JMP 16-BIT to f000:e05b
⚠️ CS.BASE CHANGED: 0xffff0000 → 0x000f0000 at RIP=0xe05b
```

### ❌ Memory Reads Return Zeros
```
📍 Memory at 0x4b2: [00, 00, 00, 00, 00, 00, 00, 00, ...]
📍 Memory at 0x506: [00, 00, 00, 00, 00, 00, 00, 00, ...]
📍 Memory at 0x508: [00, 00, 00, 00, 00, 00, 00, 00, ...]
```

These addresses (0x4b2, 0x506, 0x508, etc.) are being read but contain all zeros.

## BIOS Loading Configuration

### Current Setup
- **BIOS ROM**: BIOS-bochs-legacy (64 KB)
- **Load Address**: 0xFFFF0000 (calculated as ~(size-1) = ~0xFFFF)
- **Offset in ROM array**: 0xFFFF0000 & 0x3FFFFF = 0xFFF0000 (with 4MB ROM)
- **bios_rom_addr**: Set to 0xFFFF0000

### Loading Verified
```
✓ BIOS loaded: 65536 bytes
```

But detailed verification (first 16 bytes, reset vector) is not printing, suggesting the logging might be failing or data isn't where expected.

## Memory Mapping Logic

### Original Bochs (cpp_orig/bochs/memory/misc_mem.cc:520-594)

For address 0xFE05B with bios_rom_addr = 0xFFFF0000:

```c
bool is_bios = (a20addr >= (bx_phy_address)BX_MEM_THIS bios_rom_addr);
// is_bios = (0xFE05B >= 0xFFFF0000) = false!

// Falls through to line 576:
else if ((a20addr & 0xfffe0000) == 0x000e0000) {
    // last 128K of BIOS ROM mapped to 0xE0000-0xFFFFF
    *buf = BX_MEM_THIS rom[BIOS_MAP_LAST128K(a20addr)];
}
```

**Key insight**: Address 0xFE05B is NOT >= 0xFFFF0000, so `is_bios = false`!

It should be handled by the `0xE0000-0xFFFFF` range check at line 576:
- `0xFE05B & 0xFFFE0000 = 0xE0000` ✓
- Uses `BIOS_MAP_LAST128K(0xFE05B)` = `((0xFE05B | 0xFFF00000) & 0x3FFFFF)` = `0xFFE05B & 0x3FFFFF` = `0x3FE05B`

### Our Implementation (rusty_box/src/memory/misc_mem.rs:74)

```rust
let is_bios = (a20_addr >= 0xE0000 && a20_addr < 0x100000)
           || a20_addr >= self.bios_rom_addr.into();
```

For 0xFE05B:
- `0xFE05B >= 0xE0000 && 0xFE05B < 0x100000` = `true && true` = **true** ✓
- Should use BIOS mapping

Then at line 141-167:
```rust
else if (a20_addr & 0xfffe0000) == 0x000e0000 {
    let mapped = bios_map_last128k(a20_addr.try_into()?);
    // Returns &mut rom[mapped..]
}
```

## Investigation Needed

### Hypothesis 1: ROM Array Offset Calculation Wrong

**BIOS loaded at offset:**
```rust
let offset = (rom_address as usize) & (BIOSROMSZ - 1);
// offset = 0xFFFF0000 & 0x3FFFFF = 0x3FF0000
```

Wait, that's wrong! With BIOSROMSZ = 4MB (0x400000), BIOS_MASK = 0x3FFFFF:
- 0xFFFF0000 & 0x3FFFFF = 0x3F0000 (not 0xFFF0000!)

**BIOS_MAP_LAST128K calculation:**
```rust
fn bios_map_last128k(addr: usize) -> usize {
    ((addr) | 0xfff00000) & BIOS_MASK
}
// For 0xFE05B:
// (0xFE05B | 0xFFF00000) & 0x3FFFFF
// = 0xFFFFE05B & 0x3FFFFF
// = 0x3FE05B
```

**The Issue:**
- BIOS loaded at rom[0x3F0000..0x3F0000+0x10000] = rom[0x3F0000..0x400000]
- Reading 0xFE05B maps to rom[0x3FE05B]
- But 0x3FE05B > 0x400000 (out of bounds!)
- Returns zeros or default value

### Root Cause

The BIOS ROM array is only 4MB (0x400000 bytes), but we're trying to load BIOS at offset 0x3F0000 (4,128,768 bytes). When we add the 64KB BIOS size:
- End offset = 0x3F0000 + 0x10000 = 0x400000

This exactly fills the ROM array. But when we try to read via BIOS_MAP_LAST128K:
- Mapped offset = 0x3FE05B (for linear 0xFE05B)
- This is > 0x400000 = out of bounds!

### The Fix

The BIOS should be loaded at a lower offset in the ROM array. Looking at Bochs again:

```c
offset = romaddress & BIOS_MASK;
// For 0xFFFF0000: offset = 0xFFFF0000 & 0x3FFFFF = 0x3F0000
```

This is correct. But then BIOS_MAP_LAST128K should map to the same range.

**Wait, let me recalculate BIOS_MAP_LAST128K:**
```
#define BIOS_MAP_LAST128K(addr) (((addr) | 0xfff00000) & BIOS_MASK)
```

For addr = 0xFE05B:
- addr | 0xfff00000 = 0xFE05B | 0xFFF00000 = 0xFFFFE05B
- 0xFFFFE05B & 0x3FFFFF = 0x3FE05B

For BIOS at offset 0x3F0000 (64KB), valid range is 0x3F0000-0x3FFFFF.
But 0x3FE05B is within this range! (0x3F0000 <= 0x3FE05B <= 0x3FFFFF)

So the mapping should work...

### Hypothesis 2: ROM Array Too Small?

Check if ROM array is actually allocated to 4MB:
- `rusty_box/src/memory/memory_stub.rs`: `BIOSROMSZ = 1 << 22 = 4194304 bytes = 4MB`

If the vector is allocated correctly, rom[0x3FE05B] should be valid.

### Hypothesis 3: bios_map_last128k Returns Wrong Value

Check our implementation vs C++ macro:
```rust
pub(super) fn bios_map_last128k(addr: usize) -> usize {
    ((addr) | 0xfff00000) & BIOS_MASK
}
```

Looks correct. But BIOS_MASK is a static:
```rust
pub(super) static BIOS_MASK: usize = BIOSROMSZ - 1;
```

Need to verify BIOSROMSZ is actually 4MB at runtime.

### Hypothesis 4: Wrong Memory Path Taken

The code has multiple paths for reading BIOS memory. Need to add logging to see which path is taken for address 0xFE05B.

## Next Steps

1. **Add detailed logging** to memory read path:
   - Log which branch is taken (is_bios check, 0xE0000 range, etc.)
   - Log offset calculation: `bios_map_last128k(addr)` result
   - Log ROM array bounds and actual read offset
   - Log the actual bytes read from ROM array

2. **Verify ROM array allocation**:
   - Check that rom.len() == 4MB
   - Check that BIOS data is actually at rom[0x3F0000..0x400000]

3. **Verify BIOS loading**:
   - Re-enable detailed verification logging in load_ROM
   - Check that first 16 bytes are not zeros
   - Check that reset vector bytes are correct

4. **Compare with Bochs step-by-step**:
   - Trace through Bochs C++ code for address 0xFE05B
   - Trace through our Rust code for same address
   - Find where they diverge

## Related Documentation

- **FAR_JUMP_DECODER_BUG.md**: FAR JMP fix (completed)
- **BIOS_LOADING_COMPARISON.md**: BIOS loading sequence comparison
- **VGA_BIOS_LOADING_SEQUENCE_BUG.md**: BIOS loading timing fix

## Priority

**HIGH** - This blocks all BIOS execution after the initial FAR JMP. Without this, we can't test any subsequent BIOS code.
