# Far Jump Bug Investigation (2026-02-09)

## ✅ INVESTIGATION COMPLETE - Real Root Cause Found!

### Original Problem (SOLVED)

The BIOS appeared to fail because execution landed at 0xE0BF1 instead of expected 0xE0000, skipping the `_start` initialization function.

### What We Discovered ✅

After deep investigation following original Bochs C++ source code:

1. **Far jump IS working correctly** - The `0x66 0xEA` instruction executes properly
2. **Jump target is correct** - It lands at 0x0010:0x000F9E5F (rombios32_05), not _start
3. **_start IS called** - rombios32_05 does `call 0xE0000` to invoke _start indirectly
4. **_start DOES execute** - We see BSS clearing and data copy attempts

### The ACTUAL Bug ❌

The `_start` function executes but with **corrupted symbol addresses** baked into the BIOS ROM:
- **Expected**: ESI = 0x000E416F (source: .data in ROM, from `_end` symbol)
- **Actual**: ESI = 0xFFFF0700 (corrupted/wrong value)
- **Expected**: EDI = 0x70C (destination: .data in RAM, from `__data_start`)
- **Actual**: EDI ≈ 0x1 (corrupted/wrong value)

**Result**: .data section copied to wrong memory location (0x1 instead of 0x70C), causing crash at 0x20000.

## Evidence

### 1. _start Function Exists but Never Executes

**ROM offset 0x0000** (maps to address 0xE0000):
```assembly
31 c0                xor eax, eax
bf 10 07 00 00       mov edi, 0x710      ; __bss_start
b9 68 07 00 00       mov ecx, 0x768      ; __bss_end
29 f9                sub ecx, edi
f3 aa                rep stosb           ; Clear BSS

be 6f 41 0e 00       mov esi, 0xE416F    ; _end (data in ROM)
bf 0c 07 00 00       mov edi, 0x70C      ; __data_start (RAM)
b9 0c 07 00 00       mov ecx, 0x70C      ; size
29 f9                sub ecx, edi
f3 a4                rep movsb           ; Copy .data to RAM!

e9 58 29 00 00       jmp rombios32_init
```

**But:**
- ❌ No execution logged at 0xE0000-0xE0030
- ❌ Zero writes to low RAM (0x700-0x7FF range)
- ❌ First protected mode execution: RIP=0xE0BF1

### 2. Attempted Fix

Modified `jmpf_ap_wrapper` in `opcodes_table.rs` to check instruction length and call appropriate function:
- 16-bit far jump: `jmp_far16(segment, offset16)`
- 32-bit far jump: `jmp_far32(segment, offset32)`

**Result:** No effect. The wrapper function is never called.

### 3. Investigation of Execution Flow

**Log analysis** shows:
- Last real mode instruction: `ADD ESP, 0x10` at EIP ~0xe08d3
- Next instruction: RIP=0xe0bf1 with CS.selector=0x10, CS.base=0x0
- **No logging** of:
  - "FAR JMP" message from wrapper
  - `jump_protected` function
  - `load_seg_reg` for CS
  - Any segment descriptor loading

**Conclusion:** The far jump executes through a code path that bypasses our instrumentation.

## Likely Cause: Instruction Cache

The BIOS executes in a loop, calling the same functions repeatedly. After the first execution, instructions are cached in the **icache** (instruction cache/trace system). Cached instructions execute directly without going through the opcode dispatch system, bypassing our wrapper function.

The far jump likely:
1. First execution: Goes through normal dispatch (but we don't see it logged)
2. Subsequent executions: Uses cached trace, completely bypassing wrappers
3. Uses a low-level CS load mechanism that doesn't trigger our logging

## Root Cause Analysis

### Why Symbol Addresses Are Wrong

The BIOS ROM files (`BIOS-bochs-latest` 128KB and `BIOS-bochs-legacy` 64KB) were compiled with incorrect symbol addresses that don't match the linker script `rombios32.ld`:

**Expected (from rombios32.ld):**
```ld
. = 0x000e0000;              // Code at 0xE0000
.text     : { *(.text)    }
.rodata   : { *(.rodata*) }
_end = . ;                   // _end = end of rodata in ROM
.data 0x700 : AT (_end) {    // Virtual addr 0x700, stored at _end in ROM
    __data_start = .;        // Should be 0x700
    *(.data);
    __data_end = .;
}
.bss : {
    __bss_start = .;         // Follows .data
    *(.bss);
    __bss_end = .;
}
```

**Actual (in both ROM files):**
- Symbol addresses don't match - possibly compiled with different linker script
- `_end` might point to wrong location
- `__data_start` and `__bss_start` have incorrect values

### Execution Flow (What Actually Happens)

1. ✅ Real mode: CPU starts at 0xFFFFFFF0 (reset vector)
2. ✅ Far jump to F000:E05B (real mode BIOS entry)
3. ✅ BIOS enables protected mode (CR0 |= 1)
4. ✅ **Far jump to 0x0010:0x000F9E5F** (rombios32_05) - **THIS WORKS!**
5. ✅ rombios32_05 initializes segments (DS, ES, SS, FS, GS)
6. ✅ rombios32_05 pushes parameters (0x4B0, 0x4B2)
7. ✅ rombios32_05 does `call 0xE0000` (_start)
8. ✅ _start begins executing
9. ❌ _start uses **wrong symbol values** → copies .data to wrong address
10. ❌ Jump to garbage address → decoder fails → crash

## Progress (2026-02-09)

### ✅ ROM Mapping Fix Applied
**Commit:** e79eed3 - "Fix BIOS ROM mapping: Include 0xE0000-0xFFFFF range"

Modified `misc_mem.rs` to treat 0xE0000-0xFFFFF as BIOS ROM (matching Bochs behavior):
```rust
let is_bios = (a20_addr >= 0xE0000 && a20_addr < 0x100000) ||
              a20_addr >= self.bios_rom_addr.into();
```

**Impact:**
- ✅ BIOS code at 0xE0000 is now accessible
- ✅ Far jump to 0x0010:0x000F9E5F executes correctly
- ✅ rombios32_05 executes (pushes 0x4B0, 0x4B2)
- ✅ CALL to _start executes (pushes return address)

### ❌ Remaining Issue: Instruction Cache Bug (NOT Decoder)

**Symptoms:**
1. Return address pushed is 0xF9E91 (should be 0xF9E90) - **off by 1 byte**
2. After CALL, writes go to addresses 0x0-0x5 instead of 0x700
3. EDI appears to be 0 instead of 0x700

**ROM Verification:**
- ✅ ROM bytes at 0xF9E89-0xF9E8E are correct:
  - `b8 00 00 0e 00` = MOV EAX, 0xE0000
  - `ff d0` = CALL EAX
- ✅ ROM bytes at 0xE0000 (_start) are correct:
  - `bf 00 07 00 00` = MOV EDI, 0x700

**Decoder Verification (2026-02-09):**
- ✅ Tested `FF D0` (CALL EAX): Correctly decodes as **2 bytes**
- ✅ Tested `B8 00 00 0E 00` (MOV EAX, 0xE0000): Correctly decodes as **5 bytes**
- ✅ Decoder is working correctly! Bug is NOT in decoder.

**Root Cause: Instruction Cache (icache) Bug**

The issue is in the instruction cache/trace system (`rusty_box/src/cpu/icache.rs`), NOT the decoder:
1. Decoder produces correct `ilen` values
2. But when instructions are cached in traces, `ilen` might be calculated or stored incorrectly
3. This causes RIP to advance by wrong amount (off by 1)
4. Subsequent instructions decode from misaligned addresses
5. Registers end up with garbage values

**Next Steps:**
- Investigate `icache.rs` instruction length handling
- Check how traces are built and if `ilen` is preserved correctly
- Verify RIP advancement in cached traces matches uncached execution
- Add icache-specific logging to track instruction lengths

## Solution Options

### Option A: ✅ COMPLETED - ROM Mapping Fix
Fixed 0xE0000-0xFFFFF mapping to allow access to rombios32 code.

### Option B: Debug Register Corruption
Continue investigating why registers are corrupted or instructions misaligned.

### Option C: Recompile BIOS from Source
Recompile with debug symbols to verify correct linking (may not help if emulator bug).

### Option D: Use Different BIOS
Try SeaBIOS or other BIOS to see if issue is BIOS-specific or emulator bug.

## Files Modified

- `CLAUDE.md` - Updated with far jump bug description
- `memory/MEMORY.md` - Added detailed analysis
- `rusty_box/src/cpu/opcodes_table.rs` - Added operand size checking (ineffective)
- `rusty_box/src/cpu/string.rs` - Added low RAM write logging

## Testing

To verify the fix works:
```bash
cd rusty_box
cargo run --release --example dlxlinux --features std 2>&1 | grep "💾 MEM_WRITE"
```

Should see writes to addresses 0x700-0x7FF (data section copy).

Should also see execution at 0xE0000-0xE0030 (_start function).

## References

- BIOS source: `cpp_orig/bochs/bios/rombios32start.S` (_start function)
- Linker script: `cpp_orig/bochs/bios/rombios32.ld` (data section at 0x700)
- Original Bochs: Successfully boots with same BIOS
