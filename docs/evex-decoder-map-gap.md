# The EVEX decoder map gap, and what it was hiding

**Opened 2026-08-01**, when advertising AVX-512 on Skylake-X panicked an
Ubuntu 26.04 guest. **Closed the same day.** An earlier revision of this file
concluded the CPUID flip could not land; that conclusion is superseded — the
maps are generated and the flip is in place. What follows is the record of
both defects, because the second one is the more instructive.

## Defect 1 — opcode map slots that were never filled

With AVX512F/DQ/CD/BW/VL advertised, Ubuntu's kernel booted and then init
took #UD:

```
exc_invalid_op+0x5d/0x80
Attempted to kill init! exit code=0x00000004
Code: ... f3 0f 1e fa 89 f8 <62> e2 7d 28 7a c6 25 ff 0f 00 00 ...
```

The faulting byte is `62`, the EVEX prefix. `62 E2 7D 28 7A C6` is
`VPBROADCASTB ymm0, esi` (EVEX.256.66.0F38.W0 7A /r), emitted by glibc's
AVX-512 `strlen`/`memchr` IFUNC. `fetch_decode64` rejected it with
`BxIllegalOpcode`: the opcode-map slot was empty.

The handler was not missing. `evex_vpbroadcastb_gpr` existed and was
dispatched. Nothing in the decoder tables *produced* the opcode, so it never
reached the ISA gate or the dispatcher.

    EVEX opcodes in the Opcode enum      1333
    ...referenced by a decoder table       268   (before)
    ...referenced by a decoder table      1314   (after)

This corrects the standard the earlier work was held to. "All 898 Skylake-X
EVEX opcodes are dispatched" was measured enum-to-dispatcher; it was true and
insufficient. An opcode can have a correct, tested handler and remain
unreachable. Completeness has to be measured decoder-to-dispatcher.

Fixed by generating the maps from Bochs's own
`cpu/decoder/fetchdecode_opmap_evex.cc` — which *is* the table — so the Rust
is a transcription rather than a re-derivation. See
`scripts/gen_opmap_evex.py`.

Of the 19 opcodes still unreachable, 14 belong to groups Bochs defines but
never references from its own master table (`BxOpcodeGroup_EVEX_0F38D2`,
`_0F38D3`, `_0F38DA`, `_0F38DB` — VPDPWSUD, VSM4KEY4 and friends). They are
unreachable upstream too, so matching that is parity, not a gap. The other 5
are BF16/FP16 forms Skylake-X does not advertise.

## Defect 2 — the one the #UD was hiding

With the maps in place Ubuntu got further and failed differently: init exited
127 at ~9.1s, no fault, just a wrong answer somewhere. A control run with
`RUSTY_BOX_NO_AVX=1` on the same binary reached 19.8s and kept going, which
established that AVX-512 really was the trigger.

A first-seen probe over the dispatcher showed the guest executes only nine
distinct EVEX opcodes before dying:

    VPERMI2D  VPRORD  VPTERNLOGD  VPBROADCASTB  VMOVDQU64
    VPTESTNMB  VPCMPEQB  VPXORQ  VPTESTMB

VPRORD was writing the wrong register. Groups 12-14 (`0F 71/72/73`,
shift/rotate by immediate) were being given the legacy SSE operand layout, in
which the rm register is shifted in place and is therefore both source and
destination. Upstream distinguishes the encodings explicitly:

| opcode | first operand (destination) |
|---|---|
| `BX_IA_PSRLD_UdqIb` (legacy) | `OP_Wdq` — the rm operand |
| `BX_IA_V128_VPSRLD_UdqIb` (VEX) | `OP_Hdq` — VEX.vvvv |
| `BX_IA_EVEX_VPRORD_UdqIb` (EVEX) | `OP_Hdq` — EVEX.vvvv |

The VEX and EVEX forms are non-destructive three-operand instructions whose
destination is `vvvv`. Taking the legacy assignment made them write their own
source register and read whichever register the `/digit` happened to name —
`VPRORD zmm1, zmm2, 8` left `zmm1` untouched and clobbered `zmm2` with the
rotation of `zmm0`.

No fault, just corruption, which is why it presented as init exiting 127
rather than as a crash. This defect predates the map work; it became
reachable only once a guest could execute AVX-512 at all.

## Note on method

Two probe runs reported "zero EVEX opcodes executed". Both were instrument
failures, not findings: `tracing` is a no-op with no subscriber installed,
and `--display egui` routes console output into the in-app pane. Validating
the probe against a known-good case first would have caught both immediately.
A diagnostic that has not been shown to fire on a positive control is not
evidence.

## Still open

The Ubuntu boot has **not** been re-verified since the VPRORD fix. Both
defects are fixed and covered by tests, but until that boot runs, whether
Ubuntu reaches userspace with AVX-512 advertised is unknown.

## Correction: the `logger` segfault is not a cb23a6c regression

`fac0f6c` recorded a userspace segfault during `Adding live session user` as
a regression introduced by `cb23a6c`, on the strength of one run of each
commit: `cb23a6c` segfaulted, `29449d3` did not.

That inference does not hold. A second run of `cb23a6c` passes the same step
cleanly — `passwd: password changed.`, then init setup, accessibility
options, KDE services, APT cache — matching `29449d3` exactly. The fault is
**intermittent**, and one run per side cannot tell an intermittent fault from
a regression.

The whole `0F 7A` / `0F 7B` family that `cb23a6c` made reachable has since
been tested and is correct: float→qword in both truncating and rounding
forms, the unsigned integer→float vector forms at values above 2^31 and
2^63, GPR-sourced VCVTUSI2SS/SD in both widths, the memory form including
disp8×N scaling, and the masked merge form.

The segfault's cause is therefore unknown. Chasing it starts with a
reproduction rate — run one build several times and count — not with a
build-to-build comparison.

## Resolution (2026-08-02): root cause found and fixed — not EVEX at all

An adversarial parity audit found the mechanism, and it explains why the
fault looked intermittent while being fully deterministic per address
layout:

`get_icache_entry` (cpu.rs) truncated the prefetch-window distance
`RIP + eip_page_bias` to `u32` *before* comparing it against the 4 KiB
window. Bochs (`cpu.cc getICacheEntry`) compares in full `bx_address`
width, and near indirect transfers (`JMP/CALL r/m64`, `RET`) rely on that
compare — they deliberately do not invalidate the prefetch window. So an
indirect branch or return whose target lies ≥ 4 GiB away (PIE executable ↔
libc under ASLR, constantly) with bits 12..31 matching the stale window's
page base (a 2⁻²⁰ coincidence per pair, re-rolled every `exec`) falsely
passed the check and executed the **old page's bytes at the new RIP** —
which is why the kernel's `Code:` dump (the real bytes) could not decode
into the fault the CPU actually took.

Three sibling defects in the same arithmetic chain were fixed with it: the
legacy-mode page bias wrapped at 32 bits instead of 64 (masked by the
truncation; Bochs wraps mod 2⁶⁴), the CS-limit fetch-window clamp was dead
code (its condition tested the wrong variable), and ITLB entries for 2 MiB/
1 GiB code pages were filled with a hardcoded 4 KiB `lpf_mask` and never
set `ITLB.split_large`, so INVLPG left stale large-page code translations
alive after guest remaps.

Deterministic regression tests: `tests/prefetch_window_wrap.rs` (a
`V + 2³² + 0x10` alias flips which page's bytes execute) and
`tests/itlb_large_page_invlpg.rs` (huge-page remap + sibling-frame INVLPG).
Both were red before the fix and green after. Upstream Bochs never had the
bug — every piece was a Rust-port divergence.
