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

## HEADLINE FINDING 2026-07-25 (FIXED) — CPUID advertised AVX/AVX2/F16C while 15 opcode-map slots raised #UD

This supersedes everything below it in priority. `Corei7SkylakeX` advertises
**AVX2** (`CpuIdStd7Ebx::AVX2`, cpu/cpudb/intel/core_i7_skylake.rs) and **F16C**
(`CpuIdStd1Ecx::AVX_F16C`), yet these slots in `opmap_0f38.rs` / `opmap_0f3a.rs`
were `&BX_OPCODE_GROUP_ERR` — a guest that dispatched on CPUID took a `#UD` on a
CPU that had told it the feature was present:

| Encoding | Instruction | ISA |
|---|---|---|
| `VEX.66.0F38 0C/0D`, `0F3A 04/05` | VPERMILPS / VPERMILPD (variable + imm8) | AVX |
| `VEX.66.0F38 0E/0F` | VTESTPS / VTESTPD | AVX |
| `VEX.66.0F38 2C-2F` | VMASKMOVPS / VMASKMOVPD | AVX |
| `VEX.66.0F38 13`, `0F3A 1D` | VCVTPH2PS / VCVTPS2PH | F16C |
| `VEX.66.0F38 16`, `0F3A 01` | V256 VPERMPS / VPERMPD | AVX2 |
| `VEX.66.0F38 45/46/47` | VPSRLVD/Q, VPSRAVD, VPSLLVD/Q | AVX2 |
| `VEX.66.0F38 8C/8E` | VPMASKMOVD / VPMASKMOVQ | AVX2 |
| `VEX.66.0F38 90-93` | VGATHER / VPGATHER (8 opcodes) | AVX2 |

31 opcodes across 15 slots, all now implemented with decode and execution
tests, including the two parity-critical fault behaviours: masked moves must
not fault on masked-off elements, and a gather must leave a restartable mask
after a mid-instruction `#PF`.

**Scoping rule for the rest of the map.** A full sweep found 48 further slots
still `&BX_OPCODE_GROUP_ERR`: FMA4 (40 opcodes), CMPCCXADD (30), AMX, AVX-VNNI
and VNNI-INT8/INT16, AVX-IFMA, AVX-NE-CONVERT, XOP, SM3, SM4. `Corei7SkylakeX`
advertises none of those, so Bochs decodes them and then raises `#UD` on the
absent ISA bit while rusty_box raises `#UD` at decode — **identical observable
behaviour**. They become real divergences only if a cpudb model that advertises
them is added.

## CORRECTION 2026-07-25 — the earlier "VPCLMULQDQ silently corrupts" claim was WRONG

An earlier revision of this document asserted that `VEX.66.0F3A.44` decodes to
the legacy `PclmulqdqVdqWdqIb` and therefore drops `vvvv`, reuses the
destination as src1, and leaves the upper YMM lane dirty. **The decode half is
right; the consequence is not.** The legacy handler `pclmulqdq_vdq_wdq_ib`
(`rusty_box/src/cpu/aes.rs`) already branches on `instr.is_vex()` and reads
`instr.src2()` (vvvv) as op1, and `write_xmm_result` in the same file already
zeroes the upper lane for VEX encodings. The 128-bit VEX form produces correct
results today.

The real 0F3A44 divergence is narrower: Bochs `BxOpcodeGroup_VEX_0F3A44` splits
on VL into `V128_VPCLMULQDQ_VdqHdqWdqIb` and `V256_VPCLMULQDQ_VdqHdqWdqIb`,
while rusty_box has only the one legacy entry — so `VEX.256` runs the 128-bit
operation instead of the per-lane 256-bit one. That form needs the VPCLMULQDQ
CPUID feature, which `Corei7SkylakeX` does not have (Icelake+), so no guest on
this model can reach it; the correct behaviour there is `#UD`. **Still open.**

## Also open — VEX encodings that should `#UD` but do not

`VPEXTRB/W/D/Q` (0F3A 14-16) and `VPCMPESTRM/ESTRI/ISTRM/ISTRI` (0F3A 60-63)
carry only legacy `SSE_PREFIX_66` table entries, so a VEX prefix falls through
to the legacy opcode. Unlike VPINSR, these forms have **no `vvvv` source**, and
the legacy handlers read the same operands the VEX forms specify — so the
computed results are correct. What is missing is fault behaviour: Bochs marks
all of these `ATTR_VL128`, so `VEX.256` must `#UD`, and Intel reserves every
encoding with `VEX.vvvv != 1111b`. rusty_box currently accepts both.

Fix shape: `remap_sse_to_vex` arms returning `IaError` for `vl != 0` (the
`PinsrwVdqEwIb` arm in decode64.rs is the model), plus entries in
`validate_reserved_vex_vvvv`. The `V128Vpextr*` / `V128Vpcmp*` opcode variants
and their typed.rs arms already exist.

## 2026-07-25 (later) — remaining VEX fall-throughs closed, ISA gate landed

The audit's remaining items are done. What the systematic sweep turned up beyond
the original backlog:

**Silent data corruption (the VPINSR class, wrong results with no fault):**
`VMOVD` / `VMOVQ` to xmm and `VPCMPISTRM` / `VPCMPESTRM` wrote through
`write_xmm_reg_lo128`, which *preserves* bits [255:128]. Bochs uses
`BX_WRITE_XMM_REGZ`, which preserves for legacy SSE but clears for VEX. Every
`VMOVD xmm, eax` leaked stale YMM data — and VMOVD/VMOVQ are far more common
than VPINSR. Fixed with a shared `write_xmm_regz` helper (`cpu/xmm.rs`).

**Instructions that did not exist at all:** `MOVNTDQA` (legacy SSE4.1 *and*
VEX) and `PEXTRW r/m16, xmm, imm8` (66 0F3A 15) had no dispatcher arm, so both
raised #UD where Bochs implements them.

**Encoding limits:** `VPEXTRB/W/D/Q`, `VPCMPxSTRx`, `VMOVLPS/VMOVHPS` stores,
`VMOVD/VMOVQ`, `VPEXTRW` (0F C5), `VMASKMOVDQU`, `VLDMXCSR`/`VSTMXCSR` now
enforce Bochs's `ATTR_VL128` / `ATTR_MODC0` / `ATTR_MOD_MEM` and reserved
`VEX.vvvv`. 0F AE additionally rejects every non-MXCSR nnn under a VEX prefix.
Note 0F 2C/2D/2E/2F carry **no** VL attribute in Bochs, so `VEX.256`
`VUCOMISS`/`VCVTTSS2SI` stay legal — only `vvvv` is reserved.

**`VPCLMULQDQ`** now decodes to its 3-operand VEX form with a real per-lane
handler; the 256-bit form is gated on the VPCLMULQDQ CPUID feature, which
Corei7SkylakeX lacks, so it correctly #UDs on this model.

**Per-instruction ISA gate** (`rusty_box_decoder/src/opcode_isa.rs`, generated
by `scripts/gen_opcode_isa.py`): 2900 of 3677 opcodes now carry Bochs's CPUID
feature requirement, applied at icache fill. On Corei7SkylakeX this turns 1052
opcodes into #UD — every one a feature the model does not advertise. Notably
that includes the ~400 EVEX AVX-512 opcodes rusty_box has no handler for: they
previously reached the dispatcher catch-all and produced an *emulator-level*
`UnimplementedOpcode` error (the host stops) instead of a guest #UD.

**`#AC`** is now raised on misaligned user-privilege word/dword/qword accesses
(`cpu/access.rs check_alignment`); `alignment_check_mask` was previously stored
and snapshotted but never consulted.
