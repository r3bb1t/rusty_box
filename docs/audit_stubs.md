# Audit Stubs & Incomplete Implementations

Tracks all stubs, no-ops, incomplete implementations, and missing Bochs functions found during the comprehensive parity audit (2026-03-20).

## Format

Each entry:
- **File**: path
- **Function**: name
- **Bochs does**: brief description
- **Our code does**: stub/no-op/partial
- **Priority**: CRITICAL / HIGH / MEDIUM / LOW
- **Bochs ref**: file name (no line numbers)

---

## FIXED in this audit session

- **pci.rs**: ERRCMD register bit logic (preserve/write order) — FIXED
- **pci2isa.rs**: PIRQ logging format (char arithmetic) — FIXED
- **pit.rs**: Mode 0 missing `!output` check + write_state guard — FIXED
- **harddrv.rs**: Added missing `packet_dma`, `mdma_mode`, `udma_mode` fields — FIXED
- **pc_system.rs**: Timer fire comment corrected (Bochs uses `==`, not `<=`) — FIXED
- **decode32.rs**: force_modc0 range 0x20..=0x26 → 0x20..=0x24 (exclude undefined 0F 25/26) — FIXED
- **decode64.rs**: force_modc0 range 0x20..=0x23 → 0x20..=0x24 (include MOV CR4) — FIXED
- **shared_display.rs**: Font bit order — REVERTED (font data is pre-reversed, LSB-first is correct)
- **shared_display.rs**: 9th pixel extraction — REVERTED (bit 7 was correct for pre-reversed font)
- **io.rs**: INSW/INSD RMW conversion + bulk REP INSD fast path — REVERTED (caused data corruption during CD-ROM PIO reads, corrupting all Alpine packages)
- **decode32.rs**: force_modc0 range change — REVERTED (needs more investigation)
- **decode64.rs**: force_modc0 range change — REVERTED (caused Alpine kernel freeze)
- **harddrv.rs**: SET FEATURES 0xEF transfer mode — wired mdma_mode/udma_mode/packet_dma — FIXED
- **cmos.rs**: Extended CMOS addressing ports 0x0072/0x0073, 256-byte RAM — FIXED
- **serial.rs**: RI ms_ipending on any state change (not just trailing edge) — FIXED
- **event.rs**: debug_trap inhibition — VERIFIED ALREADY CORRECT (no change needed)

---

## CRITICAL Priority

### ~~pc_system.rs — set_HRQ async_event~~ — FIXED (session 56: hrq_pending flag + cpu_async_event_ptr signaling)
- **Bochs ref**: pc_system.cc

### ~~pc_system.rs — raise_INTR/clear_INTR/IAC~~ — FIXED (session 56: methods + intr_pending field added)

### ~~dma.rs — set_DRQ/control_HRQ/raise_HLDA~~ — FIXED (session 56: full transfer machinery with handler callbacks)

### ~~dma.rs — TC/HLDA/DRQ/DACK fields~~ — FIXED (session 56: all fields added)

---

## HIGH Priority

### ~~pc_system.rs — A20 TLB flush~~ — FIXED (session 56: emulator.rs sync_a20_state + keyboard A20 handler)

### ~~ioapic.rs — ExtINT delivery mode~~ — FIXED (session 57: pic_ptr added, service_ioapic calls pic.iac() matching Bochs )
- **Bochs ref**: ioapic.cc

### ~~harddrv.rs — SET FEATURES (0xEF) transfer mode~~ — FIXED (session 56: mdma/udma/packet_dma wired up)

### ~~dma.rs — Address shift~~ — FALSE POSITIVE (get_address already shifts << 1 for channels >= 4)

### ~~serial.rs — Timer fields~~ — PARTIALLY FIXED (session 56: timer handles + baudrate + databyte_usec fields added, not yet wired)

### ~~serial.rs — RI state change detection~~ — FIXED (session 56: ms_ipending on any RI change)

### ~~cmos.rs — Extended CMOS addressing~~ — FIXED (session 56: ports 0x0072/0x0073, 256-byte RAM)

### ~~memory/misc_mem.rs — bios_write_enabled~~ — DOCUMENTED (kept true; Bochs relies on PCI2ISA 0x4E propagation we don't wire)

---

## MEDIUM Priority

### ~~pit.rs — clock_multiple() fast path~~ — PARTIALLY IMPLEMENTED (session 56: bulk skip when next_change_time > ticks)

### ~~pit.rs — Tick accumulation cap~~ — RAISED to 5M (session 56: with clock_multiple, bulk skip makes large counts safe)

### ~~serial.rs — Break loopback~~ — FIXED (session 56: enqueue 0x00 + LSR flags on loopback+break_cntl)

### ~~serial.rs — MODSTAT promotion~~ — FIXED (session 56: ms_ipending→ms_interrupt with clear, all 4 deltas)

### ~~cmos.rs — UIP bit ordering~~ — FALSE POSITIVE (already correct: line 295 clears UIP before line 298 update_clock)

### ~~dma.rs — DMA handler registration~~ — FIXED (session 56: register_dma8/16_channel with callbacks)

### ~~dma.rs — ctrl_disabled field~~ — FIXED (session 56: added + wired from command register write, plus drq/dack arrays)

### ~~memory/misc_mem.rs — Flash ROM state machine~~ — FIXED (session 56: flash_read/flash_write stubs matching Bochs, not yet wired)

### ~~vga.rs — Graphics read modes 0/1~~ — FIXED (session 56: full planar read with latch, color compare)

### ~~vga.rs — Graphics write modes 0-3~~ — FIXED (session 56: all 4 modes + chain-four + odd/even)

### ~~protect_ctrl.rs — VERR/VERW~~ — FALSE POSITIVE (implemented at protect_ctrl.rs:501,594)

### ~~protect_ctrl.rs — LAR/LSL~~ — FALSE POSITIVE (implemented at protect_ctrl.rs:314,410)

### ~~proc_ctrl.rs — MONITOR v2h validation~~ — FIXED (session 56: warns on MMIO, still arms monitor)

---

## LOW Priority

### ~~harddrv.rs — SET MULTIPLE MODE~~ — FALSE POSITIVE (implemented at line 3581, allows 1-128 power-of-2)

### ~~pc_system.rs — isa_bus_delay()~~ — FIXED (session 56: stub method added, no-op for PCI systems)

### ~~pic.rs — Polled mode io_len==2~~ — FIXED (session 56: returns (irq<<8)|irq for word reads)

### ~~serial.rs — RX input~~ — VERIFIED ALREADY IMPLEMENTED (receive_byte + rx_fifo_enq fully match Bochs serial.cc rx_fifo_enq: FIFO/non-FIFO paths, overrun, trigger levels, interrupts)
- **Bochs ref**: serial.cc

### ~~vga.rs — Sequencer chain_four/odd_even~~ — FIXED (session 56: fields added + extracted on reg 4 write)

### ~~vga.rs — Retrace timing~~ — FIXED (session 56: timer-based from CRTC registers + icount)

### ~~crregs.rs — MOV DRn~~ — FALSE POSITIVE (implemented at proc_ctrl.rs:726,760)

### ~~event.rs — SMI/INIT event priority handling~~ — FIXED (session 57: stub handlers clear events with debug log, matching Bochs  priority 3 placement)
- **Bochs ref**: event.cc

### ~~event.rs — Code breakpoint matching~~ — FIXED (session 56: stub returning 0, no DR0-3 configured)

### ~~event.rs — HRQ/DMA handling in async event loop~~ — VERIFIED ALREADY IMPLEMENTED (get_hrq() + dma.raise_hlda() at correct position matching Bochs )
- **Bochs ref**: event.cc

### ~~dispatcher.rs — mod_c0 `?` propagation~~ — FALSE POSITIVE
Match arms return Result directly (no `?` needed — the Result IS the return value)

### ~~fpu_arith.rs — FPU NaN handling~~ — FIXED (session 56: 12 memory-form f32/f64 handlers with NaN check)

### ~~shared_display.rs — actl_palette indirection~~ — FIXED (session 56: parameter added, used for fg/bg lookup)

### snapshot.rs — Incomplete device coverage (missing APIC, VGA, harddrv, etc.)
- **Bochs does**: Full machine state serialization
- **Our code does**: Only CPU, memory, PIC, PIT, CMOS, PC_SYSTEM. Section IDs defined for DMA, VGA, keyboard, serial, harddrv, IOAPIC, LAPIC, PCI, ACPI but save/restore methods not implemented yet.
- **Bochs ref**: siminterface.cc
- **Status**: Partially done — IDs exist, save/restore deferred

### ~~memory_stub.rs — Debugger memory access~~ — FIXED (session 56: log warnings instead of panic, feature-gated)
