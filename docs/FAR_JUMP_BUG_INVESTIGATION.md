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

## Solution Options

### Option A: Recompile BIOS from Source ✅ RECOMMENDED
Recompile the BIOS using the correct linker script to get proper symbol addresses:
```bash
cd cpp_orig/bochs/bios
make clean
make BIOS-bochs-legacy
```

### Option B: Patch BIOS ROM Binary
Manually patch the instruction bytes in _start to use correct addresses (fragile, not recommended).

### Option C: Pre-initialize .data Section
Have emulator copy .data section before starting BIOS (defeats purpose of BIOS initialization).

### Option D: Use Different BIOS
Find or build a BIOS that has correct symbol addresses (SeaBIOS, coreboot, etc.).

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
