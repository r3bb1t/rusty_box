# Decoder Bug: Missing Immediate Size for Group 3a (0xF6/0xF7)

## Date
2026-02-02

## Summary
The decoder in `fetchdecode32.rs` fails to account for immediate bytes in Group 3a TEST instructions (opcodes 0xF6/0xF7 with ModRM.nnn=0 or 1), causing instruction length miscalculation and RIP misalignment.

## Root Cause
In `get_immediate_size_32()` (fetchdecode32.rs:986-1085), the function returns 0 for opcode 0xF6, but should return 1 for TEST variants (F6/0 and F6/1).

### Problem Code
```rust
// Line 1022-1027: Ib immediates
0x80 | 0x82 | 0x83 | 0xC0 | 0xC1 | 0xC6 => 1,
// 0xF6 and 0xF7 are MISSING!
```

### Complication
Not all F6/F7 variants have immediates:
- F6 /0 (TEST r/m8, imm8) - HAS immediate ✓
- F6 /1 (TEST r/m8, imm8) - HAS immediate ✓ (undocumented alias)
- F6 /2 (NOT r/m8) - NO immediate
- F6 /3 (NEG r/m8) - NO immediate
- F6 /4 (MUL r/m8) - NO immediate
- F6 /5 (IMUL r/m8) - NO immediate
- F6 /6 (DIV r/m8) - NO immediate
- F6 /7 (IDIV r/m8) - NO immediate

Same pattern for F7 (16/32-bit versions).

## Impact
When the decoder encounters `TEST BYTE PTR [disp32], imm8` (F6 /0 with mod=00, rm=101):
1. Calculates length as 6 bytes (missing the immediate byte)
2. RIP advances by 6 instead of 7
3. Next instruction decoded from wrong offset
4. Eventually hits "illegal opcode" when decoding from middle of instruction

## Example from BIOS Execution
```
0xe1d44: f6 05 31 07 00 00 02  = TEST BYTE PTR [0x731], 0x02
         ^^opcode
            ^^ModRM (mod=00, reg=000, rm=101)
               ^^ ^^ ^^ ^^disp32 = 0x00000731
                           ^^imm8 = 0x02

Correct length: 7 bytes (1+1+4+1)
Decoder calculated: 6 bytes (missing immediate!)

Next RIP: 0xe1d44 + 6 = 0xe1d4a (WRONG! Should be 0xe1d4b)
```

This misalignment cascades through subsequent instructions until 0xe1d59, where execution attempts to decode `fe b9` (invalid: Group 4 with /7 extension).

## Fix Required
The `get_immediate_size_32()` function needs to handle Group 3a/3b instructions. However, it currently only receives the opcode byte, not the ModRM byte, so it cannot distinguish TEST (needs immediate) from NOT/NEG/etc (no immediate).

### Possible Solutions
1. **Pass ModRM.nnn to function** - Check if nnn=0 or 1 for F6/F7
2. **Post-ModRM immediate handling** - Move immediate size calculation after ModRM parsing
3. **Add F6/F7 with conditional** - Check ModRM later and adjust ilen

## Files Affected
- `rusty_box_decoder/src/fetchdecode32.rs` - `get_immediate_size_32()` function
- `rusty_box_decoder/src/fetchdecode64.rs` - Same issue likely exists for 64-bit mode

## Test Case
```rust
// TEST BYTE PTR [0x00000731], 0x02
let bytes = vec![0xf6, 0x05, 0x31, 0x07, 0x00, 0x00, 0x02];
let instr = fetch_decode32(&bytes, true).unwrap();
assert_eq!(instr.ilen(), 7); // Currently fails: returns 6
```

## Related Issues
- Immediate BIOS illegal opcode error at RIP 0xe1d59
- Exception handling implementation (separate issue)
- Protected mode with uninitialized IDT (separate issue)
