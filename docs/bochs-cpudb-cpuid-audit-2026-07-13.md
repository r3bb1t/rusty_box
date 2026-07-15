# Bochs cpudb / CPUID-infrastructure bug audit (upstream @ 70da922c)

**Date:** 2026-07-13 · **Scope:** all 33 `cpu/cpudb/**` models + shared CPUID/MSR
infrastructure in upstream Bochs. **Method:** 7-agent parallel discovery
(Fable), then 4-cluster adversarial verification + synthesis (Opus 4.8, each
verifier instructed to *refute* before confirming, re-tracing current source and
cross-checking each model's own in-tree `.txt` reference dump).

Motivation: the leaf 0x15/0x16 frequency-lie bug (issue
[#791](https://github.com/bochs-emu/Bochs/issues/791) / PR
[#792](https://github.com/bochs-emu/Bochs/pull/792)) was one instance of a
general archetype — *hardcoded real-hardware CPUID dumps that mislead the guest
about the emulated machine, or contradict what Bochs actually emulates*. This
audit hunts the rest of that class. The 0x15/0x16 finding itself is excluded
(already fixed).

"Dump-contradiction" below = the code disagrees with the model's own shipped
`.txt` dump = bulletproof, no external spec needed.

---

## Verdict table

| ID | Model / area | Verdict | Severity | File-worthy |
|----|--------------|---------|----------|-------------|
| A11 | corei5 lynnfield/arrandale — GETSEC panic | CONFIRMED | guest-breaking (host DoS) | ✅ |
| F1 | amd_k6_2 — LSTAR/WHCR MSR collision | CONFIRMED | guest-breaking | ✅ |
| A4 | arrow_lake — leaf 7.1 EDX=0 hides ISA | CONFIRMED (dump) | guest-visible | ✅ |
| A5 | corei3_cnl — leaf 4 = skylake-x copy-paste | CONFIRMED (dump) | guest-visible | ✅ |
| A6 | tigerlake — leaf 0x18 TLB row-splice | CONFIRMED (dump) | guest-visible | ✅ |
| A9 | p2_klamath — APIC bit clear vs live LAPIC | CONFIRMED (dump) | guest-visible | ✅ |
| A10 | p4_prescott_celeron_336 — SSSE3 on NetBurst | CONFIRMED (dump) | guest-visible | ✅ |
| A7 | pentium — Pentium-MMX signature | CONFIRMED (dump) | guest-visible | ✅ |
| F2 | athlon_xp — wrong F/M/S | CONFIRMED (dump) | guest-visible | ✅ |
| A3 | ryzen/zambezi/trinity — leaf 0x8000001E constant topology | CONFIRMED (dump) | guest-visible | ✅ |
| B7 | ryzen/zambezi/trinity/phenom/turion — 0x80000008 NC | CONFIRMED (dump) | guest-visible | ✅ |
| A1 | shared — leaf 0xB per-level vs cumulative shift | **CONFIRMED (SDM-verified)** | guest-visible | ✅ |
| A2 | shared — XSAVES 0xD.1:EBX omits 64B align | **CONFIRMED (SDM + real-HW verified)** | guest-breaking | ✅ |
| A15 | sapphire_rapids — AMX without XFD | **CONFIRMED (Linux-compat gap, SDM-verified)** | guest-visible | ✅ |
| A12 | ryzen — SME/SEV advertised, no MSRs | PARTIAL (masked by default knob) | guest-visible | ❌ |
| A16 | arrow_lake — hybrid VFM, non-hybrid enum | PARTIAL (audit chain inverted) | cosmetic | ❌ |
| A13 | config.cc — cpuid_limit_winnt doc says "3" | CONFIRMED | cosmetic | ❌ (fold into a PR) |
| A14 | msr.cc — ARCH_CAPABILITIES sets RSBA | NEEDS_EXTERNAL_SPEC | cosmetic | ❌ |
| B1 | Intel models — leaf 0xA PMU lie | CONFIRMED (systemic) | guest-visible | ✅ (one issue, #792-style) |

Two overclaims were caught and corrected by the adversarial pass, which is
evidence the verification worked rather than rubber-stamped:
- **A5** — the original audit named the divergent SL2 field as ECX; it is
  actually **EBX** (ways). SL2 ECX matches the dump. SL3 divergences stand.
- **A16 / A12** — downgraded to non-bugs (see §3).

---

## 1. Ready to file — CONFIRMED, code-settled, no external spec

### Guest-breaking (behavioral)

**A11 — GETSEC on SMX-enabled Core-i5 aborts the emulator (guest-triggerable host DoS)**
`corei5_lynnfield_750.cc` + `corei5_arrandale_m520.cc` ctors both call
`enable_cpu_extension(BX_ISA_SMX)`. `BX_ISA_SMX` gates (1) `GETSEC` decode
(`decoder/ia_opcodes.def`) and (2) the `CR4.SMXE` allow-mask (`crregs.cc`) —
**not** the CPUID SMX bit (deliberately unset). A guest ring-0 sequence
`MOV CR4 (set SMXE); GETSEC (0F 37)` reaches
`BX_PANIC("GETSEC: SMX is not implemented yet !")` in `vmx.cc::GETSEC` → fatal
abort of the whole emulator. Four-sided trace verified; no knob masks it.
*Fix:* in `vmx.cc::GETSEC` replace `BX_PANIC` with
`exception(BX_UD_EXCEPTION, 0)` (protects any future SMX model), or drop the two
`enable_cpu_extension(BX_ISA_SMX)` calls. Prefer the handler fix.

**F1 — LSTAR/WHCR MSR-encoding collision → unconditional #GP on AMD K6-2**
`msr.h`: `BX_MSR_LSTAR == 0xC0000082 == MSR_K6_WHCR`. In `msr.cc::wrmsr` the
typed `case BX_MSR_LSTAR:` (under `#if BX_SUPPORT_X86_64`) returns `false` when
`!BX_ISA_LONG_MODE`; `WRMSR` turns that into `exception(BX_GP,0)` **without**
reaching `handle_unknown_wrmsr`, so `ignore_bad_msrs` never applies.
`amd_k6_2_chomper.cc` has no long-mode ISA, so a legal K6 write-allocate MSR
write hard-#GPs on the shipping full-cpudb (x86-64) build. Linux `init_amd_k6`
issues exactly this write on family-5/model-8.
*Fix:* route the long-mode-gated high-MSR cases (STAR/LSTAR/CSTAR/FMASK) to
`handle_unknown_{wr,rd}msr` when the feature is absent, so `ignore_bad_msrs`
governs instead of a hard fault.

### Guest-visible — dump-contradictions (bulletproof)

**A4 — Arrow Lake hides AVX-VNNI-INT8/INT16/NE-CONVERT that the decoder executes**
`arrow_lake.cc::get_std_cpuid_leaf_7` case 1 hardcodes `leaf->edx = 0`, but the
ctor enables `BX_ISA_AVX_VNNI_INT8/INT16/AVX_NE_CONVERT`; the generic helper
would emit `0x430`. Dump `arrow_lake.txt` (7,1).EDX = `0x00040430`. Features
execute yet are unadvertised → feature-detecting guests (glibc, libcrypto,
oneDNN) skip working paths.
*Fix:* `leaf->edx = get_std_cpuid_leaf_7_subleaf_1_edx();` (optionally OR
CET_SSS bit18 — a separate generic-helper gap — to fully match the dump).

**A5 — Cannon Lake (CNL) leaf 4 cache descriptors copy-pasted from Skylake-X**
`corei3_cnl.cc::get_std_cpuid_leaf_4` case2/3 byte-identical to `skylake-x.cc`.
Vs `corei3_cnl.txt`: SL2 **EBX** `0x03c0003f`(code, 16-way) vs `0x00c0003f`
(dump, 4-way); SL3 EBX `0x0280003f`→`0x03c0003f`, ECX `0x2fff`→`0x0fff`, EDX
`0x4`→`0x6`. Corrupts reported L2 size and L3 ways/inclusivity.
*Fix:* case2 `ebx=0x00c0003f`; case3 `ebx=0x03c0003f, ecx=0x00000fff, edx=0x00000006`.

**A6 — Tiger Lake leaf 0x18 (TLB) row-splice + missing subleaf 8**
`tigerlake.cc::get_std_cpuid_leaf_18` case6 = SL06.EBX spliced with SL07.ECX/EDX;
the real SL07 descriptor `0x00080007` is dropped; case0 EAX=8 advertises subleaf
8 but the switch stops at case7, so subleaf 8 returns all-zero.
`sapphire_rapids.cc` handles the same family correctly (house-style ruled out).
*Fix:* case6 `ecx=0x1, edx=0x4124`; case7 `ebx=0x00080007, ecx=0x80, edx=0x4043`;
add case8 `ebx=0x00080009, ecx=0x80, edx=0x4043`.

**A9 — Pentium II Klamath: APIC bit clear despite live LAPIC + dump asserting it**
`p2_klamath.cc` ctor omits `enable_cpu_extension(BX_ISA_XAPIC)`; `cpuid.cc`
gates EDX[9] on it → guest EDX `0x0080F9FF`. Dump `p2_klamath.txt` EDX
`0x0080FBFF` (bit9 **set**), the leaf-1 comment stars APIC, and `init.cc` reset
enables the LAPIC (MMIO-live) regardless. The bit can never become 1. (Contrast
p3_katmai, which is legitimately APIC-less and self-consistent.) On SMP configs
NT/2000/Linux drop to uniprocessor.
*Fix:* add `enable_cpu_extension(BX_ISA_XAPIC);` to the ctor.

**A10 — P4 Prescott Celeron 336 advertises + decodes SSSE3 the part never had**
`p4_prescott_celeron_336.cc` ctor calls `enable_cpu_extension(BX_ISA_SSSE3)` →
leaf1 ECX bit9=1 and SSSE3 opcodes execute. Dump ECX `0x0000651D` (bit9
**clear**). No NetBurst part shipped SSSE3.
*Fix:* remove the `enable_cpu_extension(BX_ISA_SSSE3);` call.

**A7 — Plain Pentium reports the Pentium-MMX model signature**
`pentium.cc` leaf1 EAX `0x00000543` (family5/**model4**=P55C) — identical to
`pentium_mmx.cc` — yet the ctor omits `BX_ISA_MMX`, so EDX[23]=0: an MMX-model
signature with MMX off. Dump `pentium.txt` (P54C) EAX `0x00000525` (model2).
*Fix:* `leaf->eax = 0x00000525`.

**F2 — Athlon XP wrong family/model/stepping**
`athlon_xp.cc` leaf1 EAX `0x00000622`, ext-leaf1 EAX `0x00000722` (the ext
comment even claims "same as 0x1.EAX" yet differs). Dump `athlon_xp.txt`
(Thoroughbred-A "XP 2200+") = `0x00000680` / `0x00000780`; brand, caches and SSE
all match `0x680`. Pure transcription.
*Fix:* `0x00000680` (std) and `0x00000780` (ext).

**A3 — AMD leaf 0x8000001E emits constants instead of per-CPU topology**
`ryzen/zambezi/trinity_apu::get_ext_cpuid_leaf_1E` share a byte-identical
constant body `ebx=(ncores-1)<<8, eax=ecx=edx=0` — although the same file's
leaf1 already reads `cpu->get_apic_id()`. Dump `ryzen.txt`: EAX walks
`0x0..0xF` (unique ExtApicId), EBX `0x100..0x107` (CoreId 0-7, ThreadsPerCore-1
= 1). Bochs pins CoreId=0 on every vCPU and mis-stuffs `ncores-1` into the
threads field. TOPOEXT is set so guests consult it → all CPUs become SMT
siblings of one core; broken AMD SMP topology.
*Fix:* derive `eax=get_apic_id()`, EBX[7:0]=core id, EBX[15:8]=nthreads-1 from
the per-CPU APIC id (mirror leaf1).

**B7 — AMD leaf 0x80000008 ECX reports physical (ncores-1), not logical**
`ryzen.cc::get_ext_cpuid_leaf_8` overwrites `leaf->ecx = ncores-1`, discarding
nthreads and zeroing ApicIdSize[15:12]. Dump `ryzen.txt` ECX `0x0000400F`
(NC=15=logical-1, ApicIdSize=4). Leaf1 EBX[23:16]=ncores*nthreads confirms NC
counts logical. Also affects zambezi/trinity/phenom/turion; diverges whenever
nthreads>1.
*Fix:* `ecx = (ncores*nthreads - 1)` for NC[7:0]; ApicIdSize[15:12] =
`ceil_log2(ncores*nthreads)`.

---

## 2. SDM-verified (2026-07-13) — ready to file

All three were checked against the Intel SDM (primary + felixcloutier/sandpile/
geoffchappell mirrors), and A2/A15 against real Sapphire Rapids CPUID dumps
(InstLatx64) and the actual Linux source. Verdicts and exact wording below.

**A1 — leaf 0x0B per-level vs cumulative x2APIC shift — CONFIRMED on all 3 counts**
`cpuid.cc::get_std_cpuid_extended_topology_leaf` sets each subleaf EAX to a
**per-level** width, and `ilog2(0)=0` makes `nthreads==1` emit subleaf0 EAX=1
(should be 0). `apic.cc` BX_PANICs on `apic_id>=bx_cpu_count`, forcing dense
0..N-1 IDs. SDM verdict (HIGH confidence):
- **Cumulative shift (SDM_CONFIRMS_BUG):** SDM defines EAX as bits to shift the
  x2APIC ID right to reach the *next* level type — taken against the full ID, so
  it **accumulates**. Intel's own Clarkdale worked example (2c×2t) shows
  subleaf0 EAX=1, subleaf1 EAX=4 (not the per-level 1). For 4c×2t dense IDs the
  spec requires 3; Bochs computes 2. The model's own `2600K.txt` confirms:
  cumulative EAX (`0x1/0x4`) + **strided** x2APIC EDX (0,2,4,6). Airtight.
- **Level type 3 (SDM_CONFIRMS_BUG):** in leaf 0BH, ECX[15:8]=3 is *Reserved*
  (only 0=Invalid/1=SMT/2=Core valid); Module=3 exists only in leaf 1FH, and 0BH
  does not enumerate a package level at all. Bochs's type-3 subleaf is
  non-conformant on two counts.
- **1-wide level (SDM_CONFIRMS_BUG, MED-HIGH):** EAX equals the sub-field
  bit-width, which is 0 for a single-processor level; Bochs reports 1.
*Wording:* file as "CPUID.0BH EAX shift counts are per-level, not cumulative; a
Reserved/1FH-only level type (3) is emitted; 1-wide levels report width 1 not
0" — non-conformant with the SDM Vol 2 CPUID 0BH definition. Do **not** claim
catastrophic guest breakage (topology-parsing OSes misread core/socket counts).
*Fix:* cumulative running shift + a `ceil_log2` returning 0 for x≤1.

**A2 — XSAVES compacted size (CPUID.0xD.1:EBX) omits 64-byte alignment — CONFIRMED (SDM + real HW)**
`cpuid.cc::xsave_max_size_required_by_xsaves_features` sums component `len` with
**no** alignment and feeds subleaf1 EBX, while the XSAVES write path
(`xsave.cc xsave_offset_align64_if_needed`, and `xsave_area_last_byte(compaction=
true)`) rounds aligned components up to 64B. SDM verdict (VERY HIGH):
- SDM §13.4.3 / CPUID ECX[1]: an aligned component sits at the next 64-byte
  boundary in compacted format. CPUID.(0DH,1):EBX = the size XSAVES actually
  writes, so it **must** include that padding. A size omitting it under-reports
  the footprint → SDM violation.
- **Real Sapphire Rapids (InstLatx64):** XTILECFG at offset 2752 (=43×64) after
  PKRU ends at 2696 — a visible 56-byte alignment gap; the aggregate
  subleaf-1 EBX = 10880 **includes** the padding. Bochs's helper omits it.
- **CORRECTION to the discovery finding:** the earlier claim that "only
  XTILEDATA should be aligned" is **wrong** — real HW sets the align bit on
  *both* XTILECFG (ECX=0x2) and XTILEDATA (ECX=0x6), and Bochs correctly marks
  both `BX_XSAVE_ALIGN64`. The alignment attrs are right; **do not touch them.**
  The sole defect is the size accumulator.
*Wording:* "CPUID.(0Dh,ECX=1):EBX under-reports the compacted XSAVES size by the
64-byte alignment padding (0–56 bytes) whenever an aligned component (AMX
XTILECFG/XTILEDATA) is enabled behind non-64-aligned components; XSAVES then
writes past a guest buffer sized from that EBX." Present the overrun as a
consequence for a guest that trusts subleaf-1 EBX (a guest that sums the
per-component subleaves gets the right size — those offsets are correct).
*Fix:* make the size helper mirror `xsave_area_last_byte(compaction=true)`
(apply `xsave_offset_align64_if_needed` per component) — size helper only.
*Note:* QEMU-TCG's `xsave_area_size(compacted)` shares the same alignment
omission (its `ESA_FEATURE_ALIGN64_MASK` is unused); its write path wasn't
verified, so make no claim about QEMU. The anchor is: real Intel HW includes the
padding, so Bochs diverges from hardware regardless.

**A15 — Sapphire Rapids AMX with no XFD — CONFIRMED as a Linux-compatibility gap**
AMX enabled (`BX_ISA_AMX/AMX_INT8/AMX_BF16`, leaves 0x1D/0x1E, XCR0 17:18) but
the generic xsave leaf never sets (0xD,1).EAX bit4 (XFD), and `IA32_XFD`/
`IA32_XFD_ERROR` (0x1c4/0x1c5) are enum-only — no MSR handler. Verified:
- **Code fact (settled locally):** both AMX components carry `attr =
  BX_XSAVE_ALIGN64` (0x2) only; `BX_XSAVE_XFD_SUPPORT = 0x4` is defined but never
  set, so Bochs returns CPUID.(0xD,18):ECX[2] = **0**. Bochs is therefore
  *internally self-consistent* (no XFD anywhere) — this is **not** a
  contradictory-CPUID spec violation.
- **SDM:** a real AMX processor *must* enumerate CPUID.(0xD,18):ECX[2]=1 (XFD-
  managed) and (0xD,1):EAX[4]=1; there is no explicit "AMX requires XFD" MUST
  sentence, but no shipping/conforming AMX CPU omits XFD. Real SPR dump confirms
  ECX[2]=1. Bochs diverges from that mandated value.
- **Linux (provable):** `fpu__init_system_xstate()` in
  `arch/x86/kernel/fpu/xstate.c` does `if (!cpu_feature_enabled(X86_FEATURE_XFD))
  max_features &= ~XFEATURE_MASK_USER_DYNAMIC`, and `XFEATURE_MASK_USER_DYNAMIC`
  *is* XTILEDATA — so with XFD absent, Linux ≥5.16 (merged Dec 2021) strips
  XTILEDATA, `ARCH_REQ_XCOMP_PERM` fails, and AMX is unreachable from userspace.
*Wording:* file as a **Linux-compatibility bug**, not a bare spec violation:
"sapphire_rapids advertises AMX but leaves CPUID.(0Dh,1):EAX[4]=0 and (0Dh,18):
ECX[2]=0 and implements neither IA32_XFD nor IA32_XFD_ERR; Linux ≥5.16 clears
XFEATURE_MASK_XTILE_DATA in fpu__init_system_xstate(), so AMX is unreachable in
any current Linux guest despite Bochs implementing the instructions."
*Fix:* set (0xD,1):EAX[4]=1 and (0xD,18):ECX[2]=1 when AMX is enabled, and
implement IA32_XFD / IA32_XFD_ERR with #NM-on-XFD semantics (SDM §3.2.6).

---

## 3. Refuted / downgraded (do NOT file)

- **A12 (PARTIAL)** — Ryzen leaf 0x8000001F advertises SME/SEV but SYSCFG/
  SEV_STATUS MSRs are absent. Unlike F1 these take the `handle_unknown_rdmsr`
  **soft** path → default `ignore_bad_msrs=1` returns 0, SME stays inactive,
  boot proceeds. Only hard-#GPs under explicit `ignore_bad_msrs=0`. Masked by
  default; not a distinct bug.
- **A16 (PARTIAL)** — Arrow Lake real VFM/brand paired with non-hybrid
  enumeration. The original audit's failure chain is **inverted**: Linux reads
  CPUID 0x1A only when `X86_FEATURE_HYBRID_CPU` (leaf7.EDX[15]) is set, which it
  is not here, so "Unknown hybrid CPU type" never fires. A self-consistent
  modeling simplification, not a bug.
- **A13 (cosmetic)** — `config.cc` `cpuid_limit_winnt` label says "limit to 3";
  code clamps to 2 (1 on Ryzen), and `.bochsrc`/man page say 2. Doc-string typo
  only; fold into any adjacent PR, don't file standalone.
- **A14 (cosmetic / needs-spec)** — `IA32_ARCH_CAPABILITIES=0x1F` sets RSBA
  (bit2) alongside the "not vulnerable" bits. Over-mitigates (conservative
  direction), never unsafe. Only file if SDM confirms RSBA=1 is semantically
  contradictory with a fully-mitigated vector; then `0x1F→0x1B`. Low priority.

---

## 4. Tier-B systemic class — "CPUID advertises, MSR absent"

Do **not** file the whole class as individual bugs; most is the accepted
`ignore_bad_msrs` posture. Split:

- **File one distinct bug: the PMU / leaf 0xA gap (B1).** Intel models hardcode
  a nonzero leaf 0xA (arch PMU v3–v6, GP+fixed counters) + PDCM, but IA32_PMCx /
  fixed 0x309–0x30B / GLOBAL_* 0x38D–0x38F / PERF_CAPABILITIES 0x345 are
  unimplemented (models even `BX_INFO "not implemented"` at read time). Linux
  `check_hw_exists` actively write-then-readback probes, fails, and prints
  **"Broken PMU hardware detected, using software events only"**, disabling perf
  and the NMI watchdog — a real guest-visible dmesg regression, *not* fully
  papered over by `ignore_bad_msrs`. Frame it exactly like PR #792: make the
  declaration consistent — zero leaf 0xA and drop PDCM (leaf1 ECX bit15) so the
  unbacked feature is uniformly not advertised.
- **Group the rest as one "advertise-without-backing" tracking issue, not
  bugs:** leaf6 HWP/HDC (0x770–7, 0xDB0–2), EST/thermal (0x198–0x19C, 0x1A0),
  IA32_CORE_CAPABILITIES (0xCF), arrow_lake HFI/ITD (0x17D0/1), AMD TCE/IBS/OSVW/
  PowerNow. All read-0/drop-write under default `ignore_bad_msrs=1`; no observed
  guest breakage. Same "declare-the-gap" remedy, low priority.

---

## 5. Recommended first filing

**Open A4 (Arrow Lake leaf 7.1 EDX=0) first.** It is the closest analog to
#791/#792: a single-line data defect that **contradicts the model's own in-tree
dump** (`arrow_lake.txt` (7,1).EDX=`0x00040430`), one-line fix
(`leaf->edx = get_std_cpuid_leaf_7_subleaf_1_edx();`), concrete guest-visible
impact (three ISA extensions enabled + executed by the decoder yet reported
absent). Bulletproof, high-value, minimal-diff, no spec dependency — the ideal
pattern-setter.

Then batch the remaining dump-contradictions as a companion series, sequenced by
blast radius: **A5, A6** (modern Intel cache/TLB) → **A3, B7** (AMD topology,
several models) → **A9, A10, A7, F2** (legacy single-model constants). Track the
two guest-breaking behavioral fixes (**A11, F1**) as a **separate higher-severity
PR** — their fixes are handler/design changes, not cpudb constants, and warrant
independent review.

---

*Raw pre-verification inventory (all findings incl. Tier-C cosmetics):
`scratchpad/cpudb-audit-raw-findings.md` in the session working dir.*
