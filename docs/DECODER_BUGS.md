# Decoder Bug History

This document records significant decoder bugs found and fixed in the Rusty Box x86 instruction decoder.

## VEX Immediate Clobbers Vector Length (Session 52, 2026-03-18)

**Severity**: CRITICAL
**Files**: `rusty_box_decoder/src/decoder/decode64.rs`, `rusty_box_decoder/src/decoder/decode32.rs`
**Symptom**: All 256-bit VEX instructions with an imm8 operand silently executed as 128-bit
**Impact**: OpenSSL SHA-1 AVX2 produced wrong hash, causing Alpine `apk` to reject package signatures ("BAD signature")

### Root Cause

The `immediate` field in `Instruction` stores both the 8-bit immediate value (byte 0) and VEX metadata (bytes 1-3):
- Byte 0: imm8 value
- Byte 1: VL (vector length: 0=128-bit, 1=256-bit)
- Byte 2: VEX.W bit (bit 4), VEX flag (bit 5)
- Byte 3: reserved

The decoder set VL correctly during VEX prefix parsing (`set_vl(vex_l)` at line 756). But the immediate parsing phase (line 799) then executed:
```rust
instr.immediate = byte_val as u32;  // Overwrites ALL 4 bytes!
```
This zeroed bytes 1-3, destroying VL and VEX flags. Any VEX instruction with an imm8 operand (VPALIGNR, VPBLENDD, VPSHUFD, VPSLLD/VPSRLD imm, VPSRLDQ/VPSLLDQ imm, etc.) had VL=0 and executed the 128-bit path.

### Fix

Write only byte 0 of the `immediate` field for non-sign-extended immediates:
```rust
let mut ib = instr.immediate.to_ne_bytes();
ib[0] = byte_val;
instr.immediate = u32::from_ne_bytes(ib);
```

Sign-extended immediates (branches, Group 1 EqsIb, PUSH imm8, IMUL imm8) still overwrite the full field since they're never VEX-encoded.

### Why It Was Hard to Find

- All VEX instructions WITHOUT imm8 (VPADDD, VPXOR, VPSHUFB, etc.) worked correctly because the immediate parsing was skipped (imm_size=0), leaving VL intact.
- The SHA-1 AVX2 function uses a mix of imm8 and non-imm8 instructions. The data-shuffling instructions (VPSHUFB) worked correctly at 256-bit, but the rotation/alignment instructions (VPALIGNR, shift-by-immediate) silently fell back to 128-bit, producing subtly wrong results.
- 50+ audit agents verified individual instruction handlers were correct against Bochs -- because the handlers ARE correct. The bug was in the decoder, upstream of all handlers.
- The symptom (wrong SHA-1 hash) only manifested when CPUID advertised AVX2, causing OpenSSL to select the AVX2 code path. Disabling AVX2 in CPUID made the 128-bit SSE path work correctly, which appeared to confirm an instruction handler bug.

### Affected Instructions

All VEX-encoded instructions with imm8 operand, including:
- VPALIGNR (VEX 0F3A 0F): byte-align-right, critical for SHA-1 message schedule
- VPBLENDD (VEX 0F3A 02): blend dwords by immediate mask
- VPSHUFD (VEX 0F 70): shuffle dwords
- VPSLLD/VPSRLD/VPSRAD imm (VEX 0F 72): shift dwords by immediate
- VPSLLQ/VPSRLQ imm (VEX 0F 73): shift qwords by immediate
- VPSLLDQ/VPSRLDQ imm (VEX 0F 73): shift double-quadword by immediate
- VPERM2I128 (VEX 0F3A 46): permute 128-bit lanes
- All other VEX instructions dispatched via opcode tables with Ib operand

## CPUID Feature Status

As of session 52, the following features are enabled/disabled in the Skylake-X CPUID model:

| Feature | CPUID Leaf | Status | Notes |
|---------|-----------|--------|-------|
| SSE through SSE4.2 | 1 ECX/EDX | Enabled | Full handler coverage |
| AVX | 1 ECX | Enabled | 128-bit VEX path |
| AVX2 | 7 EBX | Enabled | 256-bit VEX path, decoder VL fix applied |
| FMA | 1 ECX | Enabled | |
| AES-NI | 1 ECX | Enabled | |
| BMI1/BMI2 | 7 EBX | Enabled | |
| XSAVE/XSAVEC/XSAVEOPT | 1 ECX / 0xD | Enabled | Compacted format |
| AVX-512 (F/DQ/CD/BW/VL) | 7 EBX | **Disabled** | 512-bit handlers not implemented |
| VMX | 1 ECX | **Disabled** | Virtualization not implemented |
