---
name: Bochs Initialization Verification and Fixes
overview: Verify and fix the Rust emulator initialization sequence to match the original Bochs code exactly, ensuring all calls from bx_init_hardware() are properly implemented and in the correct order.
todos:
  - id: verify-cpu-register-state
    content: Verify CPU register_state() is called during initialization and matches original Bochs order
    status: completed
  - id: implement-load-ram
    content: Implement memory.load_RAM() method and Emulator::load_ram() wrapper matching original Bochs
    status: completed
  - id: verify-device-init-order
    content: Compare device initialization order with bochsout.txt and ensure all devices initialize correctly
    status: completed
  - id: verify-reset-sequence
    content: "Verify reset sequence: A20 enable → CPU reset → Device reset matches original"
    status: completed
  - id: verify-io-handlers
    content: Verify all I/O port handlers are registered correctly during device initialization
    status: completed
  - id: trace-outgoing-calls
    content: Trace all outgoing calls from bx_init_hardware() and verify they are implemented
    status: completed
  - id: compare-initialization-logs
    content: Compare runtime initialization logs with bochsout.txt to find discrepancies
    status: completed
    dependencies:
      - verify-device-init-order
      - verify-io-handlers
  - id: fix-terminal-gui-output
    content: Fix terminal GUI to properly display BIOS messages and emulator output to terminal
    status: completed
  - id: test-boot-sequence
    content: Test dlxlinux.rs example and verify BIOS boot sequence matches original Bochs, including terminal output
    status: completed
    dependencies:
      - verify-cpu-register-state
      - implement-load-ram
      - verify-reset-sequence
      - trace-outgoing-calls
      - fix-terminal-gui-output
---

# Verify and Fix Bochs Initialization Sequence

This plan ensures the Rust emulator's initialization sequence matches the original Bochs `bx_init_hardware()` function from `main.cc:1192-1401` exactly.

## Current State Analysis

**Already implemented correctly:**

- `bx_pc_system.initialize(ips)` → `pc_system.initialize()` ✓
- `BX_MEM(0)->init_memory()` → `memory.init_memory()` ✓
- `BX_MEM(0)->load_ROM()` → `load_bios()` / `load_optional_rom()` ✓
- `BX_CPU(0)->initialize()` → `cpu.initialize()` ✓
- `BX_CPU(0)->sanity_checks()` → `cpu.sanity_checks()` (called inside initialize) ✓
- `DEV_init_devices()` → `devices.init()` + `device_manager.init()` ✓
- `bx_pc_system.register_state()` → `pc_system.register_state()` ✓
- `DEV_register_state()` → `devices.register_state()` ✓
- `bx_pc_system.Reset()` → `reset()` ✓
- `bx_pc_system.start_timers()` → `start_timers()` ✓

**Potential issues to verify:**

1. Missing `BX_CPU(0)->register_state()` call (called inside initialize in original)
2. Missing `BX_INSTR_INITIALIZE(0)` (may be optional instrumentation)
3. Missing `load_RAM()` for optional RAM images
4. Device initialization order and completeness
5. A20 line handling during reset sequence

## Implementation Tasks

### 1. Verify CPU register_state() call

**File:** `rusty_box/src/cpu/init.rs`

- [ ] Verify `cpu.register_state()` is called during initialization (may be in `initialize()` method)
- [ ] Compare with original Bochs: `BX_CPU(0)->register_state()` is called after `sanity_checks()`
- [ ] If missing, add explicit call in `Emulator::initialize()` matching original order

### 2. Add missing load_RAM() method

**Files:** `rusty_box/src/memory/misc_mem.rs`, `rusty_box/src/emulator.rs`

- [ ] Implement `memory.load_RAM()` method matching original `BX_MEM(0)->load_RAM()`
- [ ] Add `Emulator::load_ram()` wrapper method
- [ ] Verify ROM/RAM loading flags (type parameter: 0=BIOS, 2=optional ROM, ?=RAM)

### 3. Verify device initialization completeness

**Files:** `rusty_box/src/iodev/devices.rs`, `rusty_box/src/emulator.rs`

Compare device initialization order with original Bochs log (`bochsout.txt`):

- [ ] PCI (if enabled) - line 40
- [ ] PIIX3 PCI-to-ISA bridge - line 42
- [ ] CMOS - line 44
- [ ] DMA - line 47
- [ ] PIC - line 49
- [ ] PIT - line 50
- [ ] VGA - line 52
- [ ] Floppy - line 65
- [ ] ACPI - line 73
- [ ] HPET - line 75
- [ ] IOAPIC - line 78
- [ ] Keyboard - line 82
- [ ] HardDrive - line 83
- [ ] PCI IDE - line 88
- [ ] Other devices (unmapped, biosdev, speaker, etc.)

Verify all devices from log are initialized in correct order.

### 4. Verify reset sequence matches original

**Files:** `rusty_box/src/pc_system.rs`, `rusty_box/src/emulator.rs`

Original sequence (`main.cc:1363`):

```cpp
bx_pc_system.Reset(BX_RESET_HARDWARE);  // Enables A20, resets CPU/devices
```

Verify:

- [ ] A20 is enabled in `pc_system.reset()` before CPU reset
- [ ] CPU reset happens after A20 enable
- [ ] Device reset happens after CPU reset (hardware reset only)
- [ ] Reset order: A20 → CPU → Devices

### 5. Verify optional ROM/RAM loading and configuration

**File:** `rusty_box/examples/dlxlinux.rs`

**Critical Bug Found:** Line 124 sets `guest_memory_size: 128 * 1024 * 1024` but comment says "32 MB" - should be `32 * 1024 * 1024` to match bochsrc.bxrc `megs: 32`

Original loads from bochsrc.bxrc:

- System BIOS at `0xfffe0000` (128KB) from `../BIOS-bochs-latest` - ✓ Done
- VGA BIOS at `0xc0000` (optional ROM) from `../VGABIOS-lgpl-latest.bin` - ✓ Done
- Optional ROM images (0-CX_N_OPTROM_IMAGES) - ⚠ Not checked (not in bochsrc.bxrc)
- Optional RAM images (0-BX_N_OPTRAM_IMAGES) - ⚠ Missing (not in bochsrc.bxrc)

Verify example matches bochsrc.bxrc exactly.

### 6. Verify timing and IPS configuration

**Files:** `rusty_box/examples/dlxlinux.rs`, `rusty_box/src/pc_system.rs`

**Reference:** `dlxlinux/bochsrc.bxrc` (successfully booted DLX Linux in original Bochs)

Verify configuration matches bochsrc.bxrc exactly:

- [ ] Memory size: 32 MB (`megs: 32`)
- [ ] IPS value: 15000000 (`cpu: ips=15000000`)
- [ ] BIOS path: `../BIOS-bochs-latest` (loaded at `0xfffe0000`)
- [ ] VGA BIOS path: `../VGABIOS-lgpl-latest.bin` (loaded at `0xc0000`)
- [ ] Hard disk: `hd10meg.img` with CHS=306/4/17 (`ata0-master: type=disk, path="hd10meg.img", cylinders=306, heads=4, spt=17`)
- [ ] Boot device: disk (`boot: disk`)
- [ ] Mouse: disabled (`mouse: enabled=0`)
- [ ] Timer initialization happens after reset (line 1384: `bx_pc_system.start_timers()`)

### 7. Check for missing outgoing calls

**Files:** All initialization-related files

Trace all calls from `bx_init_hardware()`:

- [ ] `bx_pc_system.initialize()` - all internal calls implemented?
- [ ] `BX_MEM(0)->init_memory()` - all internal setup done?
- [ ] `BX_CPU(0)->initialize()` - verify all CPU initialization steps
- [ ] `DEV_init_devices()` - verify all device init methods called
- [ ] `bx_pc_system.Reset()` - verify all reset handlers called
- [ ] `bx_pc_system.start_timers()` - verify timer setup

### 8. Verify I/O handler registration

**Files:** `rusty_box/src/iodev/mod.rs`, `rusty_box/src/iodev/devices.rs`

Original Bochs registers I/O handlers during device initialization. Verify:

- [ ] All device I/O handlers registered in correct order
- [ ] Port 92h (System Control) registered during `devices.init()`
- [ ] Default handlers return correct values for unhandled ports

### 9. Compare initialization logs

**Files:** Compare runtime logs with `dlxlinux/bochsout.txt`

- [ ] Compare device initialization messages
- [ ] Compare memory allocation messages
- [ ] Compare CPU initialization messages
- [ ] Verify BIOS loading messages match

### 10. Ensure terminal GUI properly displays BIOS messages

**Files:** `rusty_box/src/gui/term.rs`, `rusty_box/src/emulator.rs`

**Requirement:** Terminal GUI must output BIOS messages and emulator output to terminal

**Issues identified:**

1. Cursor save/restore (`\x1b[s` and `\x1b[u`) in `render_text_mode()` may hide output
2. Need to ensure output is flushed and visible after rendering
3. Original Bochs uses curses `refresh()` after updates - we need equivalent behavior
4. GUI updates may not happen frequently enough during BIOS execution

**Fixes needed:**

1. **Fix `render_text_mode()` in `term.rs`:**

   - [ ] Remove or fix cursor save/restore that may hide output
   - [ ] After rendering, move cursor to actual BIOS cursor position (don't restore to saved position)
   - [ ] Ensure `stdout().flush()` is called after all output
   - [ ] Use cursor positioning similar to original Bochs (move to `cursor_y, cursor_x` after rendering)
   - [ ] Hide cursor if outside visible area

2. **Improve GUI update frequency in `emulator.rs`:**

   - [ ] Ensure `update_gui()` is called frequently enough during BIOS execution (currently only when text is dirty)
   - [ ] Check that VGA dirty flag detection triggers updates correctly
   - [ ] Consider periodic updates even when not dirty (like Bochs timer-based updates)

3. **Verify VGA text memory access:**

   - [ ] Ensure BIOS writes to 0xB8000 (VGA text memory) are captured
   - [ ] Verify VGA dirty flag is set when text memory changes
   - [ ] Test that character writes trigger immediate GUI updates

4. **Compare with original Bochs term.cc:**

   - [ ] Original uses curses `mvaddch()` and `refresh()` for each update
   - [ ] Original moves cursor to actual position with `move(cursor_y, cursor_x)`
   - [ ] Original shows cursor with `curs_set()` based on cursor visibility
   - [ ] Implement similar behavior without curses library

5. **Test output visibility:**

   - [ ] Run example and verify BIOS boot messages appear immediately
   - [ ] Verify text is not hidden by cursor position issues
   - [ ] Check that output updates in real-time as BIOS writes to screen

### 11. Test and verify boot sequence

**File:** `rusty_box/examples/dlxlinux.rs`

- [ ] Run example and compare boot behavior with original Bochs
- [ ] Verify BIOS starts executing correctly
- [ ] Verify BIOS messages appear in terminal GUI output
- [ ] Check for any missing device responses during boot
- [ ] Verify interrupt handling works (PIC, keyboard, etc.)

## Files to Modify

1. **[rusty_box/src/memory/misc_mem.rs](rusty_box/src/memory/misc_mem.rs)** - Add `load_RAM()` method
2. **[rusty_box/src/emulator.rs](rusty_box/src/emulator.rs)** - Add `load_ram()` wrapper, verify initialization order
3. **[rusty_box/src/cpu/init.rs](rusty_box/src/cpu/init.rs)** - Verify `register_state()` is called
4. **[rusty_box/src/iodev/devices.rs](rusty_box/src/iodev/devices.rs)** - Verify device init order matches original
5. **[rusty_box/examples/dlxlinux.rs](rusty_box/examples/dlxlinux.rs)** - Fix memory size bug (128 MB → 32 MB), verify all settings match bochsrc.bxrc
6. **[rusty_box/src/gui/term.rs](rusty_box/src/gui/term.rs)** - Fix terminal output to properly display BIOS messages
7. **[rusty_box/src/emulator.rs](rusty_box/src/emulator.rs)** - Ensure GUI updates happen frequently enough during BIOS execution

## Testing Strategy

1. Run `dlxlinux.rs` example and capture full initialization log
2. Compare log with `dlxlinux/bochsout.txt` line by line
3. Verify each device initialization message appears
4. Check BIOS execution starts at correct address
5. Verify A20 line state matches original
6. Test boot sequence progresses through BIOS initialization