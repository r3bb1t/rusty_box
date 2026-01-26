---
name: Decoder const fn rewrite
overview: Create const fn versions of fetch_decode64/32 in separate files (attempt with likely fallback to deletion), remove undefined cfg features by always enabling the code, and fill in unimplemented VEX/EVEX/XOP/3DNow decoder logic based on the C++ reference.
todos:
  - id: remove-cfg
    content: Remove undefined cfg(feature) attributes for 3dnow/avx/x86_64/evex
    status: completed
  - id: impl-vex
    content: Implement full VEX decoder (decoder_vex64/32) based on C++ reference
    status: completed
    dependencies:
      - remove-cfg
  - id: impl-evex
    content: Implement full EVEX decoder (decoder_evex64/32) based on C++ reference
    status: completed
    dependencies:
      - remove-cfg
  - id: impl-xop
    content: Implement full XOP decoder (decoder_xop64/32) based on C++ reference
    status: completed
    dependencies:
      - remove-cfg
  - id: const-attempt
    content: Create const_fetchdecode64/32.rs and attempt const fn rewrite
    status: completed
    dependencies:
      - impl-vex
      - impl-evex
      - impl-xop
  - id: cleanup
    content: Delete const files if not achievable, document limitations
    status: completed
---

# Decoder Const Fn Rewrite and Feature Cleanup

## Overview

This plan addresses three main tasks:
1. Attempt `const fn` rewrite in separate files (likely to fail due to Rust limitations)
2. Remove undefined `cfg(feature = ...)` attributes by always enabling the code
3. Fill in unimplemented VEX/EVEX/XOP/3DNow decoder logic

---

## 1. Const Fn Attempt (Experimental)

Create new files to attempt `const fn` versions of the decoders:
- [`src/cpu/decoder/const_fetchdecode64.rs`](src/cpu/decoder/const_fetchdecode64.rs)
- [`src/cpu/decoder/const_fetchdecode32.rs`](src/cpu/decoder/const_fetchdecode32.rs)

**Challenges (likely blockers):**
- Rust `const fn` cannot use mutable references (`&mut BxInstructionGenerated`)
- Function pointers in `DECODE*_DESCRIPTOR` arrays cannot be called in const context
- `?` operator not stable in const fn
- `tracing` macros are not const-compatible

**Approach:**
- Create a simplified const-compatible decoder that returns raw bytes/metadata without mutation
- If not achievable, delete the files and report limitations

---

## 2. Remove Undefined Cfg Features

Remove `cfg(feature = ...)` attributes for features NOT in [`Cargo.toml`](Cargo.toml):

| Feature | Location | Action |
|---------|----------|--------|
| `3dnow` | `fetchdecode64.rs:538,576` | Remove cfg, keep both implementations merged |
| `avx` | `fetchdecode64.rs:620,633,648,661,676,689` | Remove cfg, always compile AVX code |
| `x86_64` | `fetchdecode32.rs:3132,3175` | Remove cfg, always compile 64-bit handling |
| `evex` | `fetchdecode32.rs:3450` | Remove cfg, always compile EVEX code |

Also update [`src/cpu/decoder/instr_generated.rs`](src/cpu/decoder/instr_generated.rs) to remove similar cfg attributes.

---

## 3. Fill Unimplemented Decoder Logic

Based on C++ reference at `C:\Users\Aslan\rusty_box_cursor\cpp_orig\bochs\cpu\decoder\`:

### 3.1 VEX Decoder (`decoder_vex64`, `decoder_vex32`)
Implement full VEX prefix decoding (lines 764-887 in fetchdecode64.cc):
- Parse 2-byte (0xC5) and 3-byte (0xC4) VEX prefix
- Extract `rex_r`, `rex_x`, `rex_b`, `vex_w`, `vvv`, `vex_l`
- Use `BxOpcodeTableVEX` for opcode lookup
- Handle immediates and call `assign_srcs_avx`

### 3.2 EVEX Decoder (`decoder_evex64`, `decoder_evex32`)
Implement full EVEX prefix decoding (lines 889+ in fetchdecode64.cc):
- Parse 4-byte EVEX prefix (0x62)
- Extract EVEX-specific fields: opmask, evex_b, evex_z, VL/RC
- Use `BxOpcodeTableEVEX` for opcode lookup
- Handle compressed displacement encoding

### 3.3 XOP Decoder (`decoder_xop64`, `decoder_xop32`)
Implement XOP prefix decoding:
- Parse 3-byte XOP prefix (0x8F with specific modrm)
- Extract XOP-specific fields similar to VEX
- Use `BxOpcodeTableXOP` for opcode lookup

### 3.4 3DNow Decoder (`decoder64_3dnow`, `decoder32_3dnow`)
- Already mostly implemented, just needs cfg removal
- Uses `Bx3DNowOpcode` table with suffix byte

---

## File Changes Summary

| File | Changes |
|------|---------|
| `src/cpu/decoder/const_fetchdecode64.rs` | NEW - const fn attempt |
| `src/cpu/decoder/const_fetchdecode32.rs` | NEW - const fn attempt |
| `src/cpu/decoder/fetchdecode64.rs` | Remove cfg, implement VEX/EVEX/XOP |
| `src/cpu/decoder/fetchdecode32.rs` | Remove cfg, implement VEX/EVEX/XOP |
| `src/cpu/decoder/instr_generated.rs` | Remove cfg for avx/evex/x86_64 |
| `src/cpu/decoder/mod.rs` | Add const_ module exports |

---

## Execution Order

1. Remove all undefined cfg attributes (quick cleanup)
2. Implement VEX decoder based on C++ reference
3. Implement EVEX decoder based on C++ reference  
4. Implement XOP decoder based on C++ reference
5. Create const fn files and attempt rewrite
6. If const fn fails, delete const files and document limitations