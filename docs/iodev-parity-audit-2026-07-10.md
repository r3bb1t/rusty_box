# I/O Device Subsystem Parity Audit — rusty_box vs Bochs

Date: 2026-07-10. Method: read-only diff of each `rusty_box/src/iodev/` device
against its `cpp_orig/bochs/bochs/iodev/` counterpart, register/command handler
by handler. Confidence CONFIRMED = both sides read and verified.

Excluded (already settled elsewhere): the BM-DMA parity work, the VGA PCI
feature, the PCI command-register BAR-gating gap, the benign keyboard warnings,
the deliberate tracing-level demotion, and the synchronous-command port
convention.

---

## TIER 1 — Guest-breaking or host-DoS

1. **CMOS: guest-triggerable host panic/hang on malformed date registers** —
   `cmos.rs update_timeval()` indexes `DAYS_IN_MONTH[m-1]` with a
   guest-controlled month (>12 → OOB panic) and `mday - 1` underflows u64 for
   mday=0 (release: timeval ≈ 2^64 makes `update_clock()`'s year loop spin
   ~10^11 iterations). Bochs `cmos.cc update_timeval` normalizes via `timeutc()`
   and clamps in `update_clock`. A guest writing month=0x13/mday=0 then clearing
   CRB.SET panics or hangs the emulator. CONFIRMED. **small**.

2. **Port 0xCF9 (PIIX3 reset control) never registered** — Bochs `pci2isa.cc
   init` registers r/w at 0x0CF9; Rust `devices.rs register_pci_handlers`
   registers only 0xB2/0xB3/0x4D0/0x4D1 and neither `pci_io_read` nor
   `pci_write` routes 0x0CF9. The `pci2isa.rs` 0xCF9 handler and the emulator's
   `pci2isa.reset_request` drain are complete but unreachable. `OUT 0xCF9,0x06`
   (Linux `reboot=p`, ACPI/UEFI reset fallbacks) is silently swallowed; reads
   return 0xFF. CONFIRMED (verified end-to-end). **one-liner** (register + two
   match arms).

3. **ELCR writes never reach the PIC — everything stays edge-triggered** — Bochs
   `pci2isa.cc` `case 0x04d0: ... DEV_pic_set_mode(1, elcr1)`. Rust
   `pci2isa.rs write` sets `elcr1_changed`/`elcr2_changed` which nothing drains;
   `BxPicC::set_mode` has zero callers; the PIC's own 0x4D0/0x4D1 handler is
   dead because `register_pci_handlers` silently overwrites the earlier PIC
   registration. Readback is correct but `edge_level` stays 0, so `iac()` always
   clears IRR on ack. Level-triggered ACPI SCI (IRQ9) and shared PIRQ in 8259
   mode get one delivery per edge → lost/stuck ACPI events. CONFIRMED. **small**.

4. **ACPI PM-timer-overflow SCI never fires** — Bochs `acpi.cc pm_update_sci`
   schedules the TMROF deadline and `timer()` re-runs it. Rust `acpi.rs tick()`
   only advances `time_usec`; `pm_update_sci` is called only from PM1_STS/PM1_EN
   writes, so with TMROF_EN set `irq9_level` never rises at overflow. Polling
   PM1_STS still works; an OS waiting on the interrupt hangs. CONFIRMED.
   **small**.

5. **VGA: write to 0x3CC applied as a Misc Output write** — Bochs `vgacore.cc
   write` `case 0x03cc: // ignore`. Rust `vga.rs write_port` `VGA_MISC_OUTPUT =>`
   performs a full Misc Output write incl. bit 0 + retrace recalc. EGA-era
   guests writing "Graphics 1 Position"=0x00 at 0x3CC flip the adapter to mono
   (CRTC → 0x3B4, 0x3Dx reads → 0xFF). CONFIRMED. **one-liner**.

6. **VGA: index-register masking causes register aliasing on SVGA probes** —
   Rust masks CRTC index `& 0x1F` (Bochs `& 0x3f`), SEQ `& 0x07`, GFX `& 0x0F`
   (Bochs stores unmasked; out-of-range data writes are no-ops).
   `outb(0x3C4,8); outb(0x3C5,v)` hits sequencer reset in Rust vs ignored in
   Bochs; CRTC index 0x22 aliases to CR2 and Bochs's latch read-back at 0x22 is
   missing. Extension-probing guests silently corrupt core state. CONFIRMED.
   **small**.

7. **VGA: CR11 bit-7 write protection of CRTC 0-7 missing entirely** — Bochs
   drops writes to CR0-CR6 when write-protect is set and lets CR7 update only
   bit 4; Rust accepts all writes and never tracks CR11.7. CONFIRMED. **small**.

8. **i440FX SMRAM register works but memory never switches** — `pci.rs
   smram_control` arithmetic is parity, but Bochs calls
   `enable_smram(DOPEN,DCLS)/disable_smram()`; Rust only logs.
   `smram_available` in `memory/misc_mem.rs` is never set true, so A0000 DRAM
   aliasing for SMM can never open. CONFIRMED. **small** (add
   `BxMemC::enable_smram`, drain via a PAM-style deferred flag).

9. **CMOS: time writes outside SET mode never take effect; century reg 0x37
   missing** — Bochs `cmos.cc write`: `if (CRB & 0x80) timeval_change=1; else
   update_timeval();`. Rust has only the SET branch — the RTC reverts a
   guest clock-set within 1 s. Also PS/2 century register 0x37 (Bochs mirrors
   0x37↔0x32 in write and update_clock; "critical in getting WinXP to run") is
   absent. Both CONFIRMED. **small**.

## TIER 2 — Guest-observable, moderate

10. **harddrv: abort paths that skip `command_aborted` never raise IRQ** —
    `execute_command` READ/WRITE MULTIPLE `multiple_sectors==0` arms and the
    READ-MULTIPLE read-failure arm set `error=ABRT; status=ERR|DRDY` manually
    with no IRQ, no `current_command=0`, no `buffer_index=0`; the data-phase
    `ide_write_sector` failure path likewise. Bochs routes all through
    `command_aborted` (which raises the interrupt). A guest waiting on the IRQ
    hangs/times out. CONFIRMED. **small**.

11. **harddrv: LBA48 command gaps** — Bochs implements 0x29 READ MULTIPLE EXT,
    0x39 WRITE MULTIPLE EXT, 0x42 READ VERIFY EXT; Rust aborts all three
    (comments say "0x29 EXT not yet" — a no-partial-work violation). Also
    missing 0x27/0xF8 READ NATIVE MAX (EXT), 0x37/0xF9 SET MAX (latent —
    neither side advertises HPA). CONFIRMED. **small**.

12. **Serial: no TX/RX/FIFO timers** — Bochs paces THR→TSR via `tx_timer`, sets
    DR only at trigger/`fifo_timer`, raises CTI once per idle gap. Rust
    transmits instantly (LSR always 0x60), sets DR + CTI per byte below
    trigger, `fifo_timeout_ticks` declared but never read ("Timer integration
    deferred" — law violation). Also RX trigger `>=` vs Bochs `==`; non-FIFO
    overrun drops the new byte and skips RDA (Bochs overwrites RBR + raises);
    IER write emits an unconditional raise/lower pulse. CONFIRMED. **subsystem**
    (timers), small for the rest.
    - **TX timer RESOLVED 2026-07-24** — ported Bochs `serial.cc tx_timer`: a THR
      write now moves the byte into the shift register and arms a per-UART
      one-shot for `databyte_usec` instead of transmitting instantly; the timer
      fire emits the byte, reloads the next FIFO/THR byte, and raises THRE once
      per byte *actually transmitted* (fixing the 515-vs-242 THRE count). LSR now
      reflects `tsr_empty`/`thr_empty` mid-transmission. Wiring: `TimerOwner::
      SerialTx` + `DeviceTimerOwner::SerialTx`, snapshot v7. The RX FIFO-timeout
      timer (`fifo_timer`) was wired in an earlier pass. **Still open:** RX byte
      *arrival* stays immediate (not baud-paced — a deliberate, separate
      divergence); the RX-trigger `>=`-vs-`==`, non-FIFO overrun RDA, and
      IER-pulse sub-items are untouched by this change and need their own review.

13. **Serial: batched IRQ actions reordered raise-before-lower** —
    `take_pending_irqs` always yields raise then lower regardless of chronology;
    an RBR read (lower) then a new byte (raise) in the same tick nets the line
    LOW with `rx_interrupt` still true — a purely interrupt-driven guest can
    stall on the last byte. CONFIRMED structural, loss timing-dependent.
    **small**.

14. **Keyboard: `periodic()` never calls `create_mouse_packet(0)`** — Bochs
    flushes residual `delayed_dx/dy` when the kbd buffer is idle; Rust omits it,
    so clamped/deferred mouse motion sticks until the next host event → pointer
    lag/jump, likely felt on the current GUI branch. CONFIRMED. **one-liner**.

15. **Keyboard: IRQ for a just-transferred byte delivered in the same periodic
    call** — Bochs snapshots `retval` at entry; the new `irq1_requested` reaches
    the PIC one serial-delay later. Rust ORs it into the current return →
    polling guests take a spurious IRQ1 with OBF already 0, and the flag
    re-raises next tick. CONFIRMED. **small**.

16. **PIT: port writes don't advance the counter to "now" first** — Bochs
    `pit.cc write` runs `periodic(time_passed)` before `timer.write(...)` (same
    for port 0x61 GATE2); Rust `pit.rs write` has no icount and never syncs
    (reads do). Elapsed ticks replay under the new program → first-period skew,
    wrong calibration reads (Linux `pit_calibrate_tsc`). CONFIRMED. **small**.

17. **PIT: bulk-skip fast path miscounts mode 3 and freezes BCD** — Bochs
    `clock_multiple` decrements mode 3 by `2*cycles`; Rust decrements 1/tick
    (count too high, can be odd, square wave ~2× slow). BCD counters return
    without decrementing while the caller consumes the ticks. Reachable when all
    three counters are actively counting. CONFIRMED. **small**.

18. **PIT: port 0x61 semantics** — bit 4 toggles on every read (Bochs derives
    `(usec/15)&1`), bits 2/3/6/7 echo writes and bit 0 initially reads 0 (Bochs
    composes fresh: GATE2=1 after init). BIOS refresh-loop delays collapse to
    ~0. CONFIRMED. **small**.

19. **CMOS: reset doesn't mask CRB bits 4-6 or stop the periodic timer** — Bochs
    `reset` does `reg[STAT_B] &= 0x8f` + `CRA_change()`; Rust clears only
    address/STAT_C → IRQ8 keeps firing into the fresh boot after guest reset.
    Related: periodic timer not restarted on STAT_A rewrites; UF/AF set even
    when UIE/AIE disabled; checksum auto-recomputed on guest writes to
    0x10-0x2D (Bochs never does from I/O). All CONFIRMED. **small** each.

20. **i440FX: PAM survives guest reset; status-high w1c wrong** — (a) Bochs
    `reset` re-applies memory type for all PAM areas; Rust zeroes the config
    bytes but never re-applies → post-reboot C0000-FFFFF stays shadow-RAM.
    **one-liner** (`pam_needs_update=true` in `DeviceManager::reset`). (b)
    Config 0x07 write: Bochs nets `(old & written) & 0xFD`; Rust computes
    `(old & !written) | 0x02`. **one-liner**. Both CONFIRMED.

21. **PCI: common read-only filter missing** — Bochs
    `pci_write_handler_common` drops writes to 0x00-0x03/0x08-0x0B/0x0E/0x3D
    before any device handler; Rust dispatches straight to devices whose default
    arm stores → vendor/device ID, class, header type, intpin all
    guest-writable on every function. Bochs also stores only one byte for 0x3C
    regardless of io_len. CONFIRMED. **small**. (Related to the already-known
    command-register gating gap but distinct.)

22. **DMA: word I/O dropped; registration mask 1 vs Bochs 7** — Bochs `dma.cc
    init` registers 0x00-0x0F/0x80-0x8F/0xC0-0xDE with mask 7 and splits 16-bit
    accesses (incl. the 0x0B word-write → 0x0C flip-flop-clear case); Rust uses
    mask 0x1, so `OUT dx,ax` vanishes and word reads return 0xFFFF. The
    io_len-split code in `dma.rs` is dead. CONFIRMED. **small**. Related DMA
    items: master clear doesn't reset channel mode registers; HRQ→CPU is a
    `tracing::trace!("would assert HRQ")` stub (`set_hrq` has no callers → no
    ISA DMA can complete — latent, law violation); handler-less read advances by
    `maxlen` vs Bochs 1; `current_count + 1` overflow panics in debug at
    count=0xFFFF; DMA memory access bypasses A20/handlers/SMC-invalidations
    (latent).

23. **PIC: reset/init gaps** — (a) ICW1 to master never signals
    `BX_CLEAR_INTR()` (Rust handles only the slave side) → stale PENDING_INTR
    latch (masked today by double-gating on `has_interrupt()`). (b) `reset()`
    misses `polled`, `read_reg_select`, `special_mask`, `auto_eoi`,
    `lowest_priority`, `interrupt_offset`, `edge_level` → wrong first reads of
    0x20 after guest reboot. CONFIRMED. **one-liner/small**.

24. **A20 state is tri-latched, reads go stale** — Bochs port 0x92 read and KBC
    output-port 0xD0 both return live `BX_GET_ENABLE_A20()`. Rust:
    `SystemControlPort::read` returns its own latch (not updated on KBC A20
    changes), and keyboard 0xD0 returns `self.a20_enabled` (not updated on
    port-92 changes). A loader toggling A20 via one interface and reading via
    the other sees stale. CONFIRMED. **small**.

25. **harddrv: taskfile reads for absent drives** — Bochs answers 0x1F1-0x1F5
    from the selected drive's registers when any drive is present on the channel
    (0 if none) and composes 0x1F6 unconditionally; Rust returns 0xFF whenever
    the selected drive is absent. Slave-probe sequences observe different
    values. Commands to an absent selected master are silently dropped (Bochs
    aborts-with-IRQ; CALIBRATE returns error 0x02). CONFIRMED. **small**.

26. **harddrv: port registration masks** — Bochs registers the data port with
    mask 6 (16/32-bit only) and 0x1F1-0x1F7 with mask 1; Rust registers all of
    0x1F0-0x1F7 with 0x7 (8-bit read of 0x1F0 consumes a buffer byte vs Bochs
    0xFF+BX_ERROR; word reads of non-data ports reach the handler). Bochs also
    registers 0x3F7/0x377 when no floppy is present; Rust doesn't. CONFIRMED.
    **small**.

27. **ATAPI: 0x42 READ SUB-CHANNEL missing** — Bochs implements it; Rust's
    default arm returns ILLEGAL_OPCODE. CD status ioctls fail. CONFIRMED.
    **small**.

28. **APM port 0xB2** — Rust force-clears `apms=0` after each command (Bochs
    never does — the APM handshake polls 0xB3) and registers 0xB2 with mask 1 vs
    Bochs mask 3 (word writes to SMI_CMD dropped). CONFIRMED. **one-liners**.

29. **ACPI sleep states** — S5 (`sus_typ=0`): Bochs terminates the simulation;
    Rust logs and keeps running. S3 (`sus_typ=1`): Bochs writes CMOS 0xF=0xFE +
    `Reset(HARDWARE)`; Rust only sets RSM_STS|PWRBTN_STS. `poweroff`/S3 are
    silent no-ops. CONFIRMED. **small**.

30. **VGA rendering-semantics cluster (all CONFIRMED, small unless noted)** —
    sequencer regs have no reset side effects / no clear_screen bit / read-back
    unmasked; **guest font loads invisible** (fixed built-in font;
    `charmap_address` never tracked — **subsystem**); `skip_update()` gating
    absent (no blank during mode-set/reset/video-disable); retrace recalc
    missing on CR0/CR2 writes; Feature Control absent + Input Status 0 reads
    0xFF vs Bochs 0x00; 16-bit port reads not split (`inw(0x3D4)` returns index
    only); attribute-controller data masks + redraw flags missing; writes
    accepted at 0x3C1 (Bochs ignores — makes the common `outw 0x3C0` idiom
    double-write); chain-4 LFB-compat path wraps at 128KB vs Bochs 256KB (GRUB
    video save/restore); start-address not latched per-frame (tearing);
    `y_doublescan` from CR9 ignored in classic modes; text palette ignores pel
    mask; 0x3DA timing matches but lacks the per-frame `display_start_usec`
    re-anchor and falls back to `status ^= 0x09` without icount.

## TIER 3 — Minor, latent, or needing a decision

31. **Keyboard details** — mouse 0xEB response bypasses `mouse_internal_buffer`
    (instant, not aux-clock-gated); 0xBB gets an ACK (Bochs sends nothing — OS/2
    probe); **0xE6 sets scaling=1 where Bochs sets 2 (upstream Bochs bug — Rust
    matches real hardware; needs a parity decision)**; internal buffer 256 vs
    Bochs `BX_KBD_ELEMENTS 16` (different typeahead-overflow); **scancode sets
    1/3 selectable but never applied** (GUI feeds raw set-2; subsystem);
    serial-delay pacing = tick cadence vs Bochs's fixed 150 µs. CONFIRMED.

32. **PIT details** — guest reset reinitializes counters (Bochs `reset()` is
    empty); port 0x43 read returns 0xFF vs Bochs 0; `PIT_FREQUENCY 1193182` vs
    Bochs `1193181`; OUT transitions from control-word writes generate no IRQ0
    edge; IRQ0 modeled as lower+raise pulse, never mirrors OUT level;
    **mode-3-count-0 gives 18.2 Hz vs Bochs's underflow ~9.1 Hz (Bochs quirk —
    decision needed)**; latched-read corner returns 0 forever; **speaker
    subsystem absent** (flagged per CLAUDE.md). CONFIRMED.

33. **CMOS details** — 0x72/0x73 registered unconditionally vs Bochs only with a
    256-byte image; initial clock UTC vs Bochs local; UIP 244 µs window
    unobservable under the ≥1 ms tick quantum (subsystem); one-second reload
    underflow if a tick exceeds 1 s; PIE-with-disabled-divider corner; divider
    TEST values Bochs BX_PANICs, Rust continues. CONFIRMED/SUSPECTED as labeled.

34. **harddrv details** — 0x31 WRITE SECTORS NO RETRY implemented where Bochs
    aborts; WRITE address not validated at command issue (Bochs aborts before
    the data phase); SEEK 0x70 / READ VERIFY on CD-ROM succeed vs Bochs abort;
    SET MULTIPLE accepts ≤128 vs Bochs `MAX_MULTIPLE_SECTORS` 16;
    `set_signature` side effect `drive_select=0` missing from EXECUTE DIAGNOSTIC
    and SRST-deassert; **0xA1 sets cylinder_no=0xEB14 (added vs Bochs, documented
    ata_piix rationale — decision needed)**; nIEN 1→0 deferred IRQ re-raise
    (workaround tied to the sync-completion convention); HOB cleared only for
    offsets 1-7 vs Bochs any-port; command prologue zeroes the whole error
    register (Bochs clears only status.ERR); IDENTIFY words 1/54 missing the
    16383-cylinder cap; `calculate_logical_address` misses the `<0` check.
    CONFIRMED except where noted.

35. **PCI/ACPI details** — i440FX regs 0x73/0xB4/0xB9-0xBB/0xF0 writable (Bochs
    ignores); PAM apply lacks Bochs's TLB flush (SUSPECTED — Rust TLB caches
    host pointers); XBCS 0x4E/0x4F BIOS write-protect unwired (`misc_mem.rs`
    hardcodes `bios_write_enabled: true` — ROM permanently writable); IOAPIC
    enable/relocate via 0x4F/0x80 unwired; `pci_set_irq` latent deltas (zero
    callers today); ACPI PM/SM BAR size-probe write 0xFFFFFFFF not skipped, old
    port range never unregistered on move, registration deferred a batch (guest
    can read 0xFFFFFFFF from the PM timer mid-batch); `generate_smi` never
    delivers an actual SMI. CONFIRMED except as marked.

36. **Port dispatch (mod.rs)** — port 0x8900 shutdown protocol ("Shutdown"
    string → exit) from `unmapped.cc` missing. CONFIRMED, **small**. (Port
    0x80/0x8e read-back is NOT a divergence — both route 0x80-0x8F to the DMA
    ext-page registers; consequently `port80_output` capture in
    `default_write_handler` is dead code.)

37. **Housekeeping** — `src/iodev/pic.rs` is an orphaned, uncompiled duplicate
    of `src/pic.rs` (only whitespace differs; `mod.rs` re-exports `crate::pic`)
    — delete-candidate.

---

## Verified at parity (do not re-audit)

- **Port dispatch core**: mask-vs-io_len check, unhandled-port
  0xFF/0xFFFF/0xFFFFFFFF, 0xE9 hack, mask-fail fallback, PCI mechanism #1 (0xCF8
  store/readback, enable bit, reg+offset math, absent-devfunc 0xFFFFFFFF,
  dword-only 0xCF8), fw_cfg port set/masks incl. DMA 0x514-0x51B, keyboard
  0x60/0x64 masks, port92 semantics per se.
- **PIC**: full OCW1/OCW2/OCW3 matrix, poll mode, special mask, priority
  rotation, IAC incl. spurious IRQ7/15 and cascade, raise/lower edge detection,
  ELCR value masks/readback, power-on defaults.
- **DMA**: flip-flop, status read-clear, request/mask registers, mode decode,
  page registers, `set_DRQ` boundary math, `raise_hlda` transfer/TC/autoinit/
  cascade incl. the 16-bit len-quirk.
- **PIT**: modes 0/1/2/4/5 state machines; mode 3 in the per-tick path;
  write/read/latch state machines; read-back command; status byte; set_GATE;
  BCD conversions; usec→tick accumulator.
- **CMOS**: 0x70/0x71 semantics, STAT_A/B/C/D bit handling, periodic rate table,
  alarm don't-care, update_clock BCD/12h encoding, checksum range, power-up
  defaults.
- **Serial**: register map/DLAB, IIR priority + clear, IER promotion/demotion,
  FCR/trigger table, FIFO-full overrun, MCR loopback wiring incl. delta bits,
  break-in-loopback, reset values.
- **Keyboard**: status composition, full 0x64 command set, CCB r/w, 0x60 drain
  quirks, enqueue rules, kbd-to-device command set, mouse ACK sequencing, ImPS/2
  knock, packet encoding/remainders, 8042 translation table (all 256 bytes).
- **harddrv**: lba48_transform, calculate_logical_address arithmetic,
  command_aborted status bits, SRST assert/deassert sequences and
  multiple_sectors=0, raise_interrupt nIEN gate + bmdma_set_irq, HOB read-back
  gating, data-port DRQ checks, ATAPI PACKET/IDENTIFY-PACKET protocol, IDENTIFY
  words 0-93/100-103 (incl. BM-DMA-gated 49/63/88), ATAPI command set minus 0x42.
- **PCI**: i440FX identity/reset/masks/PAM mapping/DRB, PIIX3
  identity/reset/PIRQ masks/0xCF9 decode logic itself, ACPI identity/PM-timer
  math/PM1 semantics/SMBus stubs/io-mask tables, deferred-PAM drained same-outp.
- **VGA**: 0x3C0 flip-flop protocol, DAC 3-cycle state machines, memory
  read/write modes 0-3, odd/even, chain-4, window gating/banking, misc-output
  write decode, retrace-timing formulas + clamps, power-on register state.

---

## Highest-leverage fixes

- **#2 / #3 / #4** — reboot (0xCF9) + level-triggered IRQ (ELCR) + PM-timer SCI.
  All interact with ACPI-era guests; together they explain a class of "modern
  OS won't reboot / hangs waiting on an interrupt" symptoms.
- **#1** — host DoS from a malformed guest CMOS write.
- **#5 / #6 / #7** — VGA probe corruption for EGA/SVGA-aware guests.
- **#10** — IDE error-path IRQ omission (guest hangs on a failed command).
- **#14** — mouse lag/jump on the current GUI branch.
- **#16** — PIT counter-sync (breaks TSC/PIT calibration on some kernels).

## Needs an explicit parity ruling (reproduce/fix upstream Bochs bugs)

- **#31** keyboard 0xE6 scaling (Rust matches real hardware, Bochs has 2).
- **#32** PIT mode-3-count-0 rate (Bochs underflow quirk).
- **#34** harddrv 0xA1 cylinder signature (added with documented ata_piix
  rationale) and 0x31 acceptance.
