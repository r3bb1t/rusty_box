# Decoder Operand Direction Bug Investigation (2026-02-10)

## ✅ CRITICAL BUG FIXED: Ed,Gd Operand Direction

### The Bug

The decoder was assigning dst/src operands **backwards** for Ed,Gd format instructions.

**Affected opcodes**: 0x01, 0x09, 0x11, 0x19, 0x21, 0x29, 0x31, 0x89
**Examples**: ADD Ed,Gd | SUB Ed,Gd | MOV Ed,Gd

### Root Cause

In fetchdecode32.rs and fetchdecode64.rs, for ModRM instructions, the decoder was:
```rust
// WRONG - assumed nnn is always dest
instr.meta_data[BX_INSTR_METADATA_DST] = nnn as u8;  // reg field
instr.meta_data[BX_INSTR_METADATA_SRC1] = rm as u8;   // r/m field
```

But for Ed,Gd format (opcodes ending in 1 or 9):
- **Ed** (r/m field) is the DESTINATION
- **Gd** (reg field) is the SOURCE

### The Fix

Added operand direction check based on opcode low nibble:

```rust
if ((b1 & 0x0F) == 0x01) || ((b1 & 0x0F) == 0x09) || b1 == 0x89 {
    // Ed,Gd format: rm is dest, reg is source
    instr.meta_data[BX_INSTR_METADATA_DST] = rm as u8;
    instr.meta_data[BX_INSTR_METADATA_SRC1] = nnn as u8;
} else {
    // Gd,Ed format: reg is dest, rm is source
    instr.meta_data[BX_INSTR_METADATA_DST] = nnn as u8;
    instr.meta_data[BX_INSTR_METADATA_SRC1] = rm as u8;
}
```

### Impact on BIOS

**Before fix**:
- SUB ECX,EDI (29 F9) executed as `EDI = EDI - ECX` (WRONG!)
- BIOS _start function had corrupted registers
- ESI=0xFFFF0700, EDI=0 (should be ESI=0xE416F, EDI=0x700)
- Data copied to wrong addresses
- Execution jumped to garbage address 0x20000

**After fix**:
- SUB ECX,EDI now executes as `ECX = ECX - EDI` (CORRECT!)
- BIOS _start executes with correct register values
- BSS clearing and .data section copy work properly
- ✅ Emulator progresses much further

### Files Modified

- `rusty_box_decoder/src/fetchdecode32.rs` - Fixed Ed,Gd operand direction
- `rusty_box_decoder/src/fetchdecode64.rs` - Fixed Ed,Gd operand direction
- `rusty_box/src/cpu/ctrl_xfer32.rs` - Added CALL logging
- `rusty_box/src/cpu/cpu.rs` - Added _start and progress logging

### Verification Tools

Created test programs to verify decoder and BIOS ROM:
- `check_start.rs` - Verifies _start function bytes in BIOS ROM
- `check_addr_19.rs` - Verifies instructions in real-mode loop
- `check_065d.rs` - Verifies instructions at corruption points
- `test_mov_decode.rs` - Verifies MOV BP,SP decoding logic

## ❌ REMAINING ISSUE: Real Mode Infinite Loop

### Symptoms

After the decoder fix, the emulator gets stuck in a tight loop in **real mode** before reaching protected mode:

```
Infinite loop: RIP 0x0 → 0x15 → 0x19 → back to 0x0
SP CORRUPTION! 0xfffe -> 0x0 at RIP=0x19
Executes 400,000+ iterations without progressing
```

### Analysis

**Loop pattern**:
- F000:0000: `55 89 E5 ...` = PUSH BP; MOV BP,SP; ... (function prologue)
- F000:0015-0x19: `5F 07 59 58 5D C3` = POP DI; POP ES; POP CX; POP AX; POP BP; RET (epilogue)
- Returns to 0x0, repeats infinitely

**SP corruption**:
- SP starts at 0x7000 (correct)
- During function execution, SP becomes 0xfffe
- At POP BP (0x5D), SP suddenly becomes 0x0
- POPs read garbage from address 0x0
- RET jumps back to 0x0, repeating the loop

**Decoder verification**:
- ✅ MOV BP,SP (89 E5) decodes correctly: dst=BP(5), src=SP(4)
- ✅ POP BP (5D) decodes correctly: dst=BP(5)
- ✅ Single-byte opcodes not affected by Ed,Gd fix

### Possible Causes

1. **Stack pointer corruption**: Something is writing 0 to SP register
2. **Memory addressing bug**: Stack operations reading/writing wrong addresses
3. **16-bit mode issue**: Real mode might have different requirements
4. **Different decoder bug**: Another class of instructions has wrong operand order

### Next Steps

1. Add detailed logging for SP register changes
2. Log every instruction that modifies SP
3. Check if PUSH/POP are modifying SP correctly
4. Verify stack memory reads/writes go to correct addresses
5. Compare with original Bochs execution trace

## Commits

- **17cb19c**: CRITICAL FIX: Decoder operand direction bug for Ed,Gd instructions
- **e79eed3**: Fix BIOS ROM mapping: Include 0xE0000-0xFFFFF range
- **8bad6b0**: Add decoder tests and identify real bug in icache (not decoder)
