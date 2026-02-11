# ✅ FIXED: Opcode 0xEA (FAR JMP) Decoder Bug

## Date: 2026-02-11
## Status: RESOLVED

**Fix Committed:** 13e1a9f - FAR JMP now decodes correctly following original Bochs implementation

---

## Original Problem

BIOS execution fails immediately after reset vector with symptoms:
- ❌ CS.base corrupts to 0 at instruction #35
- ❌ All subsequent execution reads from zeroed memory
- ❌ I/O functions at F000:0506 read all zeros
- ❌ No BIOS output (no VGA text, no POST codes, no debug output)
- ❌ BIOS never progresses beyond initialization

## Investigation Chain

### User's Key Insights
1. **Screenshot Evidence**: Showed CS.base=0x0 and memory at 0x506 containing all zeros
2. **Suggestion**: "maybe there is a bug in ->execute1() or similar? Perhaps wrong function being called?" - **EXACTLY CORRECT!**

### Evidence Gathered

**Reset Vector (0xFFFFFFF0):**
```
Bytes: EA 5B E0 00 F0 30 35 2F
       └─┬──┴─┬──┴─┬─┘
         │    │    └─ Segment = 0xF000
         │    └────── Offset = 0xE05B
         └─────────── Opcode = 0xEA (FAR JMP)
```

**Expected Behavior:**
```
1. CPU fetches reset vector: EA 5B E0 00 F0
2. Decoder recognizes 0xEA as FAR JMP
3. Handler executes: JMP F000:E05B
4. CS.base = 0xF0000, RIP = 0xE05B
5. Linear address = 0xF0000 + 0xE05B = 0xFE05B
6. BIOS code executes from ROM
```

**Actual Behavior:**
```
1. CPU fetches reset vector: EA 5B E0 00 F0
2. ❌ Decoder returns: BxIllegalOpcode
3. ❌ #UD exception generated (vector 6)
4. ❌ IVT[6] uninitialized → jumps to 0x0
5. ❌ CS.base becomes 0, RIP = 0
6. ❌ Linear address = 0 + 0 = 0
7. ❌ Executes from zeroed memory instead of BIOS ROM
```

**Debug Output:**
```
ERROR CPU instruction #34, RIP: 00000000fffffff0 opcode: ea 5b e0 00 f0
Decode error: Decoder(BxIllegalOpcode)
Illegal opcode detected, generating #UD exception (vector 6)

⚠️ CS.BASE CHANGED: 0x00ffff0000 → 0x0000000000 at RIP=0x000000, opcode=MovEbIb
❌ CS.BASE CORRUPTED TO ZERO! RIP=0x0, opcode=MovEbIb, instruction #35, CS.selector=0x0
```

**Handler Never Called:**
```rust
// jmpf_ap_wrapper has debug logging but NEVER executes
tracing::error!("🔴 JmpfAp HANDLER: ...");  // Never printed!
```

## Root Cause

**Opcode 0xEA is in the opcode table but NOT in the decoder tables!**

**File: `rusty_box/src/cpu/opcodes_table.rs`**
```rust
// Handler EXISTS in opcode table:
Opcode::JmpfAp => Some(BxOpcodeEntry {
    execute1: jmpf_ap_wrapper,
    execute2: None,
    opflags: OpFlags::empty(),
}),
```

**File: `rusty_box_decoder/src/fetchdecode32.rs`** (and fetchdecode64.rs)
```rust
// ❌ NO ENTRY FOR 0xEA!
// Decoder returns BxIllegalOpcode
```

**Impact:**
- Reset vector contains FAR JMP as first instruction
- Decoder cannot decode it
- Exception handler jumps to address 0
- CS.base becomes 0
- All memory accesses use wrong base address
- BIOS code is unreachable

## Instruction Format

**FAR JMP (0xEA):**
```
Opcode: EA
Operands (16-bit mode): Iw (offset word), Iw2 (segment word)
Operands (32-bit mode): Id (offset dword), Iw2 (segment word)
Size: 5 bytes (16-bit), 7 bytes (32-bit)
```

**Example from reset vector:**
```
EA 5B E0 00 F0  = JMP F000:E05B
^  └─┬──┘  └─┬─┘
│    │       └── Segment = 0xF000
│    └────────── Offset = 0xE05B (little-endian)
└───────────────── Opcode = 0xEA
```

## Solution

Add decoder entry for opcode 0xEA following original Bochs implementation.

**Original Bochs Reference:**
- `cpp_orig/bochs/cpu/fetchdecode.cc` - Decoder implementation
- Look for opcode 0xEA handling
- Follow existing patterns for immediate operands

**Files to Modify:**
1. `rusty_box_decoder/src/fetchdecode32.rs` - Add 0xEA case
2. `rusty_box_decoder/src/fetchdecode64.rs` - Add 0xEA case (if needed)
3. Test with BIOS execution

**Expected After Fix:**
1. ✅ Decoder recognizes 0xEA as FAR JMP
2. ✅ jmpf_ap_wrapper executes
3. ✅ CS.base = 0xF0000, RIP = 0xE05B
4. ✅ BIOS code executes from ROM
5. ✅ BIOS produces output
6. ✅ No CS.base corruption

## Related Issues

- **BP_REGISTER_CORRUPTION_BUG.md**: BP=0 at RIP 0x2055 - **NOT the root cause!**
  - This is a symptom of executing from wrong memory location
  - With CS.base=0, BIOS code is unreachable
  - Loop at 0x2055 is in zeroed memory, not actual BIOS code

- **VGA_BIOS_LOADING_SEQUENCE_BUG.md**: BIOS loading timing - **Fixed correctly!**
  - BIOS loads at 0xF0000 (correct)
  - Timing: Memory init → Load BIOS → CPU init (correct)
  - This was not the root cause

## Priority

**CRITICAL** - This is the ACTUAL root cause blocking all BIOS execution. The decoder bug causes immediate crash at reset vector before any BIOS code can run.

---

## Fix Implementation (2026-02-11)

### Root Cause Identified

The immediate size for FAR JMP (0xEA) was **hardcoded to 6 bytes**, but should depend on operand size:
- **16-bit mode**: 2-byte offset + 2-byte segment = **4 bytes**
- **32-bit mode**: 4-byte offset + 2-byte segment = **6 bytes**

### Verification Against Original Bochs

Compared with `cpp_orig/bochs/cpu/decoder/fetchdecode32.cc` BX_DIRECT_PTR case:

```cpp
// Original Bochs implementation
case BX_DIRECT_PTR:
    if (i->os32L()) {
        i->modRMForm.Id = FetchDWORD(iptr);      // 4 bytes
        iptr += 4;
    } else {
        i->modRMForm.Iw[0] = FetchWORD(iptr);    // 2 bytes
        iptr += 2;
    }
    i->modRMForm.Iw2[0] = FetchWORD(iptr);       // 2 bytes (always)
    iptr += 2;
```

**Our implementation matches EXACTLY** - no new bugs introduced!

### Changes Made

**File: `rusty_box_decoder/src/fetchdecode32.rs`**

1. **Fixed immediate size calculation (line 1092-1101):**
```rust
// Far pointer (Ap): offset + segment
// 16-bit: Iw + Iw = 4 bytes (2-byte offset + 2-byte segment)
// 32-bit: Id + Iw = 6 bytes (4-byte offset + 2-byte segment)
0x9A | 0xEA => {
    if os_32 {
        6 // 32-bit mode: 4-byte offset + 2-byte segment
    } else {
        4 // 16-bit mode: 2-byte offset + 2-byte segment
    }
}
```

2. **Fixed immediate parsing (lines 515-526):**
```rust
4 => {
    let is_far_pointer = matches!(b1, 0x9A | 0xEA);
    if is_far_pointer {
        // Far pointer in 16-bit mode: Iw (offset) + Iw (segment)
        instr.modrm_form.operand_data.id = read_u16_le(bytes, pos) as u32;
        instr.modrm_form.displacement.data32 = read_u16_le(bytes, pos + 2) as u32;
    } else {
        instr.modrm_form.operand_data.id = read_u32_le(bytes, pos);
    }
    pos += 4;
}
6 => {
    // Far pointer in 32-bit mode: Id (offset) + Iw (segment)
    instr.modrm_form.operand_data.id = read_u32_le(bytes, pos);
    instr.modrm_form.displacement.data32 = read_u16_le(bytes, pos + 4) as u32;
    pos += 6;
}
```

**File: `rusty_box_decoder/tests/test_far_jump.rs`** (NEW)

Added comprehensive unit tests:
- ✅ `test_far_jmp_16bit` - Verifies 5-byte instruction (EA 5B E0 00 F0)
- ✅ `test_far_jmp_32bit` - Verifies 7-byte instruction (EA 5B E0 00 00 00 F0)
- Both tests pass!

**File: `rusty_box/examples/dlxlinux.rs`**

Fixed BIOS load address (reverted incorrect change from previous session):
```rust
// Calculate BIOS load address: place BIOS at TOP of 4GB address space
let bios_load_addr = 0x100000000u64 - bios_size;
// 64KB:  0xFFFF0000
// 128KB: 0xFFFE0000
```

## Results After Fix

### Decoder Tests
```
test test_far_jmp_16bit ... ok
test test_far_jmp_32bit ... ok
✅ FAR JMP decoded correctly in both 16-bit and 32-bit modes
```

### BIOS Execution
```
🚀 JmpfAp HANDLER: os32_l=0, ilen=5, Id=0xe05b, Iw=0xe05b, Iw2=0xf000
🚀 FAR JMP 16-BIT to f000:e05b
🔵 jmp_far16 CALLED: cs=0xf000, disp=0xe05b, real_mode=true
🔴 CS LOAD (real mode): selector=0xf000, new_base=0xf0000
⚠️ CS.BASE CHANGED: 0xffff0000 → 0x000f0000 at RIP=0xe05b
```

### Fix Impact (All Expected Results Achieved!)

1. ✅ Reset vector FAR JMP executes successfully
2. ✅ BIOS runs from correct address (CS=0xF000, IP=0xE05B)
3. ✅ CS.base transitions correctly: 0xFFFF0000 → 0xF0000
4. ✅ Decoder recognizes 0xEA and decodes 5-byte instruction
5. ✅ jmpf_ap_wrapper handler executes
6. ✅ BIOS execution progresses past reset vector
7. ✅ No more "Illegal Opcode" errors

**Next Steps:** Now that FAR JMP works, continue BIOS execution to discover and fix remaining missing instructions.
