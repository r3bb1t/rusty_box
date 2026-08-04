# Roadmap

What is left, why it is left, and what it would take. Everything here was
verified against the tree on 2026-08-04 rather than carried over from notes —
where a claim is checkable, the command that checks it is included.

This is a *backlog*, not a plan. Nothing here is scheduled, and several items
are deliberate decisions to stop rather than unfinished work.

**Where the project stands:** DLX Linux boots to a bash shell, Alpine boots
fully, Ubuntu 26.04 reaches userspace with AVX-512 on, Windows 7 SP1 reaches
Setup. Gates: 824 `rusty_box` tests, 159 decoder tests, `--no-default-features`
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

---

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

### 2.2 XOP

`0x8F` returns `BxIllegalVexXopOpcodeMap`. Bochs has `decoder_xop32`/
`decoder_xop64` and a 91-entry table. Neither shipped model advertises XOP:

```bash
grep -oE "X86Feature::Isa\w+" rusty_box/src/cpu/cpudb/amd/amd_ryzen.rs | sort -u
```

`AmdRyzen` advertises AVX, AVX2, F16C, FMA and SSE4A — **not** XOP, FMA4 or TBM.
So adding the AMD model did not make §2.1 or §2.2 observable, contrary to what
the old AVX plan assumed.

### 2.3 VEX map 7

Holds only WRMSRNS/RDMSR/UWRMSR/URDMSR, gated on `BX_ISA_MSR_IMM` and
`BX_ISA_USER_MSR`. Rejected when the prefix is parsed. Note map 7 takes a
**dword** immediate, not a byte, if it is ever implemented.

### 2.4 AMX

Not implemented. The tile state struct exists but nothing decodes to it. All
Bochs tables here are extracted with `BX_SUPPORT_AMX = 0`, including the VEX
slot bitmap — see `scripts/gen_vex_slots.py`.

### 2.5 Skylake CPUID max leaf 0x14

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
