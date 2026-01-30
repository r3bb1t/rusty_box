# BIOS Stack Corruption Investigation

## Status: ✅ RESOLVED
**Date:** 2026-01-30
**Priority:** P0 - Blocks BIOS execution
**Resolution Date:** 2026-01-30
**Root Cause:** Decoder bug in fetchdecode32.rs - Group opcodes using wrong ModR/M field

---

## Executive Summary

The BIOS fails to display messages due to stack corruption around instruction 40,000. The root cause is traced to a memory initialization routine at F000:A124 where a 32-bit shift operation (SHL EAX, 0x10) appears to fail, causing the wrong value to be written to memory via REP STOSD.

---

## Bug Symptoms

1. **No BIOS messages displayed** - VGA text buffer remains empty
2. **Execution crashes** - Hits illegal opcode at RIP=0xFFEA
3. **Stack corruption** - RET pops garbage value 0xFF53 instead of correct return address
4. **Invalid memory execution** - Jumps to regions containing all zeros or invalid opcodes

---

## Timeline of Events

### What Works (0-40,000 instructions)
- ✅ Decoder correctly decodes A3 (MOV [moffs], AX) as 3 bytes
- ✅ BIOS executes normally through IVT setup
- ✅ Stack operations work correctly (SP starts at 0xFFFE, decrements normally)
- ✅ CALL/RET pairs execute correctly

### Where It Fails (~40,000 instructions)

**Location:** F000:E0CC → CALL F000:A124

**Function at A124 (Memory Initialization):**
```assembly
F000:A124  31 FF           XOR DI, DI              ; DI = 0
F000:A126  B9 78 00        MOV CX, 0x78            ; CX = 120 iterations
F000:A129  B8 00 F0        MOV AX, 0xF000          ; AX = 0xF000
F000:A12C  66 C1 E0 10     SHL EAX, 0x10           ; EAX should = 0xF0000000
F000:A130  B8 53 FF        MOV AX, 0xFF53          ; EAX should = 0xF000FF53
F000:A133  FC              CLD                      ; Clear direction flag
F000:A134  F3 66 AB        REP STOSD               ; Store EAX 120 times
F000:A137  BB 20 00        MOV BX, 0x20
...
F000:A1CC  C3              RET                      ; Returns to garbage!
```

**Expected Behavior:**
1. MOV AX, 0xF000 → EAX = 0x0000F000
2. SHL EAX, 0x10 → EAX = 0xF0000000 (shift left 16 bits)
3. MOV AX, 0xFF53 → EAX = 0xF000FF53 (set lower 16 bits)
4. REP STOSD writes 0xF000FF53 to ES:DI 120 times

**Actual Behavior:**
```
STOSD16: EAX=0x0000ff53 -> ES:0000
```
- EAX = 0x0000FF53 (upper 16 bits are 0x0000 instead of 0xF000!)
- This means either:
  - SHL EAX, 0x10 didn't work (didn't shift)
  - OR MOV AX, 0xFF53 cleared all 32 bits instead of just lower 16

**Consequence:**
- Wrong value written to memory
- Corrupts something (possibly stack or critical data structures)
- When function returns, RET pops 0xFF53 (garbage) instead of 0xE0CC
- Jumps to F000:FF53 (contains IRET instruction)
- IRET pops from corrupted stack (SP=0x0002)
- Returns to FF53:0000 containing zeros
- Executes invalid instructions until hitting illegal opcode

---

## Root Cause Analysis

### Hypothesis 1: SHL EAX, 0x10 Bug in 16-bit Mode ⚠️ LIKELY

**Evidence:**
- Instruction bytes: `66 C1 E0 10`
  - 66 = Operand size prefix (use 32-bit operand in 16-bit mode)
  - C1 E0 10 = SHL EAX, 0x10
- Our decoder recognizes it as `ShlEdIb` (correct)
- Implementation in `shift.rs::shl_ed_ib()` looks correct on surface
- But EAX value shows shift didn't happen

**Code Location:** `rusty_box/src/cpu/shift.rs:171-184`
```rust
pub fn shl_ed_ib(&mut self, instr: &BxInstructionGenerated) {
    let count = (instr.ib() & 0x1F) as u32;
    if count == 0 { return; }

    let dst = instr.meta_data[0] as usize;
    let op1 = self.get_gpr32(dst);  // Read EAX as 32-bit

    let result = op1 << count;      // Shift left
    self.set_gpr32(dst, result);    // Write back to EAX

    let cf = ((op1 << (count - 1)) & 0x80000000) != 0;
    let of = if count == 1 { ((result ^ op1) & 0x80000000) != 0 } else { false };
    self.update_flags_shl32(result, cf, of);
}
```

**Possible Issues:**
- `instr.meta_data[0]` might be wrong register index
- `get_gpr32()` might not read full 32 bits correctly
- `set_gpr32()` might not write full 32 bits correctly
- Operand size prefix handling in decoder might be wrong

### Hypothesis 2: MOV AX, 0xFF53 Overwrites All 32 Bits ⚠️ POSSIBLE

**Evidence:**
- After SHL, EAX should be 0xF0000000
- MOV AX, 0xFF53 should only set lower 16 bits → EAX = 0xF000FF53
- But we see EAX = 0x0000FF53 (all 32 bits affected?)

**Code Location:** `rusty_box/src/cpu/data_xfer_ext.rs:212-217`
```rust
pub fn mov_rw_iw(&mut self, instr: &BxInstructionGenerated) {
    let dst = instr.meta_data[0] as usize;
    let imm = instr.iw();
    self.set_gpr16(dst, imm);  // Should only set lower 16 bits!
    tracing::trace!("MOV: reg{} = {:#06x}", dst, imm);
}
```

**Register Union Layout (Little Endian):**
```rust
pub union BxGenReg {
    pub word: BxGenRegWord,  // 16-bit access
    pub rrx: u64,            // 64-bit access
    pub dword: BxGenRegDword // 32-bit access
}

pub union BxGenRegWord {
    pub rx: u16,             // Lower 16 bits
    pub byte: BxWordByte,
    pub word_filler: u16,
    pub dword_filler: u16,
}

pub struct BxGenRegDword {
    pub erx: u32,            // Lower 32 bits (in little endian)
    pub hrx: u32,            // Upper 32 bits
}
```

**Memory Layout:** For a 64-bit register like RAX:
```
Offset 0-1: word.rx (AX)
Offset 0-3: dword.erx (EAX)
Offset 0-7: rrx (RAX)
```

**set_gpr16 Implementation:** `cpu_getters_and_setters.rs:500-502`
```rust
pub fn set_gpr16(&mut self, reg: usize, val: u16) {
    self.gen_reg[reg].word.rx = val;  // Should only write bytes 0-1
}
```

**Analysis:**
- Union should work correctly - writing to `word.rx` should only modify bytes 0-1
- BUT: Need to verify in practice with actual execution

### Hypothesis 3: Register Union Memory Layout Bug ⚠️ POSSIBLE

The union might not be laying out memory correctly, causing writes to affect more bytes than intended.

**Test Needed:** Add trace logging to see actual register values before/after each instruction.

---

## Investigation Steps Taken

### ✅ Completed

1. **Decoder Analysis**
   - Fixed A0-A3 opcodes (MOV with direct offset) - missing from immediate size calculation
   - Verified fix: A3 instructions now correctly show 3 bytes

2. **Execution Trace Analysis**
   - Traced execution from start to crash
   - Identified stack corruption at ~40,000 instructions
   - Found CALL/RET mismatch (called A124, returned to FF53)

3. **BIOS Code Analysis**
   - Disassembled BIOS at A124-A1CC
   - Identified REP STOSD memory initialization routine
   - Verified BIOS binary contains correct instruction bytes

4. **Stack Trace Analysis**
   - Tracked PUSH/POP operations
   - Found stack pointer wraps from 0xFFFE correctly
   - Identified RET popping garbage value 0xFF53

5. **Code Review**
   - Reviewed SHL implementation - looks correct on surface
   - Reviewed MOV implementation - looks correct on surface
   - Reviewed register union layout - should work correctly

### 🔲 TODO (Next Steps)

1. **Add Detailed Register Tracing**
   - Log EAX value before/after SHL instruction
   - Log EAX value before/after MOV AX instruction
   - Verify if SHL is executing or being skipped

2. **Test Register Operations in Isolation**
   - Create unit test for `set_gpr16` preserving upper bits
   - Create unit test for `set_gpr32` and `get_gpr32`
   - Create unit test for SHL with 32-bit operands in 16-bit mode

3. **Check Operand Size Prefix Handling**
   - Verify decoder correctly interprets 0x66 prefix
   - Check if opcode table selects correct handler
   - Verify `instr.meta_data[0]` contains correct register index

4. **Compare with Original Bochs**
   - Check Bochs C++ implementation of SHL
   - Check how Bochs handles 0x66 prefix in 16-bit mode
   - Verify register union layout matches Bochs

5. **Memory Write Analysis**
   - Check if REP STOSD is writing to correct addresses
   - Verify ES segment register value during STOSD
   - Check if memory writes are affecting the stack

---

## Code Locations

### Key Files

| File | Line | Description |
|------|------|-------------|
| `rusty_box/src/cpu/shift.rs` | 171-184 | SHL Ed, Ib implementation |
| `rusty_box/src/cpu/data_xfer_ext.rs` | 212-217 | MOV r16, imm16 implementation |
| `rusty_box/src/cpu/cpu_getters_and_setters.rs` | 500-502 | set_gpr16 implementation |
| `rusty_box/src/cpu/cpu.rs` | 94-132 | BxGenReg union definition |
| `rusty_box/src/cpu/string.rs` | 182-197 | STOSD16 implementation |
| `rusty_box_decoder/src/fetchdecode32.rs` | 950-1037 | Immediate size calculation (FIXED) |

### Related Bochs Code

| File | Description |
|------|-------------|
| `cpp_orig/bochs/cpu/shift32.cc` | Original SHL implementation |
| `cpp_orig/bochs/cpu/data_xfer32.cc` | Original MOV implementation |
| `cpp_orig/bochs/cpu/cpu.h` | Original register union layout |

---

## Test Cases Needed

### Test 1: Register Write Preservation
```rust
#[test]
fn test_set_gpr16_preserves_upper_bits() {
    let mut cpu = create_test_cpu();

    // Set EAX to 0xF0000000
    cpu.set_gpr32(0, 0xF0000000);
    assert_eq!(cpu.get_gpr32(0), 0xF0000000);

    // Set AX to 0xFF53 (lower 16 bits only)
    cpu.set_gpr16(0, 0xFF53);

    // Verify upper 16 bits preserved
    assert_eq!(cpu.get_gpr32(0), 0xF000FF53, "Upper 16 bits should be preserved!");
}
```

### Test 2: SHL 32-bit in 16-bit Mode
```rust
#[test]
fn test_shl_eax_in_16bit_mode() {
    let mut cpu = create_test_cpu();
    cpu.set_gpr32(0, 0x0000F000);  // EAX = 0xF000

    // Create SHL EAX, 0x10 instruction (66 C1 E0 10)
    let instr = decode_instruction(&[0x66, 0xC1, 0xE0, 0x10], false);

    cpu.shl_ed_ib(&instr);

    assert_eq!(cpu.get_gpr32(0), 0xF0000000, "SHL EAX, 16 should shift correctly!");
}
```

---

## Workarounds Attempted

None yet - bug must be fixed for BIOS to work.

---

## Related Issues

1. **Original Decoder Bug (FIXED)** - A0-A3 opcodes had wrong instruction length
2. **Stack Corruption (ACTIVE)** - This bug
3. **BIOS Messages Not Appearing (BLOCKED)** - Blocked by stack corruption

---

## Debug Commands

### Run with detailed tracing
```bash
cd C:\Users\Aslan\claude_rusty_box
cargo run --release --example dlxlinux --features std 2>&1 | tee execution.log
```

### Extract SHL/MOV sequence
```bash
grep -E "Execute: F000:A1(2[C-F]|3[0-3])" execution.log
```

### Check EAX values during STOSD
```bash
grep "STOSD16: EAX" execution.log | head -n 5
```

### Find stack corruption point
```bash
grep -B 10 "popped 0 from SS:SP=0000:0002" execution.log
```

---

## Resolution

### Root Cause Found

**File:** `rusty_box_decoder/src/fetchdecode32.rs` lines 412-436

**Bug:** For Group 2 instructions (opcodes C0, C1, D0-D3, F6, F7, FE, FF), the decoder was using the **reg/nnn field** as the destination register instead of the **rm field**.

For Group instructions:
- The `nnn` field (bits 3-5 of ModR/M) is the **opcode extension** (specifies which operation: ROL, ROR, SHL, SHR, etc.)
- The `rm` field (bits 0-2 of ModR/M) is the **operand register**

For instruction `66 C1 E0 10` (SHL EAX, 0x10):
- ModR/M byte = E0 = binary `11 100 000`
- mod = 11 (register direct)
- nnn/reg = 100 (4 = SHL opcode extension)
- rm = 000 (0 = EAX register)

**What went wrong:** Decoder put `nnn=4` (ESP) in meta_data[0] instead of `rm=0` (EAX), causing the shift to operate on ESP instead of EAX!

### Fix Applied

Added check for Group opcodes in fetchdecode32.rs:

```rust
if needs_modrm {
    let is_group_opcode = matches!(b1, 0xC0 | 0xC1 | 0xD0 | 0xD1 | 0xD2 | 0xD3 | 0xF6 | 0xF7 | 0xFE | 0xFF);

    if is_group_opcode {
        // Group opcodes: operand is in rm, opcode extension in nnn
        instr.meta_data[BX_INSTR_METADATA_DST] = rm as u8;
        instr.meta_data[BX_INSTR_METADATA_SRC1] = nnn as u8;
    } else {
        // Normal ModRM: nnn is dest register, rm is source
        instr.meta_data[BX_INSTR_METADATA_DST] = nnn as u8;
        instr.meta_data[BX_INSTR_METADATA_SRC1] = rm as u8;
    }
}
```

### Verification

After fix, the instruction sequence works correctly:

```
MOV AX, 0xF000    → EAX = 0x0000F000 ✅
SHL EAX, 0x10     → EAX = 0xF0000000 ✅ (was shifting ESP before!)
MOV AX, 0xFF53    → EAX = 0xF000FF53 ✅ (upper bits preserved)
REP STOSD         → Writes correct value ✅
```

BIOS now executes past 40,000 instructions without stack corruption!

### Impact

- **Before fix:** Execution crashed at ~40,000 instructions with illegal opcode at 0xFFEA
- **After fix:** Execution proceeds to 0x969C where it hits unimplemented `ShrEbIb` instruction
- **Stack corruption:** Eliminated
- **BIOS progress:** Significant - now needs more instruction implementations

---

## References

- Intel 64 and IA-32 Architectures Software Developer's Manual, Vol 2
- Bochs source code: `cpp_orig/bochs/cpu/`
- Previous investigation: `Executed instructions.txt` (outdated)
