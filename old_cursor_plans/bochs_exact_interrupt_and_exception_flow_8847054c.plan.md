---
name: bochs_exact_interrupt_and_exception_flow
overview: Align the emulator run loop and CPU exception behavior with Bochs so BIOS progresses past early init and produces observable output (debug ports and/or VGA text).
todos:
  - id: bochs-exception-longjmp
    content: "Make `exception()` match Bochs: restore RIP/RSP for FAULTs, implement is_exception_OK DF escalation, and force decode restart (Bochs longjmp) without continuing execution on errors."
    status: pending
  - id: irq-delivery-active
    content: Change `run_interactive()` to inject PIC interrupts at instruction boundaries when IF=1 (not only during HLT), mirroring Bochs interrupt delivery timing.
    status: pending
  - id: vga-write-proof
    content: Add a minimal VGA write counter/flag and print it in `dlxlinux.rs` headless summary to confirm BIOS/VGABIOS is writing into VGA aperture.
    status: pending
  - id: run-headless-validate
    content: Run headless and verify we now get either debug-port output, VGA text output, or a deterministic fault marker instead of silent RIP=0.
    status: pending
isProject: false
---

### What we know

- BIOS fetch/execute is correct (reset vector bytes show `JMP FAR F000:E05B`).
- Execution reaches `CS:IP = F000:0000` (your debug print) and then runs until instruction limit with no VGA text.
- Bochs differs from current behavior in two critical places:
- **Exception control flow**: Bochs `longjmp`s back to the main decode loop after delivering an exception (`cpp_orig/bochs/cpu/exception.cc:1052`). Continuing after an exception corrupts control flow.
- **Interrupt delivery**: Bochs checks/delivers pending interrupts at instruction boundaries, not only when halted. Injecting IRQs only when `HLT` can stall BIOS that expects timer IRQs while actively executing.

### Changes to make (mirroring Bochs)

- **A) Finish Bochs-style exception semantics (no “continue on error”)**
- Ensure `rusty_box/src/cpu/exception.rs` matches Bochs’ `exception()` sequence:
  - Use `prev_rip`/`prev_rsp` restore for FAULT exceptions.
  - Apply Bochs `is_exception_OK` double-fault escalation table.
  - After delivering the exception via `interrupt_real_mode`/`protected_mode_int`, force a restart of the decode loop (Bochs `longjmp`).
- Ensure `rusty_box/src/cpu/cpu.rs` never silently continues on `Err(_)` from instruction execution. Only two allowed paths:
  - `CpuLoopRestart`: restart decode/trace immediately.
  - other errors: propagate as a hard stop so we don’t drift into bogus state.

- **B) Deliver PIC interrupts while CPU is Active (Bochs-like)**
- Update `rusty_box/src/emulator.rs` `run_interactive()` interrupt injection condition:
  - From current: only inject when `cpu.is_waiting_for_event()`
  - To: inject whenever:
  - `has_interrupt()`
  - `cpu.get_b_if() != 0`
  - CPU isn’t in an interrupt-inhibited window (if you model it; otherwise inject as Bochs does for early bring-up).
- Keep the memory-bus wiring around `inject_external_interrupt()` as you already do.

- **C) Add minimal observability to confirm progress**
- Keep always-on debug port stream (0xE9/0x402/0x403/0x500).
- Add a single counter/flag in VGA mem handler (`rusty_box/src/iodev/vga.rs`) to confirm BIOS is actually writing to VGA aperture:
  - e.g., increment on first write to mapped window; dump that in `dlxlinux.rs` headless summary.

### Test plan (what you should see)

- Running `RUSTY_BOX_HEADLESS=1` should show at least one of:
- Bochs debug-port output beyond our own breadcrumbs, or
- Non-empty VGA text dump, or
- A deterministic stop reason (e.g., triple fault marker) instead of “silent RIP=0”.

### Key files to change

- `rusty_box/src/emulator.rs` (interrupt delivery condition)
- `rusty_box/src/cpu/exception.rs` (Bochs exception flow + restart)
- `rusty_box/src/cpu/cpu.rs` (no continue-on-error; handle restart consistently)
- `rusty_box/src/iodev/vga.rs` + `rusty_box/examples/dlxlinux.rs` (one-time VGA-write confirmation, printed in headless mode)