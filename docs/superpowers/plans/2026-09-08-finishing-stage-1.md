# Finishing Stage 1 — Completion Brief

> Continuation brief for `docs/superpowers/plans/2026-09-03-whp-vmm-shape.md` Task 1.9.
> Written 2026-09-08 at HEAD `fd41c7b` on `wip/atom-execctx`, from measured state.

**Stage 1 tasks 1.1–1.8 are all committed.** Task 1.9 is the gate, and it is
one defect away. This brief carries what the gate needs, what is already met,
and the numbers that must NOT be taken from the parent plan's prose.

---

## 1. The blocker: DLX loses IRQ 14

DLX reaches **2/3** milestones (BIOS ✅, LILO ✅, `login:` ❌). Linux 1.3.89 boots,
finds `hda`, mounts root, then loops `hda: irq timeout` → `ide0: reset` →
`end_request: I/O error` → VFS/EXT2 panic.

### What was measured (commit `e36e9dd`, one 360 s run)

```
acknowledges 35123  ==  injected 35123      ← the ledger balances EXACTLY
```

Non-zero vectors: `0x08`×151 (IRQ 0, BIOS base) · `0x20`×34964 (IRQ 0 after
Linux's remap, 100 Hz unbroken all run) · `0x21`×2 (IRQ 1) ·
**`0x2e`×6 (IRQ 14, slave base 0x28+6)**. All six IDE placements precede the
first `irq timeout` at ~24 s; none in the following 336 s across four resets.
Absent: `0x76` and every spurious vector (`0x0f/0x27/0x2f/0x77`).

### What that rules OUT — do not re-derive these

- **"A slave vector was acknowledged and never delivered, wedging the cascade."**
  Architecturally sound (a slave IRQ sets in-service on the slave AND on master
  IRQ2, which stays set until the guest EOIs the master, which is why IRQ 0 —
  higher priority than IRQ2 — would keep flowing). **It is not what happens:**
  the ledger balances exactly. REFUTED.
- **"A lost rising edge."** The pin is published as a LEVEL, not an edge:
  `emulator/scheduler.rs` does `if asserted || self.pic_pin_published`, i.e. at
  EVERY boundary while asserted, and the code's own comment gives this exact
  scenario as its reason. The dedup is one layer down in `ext_int_request`'s
  compare-exchange and is per-VECTOR-OWED. REFUTED.

### Where to start

**The raise, and the raise→pin path — not delivery.** One sharpening beyond the
counters: at the first timeout the drive status is `0x58` with **DRQ set**, i.e.
the drive model reached exactly the state at which a PIO drive raises IRQ 14,
and no INTA followed. The counters cannot separate "the IDE stopped raising"
from "it raised and the request never became an INTA". Live candidates for the
latter: the IMR, a slave ISR bit left set by an EOI that did not clear it, or a
pin edge that never reached `ext_int_pending`.

**Index trap:** `injected_per_vector` is indexed by VECTOR, not IRQ. IRQ 14 is
vector `0x2e` after Linux remaps the PICs (`0x76` under the BIOS base). Index 14
(`0x0e`) is IRQ 6 and is legitimately zero. `dlx_whp` now prints vectors in hex.

---

## 2. What the gate already has

| criterion | state |
|---|---|
| Alpine → `login:` on fast mode, faster than the interpreter | ✅ **27.9 s vs 96.1 s (3.45×)** |
| `in_run_share` ≥ 0.90 | ✅ **97.7 – 98.8 %** (measured on DLX) |
| `cargo xtask ci` green | ✅ repeatedly, most recently at `f184706` |
| DLX 3/3, three runs | ❌ **2/3** — §1 |
| `fatal_signatures = 0` over three Alpine runs | ✗ never swept |
| baseline document gains a Stage 1 row | ✗ not written |
| **STOP for user review** | pending |

---

## 3. Numbers the parent plan gets WRONG — read the source, not the prose

Task 1.9 says to check the unsafe-token baselines and explicitly warns "**read
that table, do not trust this sentence**". It was right to: **two of its three
figures are stale.** The gate enforces (`xtask/src/ci.rs`, verified 2026-09-08):

| crate | plan says | **gate actually enforces** |
|---|---|---|
| `rusty_box_whp_sys/src` | 52 | **62** |
| `rusty_box_whp/src` | 4 | **5** |
| `rusty_box_whp_engine/src` | 1 | 1 ✓ |

Also: `UNSAFE_IMPL_SEND_BASELINE = 0`, `PRODUCTION_PANIC_BASELINE = 0`,
`BLANKET_DEAD_CODE_BASELINE = 70` — and the ratchet currently reports **68**,
asking for the constant to be tightened. That nag is pre-existing (the constant
was last touched in `f9e3f83`) and wants its own one-line commit with its own
gate run.

`Send` assertions the gate wants present: `Vcpu`, `VpCounters`,
`Emulator<(), WhpEngine>`, `VcpuControl`, `VcpuThread<()>`, `DeviceThreadControl`,
`FastMachine<()>`.

---

## 4. The measurement protocol the gate demands

**Do NOT compare against any number written in the parent plan, this brief, or
the baseline document.** Absolute wall-clock on this host is not comparable
across days. The gate requires:

- **Three rounds of {fast mode, interpreter}, alternating, in ONE session**, via
  `alpine_probe` with 300 s patience and the baseline ISO.
- Compared by **median**, with a **3.7 % noise floor**.
- Load recorded per run.
- The window is **`"ISOLINUX"` → `login:`**, NOT `"Linux version"` → `login:` —
  Task 0.1 measured that `"Linux version"` is never recorded on either engine
  (the 80×25 scrape samples after that line has scrolled away). If Task 1.8's
  accumulate-distinct-lines scrape made it reliable, prefer it and say so.
- `RESULT in_run_share` ≥ 0.90 per run, as `Instant` deltas around `vcpu.run()`;
  report `guest_share` beside it as the platform's cross-check.

### This host will corrupt the measurement if you let it

- **Modern Standby took 245 s out of one 360 s run**; another run spent 91 s on
  battery at half clock (BogoMIPS 2254 vs ~4600–5560). **Neither appears in the
  benchmark's output** — a half-slept run looks exactly like a slow guest.
  Check the Windows System event log for the run's window and state what you
  found beside every number.
- Another project (`work_backend`) is developed in parallel on this machine, so
  CPU contention is normal. It shifts absolute times; the interleaved sweep is
  what makes the comparison survive it.
- **The interpreter has no stable baseline**: 110.8 s and 149.6 s unclamped on
  consecutive clean runs, 96.1 s clamped. Pre- and post-clamp times are **not
  comparable at all** — the guest changed (no ZMM/opmask state). The 76.4 s
  figure quoted earlier in this campaign is not reproducible; do not use it.

---

## 5. Standing environment traps

- **The LSP tools hang indefinitely** in this workspace — never call them.
  rust-analyzer also emits stale false-positive compile errors: **seven times on
  2026-09-08** it reported hard errors on code `cargo` builds clean. Reproduce
  every diagnostic with a real cargo command.
- **A gate run can be KILLED, not failed**, by another project's session running
  `Stop-Process` on `xtask.exe` by name. Signature: the log stops mid-step,
  `grep -c FAILED` is 0, exit `0xffffffff`. **Retry once** before blaming a
  change. Do not wait on foreign `cargo`/`xtask` processes — check their paths;
  the `work_backend` ones are dev servers that never exit. **Never kill a
  process you did not start.**
- **The gate never builds the examples** (`dlx_whp`, `alpine_bench`,
  `alpine_probe`) and builds the GUI only as
  `cargo build -p rusty_box_gui --release`. `ci: 24 steps passed` proves nothing
  about any of them. Build and run them explicitly.
- Only ONE process may hold a WHP partition and inspection port 5719 at a time.
- Disk sits around 4–12 GB free and has hit **zero** mid-edit; ENOSPC silently
  truncates files here.

---

## 6. Loose ends, none blocking the gate

- The status bar's IPS reads `---` on WHP: that path publishes `ips = 0` to the
  bridge, so no rate exists. `RunSummary.instructions_executed` is `Option<u64>`
  and `None` on hardware, because nothing counts a hardware processor's
  instructions (`60f86c3`). Giving the bridge a rate is a real, separate change.
- A third copy of the egui→key mapping lives in `examples/rusty_box_web`; no ci
  step builds it, so it can rot silently.
- `push_scancodes` has zero callers (measured) but is public API.
- `CLAUDE.md` describes the gate as "9 steps"; it runs **24**.
- Keyboard behaviour changes the user may still veto (`144c108`): Shift now
  forwarded, Space no longer doubled, Ctrl+C/X/V reach the guest, Dvorak/AZERTY
  hosts type as QWERTY (the Bochs/VMware default).

---

## 7. After the gate

Task 1.9 ends with **"Stop for user review"** — Stages 2–4 are planned from its
measurements, so do not roll on past it.

Worth knowing when Stage 2 is planned: its headline item, "exit clustering with
VirtualBox's thresholds", is the same insight as applepie's `EMULATE_STEPS = 250`
— run a burst on the shadow after an exit instead of re-entering, because
entry/exit dominates. DLX takes ~300 k port exits in 360 s; Alpine is the guest
that would benefit. That is measured prior art, not a guess.

Stage 4 is already **partly done out of order**: `60f86c3` is its
"`rusty_box_gui` `drive<E>` split into two arms by engine". Its remaining items
are front-end `run_interactive` for a `FastMachine`, `StopHandle` as
pause + terminal, and the docs.

Stage 3 carries an **open decision that must not be closed by assumption**: the
LAPIC page has no task-priority field and CR8 holds only `TPR[7:4]`, so
`TPR[3:0]` is carried by no transfer path. Invisible to an ordinary OS, not
invisible to a guest that reads TPR back — which is what the RE north star cares
about. Measure whether the platform preserves it before deciding.
