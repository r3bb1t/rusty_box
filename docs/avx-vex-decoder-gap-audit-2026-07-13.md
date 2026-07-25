# AVX/VEX decoder gap audit — Windows 10 install debugging (2026-07-13)

Triggered by a Windows 10 setup `#UD`/DECODE-FAIL on `c4 e3 7d 19 d8 01`
(`VEXTRACTF128 xmm0, ymm3, 1`). Full sweep of the three VEX opcode maps
(`0F`, `0F38`, `0F3A`) comparing Bochs `BxOpcodeTableVEX` against the Rust
merged decode tables + `remap_sse_to_vex`.

## Decoder architecture (for future audits)

- Rust merges VEX + legacy into the shared 3-byte tables (`opmap.rs`,
  `opmap_0f38.rs`, `opmap_0f3a.rs`). VEX-ness is NOT masked by SSE-prefix
  entries, so a VEX instruction matches a legacy `SSE_PREFIX_66` entry unless
  a more-specific `VEX`-flagged entry precedes it (`find_opcode_in_table`
  returns the FIRST match).
- After table lookup, `remap_sse_to_vex(op, vl)` (decode64.rs) rewrites the SSE
  opcode into the proper 128/256-bit VEX opcode for 3-operand handlers. Called
  once for every VEX (non-EVEX) instruction, ALL maps. **decode64 only** —
  decode32 does not remap (pre-existing 32-bit-VEX limitation).
- Handlers dispatch on the pre-typed `Opcode` and read raw `instr` operands.
  `instr.src2()` ALWAYS holds `vvvv` for VEX instrs (decode64.rs:826), even when
  the opcode's operand descriptors lack an explicit H operand.

## FIXED this session

| enc | insn | fix |
|-----|------|-----|
| 0F3A 19 | VEXTRACTF128 | wired `BxOpcodeTable0F3A19` + dispatcher → `vextracti128` (shared handler, Bochs parity) |
| 0F C4 | VPINSRW | `remap_sse_to_vex` arm + new `vpinsrb/w/d/q` handlers |
| 0F3A 20 | VPINSRB | remap arm + handler |
| 0F3A 22 W0/W1 | VPINSRD / VPINSRQ | remap arms + handlers (W split via existing OS64 match) |

The VPINSR family previously mis-decoded as 2-operand SSE `PINSR*` (dropped
`vvvv`, reused dst as base, didn't clear upper YMM) → **silent data corruption**.
Tests: `test_vex_pinsr_family_decode` (decode), `vex_vpinsrw_sources_vvvv_and_clears_upper` (exec).

The ymm-memcpy family Windows setup hammers (VMOVDQU/VMOVUPS ymm, VINSERTF128,
VEXTRACTF128, VZEROUPPER) is now confirmed complete end-to-end. The 2-byte VEX
`0F` map has ZERO fully-ERR bytes.

## DEFERRED backlog (needs new handlers OR is unreachable by Intel Win10 guest)

Every gap below already has a Rust `Opcode` enum variant; the miss is decode
wiring AND (for the fully-ERR ones) a missing non-EVEX handler (only EVEX
siblings exist in the dispatcher).

### AVX/AVX2/F16C — plausibly reachable, NEED NEW VEX HANDLERS
- **0F38**: `0C/0D` VPERMILPS/PD, `0E/0F` VTESTPS/PD, `13` VCVTPH2PS, `16` VPERMPS,
  `2C-2F` VMASKMOVPS/PD, `45-47` VPSRLVD/Q·VPSRAVD·VPSLLVD/Q, `8C/8E` VMASKMOVD/Q,
  `90-93` VGATHER*.
- **0F3A**: `01` VPERMPD, `04/05` VPERMILPS/PD-imm, `1D` VCVTPS2PH.

### AVX1 PARTIAL mis-decodes (VEX form → wrong operands, legacy handler exists)
- **0F3A**: `0A/0B` VROUNDSS/SD (VEX is 3-operand!), `0C/0D` VBLENDPS/PD,
  `40/41` VDPPS/PD, `42` VMPSADBW-256, `44` VPCLMULQDQ-256, `14-17/20-22`
  VPEXTR*/VINSERTPS/VPINSR* (VPINSR now fixed), `60-63` VPCMP*STR*.
- **0F (strictness only, correct at VL128)**: `13/17` VMOVLPS/HPS-store,
  `2C-2F` VCVT*2SI/VUCOMISS/VCOMISS, `6E/7E` VMOVD/Q, `AE` VLDMXCSR/VSTMXCSR,
  `C5` VPEXTRW, `F7` VMASKMOVDQU — missing VL256/`vvvv!=1111` `#UD` only.

### Won't be emitted by an Intel-CPUID (Skylake-X) guest
- AMD **XOP** (0F3A 48/49) and **FMA4** (0F3A 5C-7F): ~24 bytes.
- Newer ISAs: VNNI/IFMA/NE-CONVERT/VNNI-INT16/SM3/SM4/CMPccXADD (0F38 50-53,
  72, B0/B1, B4/B5, D2/D3, DA, E0-EF), SHA512 (0F38 CB-CD), SM3 (0F3A DE).
- AMX (0F38 49/4A/4B/5C/5E/6C): gated off in default build (matches Bochs).

### Other real bugs found (not Windows-relevant, but genuine Bochs divergences)
- **AVX-512 KSHIFT (0F3A 30-33)**: W-bit/opcode mapping scrambled vs Bochs.
  Rust groups L/R by W and steps element size across bytes; Bochs = 30/31
  KSHIFTR(b/w,d/q), 32/33 KSHIFTL(b/w,d/q). AVX-512-only.
- Cosmetic: opmap_0f3a.rs `/* 0F 3A 64 */` comment on the 0x63 (PCMPISTRI) slot.

## Recommendation

Re-run Windows setup to get GROUND TRUTH on the next actual crash rather than
implementing the AVX2 handler backlog speculatively — most of it (gather,
permute, var-shift, mask-move) is not on the setup hot path. Prioritize by real
DECODE-FAIL captures.

---

## RE-VERIFICATION 2026-07-25 — this backlog is ~half stale

Checked every entry above against the current tree (`feb28c5`). Method: count
references in `rusty_box_decoder/src/decoder/decode64.rs` (remap arms / decode
wiring) and in `rusty_box/src/cpu/` (handlers).

### Already DONE since the audit — do NOT re-implement

The entire "AVX1 PARTIAL mis-decodes" section is resolved: `VROUNDSS/SD`,
`VROUNDPS/PD`, `VBLENDPS/PD`, `VBLENDVPS`, `VDPPS/PD`, `VINSERTPS` and
`VMPSADBW` all have `remap_sse_to_vex` arms today.

From the "NEED NEW VEX HANDLERS" list, these are also wired with handlers:
`VPERMILPS/PD`, `VPERMPS`, `VPERMPD`, `VPSRLVD/Q`, `VPSRAVD`, `VPSLLVD/Q`.

### GENUINELY STILL OPEN — zero decode wiring AND zero handler

| enc | insn | note |
|---|---|---|
| 0F38 0E/0F | `VTESTPS` / `VTESTPD` | small: sets ZF/CF from sign bits, no dst write. Bochs `avx/avx_pfp.cc VTESTPS_VpsWpsR` — ~15 lines each. Best next candidate. |
| 0F38 13 | `VCVTPH2PS` | F16C |
| 0F3A 1D | `VCVTPS2PH` | F16C |
| 0F38 2C-2F | `VMASKMOVPS/PD` | masked load/store, faulting semantics |
| 0F38 8C/8E | `VMASKMOVD/Q` | as above |
| 0F38 90-93 | `VGATHER*` | large: fault-suppression + mask update per element |

Also still absent from `decode64.rs`: `VPCLMULQDQ`, `VPCMPISTRI/M`,
`VPEXTRB/W/D`. These were listed under "PARTIAL mis-decodes", so they may be
decoding through a legacy SSE entry and losing VEX semantics — **verify before
assuming they are merely missing**, since that failure mode is silent data
corruption (the same class as the VPINSR bug this audit originally found).

### Caveat

This re-verification is reference-count based, not behavioural. A non-zero
count proves wiring exists, not that it is Bochs-correct. Treat "DONE" as
"stop looking here first", not as "verified equivalent".
