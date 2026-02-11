# BP Register Corruption at RIP 0x2055 - SYMPTOM, NOT ROOT CAUSE

## Date: 2026-02-11

## ⚠️ UPDATE: This is NOT the Root Cause

**ACTUAL ROOT CAUSE**: Opcode 0xEA (FAR JMP) missing from decoder tables. See `FAR_JUMP_DECODER_BUG.md` for details.

**This document describes a SYMPTOM**: When the decoder fails to recognize opcode 0xEA at the reset vector, the CPU jumps to address 0 and executes from zeroed memory. The "BP corruption" and "infinite loop at 0x2055" are consequences of executing garbage data instead of actual BIOS code.

---

## Problem (Original Investigation)

BIOS execution gets stuck in infinite loop at RIP 0x2055-0x2074 because **BP register is corrupted to 0**.

## Evidence

### Execution Results (50M instructions):
```
Final RIP:   0x0000000000002055
EAX=c00000e0 EBX=00000000 ECX=0000c800 EDX=0000e0d5
ESP=0000fffa EBP=00000000 ESI=00000000 EDI=00000400  ← BP is ZERO!
```

### The Countdown Loop (BIOS offset 0x2055-0x2074):
```assembly
002055  8a 86 dd fd     MOV AL, [BP-547]    ; Load value
002059  30 e4           XOR AH, AH
00205b  d1 e8           SHR AX, 1           ; Divide by 4 (shift right 2x)
00205d  d1 e8           SHR AX, 1
00205f  88 86 dd fd     MOV [BP-547], AL    ; Store result

002063  8a 86 f1 fd     MOV AL, [BP-271]    ; Load counter
002067  48              DEC AX              ; Decrement
002068  88 86 f1 fd     MOV [BP-271], AL    ; Store counter

00206c  8a 86 f1 fd     MOV AL, [BP-271]    ; Load counter again
002070  84 c0           TEST AL, AL         ; Check if zero
002072  75 e1           JNZ 0x2055          ; Loop if not zero
```

**Loop Logic:**
1. Divide value at `[BP-547]` by 4
2. Decrement counter at `[BP-271]`
3. Loop until counter reaches 0

### Why It's Infinite

**With BP=0:**
- `[BP-271]` = `[0x0000-271]` = `[0xFEF1]` (wraps in 16-bit)
- `[BP-547]` = `[0x0000-547]` = `[0xFDDD]` (wraps in 16-bit)

These addresses are **NOT the intended stack locations**. The counter value read from `[0xFEF1]` is not being modified by the loop (writes go elsewhere or are ignored), so the loop never terminates.

**Expected Behavior (with BP≈0xFFFA):**
- `[BP-271]` = `[0xFFFA-271]` = `[0xFEEB]` ✓ Valid stack location
- `[BP-547]` = `[0xFFFA-547]` = `[0xFDD7]` ✓ Valid stack location
- Counter decrements properly, loop terminates

## Historical BP Values

Earlier in execution (from previous logs):
```
BP=0xfff0, SP=0xffec  ← Correct
BP=0xffca, SP=0xffc6  ← Correct
BP=0xffd6, SP=0xffd2  ← Correct
```

**Conclusion:** BP was initially correct (~0xFFF0), but got corrupted to 0 at some point before reaching 0x2055.

## Impact

### Symptoms:
- ❌ BIOS stuck in infinite loop (50M+ instructions)
- ❌ No VGA output (blank screen)
- ❌ No VGA writes detected (mapped_writes=0)
- ❌ No POST codes captured
- ❌ BIOS never progresses beyond early initialization

### Why This Breaks Everything:
1. **Stack frame invalid**: All local variable accesses use wrong base
2. **Function returns broken**: RET uses wrong return address location
3. **Parameter passing broken**: Function arguments accessed via BP are wrong
4. **Nested calls impossible**: Each call corrupts the chain further

## Root Cause Investigation

### Possible Causes:

1. **ENTER/LEAVE instruction bug**
   - LEAVE does: `MOV SP,BP; POP BP`
   - If LEAVE implementation is wrong, BP could be zeroed
   - Check: `rusty_box/src/cpu/ctrl_xfer*/ctrl_xfer*.rs`

2. **POP BP bug**
   - If stack is corrupted, `POP BP` could load 0
   - Check: `rusty_box/src/cpu/stack*.rs`

3. **MOV BP, 0 executed accidentally**
   - Control flow bug causing wrong instruction execution
   - Check execution trace before RIP 0x2055

4. **Register save/restore bug**
   - Context switch or exception handling corrupting BP
   - Check: `rusty_box/src/cpu/exception.rs`

5. **Stack overflow/underflow**
   - SP wraps, causes POP to read zeros
   - Check: Stack pointer validation

### Investigation Strategy:

**Step 1:** Add BP change tracking
```rust
// In cpu_loop, track every BP modification
static mut LAST_BP: u16 = 0xFFFF;
let current_bp = self.bp() as u16;
if current_bp != unsafe { LAST_BP } {
    tracing::warn!("BP changed: {:#x} → {:#x} at RIP={:#x}, opcode={:?}",
        unsafe { LAST_BP }, current_bp, self.rip(), instr.get_ia_opcode());
    unsafe { LAST_BP = current_bp; }
}
```

**Step 2:** Trace execution path to 0x2055
- Add logging for RIP values 0x2040-0x2054
- Identify exact instruction that jumps to 0x2055
- Check what set up BP before the loop

**Step 3:** Verify LEAVE implementation
Compare our `rusty_box/src/cpu/ctrl_xfer*/` with Bochs `cpp_orig/bochs/cpu/ctrl_xfer*.cc`

**Step 4:** Check POP BP implementation
Compare our `rusty_box/src/cpu/stack*.rs` with Bochs `cpp_orig/bochs/cpu/stack*.cc`

## Related Issues

- **MEMORY_AND_STACK_INVESTIGATION.md**: Earlier investigation of stack addressing
- **VGA_BIOS_LOADING_SEQUENCE_BUG.md**: BIOS loading timing fix (didn't solve this)
- **Plan file (lexical-snacking-bubble.md)**: Predicted stack/memory addressing issue

## Expected Fix

Once BP corruption is fixed:
1. ✅ Countdown loop will complete normally
2. ✅ BIOS will progress beyond RIP 0x2055
3. ✅ VGA BIOS will be discovered and executed
4. ✅ VGA text output will appear
5. ✅ POST codes will be captured
6. ✅ BIOS will reach boot sector loading

## Priority

**CRITICAL** - This blocks all BIOS progress. Must be fixed before any other issues can be discovered.
