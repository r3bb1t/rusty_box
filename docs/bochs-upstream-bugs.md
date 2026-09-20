# Bochs Upstream Bugs

Genuine bugs in upstream Bochs (`cpp_orig/bochs/`) found during the Rusty Box
Bochs-parity work, each written to be filed as an upstream issue. Every entry
cites the Bochs file + symbol and never a line number, because the vendored
snapshot rebases.

Each entry also says what Rusty Box does about the bug, and the answer differs
from bug to bug. Where the architectural behaviour is the better side, the port
implements it and registers the difference in `docs/bochs-parity-divergences.md`.
Some bugs are reproduced bug-for-bug for parity instead: the mouse `0xE6`
scaling, the HPET restore epoch, VRSQRT14's power-of-two case, the unreferenced
EVEX groups and the unbacked PMU leaf.

Filed upstream:
- https://github.com/bochs-emu/Bochs/issues/791 — the post-Skylake CPU models
  declare a TSC frequency in CPUID leaves 0x15/0x16 that the emulated TSC does
  not run at, and Linux 4.8 and later trust it without calibrating. The fix,
  PR #792 (upstream commit `9fceef128`), added
  `cpu: cpuid_freq=hardware|none|ips` with `hardware` as the default. This port
  defaults to `none`: divergence D7.

---

## 1. HPET interrupt routes 16–23 corrupt the slave PIC (and can panic the host)

**Severity**: Correctness bug + host abort. A guest can inject a phantom ISA
IRQ (8–15) or crash the emulator by programming an HPET timer's interrupt
route to a legal, advertised GSI in 16–23.

**Location**:
- `iodev/hpet.cc` — `bx_hpet_c::update_irq()` (the `timer_int_route` path).
- `iodev/pic.cc` — `bx_pic_c::raise_irq()` / `bx_pic_c::lower_irq()`.

**Root cause**: Except for timers 0 and 1 in legacy-replacement mode (routed
to 0 and `RTC_ISA_IRQ`), `update_irq()` computes
`route = timer_int_route(timer)` and calls
`DEV_pic_raise_irq(route)` / `DEV_pic_lower_irq(route)`, which expand to
`bx_pic_c::raise_irq(route, BX_IRQ_TYPE_ISA)`. `raise_irq` assumes the IRQ
number is a legacy PIC line (0–15):

```cpp
// iodev/pic.cc  bx_pic_c::raise_irq
void bx_pic_c::raise_irq(unsigned irq_no, Bit8u irq_type)
{
  bx_pic_t *pic = (irq_no < 8) ? &BX_PIC_THIS s.master_pic : &BX_PIC_THIS s.slave_pic;
  Bit8u mask = (1 << (irq_no & 7));
  if ((irq_type == BX_IRQ_TYPE_ISA) && ((pic->IRQ_in[irq_no & 7] & ~irq_type) != 0)) {
    BX_PANIC(("ISA IRQ %d lost", irq_no));            // <-- host abort
  }
  if ((pic->IRQ_in[irq_no & 7] & ~irq_type) == 0) {
    pic->IRQ_in[irq_no & 7] |= irq_type;              // <-- writes slave IRQ_in[route & 7]
    if ((pic->irr & mask) == 0) {                     // <-- slave IRR bit (route & 7)
      pic->irr |= mask;
      ...
      if (DEV_ioapic_present() && (irq_no != 2)) {
        DEV_ioapic_set_irq_level(irq_no, 1);          // <-- IOAPIC pin `route`, only if that IRR bit was clear
      }
    }
  }
}
```

For `route >= 8` the code selects the **slave PIC** and indexes
`IRQ_in[route & 7]`. When `route` is in 16–23 the `& 7` mask silently folds it
onto ISA IRQ 8–15:

| HPET route (GSI) | `route & 7` | Phantom ISA IRQ asserted |
|---|---|---|
| 16 | 0 | IRQ 8 (RTC) |
| 20 | 4 | IRQ 12 (PS/2 mouse) |
| 22 | 6 | IRQ 14 (primary IDE) |

So the HPET fires a spurious slave-PIC edge on an unrelated ISA device **in
addition** to the intended IOAPIC pin. If that slave line already carries a
non-ISA assertion, the `BX_PANIC("ISA IRQ %d lost")` guard aborts the host.

The same fold has two more effects:

- **A device's own line is dropped.** An HPET deassert
  (`update_irq(timer, 0)`: the guest clearing a level timer's status bit, a
  configuration write to an enabled timer whose status bit is clear, or
  `hpet_del_timer` when the HPET is disabled or reset) runs
  `bx_pic_c::lower_irq`, which clears the ISA bit of the shared `IRQ_in` slot
  and, once the slot is empty, the real IRQ 8–15 device's pending IRR request
  with it. To the 8259 that device's line reads low until it raises the line
  again. The lower half of an edge does the same for an instant, but the raise
  half sets the bit and a request again at once, so there the device's request
  merges with the HPET's.
- **The intended pin can be lost.** A level-triggered timer is raised with no
  lower first. While that slave line already has a request in its IRR, the
  forward is skipped, and the IOAPIC pin the guest programmed is never raised.

**Reachability**: `hpet.cc` advertises `HPET_ROUTING_CAP = 0xffffff` (all 24
GSIs legal for every timer), and `HPET_TN_CFG_WRITE_MASK` (0x7f4e) keeps all
five `TN_INT_ROUTE` bits writable, so a guest can legally program any timer to
route 16–23. In APIC mode an OS routinely picks a non-legacy HPET GSI ≥ 16
from the routing-cap bitmap.

**Expected**: A route ≥ 16 is an IOAPIC-only GSI and must not touch the
8259 PIC at all. Only routes 0–15 are legacy PIC lines.

**Suggested fix**: In `update_irq()`, gate the legacy-PIC call on
`route < 16` and drive routes ≥ 16 through the IOAPIC only — e.g.

```cpp
if (route < 16) {
  set ? DEV_pic_raise_irq(route) : DEV_pic_lower_irq(route);
}
// (the IOAPIC forward already happens for route < BX_IOAPIC_NUM_PINS)
```

Alternatively bound-check `irq_no` inside `bx_pic_c::raise_irq`/`lower_irq`.

**Rusty Box behavior**: delivers only the IOAPIC pin for routes 16–23
(`rusty_box/src/emulator/timers.rs` `drain_hpet_pending`). Registered as
divergence D12 in `docs/bochs-parity-divergences.md`. The owner ratified the
deviation on 2026-07-25 as a closed decision, not an open item.

**Filing status**: NOT FILED. The text above is issue-ready for
`bochs-emu/Bochs` (same shape as the already-filed #791). Hand it to the
maintainer/user to file — do not open the issue autonomously.

---

## 2. Mouse "Set Scaling 1:1" (0xE6) sets scaling to 2:1

**Severity**: Minor correctness bug — a copy/paste error that makes the PS/2
mouse report the wrong scaling in its status byte.

**Location**: `iodev/keyboard.cc` — `bx_keyb_c::kbd_ctrl_to_mouse()`, case `0xe6`.

**Root cause**: The "Set Scaling to 1:1" handler sets `scaling = 2` — identical
to the "Set Scaling to 2:1" (`0xe7`) handler just below it:

```cpp
// iodev/keyboard.cc  kbd_ctrl_to_mouse()
case 0xe6: // Set Mouse Scaling to 1:1
  controller_enQ(0xFA, 1); // ACK
  BX_KEY_THIS s.mouse.scaling = 2;          // <-- BUG: should be 1
  BX_DEBUG(("mouse: scaling set to 1:1"));
  break;
case 0xe7: // Set Mouse Scaling to 2:1
  controller_enQ(0xFA, 1); // ACK
  BX_KEY_THIS s.mouse.scaling = 2;
  BX_DEBUG(("mouse: scaling set to 2:1"));
  break;
```

**Manifestation**: The PS/2 "Get Info" command (`0xE9`) returns a status byte
whose bit 4 is set iff `scaling != 1` (`get_status_byte`:
`ret |= (scaling == 1) ? 0 : (1 << 4)`). After a `0xE6` (1:1) command Bochs
reports bit 4 **set**, telling the guest driver the mouse is in 2:1 scaling
when the guest explicitly requested 1:1.

**Expected**: `case 0xe6` should set `s.mouse.scaling = 1;`.

**Rusty Box behavior**: currently matches Bochs bug-for-bug (`scaling = 2`)
for status-byte parity, with a comment flagging the upstream quirk
(`rusty_box/src/iodev/keyboard.rs`, `MOUSE_CMD_SET_SCALING_1_1`). Will flip to
`1` if/when upstream fixes it.

---

## 3. HPET save/restore drops the counter reference epoch (lower confidence)

**Severity**: Minor / possibly by-design. After a state restore, an enabled
HPET's main counter reads absolute emulated time rather than its saved value.

**Location**: `iodev/hpet.cc` — `bx_hpet_c::register_state()`.

**Observation**: `register_state()` serializes `config`, `isr`,
`hpet_counter`, and per-timer `{config, cmp, fsb, period}`, but **not**
`hpet_reference_value`, `hpet_reference_time`, or per-timer `last_checked`.
Bochs restores onto a freshly-constructed device where those are zero, so if
the HPET was enabled at save time, the first post-restore `hpet_get_ticks()`
returns `ns_to_ticks(time_nsec())` (absolute time since the restored machine's
boot) instead of the saved counter — a discontinuity a guest clocksource
would observe.

**Note**: this may be an accepted limitation of Bochs's state model rather
than an intended-precise restore; filed as low confidence. Rusty Box
deliberately reproduces Bochs's behavior here (zeroes the reference fields on
restore) for parity — see the snapshot section's `restore` for `BxHpetC`
(`rusty_box/src/iodev/hpet.rs`).

---

## VRSQRT14 returns the wrong result for exact powers of two with an odd unbiased exponent

**Files**: `cpu/avx/avx512_rsqrt14.cc` — `approximate_rsqrt14(float16)`,
`approximate_rsqrt14(float32)`, `approximate_rsqrt14(float64)`
**Confidence**: high — arithmetic, reproducible from the source alone
**Rusty Box**: reproduced deliberately for parity, see
`rusty_box/src/cpu/avx512_rcp14.rs` and the test
`rsqrt14_reproduces_the_upstream_power_of_two_bug`

VRSQRT14 selects one of two 32K-entry tables by the parity of the biased
exponent, because halving an odd unbiased exponent leaves a factor of
sqrt(2) that the table has to absorb. `rsqrt14_table0` covers the odd
unbiased exponents (its entry 0 is ~0.4142 = 2/sqrt(2) - 1) and
`rsqrt14_table1` the even ones (entry 0 ~1.0).

All three width variants then do:

```c
  const Bit16u *rsqrt_table = (exp & 1) ? rsqrt14_table1 : rsqrt14_table0;
  exp = 0x7E - ((exp - 0x7F) >> 1);
  if (fraction)
    fraction = rsqrt_table[fraction >> 8];
  else
    exp++;                       // <-- only valid on rsqrt14_table1
```

The `else exp++` shortcut assumes a zero significand means the result is an
exact power of two. That holds on the even-exponent table — 1/sqrt(2^2k) is
2^-k — but not on the odd one, where the significand should come from table
entry 0 instead. So for any exact power of two with an odd unbiased
exponent the answer is the reciprocal square root of the *next* power of
two:

| input | Bochs  | hardware / correct |
|-------|--------|--------------------|
| 2.0   | 1.0    | 0.70709…           |
| 8.0   | 0.5    | 0.35355…           |
| 32.0  | 0.25   | 0.17677…           |
| 0.5   | 2.0    | 1.41418…           |

That is a relative error of about 41%, far outside the 2^-14 the
instruction guarantees. Even-exponent powers of two (1.0, 4.0, 16.0) and
all non-power-of-two inputs are unaffected.

The fix is to consult the table in both cases and keep `exp++` for the
even-exponent table only, or equivalently to seed `fraction` from
`rsqrt_table[0]` before the branch.

---

## EVEX opcode groups that the master table never references

**Found 2026-08-01**, while generating rusty's EVEX opcode maps from
`cpu/decoder/fetchdecode_opmap_evex.cc`.

Four groups are defined in that file and then never referenced from
`BxOpcodeTableEVEX`, so nothing can ever select them:

| group | instructions | ISA |
|---|---|---|
| `BxOpcodeGroup_EVEX_0F38D2` | VPDPWSUD, VPDPWSUDS | AVX-VNNI-INT16 |
| `BxOpcodeGroup_EVEX_0F38D3` | VPDPWUSD, VPDPWUSDS | AVX-VNNI-INT16 |
| `BxOpcodeGroup_EVEX_0F38DA` | VSM4KEY4 | SM4 |
| `BxOpcodeGroup_EVEX_0F38DB` | VSM4RNDS4 | SM4 |

`BxOpcodeGroup_EVEX_0F38DB` is not referenced at all — not even by its own
definition site being reachable — and the other three appear exactly once,
at their definition. The corresponding master-table slots hold
`BxOpcodeGroup_ERR`, so a guest executing any of these encodings takes #UD
even on a CPU model that advertises the ISA.

The handlers exist (`BX_IA_EVEX_VPDPWSUD_VdqHdqWdq` and friends are defined
in `ia_opcodes_evex.def` with real execute functions), so this is missing
wiring rather than missing implementation — the same shape of defect as the
gap this project hit on its own side: an opcode can have a correct,
dispatched handler and still be unreachable because no decoder table slot
produces it.

Reproduced deliberately in rusty for parity: `scripts/gen_opmap_evex.py`
transcribes the master table as it stands, so those slots are empty there
too. 14 of the 19 EVEX opcodes rusty cannot reach are these; if upstream
wires them up, regenerating picks them up automatically.

Detection is mechanical — for each `BxOpcodeGroup_EVEX_*` definition, count
references in the same file; a count of one means the group is orphaned.

---

## CPUID leaf 0xB sub-leaf 1 reports a core-width shift where the SDM wants core+SMT

**Severity**: Correctness bug, guest-visible. A multi-socket guest with
hyperthreading enumerates more packages than exist.

**Location**: `cpu/cpuid.cc` —
`bx_cpuid_t::get_std_cpuid_extended_topology_leaf`, `case 1`.

**Requires**: `cpu: count=P:C:T` with `T > 1`. Single-threaded topologies are
unaffected, which is likely why this has gone unnoticed.

**Root cause**:

```cpp
case 1:
   leaf->eax = ilog2(ncores-1)+1;     // <-- core field width only
   leaf->ebx = ncores * nthreads;
```

Per Intel SDM Vol 2, CPUID leaf 0BH, EAX[4:0] at sub-leaf *m* is "the number of
bits to shift right on x2APIC ID to get a unique topology ID of the **next**
level type". The next level above *core* is the package, so the shift must
clear the SMT bits as well as the core bits. EBX on the following line already
uses `ncores * nthreads` — the same quantity the shift should cover.

**Why it is guest-visible**: Linux `detect_extended_topology` stores the core
sub-leaf's EAX in a variable named `core_plus_mask_width` — "core **plus**" —
and computes `phys_proc_id = initial_apicid >> core_plus_mask_width`. Bochs
assigns APIC ids densely from the CPU index (`cpu/apic.cc`
`bx_local_apic_c::bx_local_apic_c`, `apic_id = id`), so `count=2:4:2` packs them
as `socket:1 | core:2 | thread:1`. Shifting by 2 instead of 3 leaves the top
core bit inside the package id:

| APIC id | 0–3 | 4–7 | 8–11 | 12–15 |
|---|---|---|---|---|
| package id, current | 0 | 1 | 2 | 3 |
| package id, expected | 0 | 0 | 1 | 1 |

A 2-socket guest enumerates as 4 packages, so every scheduler and NUMA decision
derived from `phys_proc_id` is placed against a topology that does not exist.

**Fix**:

```cpp
case 1:
   leaf->eax = ilog2(ncores*nthreads-1)+1;
   leaf->ebx = ncores * nthreads;
```

Sub-leaf 2's `ilog2(nprocessors-1)+1` is the shift *above* the package, where no
higher level is enumerated, so nothing depends on it.

**Not reproduced in rusty** — see divergence D2 in
`docs/bochs-parity-divergences.md`, pinned by
`cpuid_leaf_b_core_shift_separates_sockets_as_software_reads_it`
(`cpu/soft_int.rs`), which fails against the upstream formula.

---

## Fast REP string bursts defer timer interrupts by up to a page of elements

**Severity**: Timing divergence from real hardware, guest-visible as interrupt
latency.

**Location**: `cpu/io.cc` — `FastRepINSW`, `FastRepOUTSW`, and their callers
`INSW32_YwDX` / `OUTSW32_DXXw`; `cpu/faststring.cc` — `FastRepMOVSB`.

**Requires**: `BX_SUPPORT_REPEAT_SPEEDUPS`.

**Root cause**: the caller runs the whole burst before any clock movement:

```cpp
wordCount = FastRepINSW(edi, DX, wordCount);
if (wordCount) {
  BX_TICKN(wordCount-1);
  ...
}
```

`FastRepINSW` bounds `wordCount` by ECX and by the words remaining in the
destination page, and no further. Its inner loop does check

```cpp
if (BX_CPU_THIS_PTR async_event) break;
```

but virtual time has not advanced at that point, so a timer whose deadline falls
inside the burst has not fired and `async_event` is not set on its account. The
check can only observe an event that was already pending on entry.

**Why it diverges from hardware**: x86 REP string instructions are
architecturally interruptible *between iterations* — RIP stays on the prefix and
RCX/RSI/RDI carry the progress, so a pending interrupt is taken at the next
iteration boundary. Real hardware does not defer an interrupt to the end of a
2048-element REP. Under the current model a `REP INSW` of 2048 words with a PIT
deadline 300 ticks out delivers that interrupt when the clock lands at 2048 —
1748 ticks late. The overrun scales with the burst, bounded by the page-fit
limit rather than by anything the guest chose.

**Fix**: bound the burst additionally by the ticks remaining to the next event,
and advance the clock as the burst proceeds rather than once at the end, so the
existing `async_event` check can observe a deadline landing inside the burst.

**Not reproduced in rusty** — see divergence D3 in
`docs/bochs-parity-divergences.md`. Cost there is one extra `min` per chunk.

---

## The legacy 8259's INTR bypasses the local APIC, so LVT0 (LINT0) gates nothing

**Severity**: Fidelity bug, guest-observable, and able to make a BROKEN guest
configuration look healthy — see "Why it matters" below. Not a crash: Bochs is
internally consistent and boots guests correctly.

**Location**:
- `iodev/pic.cc` — `bx_pic_c::service_master_pic()` reaches `BX_RAISE_INTR()`.
- `pc_system.cc` — `bx_pc_system_c::raise_INTR()`:
  `BX_CPU(BX_BOOTSTRAP_PROCESSOR)->raise_INTR()`.
- `cpu/event.cc` — `BX_CPU_C::raise_INTR()`:
  `signal_event(BX_EVENT_PENDING_INTR)`.
- `cpu/event.cc` — `BX_CPU_C::interrupt_acknowledge()`.
- `cpu/apic.cc` — `bx_local_apic_c::reset()`: `lvt[i] = 0x10000; // all LVT are masked`.

### What Bochs does

The 8259's INTR output is wired **directly to the CPU's event word**. The local
APIC is not in the path at all:

```
iodev/pic.cc     BX_RAISE_INTR()
pc_system.cc     BX_CPU(BX_BOOTSTRAP_PROCESSOR)->raise_INTR()
cpu/event.cc     signal_event(BX_EVENT_PENDING_INTR)
```

and the acknowledge treats the local APIC and the PIC as two *parallel* sources
of one event, the APIC winning ties:

```cpp
// cpu/event.cc  BX_CPU_C::interrupt_acknowledge()
#if BX_SUPPORT_APIC
  if (is_pending(BX_EVENT_PENDING_LAPIC_INTR))
    vector = BX_CPU_THIS_PTR lapic->acknowledge_int();
  else
#endif
    // if no local APIC, always acknowledge the PIC.
    vector = DEV_pic_iac();
```

The comment says "if no local APIC", but the branch is taken whenever the APIC
has nothing pending — including when there *is* an APIC. `lvt[APIC_LVT_LINT0]`
is read and written by guests (`cpu/apic.h`: `BX_LAPIC_LVT_LINT0 = 0x350`) and
is consulted by nothing.

### What the architecture says

Intel SDM Vol. 3A, Local APIC chapter:

- The LVT's LINT0 entry selects how an interrupt arriving on the **LINT0 pin**
  is delivered, and the mask bit inhibits that source. "Virtual wire mode" for a
  legacy 8259 *is* LVT0 programmed to ExtINT, unmasked — see also Intel MP
  Specification 1.4, Virtual Wire Mode.
- ExtINT delivery mode makes the processor respond "as if the interrupt
  originated in an externally connected (8259A-compatible) interrupt
  controller", with "The external controller is expected to supply the vector
  information". LVT0 is therefore a **gate**; the vector still comes from the
  INTA cycle, not from LVT0's own vector field.
- "Local APIC State After Power-Up or Reset": "The LVT register is reset to 0s
  except for the mask bits; these are set to 1s." Bochs matches this exactly
  (`apic.cc reset()`), which is the other half of the problem below.

### Every other implementation routes through LVT0

- **KVM** — `arch/x86/kvm/lapic.c` `kvm_apic_accept_pic_intr()` gates on mask
  clear **and** delivery mode `== APIC_DM_EXTINT`.
- **QEMU** — `hw/intc/apic.c` gates on the mask bit.
- **Xen** — an exact-equality test over mask + mode, OR'd with two other
  acceptance conditions.

Bochs is the only one of the four that never consults LVT0.

### Why it matters — it can hide a broken I/O APIC

Linux's `check_timer()` (`arch/x86/kernel/apic/io_apic.c`) masks LVT0
deliberately — writing `APIC_LVT_MASKED | APIC_DM_EXTINT` — and **holds that
state across the whole I/O-APIC-routed portion of the timer probe, while IRQ0 is
still enabled in the 8259**, precisely so the virtual wire cannot deliver and
the probe measures the I/O APIC alone.

On Bochs those PIC interrupts arrive anyway. So `timer_irq_works()` can succeed
on the strength of the direct wire while the I/O APIC path is in fact broken,
and Bochs reports a working I/O APIC timer where hardware reports a broken one.
For an emulator used to bring up and debug operating systems, a divergence that
makes a failing configuration look healthy is worse than one that fails loudly.

### Why it is nonetheless self-consistent, and what a fix must include

Bochs resets every LVT entry masked (correct per the SDM) **and its own BIOS
never programs LVT0** — the only local-APIC MMIO writes in the entire `bios/`
tree are `APIC_SVR` (software-enable) and `APIC_ICR_LOW` (INIT/SIPI); `bios/
rombios.h` does not even define a constant for offset 0x350. So simply routing
the PIC through LVT0 would suppress every legacy interrupt from reset onward,
because nothing would ever unmask it.

That is not speculation — it is exactly the trap the other projects hit:

- **KVM** ships `KVM_X86_QUIRK_LINT0_REENABLED`, **enabled by default**, a
  self-described deviation from the architecture that presets
  `LVT0 = unmasked ExtINT` on the bootstrap processor at reset, because firmware
  could not be relied upon to do it.
- **QEMU** carried the identical preset — `apic_reset_common():
  s->lvt[APIC_LVT_LINT0] = 0x700;` — until Nadav Amit's 2015 series
  "target-i386: disable LINT0 after reset" removed it. **coreboot broke, and was
  fixed days later.**

Production firmware does program it: EDK2/OVMF, coreboot and SeaBIOS all set
LVT0 to unmasked ExtINT on the BSP during POST. The Bochs BIOS does not, which
is why Bochs could get away with the direct wire.

**A complete fix is therefore two changes, not one**: route the 8259's INTR
through LVT0 *and* preset `lvt[APIC_LVT_LINT0] = 0x700` on the bootstrap
processor in `bx_local_apic_c::reset()`, as KVM does and as QEMU did.

**Not reproduced in rusty** — this is one of the few upstream bugs this port
does *not* inherit. Both halves are implemented: the routing at
`BxCpuC::set_legacy_intr_level` (`cpu/cpu.rs`) and the INTA in
`acknowledge_external_interrupt` (`cpu/event.rs`), the preset at
`BxLocalApic::preset_lint0` (`cpu/apic.rs`). See divergence D6 in
`docs/bochs-parity-divergences.md` for the argument, pinned by
`a_masked_lint0_refuses_the_legacy_line_without_spending_it` and
`a_fixed_mode_lint0_does_not_acknowledge_the_8259` (`emulator/tests.rs`),
`reset_leaves_the_virtual_wire_on_the_bootstrap_processor_alone` and
`a_software_disable_closes_the_wire_but_reset_does_not` (`cpu/apic.rs`).

---

## KSHIFTLW and KSHIFTRW return zero for a shift count of 15

**Severity**: Correctness bug, guest-visible at exactly one count.

**Location**: `cpu/avx/avx512_mask16.cc` — `BX_CPU_C::KSHIFTLW_KGwKEwIbR` and
`BX_CPU_C::KSHIFTRW_KGwKEwIbR`.

**Root cause**:

```cpp
// cpu/avx/avx512_mask16.cc  KSHIFTLW_KGwKEwIbR
unsigned count = i->Ib();
Bit16u opmask = 0;
if (count < 15)                 // <-- should be count < 16
  opmask = BX_READ_16BIT_OPMASK(i->src()) << count;
```

`KSHIFTRW_KGwKEwIbR` has the same bound in front of `>>`. The SDM's operation
for both instructions shifts whenever the count is at most 15 and zeroes only
above that. The other three widths bound at their own width: `count < 8` in
`avx512_mask8.cc`, `count < 32` in `avx512_mask32.cc`, `count < 64` in
`avx512_mask64.cc`.

**Manifestation**: `KSHIFTLW k1, k2, 15` with bit 0 of `k2` set writes `0`
where the SDM gives `0x8000`; `KSHIFTRW k1, k2, 15` with bit 15 set writes `0`
where the SDM gives `1`.

**Fix**: `if (count < 16)` in both handlers.

**Not reproduced in rusty** — see divergence D8 in
`docs/bochs-parity-divergences.md`. `cpu/avx512_mask.rs`
`kshiftlw_kgw_kew_ib_r` / `kshiftrw_kgw_kew_ib_r` write zero only for
`count >= 16`.

**Filing status**: NOT FILED.

---

## PSRLQ and VPSRAVQ shift a quadword by 64, which C++ leaves undefined

**Severity**: Undefined behaviour. The result a guest sees at one count
depends on the compiler that built Bochs.

**Location**: `cpu/simd_int.h` — `xmm_psrlq` and `xmm_psravq`.

**Root cause**:

```cpp
// cpu/simd_int.h  xmm_psrlq
if(shift_64 > 64) op->clear();  // <-- should be > 63
else
{
  Bit8u shift = (Bit8u) shift_64;
  for (unsigned n=0; n < 2; n++)
    op->xmm64u(n) >>= shift;    // shift == 64: a full-width shift
}
```

`xmm_psravq` has the same `> 64` bound in front of `op1->xmm64s(n) >> shift`.
A count of exactly 64 therefore shifts a 64-bit value by its own width, which
C++ leaves undefined. Every sibling helper bounds at `> 63`: `xmm_psrlvq`,
`xmm_psraq`, `xmm_psllq` and `xmm_psllvq`.

**Reach**: `xmm_psrlq` serves every XMM-register PSRLQ form — SSE2, VEX and
EVEX, with the count in a register or an immediate
(`cpu/decoder/ia_opcodes.def`, `cpu/decoder/ia_opcodes_evex.def`).
`xmm_psravq` serves EVEX VPSRAVQ.

**Expected**: per the SDM, a count above 63 yields zero for PSRLQ and each
element's sign bit, replicated, for VPSRAVQ.

**Fix**: `> 63` in both helpers.

**Rusty Box behavior**: every XMM-register form gives the SDM's answers
(`cpu/sse.rs` `psrlq_vdq_wdq` / `psrlq_udq_ib`; `cpu/avx.rs` `vpsrlq_reg` /
`vpsrlq_imm`; `cpu/avx512.rs` `evex_vpsrlq_imm`, `evex_vpsrlq_reg`,
`evex_vpsravq`) — divergence D9 in `docs/bochs-parity-divergences.md`.
`rusty_box/tests/sse_qword_shift_count.rs` pins the SSE2 forms at a count of
64.

**Filing status**: NOT FILED.

---

## Intel models advertise an architectural PMU whose MSRs do not exist

**Severity**: Guest-visible. A Linux guest finds the PMU broken and falls back
to software perf events.

**Location**:
- `cpu/cpudb/intel/*.cc` — each of the fifteen Intel models that defines
  `get_std_cpuid_leaf_A` returns a nonzero architectural-PMU version in leaf
  0xA EAX, and each of the same fifteen sets `BX_CPUID_STD1_ECX_PDCM` (leaf 1
  ECX bit 15). `corei7_skylake-x.cc` returns EAX `0x07300404` (version 4, four
  48-bit general-purpose counters) and EDX `0x00000603` (three 48-bit fixed
  counters).
- `cpu/msr.cc` — `BX_CPU_C::rdmsr` and `BX_CPU_C::wrmsr` have no case for the
  counters `cpu/msr.h` names (`BX_MSR_PMC0`–`BX_MSR_PMC7`,
  `BX_MSR_PERF_FIXED_CTR0`–`BX_MSR_PERF_FIXED_CTR2`, `BX_MSR_FIXED_CTR_CTRL`,
  `BX_MSR_PERF_GLOBAL_CTRL`), and the `BX_SUPPORT_PERFMON` cases for
  `BX_MSR_PERFEVTSEL0`–`7` log a `BX_INFO` and then call the same handler.
  All of them reach `handle_unknown_rdmsr` / `handle_unknown_wrmsr`, which under
  the default `ignore_bad_msrs=1` (`config.cc`) read 0 and discard writes.

**Observed**: the kernel log carried in bochs-emu/Bochs#791 (stock master
`70da922c`, `corei7_skylake_x`, Ubuntu 26.04, kernel 7.0.0-14-generic) prints

```
Performance Events: PEBS fmt0-, Skylake events, 32-deep LBR, Broken PMU hardware detected, using software events only.
Failed to access perfctr msr (MSR c4 is 0)
```

Linux's probe (`check_hw_exists`, `arch/x86/events/core.c`) writes a counter
MSR and reads it back. The read returns 0, so the kernel declares the hardware
broken.

**Fix**: make the declaration match what is emulated, the way PR #792 did for
the frequency leaves: return zero in leaf 0xA and clear PDCM in every model
whose counters are not implemented. Implementing the architectural counters is
the other way out.

**Rusty Box behavior**: reproduced. The default Skylake-X model inherits both
declarations (`rusty_box/src/cpu/cpudb/intel/core_i7_skylake.rs`: leaf 0xA is
`(0x07300404, 0, 0, 0x603)`, and PDCM is in `LEAF1_ECX_BASE`). In
`cpu/proc_ctrl.rs`, RDMSR returns 0 for `BX_MSR_PMC0..=BX_MSR_PMC7` and
`BX_MSR_PERFEVTSEL0..=BX_MSR_PERFEVTSEL7`, and WRMSR sends both ranges to the
unknown-MSR arm, which drops the write under the default `ignore_bad_msrs`. The
same write-then-read-back probe therefore reads 0 here.

**Filing status**: NOT FILED. One issue covers the whole class.
