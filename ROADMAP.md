# Roadmap

What is left, why it is left, and what it would take. Everything here was
verified against the tree on 2026-08-04 rather than carried over from notes —
where a claim is checkable, the command that checks it is included.

This is a *backlog*, not a plan. Nothing here is scheduled, and several items
are deliberate decisions to stop rather than unfinished work.

**Where the project stands:** DLX Linux boots to a bash shell, Alpine boots
fully, Ubuntu 26.04 reaches userspace with AVX-512 on, Windows 7 SP1 reaches
Setup. Gates: 826 `rusty_box` tests, 163 decoder tests, `--no-default-features`
warning-free.

---

## 1. Open work

### 1.1 hwclock stalls during OpenRC boot

The Alpine guest takes ~13 extra guest-seconds to reach login versus Bochs,
from 2–4 deterministic ~3.0 s stalls in the OpenRC hwclock service. busybox
blocks *between* syscalls, with zero CMOS/RTC port reads during the stall and
matching ATA IRQ counts — so it is not obviously an I/O wait.

Parked before HPET landed. **Step one is a re-measurement, not code**: HPET now
registers as a guest clocksource and may have changed or removed the symptom.
Boot Alpine headless on both emulators and compare guest ticks to login. If it
is gone, record the measurement and close it.

If it survives, the next step is execve-argv logging. The cr3-tagged tracer for
that exists but is **not in the tree** — it is `alpine_direct.rs` in the git
stash `pre-switch-to-feature-8`. Do not drop that stash.

### 1.2 Bochs wall-ratio baseline with matched IPS

The last published ratio was measured before several correctness fixes and with
unmatched `--ips`, so it is not a usable baseline. Re-establish it before any
performance claim is made.

Read `docs/perf/PERFORMANCE-INVESTIGATION.md` first, and note the measurement
rule the hard way: **interleaved A/B only**. Absolute wall-clock on this machine
is untrustworthy — thermal state and background builds move it by more than the
effects being measured, and within-binary variance has been observed up to 4×
from ISO cache state alone.

### 1.3 62 advertised instructions abort the emulator instead of executing

`execute_instruction` ends in a catch-all returning `CpuError::UnimplementedOpcode`
— an *emulator* error that stops the host, where Bochs would point the opcode at
`BxError` and let the guest take #UD. The ISA gate normally prevents that by
rewriting unsupported opcodes to `IaError` first, but it only helps when the
model does not advertise the feature.

62 opcodes are decodable, advertised by a shipped model, and have no dispatcher
arm. Every one is a live host abort a guest can trigger:

| Group | Count | Bochs home |
|---|---|---|
| SSSE3 horizontal add/sub — PHADDW/D/SW, PHSUBW/D/SW | 6 | `simd_int.h` |
| SSE4.1 integer min/max — PMINSB/SD/UW, PMAXSB/SD/UW | 6 | `simd_int.h` |
| PCMPEQQ, PCMPGTQ, PACKUSDW, EXTRACTPS | 4 | `simd_int.h` / `simd_compare.h` |
| MMX <-> packed-FP conversions — CVTPI2PS/PD, CVT(T)PS2PI, CVT(T)PD2PI | 6 | `sse_pfp.cc` |
| VEX forms whose V128/V256 opcode has no arm of its own | 34 | as above, per lane |
| SSE4A — EXTRQ, INSERTQ, MOVNTSS, MOVNTSD | 6 | `sse.cc` |

The SSE4A six appear only on `AmdRyzen`, which advertises `IsaSse4a`. Checking a
single model would have missed them.

Pinned by `dispatcher.rs`
`every_opcode_a_model_admits_has_a_dispatcher_arm`, which is a ratchet: the list
may shrink, never grow, and a new gap on **any** model fails the build.

**Where the work goes** (the file layout mirrors Bochs, so this is not a
choice): the `xmm_*` primitives from `simd_int.h` / `simd_compare.h` belong in
`sse.rs`, which already hosts that header's blend helpers as `*_lane` functions
shared with `avx_pfp.rs` — **not** in a new `simd_int.rs`. The six conversions
go in `sse_pfp.rs`. The 34 VEX forms go in `avx.rs` and call the same lane
helpers per 128-bit lane, which first requires extracting the shared operation
out of the existing inline legacy handlers, exactly as the blend helpers were.
EXTRACTPS is a one-line dispatcher arm onto the existing PEXTRD handler —
upstream defines it as literally the same function.

**Do not classify these by opcode name.** `remap_sse_to_vex` does `use
Opcode::*` and writes its arms bare, so a scan for `Opcode::`-qualified names
misses every VEX opcode the remap produces. That mistake is why this was first
measured at 22 instead of 62.

## 2. Deliberate parity gaps

These are divergences from `cpp_orig/bochs/` that are **not** bugs under the
CLAUDE.md rule, because they are unobservable on the CPU models this port
ships. They become real the moment a model advertises the relevant feature.

### 2.1 Unimplemented VEX instruction families

51 VEX opcode slots decode to `#UD` at decode time. Bochs decodes them and then
raises `#UD` on the absent ISA bit — same observable behaviour for every model
here.

| Family | Slots |
|---|---|
| FMA4 (incl. VPERMIL2PS/PD) | 22 |
| CMPCCXADD | 16 |
| AVX-VNNI / VNNI-INT8 | 4 |
| AVX-NE-CONVERT | 3 |
| AVX-IFMA | 2 |
| VNNI-INT16 | 2 |
| SM3 | 2 |

The exact set is pinned by
`vex_shared::tests::every_populated_vex_slot_decodes_unless_deliberately_unimplemented`,
which doubles as the ledger. **Implementing a family means deleting its line
from `UNIMPLEMENTED_VEX_SLOTS`** — the test fails if the two disagree in either
direction.

### 2.2 EVEX map — swept 2026-08-04, no action needed

Recorded so the sweep is not repeated. The EVEX map does **not** have the
problem the VEX map had: `opmap_evex.rs` is generated from
`fetchdecode_opmap_evex.cc` by `scripts/gen_opmap_evex.py`, so it is a
transcription and cannot drift from upstream by hand.

410 EVEX opcodes decode but have no handler — AVX512-FP16 (194), AVX10.2 (142),
VBMI2 (20), AMX (12), AVX10.2-MOVRS (8), VNNI (8), VAES/VPCLMULQDQ (5), VBMI,
IFMA52, BF16, GFNI, BITALG, VP2INTERSECT, VPOPCNTDQ. Every one is gated on a
feature **neither shipped model advertises**, verified against the model
definitions rather than assumed, so all become a guest #UD at the ISA gate and
none can reach the dispatcher catch-all.

That stops being true the moment a model advertises one of those features — an
Icelake or Sapphire Rapids cpudb entry would bring GFNI, VAES, VBMI, VNNI, BF16
and FP16 with it. The §1.3 ratchet is what will catch that, and the answer then
is to implement the family, not to widen the ledger.

The one gap worth closing regardless: `gen_opmap_evex.py` has no `--verify`
mode, so drift after an upstream sync goes unnoticed. `gen_vex_slots.py
--verify` is the model to copy.

### 2.3 XOP

`0x8F` returns `BxIllegalVexXopOpcodeMap`. Bochs has `decoder_xop32`/
`decoder_xop64` and a 91-entry table. Neither shipped model advertises XOP:

```bash
grep -oE "X86Feature::Isa\w+" rusty_box/src/cpu/cpudb/amd/amd_ryzen.rs | sort -u
```

`AmdRyzen` advertises AVX, AVX2, F16C, FMA and SSE4A — **not** XOP, FMA4 or TBM.
So adding the AMD model did not make §2.1 or this section observable, contrary
to what the old AVX plan assumed. It did make SSE4A observable, which is where
six of the §1.3 gaps come from.

### 2.4 VEX map 7

Holds only WRMSRNS/RDMSR/UWRMSR/URDMSR, gated on `BX_ISA_MSR_IMM` and
`BX_ISA_USER_MSR`. Rejected when the prefix is parsed. Note map 7 takes a
**dword** immediate, not a byte, if it is ever implemented.

### 2.5 AMX

Not implemented. The tile state struct exists but nothing decodes to it. All
Bochs tables here are extracted with `BX_SUPPORT_AMX = 0`, including the VEX
slot bitmap — see `scripts/gen_vex_slots.py`.

### 2.6 How the ISA gate differs from upstream (deliberate)

Bochs gates instructions by **mutating a process-global table** once per CPU at
init: `init_FetchDecodeTables` (`fetchdecode32.cc`) walks every opcode and, where
the CPU lacks the feature, overwrites `BxOpcodesTable[n].execute1/execute2` with
`BxError` and zeroes `opflags` (the last part is what stops a now-#UD opcode
from also running `prepare_SSE`).

That table is a non-const file-scope global. Every CPU writes it, so it is not
thread safe and cannot represent two CPUs with different models — last writer
wins. Under CLAUDE.md's "thread safety trumps Bochs literalness" rule this port
deliberately diverges: `isa_resolve_opcode` is a pure function over the
immutable generated `OPCODE_ISA` plus **this CPU's own**
`ia_extensions_bitmask`, applied at icache fill. Nothing is mutated, and the
`opflags = 0` trick needs no analogue because `IaError` dispatches straight to
`bx_error` and is classified `CpuState::Base`, so no state gate runs on it.

Both implement the same three special cases: the 3DNow!Ext rescue of 15 MMX-era
opcodes, AVX10.1 subsuming the 12 AVX-512 sub-extensions, and LZCNT/TZCNT
falling back to BSR/BSF rather than #UD.

Bochs's fourth special case — `BX_ISA_ALT_MOV_CR8` marking the MOV CR0 opcodes
`BX_LOCKABLE` so `LOCK MOV CR0` becomes an access to CR8 — was missing here and
is now implemented, split differently: the decoders always extend CR0 -> CR8
when the prefix is present (they cannot see CPU features), and
`check_alt_mov_cr8` in `cpu/crregs.rs` vetoes it on a model without the feature.

### 2.7 Skylake CPUID max leaf 0x14

Ratified deviation, filed upstream as bochs-emu/Bochs#791. Do not re-litigate.

---

## 3. Known-imperfect, low priority

### 3.1 SSE handlers apply an SSE check to their VEX arms

The AVX/AVX-512 state gate is central now (`state_resolve_opcode` at icache
fill), and the in-handler checks were removed from the AVX-only files. The 172
handlers in `sse*.rs` keep `prepare_sse()` because they are *also* dispatched
from legacy SSE opcodes, where it is correct.

```bash
grep -c "prepare_sse()?" rusty_box/src/cpu/sse.rs      # 107
grep -c "prepare_sse()?" rusty_box/src/cpu/sse_pfp.rs  # 47
```

For a VEX-encoded instruction routed to one of those, the central gate passes
and then `prepare_sse` adds CR0.EM and CR4.OSFXSR tests that Bochs does not
apply to AVX forms. Reaching it needs `CR0.EM = 1` or `CR4.OSFXSR = 0` while
AVX state is fully enabled — a state no real OS produces, which is why this is
not chased.

The fix, if ever wanted, is `if !instr.is_vex() { self.prepare_sse()?; }` in
those handlers. Classify with `opcode_state(op) -> CpuState`, **never by opcode
name** — `Vfmadd132ps`, `Vmovdqa*`, `Vmovq*` and `Vmovmskps` are VEX opcodes
whose second character is lowercase, so a "V followed by uppercase" heuristic
misfiles them as legacy and you get a wrong answer in both directions.

### 3.2 Lazy flags read side

`docs/future-plans/lazy-flags-read-side.md`. The write side landed in `dbbb088`;
reads still go through `self.eflags.contains()`. Switching them requires that
every flag-writing site update `oszapc` first, and ~100+ sites (shifts, rotates,
BT/BTS/BTR/BTC) still write `eflags` directly. Sequenced work, no partial
credit — the read side cannot switch until the write side is total.

### 3.3 `docs/future-plans/system-snapshots.md` looks superseded

`rusty_box/src/snapshot.rs` implements a v3 format covering CPU, RAM, devices,
timers and SMP LAPICs, with round-trip tests. Confirm the doc has no remaining
scope and retire it rather than leaving a completed plan on the shelf.

### 3.4 Stale audit documents

- `docs/iodev-parity-audit-2026-07-10.md` — re-verified 2026-07-24 and roughly
  40% stale. Findings still open are real; findings marked open may already be
  fixed. Re-check before acting on any single entry.
- `docs/audit_stubs.md`, `docs/DECODER_BUGS.md` — vintage unverified.

---

## 4. Upstream Bochs bugs to file

`docs/bochs-upstream-bugs.md` and
`docs/bochs-cpudb-cpuid-audit-2026-07-13.md` hold **13 verified upstream defects
beyond the one already filed**, with issue text drafted and nothing submitted.
Two are guest-breaking:

- **A11** — GETSEC host DoS.
- **F1** — K6-2 MSR collision.

The HPET route 16–23 out-of-range IRQ indexing (`iodev/hpet.cc update_irq()`
routing pins ≥ 16 through `bx_pic_c::raise_irq`, which indexes the slave PIC
with `route & 7`) is also drafted and unfiled. The recommendation on record is
to file A4 first as a low-risk trial of the reporting format.

Filing is a **human action** — the drafts are ready to send, not to be sent
automatically.

---

## 5. Elsewhere, deliberately

- **Performance.** Separate session and working tree:
  `C:\Users\olegg\Desktop\rusty_box_perf_handoff\`. Do not touch the benchmark
  instrumentation — `vec_diag::count` in `cpu/cpu.rs` and the
  `RUSTY_BOX_BENCH_FILE` sampler in `emulator.rs` — that session depends on both.
- **`pci_vga`.** Firmware side verified, but `pci_vga=true` hangs the Ubuntu
  boot; root cause on the Linux side is unknown. Stays gated off until someone
  reproduces it deliberately.
- **EVEX equivalents of the AVX2/F16C work.** `EvexVcvtph2ps*` and
  `EvexVgatherdd*` are wired; the rest of the EVEX map has not been audited
  against Bochs the way the VEX map now has.

---

## 6. Tooling

CLAUDE.md asks for `lsp references` before modifying any exported symbol, but
no LSP is reachable — neither `mcp__lsp__*` tools nor an `lsp` binary on `PATH`.
Work is proceeding on grep, which is materially weaker for exactly the case the
rule exists for (this session added `Opcode` enum variants without it). Either
wire up the LSP server or soften the rule so it describes what is actually
available.

---

## Regenerating derived tables

Three tables are generated from the vendored Bochs tree and **must be
regenerated after syncing `cpp_orig/bochs/`**, or new opcodes silently `#UD`:

```bash
python scripts/gen_opcode_isa.py     # ISA gate + per-opcode CPU-state class
python scripts/gen_vex_slots.py --verify   # VEX slot bitmap; exits 1 on drift
python scripts/gen_opmap_evex.py
```

`gen_opcode_isa.py` fails rather than guessing if a Bochs feature has no
`X86Feature` variant, if `KNOWN_UNMATCHED` goes stale, or if any
`Evex*`/`V128*`/`V256*`/`V512*` opcode ends up with no CPU-state class.
`gen_vex_slots.py` refuses to parse an unrecognised preprocessor condition
inside the VEX table for the same reason.
