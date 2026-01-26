---
name: fix_hlt_timer_interrupts_for_bios_progress
overview: "BIOS appears to HLT/wait forever because virtual time and IRQ delivery are not wired: DeviceManager.tick() never advances PIT/RTC, Emulator.run_interactive() never calls tick_devices(), and the CPU never gets an external interrupt to wake from HLT. This plan wires Bochs-like time/interrupt flow so BIOS/VGABIOS can progress and eventually write VGA text."
todos:
  - id: devices_tick_advance
    content: Fix `DeviceManager::tick(usec)` to actually advance PIT/RTC (`pit.tick(usec)`, `cmos.tick(usec)`) and raise IRQ0/IRQ8 appropriately.
    status: completed
  - id: run_interactive_tick
    content: In `Emulator::run_interactive`, advance virtual time each batch and call `tick_devices(usec)` based on executed instructions and configured IPS.
    status: completed
  - id: interrupt_injection
    content: Inject pending PIC interrupts into the CPU (wake from HLT) by calling `iac()` and invoking the correct CPU interrupt path (real/protected).
    status: completed
  - id: headless_debug_option
    content: Add a temporary headless mode to `examples/dlxlinux.rs` (env var) to avoid terminal repaint and to dump port 0xE9 / VGA text for validation.
    status: completed
isProject: false
---

## Diagnosis (why you see endless cursor jumping)
- `TermGui` repaints every ~100ms, so the cursor constantly moves even if the screen is blank.
- Your run **never finishes** because the CPU likely enters `HLT` waiting for IRQ0 (PIT timer), but:
  - [`rusty_box/src/iodev/devices.rs`](rusty_box/src/iodev/devices.rs) `DeviceManager::tick(usec)` **does not call** `pit.tick(usec)` / `cmos.tick(usec)`; it only checks `check_irq*()`.
  - [`rusty_box/src/emulator.rs`](rusty_box/src/emulator.rs) `run_interactive()` **never calls** `tick_devices(usec)` at all.
  - [`rusty_box/src/cpu/event.rs`](rusty_box/src/cpu/event.rs) `handle_wait_for_event()` explicitly returns to caller and contains TODOs for pending interrupt checks.

This matches the observed symptom: each batch returns quickly, the GUI refresh loop continues forever, and BIOS never progresses to VGA output.

## Bochs source-of-truth behavior
- In Bochs, `HLT`/wait states advance time (tick) until an interrupt becomes pending, then the CPU wakes and services it (`event.cc:handleWaitForEvent` + device timers + PIC).

## Implementation plan
### 1) Make device ticking real (PIT/RTC)
Update [`rusty_box/src/iodev/devices.rs`](rusty_box/src/iodev/devices.rs):
- In `DeviceManager::tick(usec)`, call:
  - `self.pit.tick(usec)` and when it indicates IRQ0 should fire, do `self.pic.raise_irq(0)`.
  - `self.cmos.tick(usec)` and when it indicates IRQ8 should fire, do `self.pic.raise_irq(8)`.
- Keep existing IRQ checks (keyboard/harddrv), but ensure timer-driven devices actually advance.

### 2) Advance virtual time in the main loop
Update [`rusty_box/src/emulator.rs`](rusty_box/src/emulator.rs) `run_interactive()`:
- After each CPU batch returns `executed`, compute elapsed emulated time:
  - `usec = executed * 1_000_000 / ips` (use `self.config.ips`).
- Call `self.tick_devices(usec)` every batch.

### 3) Deliver external interrupts to the CPU (wake from HLT)
In `run_interactive()` after ticking devices:
- If `self.has_interrupt()` and CPU IF=1, call `let vector = self.iac();` then inject it into the CPU:
  - In real mode: `cpu.interrupt_real_mode(vector)`.
  - In protected mode: call the existing protected-mode path (similar to `exception()`), e.g. `cpu.protected_mode_int(vector, false, false, 0)`.
- Also ensure the CPU wakes from `HLT`:
  - Set `cpu.activity_state = Active` when delivering the interrupt.

### 4) Optional: headless debug mode (to avoid repaint while validating)
Since you approved headless debugging, add a simple switch in [`rusty_box/examples/dlxlinux.rs`](rusty_box/examples/dlxlinux.rs) (e.g. env var `RUSTY_BOX_HEADLESS=1`) to use `NoGui` and periodically dump:
- drained port `0xE9` output
- current VGA text buffer (once it becomes non-empty)

This will make it obvious whether BIOS is progressing even before the terminal GUI looks correct.

## Expected result
- CPU will no longer “busy-loop” around `HLT`.
- BIOS/VGABIOS will progress, generate timer IRQs, and eventually write VGA text memory.
- `TermGui` will repaint meaningful content instead of an always-blank screen.