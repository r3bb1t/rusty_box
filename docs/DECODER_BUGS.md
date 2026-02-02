# Decoder Bug Fixes and Validation

This document records historical bugs found in the x86 instruction decoder and their fixes.

## Invalid Segment Register Handling

### Bug Description

**Discovered:** 2026-02-01
**Severity:** Medium (incorrect CPU behavior, masking decoder bug)

The original decoder could generate segment register indices 6 and 7 for instructions:
- `0x8C` - MOV r/m16, Sreg (move segment register to general register)
- `0x8E` - MOV Sreg, r/m16 (move general register to segment register)

These indices are invalid per x86 specification and should cause a `#UD` (Undefined Opcode) exception.

**Symptoms:**
```rust
// Warning message observed in logs:
Invalid segment register 6 - using DS as workaround
Invalid segment register 7 - using DS as workaround
```

**Workaround (removed):**
The CPU implementation in `rusty_box/src/cpu/data_xfer_ext.rs` contained a workaround that silently mapped invalid indices to DS (segment register 3):

```rust
// REMOVED WORKAROUND:
let actual_seg = if src_seg == 6 || src_seg == 7 {
    tracing::warn!("Invalid segment register {} - using DS as workaround", src_seg);
    3 // DS
} else {
    src_seg
};
```

This workaround masked a decoder bug and violated x86 semantics.

### x86 Specification

**Segment Register Encoding (ModRM.nnn field, bits 3-5):**
- `0` = ES (Extra Segment)
- `1` = CS (Code Segment)
- `2` = SS (Stack Segment)
- `3` = DS (Data Segment)
- `4` = FS (Additional Segment, 386+)
- `5` = GS (Additional Segment, 386+)
- `6`, `7` = INVALID (should cause #UD exception)

**Affected Instructions:**
- Opcode `0x8C`: MOV r/m16, Sreg - Move segment register to r/m16
- Opcode `0x8E`: MOV Sreg, r/m16 - Move r/m16 to segment register

The ModRM byte format:
```
  7   6   5   4   3   2   1   0
+---+---+---+---+---+---+---+---+
|  mod  |   nnn     |    r/m    |
+---+---+---+---+---+---+---+---+
        ^-----------^
        segment register index
```

### Root Cause

**Location:** `rusty_box_decoder/src/fetchdecode32.rs` and `fetchdecode64.rs`

The decoder extracted the `nnn` field from the ModRM byte but did not validate that segment register indices must be in the range 0-5:

```rust
// Original (buggy) code:
let nnn = (modrm_byte >> 3) & 0x7;  // Extract 3 bits
instr.meta_data[0] = nnn as u8;      // Store without validation
```

This allowed invalid values (6 and 7) to propagate to the CPU execution layer.

### Fix Implementation

**Date:** 2026-02-01
**Files Modified:**
- `rusty_box_decoder/src/fetchdecode32.rs`
- `rusty_box_decoder/src/fetchdecode64.rs`
- `rusty_box_decoder/src/error.rs`
- `rusty_box/src/cpu/data_xfer_ext.rs`

**Step 1: Add validation in decoder**

Added explicit validation after extracting ModRM.nnn field:

```rust
// Validate segment register for MOV instructions (0x8C, 0x8E)
if matches!(b1, 0x8C | 0x8E) && nnn > 5 {
    return Err(DecodeError::InvalidSegmentRegister {
        index: nnn as u8,
        opcode: b1 as u8,
    });
}
```

**Step 2: Add error variant**

Added new error type to `DecodeError` enum:

```rust
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    // ... existing variants ...

    #[error("Invalid segment register index {index} in opcode {opcode:#04x} (valid: 0-5)")]
    InvalidSegmentRegister { index: u8, opcode: u8 },
}
```

**Step 3: Remove CPU workaround**

Replaced the workaround with a debug assertion to catch future decoder bugs:

```rust
pub fn mov_ew_sw(&mut self, instr: &BxInstructionGenerated) {
    let dst = instr.meta_data[0] as usize;
    let src_seg = instr.meta_data[1] as usize;

    // Decoder should never give us invalid segment registers
    debug_assert!(
        src_seg <= 5,
        "Invalid segment register {} from decoder",
        src_seg
    );

    let seg_val = self.sregs[src_seg].selector.value;
    self.set_gpr16(dst, seg_val);
    tracing::trace!("MOV: reg{} = seg{} ({:#06x})", dst, src_seg, seg_val);
}
```

**Step 4: Add comprehensive tests**

Added tests to verify decoder correctly accepts valid segment registers (0-5) and rejects invalid ones (6-7):

```rust
#[cfg(test)]
mod segment_register_tests {
    use super::*;

    #[test]
    fn test_mov_segment_valid() {
        // Test opcodes 0x8C and 0x8E with nnn=0 through nnn=5
        for seg in 0..=5 {
            let modrm = 0xC0 | (seg << 3);  // MOD=11, REG=seg, R/M=0

            // 0x8C: MOV r/m16, Sreg
            let bytes = vec![0x8C, modrm];
            let result = fetch_decode32_chatgpt_generated_instr(&bytes, true);
            assert!(result.is_ok(), "Failed to decode valid segment {}", seg);

            // 0x8E: MOV Sreg, r/m16
            let bytes = vec![0x8E, modrm];
            let result = fetch_decode32_chatgpt_generated_instr(&bytes, true);
            assert!(result.is_ok(), "Failed to decode valid segment {}", seg);
        }
    }

    #[test]
    fn test_mov_segment_invalid() {
        // Test opcodes 0x8C and 0x8E with nnn=6 and nnn=7
        for seg in 6..=7 {
            let modrm = 0xC0 | (seg << 3);

            // Should fail with InvalidSegmentRegister
            let bytes = vec![0x8C, modrm];
            let result = fetch_decode32_chatgpt_generated_instr(&bytes, true);
            assert!(
                matches!(result, Err(DecodeError::InvalidSegmentRegister { .. })),
                "Should reject invalid segment register {}", seg
            );

            let bytes = vec![0x8E, modrm];
            let result = fetch_decode32_chatgpt_generated_instr(&bytes, true);
            assert!(
                matches!(result, Err(DecodeError::InvalidSegmentRegister { .. })),
                "Should reject invalid segment register {}", seg
            );
        }
    }
}
```

### Testing

**Test Coverage:**
- ✅ Valid segment registers (0-5) decode successfully for both 0x8C and 0x8E
- ✅ Invalid segment registers (6-7) return `DecodeError::InvalidSegmentRegister`
- ✅ Both 32-bit and 64-bit decoder modes tested
- ✅ All test cases pass

**Verification:**
```bash
cd rusty_box_decoder
cargo test segment_register -- --nocapture
```

**Expected Output:**
```
running 4 tests
test fetchdecode32::segment_register_tests::test_mov_segment_invalid ... ok
test fetchdecode32::segment_register_tests::test_mov_segment_valid ... ok
test fetchdecode64::segment_register_tests::test_mov_segment_invalid ... ok
test fetchdecode64::segment_register_tests::test_mov_segment_valid ... ok
```

### Impact

**Before Fix:**
- Decoder could produce invalid segment register indices
- CPU implementation silently mapped invalid indices to DS
- Violated x86 semantics (should cause #UD exception)
- Masked decoder bugs from being detected

**After Fix:**
- Decoder rejects invalid segment register encodings at decode time
- CPU implementation trusts decoder output (with debug assertions)
- Correct x86 behavior - invalid encodings are detected early
- No silent workarounds - failures are explicit

### Lessons Learned

1. **Validate at the earliest point:** Decoder should validate all instruction encodings before producing `BxInstructionGenerated` structures.

2. **Avoid silent workarounds in execution layer:** The CPU should trust the decoder. If invalid data appears, it indicates a decoder bug that must be fixed at the source.

3. **Use debug assertions to catch regressions:** After fixing decoder bugs, add `debug_assert!()` in the CPU to catch future regressions.

4. **Comprehensive testing:** Add tests for both valid and invalid cases to ensure validation works correctly.

5. **Document x86 specifications:** Include spec references in validation code to explain why certain values are invalid.

## Future Validation Opportunities

Other areas where decoder validation could be added:

1. **Control register indices** - CR0-CR4, CR8 valid; CR5-CR7, CR9-CR15 invalid (except CR8 in 64-bit mode)
2. **Debug register indices** - DR0-DR3, DR6-DR7 valid; DR4-DR5 aliased or invalid depending on CR4.DE
3. **REX prefix in 32-bit mode** - Should cause #UD exception
4. **LOCK prefix** - Only valid on specific memory-destination instructions
5. **Operand size conflicts** - Certain combinations of prefixes/modes are invalid
6. **VEX/EVEX encoding validation** - Many constraints on prefix bytes and opcode maps

Each validation added catches bugs earlier and makes the emulator more correct and maintainable.
