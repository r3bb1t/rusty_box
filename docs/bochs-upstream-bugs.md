# Bochs Upstream Bugs — Device Models & Timers

Genuine bugs in upstream Bochs (`cpp_orig/bochs/`) discovered during the
Rusty Box Bochs-parity work on the HPET, i8042, PIT/CMOS, and BIOS message
devices (2026-07). Each entry is written to be filed directly as an upstream
issue. Line numbers are from the vendored snapshot and may rebase; the file +
symbol is authoritative.

Rusty Box intentionally does **not** reproduce these bugs (it implements the
architecturally correct behavior); the divergences are documented at each
Rusty Box call site.

Related bug inventories:
- `docs/bochs_bugs_found.md` — AVX-512 handler bugs (VPCONFLICT off-by-one, KSHIFT threshold, VPSRLQ shift-by-64 UB).
- `docs/bochs-cpudb-cpuid-audit-2026-07-13.md` — 13 verified CPUID/cpudb bugs (2 guest-breaking).
- `docs/bochs-upstream-issue-tsc-cpuid.md` — TSC/CPUID max-leaf issue (filed as bochs-emu/Bochs#791).

---

## 1. HPET interrupt routes 16–23 corrupt the slave PIC (and can panic the host)

**Severity**: Correctness bug + host abort. A guest can inject a phantom ISA
IRQ (8–15) or crash the emulator by programming an HPET timer's interrupt
route to a legal, advertised GSI in 16–23.

**Location**:
- `iodev/hpet.cc` — `bx_hpet_c::update_irq()` (the non-legacy routing path).
- `iodev/pic.cc` — `bx_pic_c::raise_irq()` / `bx_pic_c::lower_irq()`.

**Root cause**: For a non-legacy HPET timer (or timer 0/1 outside legacy
mode), `update_irq()` computes `route = timer_int_route(timer)` and calls
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
    ...
    if (DEV_ioapic_present() && (irq_no != 2)) {
      DEV_ioapic_set_irq_level(irq_no, 1);            // <-- also forwards the (correct) IOAPIC pin
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

**Rusty Box behavior**: delivers only the correct IOAPIC pin for routes
16–23; documented in `rusty_box/src/emulator.rs` `drain_hpet_pending`. The
deviation was **permanently ratified** on 2026-07-25 — it is a closed decision,
not an open item.

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
restore) for parity — see `rusty_box/src/iodev/hpet.rs` `restore_snapshot_v3`.
