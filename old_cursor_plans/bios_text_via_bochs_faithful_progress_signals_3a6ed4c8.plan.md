---
name: bios_text_via_bochs_faithful_progress_signals
overview: Get BIOS/VGABIOS text output by first making progress signals Bochs-faithful (debug ports + POST + VGA I/O), then fixing the earliest Bochs-defined stall point indicated by those signals.
todos:
  - id: print-debug-port-stream-always
    content: Update example output to always print the Bochs debug-port stream section (0xE9/0x402/0x403/0x500), even if empty.
    status: pending
  - id: add-io-activity-probes
    content: Add counters/first-seen probes for key BIOS-polled ports (8042/PIT/PIC/VGA I/O) and print summary at end of headless run.
    status: pending
  - id: fix-stall-root-cause
    content: Based on probe results, align the first mismatching device behavior with Bochs (likely keyboard controller status, PIT readback, or PIC EOI/edge semantics).
    status: pending
  - id: confirm-vga-text-output
    content: After BIOS progresses, confirm VGA memory writes begin and headless VGA dump shows BIOS/VGABIOS text.
    status: pending
isProject: false
---

### Current evidence
- BIOS executes real code (reset vector `EA 5B E0 00 F0` → `JMP FAR F000:E05B`).
- You hit `CS:IP = F000:0000` and then run until instruction limit with no text.
- VGA aperture writes are **zero** (mapped/unmapped both 0) → BIOS never reaches VGA memory updates.
- POST codes on `0x80/0x84` are **none** (may be normal for this BIOS build).
- Bochs reference run (`dlxlinux/bochsout.txt`) reaches VGABIOS init messages around ~15M instructions and has VGA memory handlers installed early.

### Working hypothesis (Bochs-aligned)
We’re stalling **before** VGA init because one of the early I/O polling loops (8042 keyboard controller, PIT, CMOS/RTC, PIC) returns a value that differs from Bochs, causing BIOS to loop, or we corrupt control flow so BIOS falls into an unexpected path (`F000:0000`).

### Plan
#### 1) Restore/print all early-output channels every run
- Ensure the example prints a dedicated section for:
  - **Bochs debug-port stream** (0xE9/0x402/0x403/0x500) even if empty.
  - **POST codes** (0x80/0x84) (already added) but ensure it always prints.

#### 2) Add Bochs-faithful “I/O activity probes” (no behavior change)
- Add lightweight counters + first-seen values for **key BIOS-polled ports**, without changing device behavior:
  - 8042: `0x64` (status), `0x60` (data)
  - PIT: `0x40`..`0x43`
  - PIC: `0x20/0x21`, `0xA0/0xA1`
  - VGA *I/O* init (not memory): `0x3C0/0x3C1`, `0x3C4/0x3C5`, `0x3CE/0x3CF`, `0x3D4/0x3D5`
- Print a compact summary at end of headless run so we can compare against expected Bochs early behavior.

#### 3) Fix the first Bochs-defined stall based on probes
- If BIOS is spinning on 8042 status bits:
  - Align `keyboard` device port returns and state machine with Bochs.
- If BIOS is spinning on PIT reads or never sees timer progress:
  - Align PIT counter readback and mode behavior with Bochs’ 8254 model.
- If BIOS is frequently acknowledging PIC but never progresses:
  - Align PIC IRR/ISR edge/level behavior and EOI handling.

#### 4) Only after BIOS progresses: VGA text output
- Once probes show VGA I/O init begins, we should see either:
  - VGA memory writes start (your existing VGA write probe becomes non-zero), or
  - Bochs debug-port output appears.
- Then confirm text output via:
  - headless VGA dump (already fixed to use CRTC start address), and/or
  - TermGui refresh.

### Files likely touched
- `rusty_box/examples/dlxlinux.rs` (always print debug-port stream section)
- `rusty_box/src/iodev/mod.rs` (always-on capture formatting + optional port activity counters)
- `rusty_box/src/iodev/keyboard.rs`, `pit.rs`, `pic.rs` (only after probes identify the mismatch)
- `rusty_box/src/iodev/vga.rs` (I/O init counters if needed)

### Success criteria
- Within a run, we see at least one of:
  - non-empty debug-port output, or
  - non-zero VGA mapped writes, followed by non-blank headless VGA text dump.
