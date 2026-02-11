# CRITICAL BUG: Opcode 0xEA (FAR JMP) Missing from Decoder

## Date: 2026-02-11

## Problem

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

## Expected Fix Impact

Once decoder recognizes opcode 0xEA:
1. ✅ Reset vector FAR JMP will execute
2. ✅ BIOS will run from correct address (0xF0000 + offset)
3. ✅ CS.base will remain 0xF0000
4. ✅ BIOS will produce output (VGA text, POST codes)
5. ✅ Emulator will progress beyond initialization
6. ✅ Other bugs (if any) will become visible
