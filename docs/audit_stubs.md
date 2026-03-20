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

### pc_system.rs — Missing set_HRQ async_event signal
- **Bochs does**: `set_HRQ()` sets `BX_CPU(0)->async_event = 1` to break CPU loop for DMA
- **Our code does**: Only sets `self.hrq = value`, no CPU signal
- **Bochs ref**: pc_system.cc

### pc_system.rs — Missing raise_INTR/clear_INTR/IAC methods
- **Bochs does**: Delegates interrupt signal to bootstrap CPU; IAC gets vector from PIC
- **Our code does**: Methods don't exist
- **Bochs ref**: pc_system.cc

### dma.rs — Missing set_DRQ/control_HRQ/raise_HLDA
- **Bochs does**: Full DMA transfer machinery — request, hold, acknowledge, data transfer
- **Our code does**: Basic channel state only, no actual transfers
- **Bochs ref**: dma.cc

### dma.rs — Missing TC/HLDA/DRQ/DACK fields
- **Bochs does**: Terminal count, hold acknowledge, request/acknowledge arrays
- **Our code does**: Fields don't exist
- **Bochs ref**: dma.h

---

## HIGH Priority

### pc_system.rs — Missing TLB flush on A20 change
- **Bochs does**: Calls `MemoryMappingChanged()` → `TLB_flush()` on all CPUs when A20 changes
- **Our code does**: Logs debug message, relies on caller to flush
- **Bochs ref**: pc_system.cc

### ioapic.rs — ExtINT delivery mode uses entry.vector() instead of PIC IAC
- **Bochs does**: When delivery_mode==7 (ExtINT), calls `DEV_pic_iac()` for vector
- **Our code does**: Uses entry.vector() as fallback
- **Bochs ref**: ioapic.cc

### ~~harddrv.rs — SET FEATURES (0xEF) transfer mode~~ — FIXED (session 56: mdma/udma/packet_dma wired up)

### dma.rs — Address shift hardcoded for 16-bit channels
- **Bochs does**: Uses `ma_sl` variable (0 for 8-bit, 1 for 16-bit) for address shift
- **Our code does**: Hardcoded `<< 1`
- **Bochs ref**: dma.cc

### serial.rs — Missing timer-based TX/RX scheduling
- **Bochs does**: Registers tx_timer, rx_timer, fifo_timer with pc_system for paced TX/RX
- **Our code does**: TX is immediate, no RX polling, no FIFO timeout timer
- **Bochs ref**: serial.cc

### ~~serial.rs — RI state change detection~~ — FIXED (session 56: ms_ipending on any RI change)

### ~~cmos.rs — Extended CMOS addressing~~ — FIXED (session 56: ports 0x0072/0x0073, 256-byte RAM)

### memory/misc_mem.rs — bios_write_enabled defaults true (Bochs: false)
- **Bochs does**: BIOS ROM write-protected by default
- **Our code does**: Write-enabled at startup
- **Bochs ref**: misc_mem.cc

---

## MEDIUM Priority

### ~~pit.rs — clock_multiple() fast path~~ — PARTIALLY IMPLEMENTED (session 56: bulk skip when next_change_time > ticks)

### ~~pit.rs — Tick accumulation cap~~ — RAISED to 5M (session 56: with clock_multiple, bulk skip makes large counts safe)

### serial.rs — Break control in loopback mode not implemented
- **Bochs does**: Enqueues break character (0x00) into RX when entering loopback with break_cntl
- **Our code does**: Missing logic
- **Bochs ref**: serial.cc

### serial.rs — MODSTAT interrupt pending→interrupt promotion incomplete
- **Bochs does**: Promotes ms_ipending to ms_interrupt when modstat_enable becomes true
- **Our code does**: Partial — calls raise_interrupt but doesn't match Bochs logic exactly
- **Bochs ref**: serial.cc

### ~~cmos.rs — UIP bit ordering~~ — FALSE POSITIVE (already correct: line 295 clears UIP before line 298 update_clock)

### dma.rs — Missing DMA handler registration (registerDMA8/16Channel)
- **Bochs does**: Devices register read/write callbacks for DMA channels
- **Our code does**: No registration method
- **Bochs ref**: dma.cc

### ~~dma.rs — ctrl_disabled field~~ — FIXED (session 56: added + wired from command register write, plus drq/dack arrays)

### memory/misc_mem.rs — Flash ROM state machine not implemented
- **Bochs does**: Full flash_read/flash_write state machine (FLASH_READ_ARRAY, INT_ID, etc.)
- **Our code does**: Constants defined, no implementation
- **Bochs ref**: misc_mem.cc

### vga.rs — Graphics read modes 0/1 not implemented
- **Bochs does**: Latch-based read with color compare/don't-care
- **Our code does**: Returns text buffer directly
- **Bochs ref**: vgacore.cc

### vga.rs — Graphics write modes 1-3 not implemented
- **Bochs does**: AND/OR/XOR logical operations with set/reset, bitmask
- **Our code does**: Text mode write only
- **Bochs ref**: vgacore.cc

### ~~protect_ctrl.rs — VERR/VERW~~ — FALSE POSITIVE (implemented at protect_ctrl.rs:501,594)

### ~~protect_ctrl.rs — LAR/LSL~~ — FALSE POSITIVE (implemented at protect_ctrl.rs:314,410)

### proc_ctrl.rs — MONITOR missing v2h_write_byte validation
- **Bochs does**: Checks host pointer valid (not I/O mapped) before arming monitor
- **Our code does**: Skips validation
- **Bochs ref**: mwait.cc

---

## LOW Priority

### harddrv.rs — SET MULTIPLE MODE (0xC6) not dispatched
- **Bochs does**: Validates count (power of 2, <= 16), sets multiple_sectors
- **Our code does**: Command not in dispatch table
- **Bochs ref**: harddrv.cc

### pc_system.rs — Missing isa_bus_delay() method
- **Bochs does**: Emulates 8 MHz ISA bus timing
- **Our code does**: Not implemented
- **Bochs ref**: pc_system.cc

### pic.rs — Polled mode return format wrong for io_len==2
- **Bochs does**: Duplicates IRQ byte in high 8 bits
- **Our code does**: Only sets low 8 bits
- **Bochs ref**: pic.cc

### serial.rs — RX input not implemented (only TX output works)
- **Bochs does**: Polls file/socket/TTY/pipe for input
- **Our code does**: Only receive_byte() stub exists
- **Bochs ref**: serial.cc

### vga.rs — Sequencer chain_four/odd_even not tracked as fields
- **Bochs does**: Extracts and caches these bits for memory access decisions
- **Our code does**: Raw register stored, bits not extracted
- **Bochs ref**: vgacore.cc

### vga.rs — Retrace timing uses simple toggle, not timer calculation
- **Bochs does**: Calculates from virtual timer and CRTC register values
- **Our code does**: XOR toggle on each status read
- **Bochs ref**: vgacore.cc

### ~~crregs.rs — MOV DRn~~ — FALSE POSITIVE (implemented at proc_ctrl.rs:726,760)

### memory_stub.rs — Debugger memory access functions unimplemented
- **Bochs does**: dbg_set_mem, dbg_crc32 for debugger
- **Our code does**: `unimplemented!()`
- **Bochs ref**: memory.cc

### event.rs — Missing SMI/INIT event priority handling
- **Bochs does**: Priority 3 events: enter_system_management_mode() on SMI, CPU reset on INIT
- **Our code does**: Not implemented (jumps from Priority 2 to Priority 4)
- **Bochs ref**: event.cc

### event.rs — Missing code breakpoint matching in Priority 4
- **Bochs does**: Calls code_breakpoint_match(prev_rip) and ORs into debug_trap
- **Our code does**: Only checks TF single-step, not DR0-DR3 code breakpoints
- **Bochs ref**: event.cc

### event.rs — Missing HRQ/DMA handling in async event loop
- **Bochs does**: Checks BX_HRQ and calls DEV_dma_raise_hlda()
- **Our code does**: No DMA integration in event handling
- **Bochs ref**: event.cc

### ~~dispatcher.rs — mod_c0 `?` propagation~~ — FALSE POSITIVE
Match arms return Result directly (no `?` needed — the Result IS the return value)

### fpu_arith.rs — Missing FPU_handle_NaN for memory-form arithmetic
- **Bochs does**: Checks NaN before converting memory operand and performing arithmetic
- **Our code does**: Skips NaN check, converts and operates directly
- **Bochs ref**: fpu_arith.cc

### shared_display.rs — Missing actl_palette indirection in color lookup
- **Bochs does**: Uses actl_palette[attr & 0x0f] for color indexing with PEL mask
- **Our code does**: Uses attr & 0x0F directly as palette index
- **Bochs ref**: gui.cc, vgacore.cc

### snapshot.rs — Incomplete device coverage (missing APIC, VGA, harddrv, etc.)
- **Bochs does**: Full machine state serialization
- **Our code does**: Only CPU, memory, PIC, PIT, CMOS, PC_SYSTEM
- **Bochs ref**: siminterface.cc

### memory_stub.rs — Debugger memory access functions unimplemented
- **Bochs does**: dbg_set_mem, dbg_crc32 for debugger
- **Our code does**: `unimplemented!()`
- **Bochs ref**: memory.cc
