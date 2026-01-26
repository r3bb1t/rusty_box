---
name: Align Hardware Initialization with Original Bochs
overview: Fix the hardware initialization sequence in the Rust emulator to exactly match the original Bochs `bx_init_hardware()` function, ensuring proper order of operations for PC system, memory, CPU, device initialization, and GUI (especially text GUI for BIOS/Linux text output).
todos:
  - id: fix-cpu-init-order
    content: Remove sanity_checks() call from inside initialize() in cpu/init.rs, make it a separate public method
    status: completed
  - id: align-emulator-init
    content: "Reorder emulator.rs::initialize() to match exact sequence from bx_init_hardware(): PC init → Memory init → BIOS load → CPU init → CPU sanity → CPU register_state → Device init → Register states → Reset → GUI init → Start timers"
    status: completed
    dependencies:
      - fix-cpu-init-order
  - id: verify-device-init
    content: Verify DeviceManager::init() matches device loading order from original devices.cc
    status: completed
  - id: check-memory-init
    content: Verify memory initialization timing - should be in initialize(), check if current placement in new() is correct
    status: completed
  - id: verify-gui-init
    content: Ensure GUI (TermGui) is initialized before hardware init, and signal handlers are called after reset. Verify text output works for BIOS and Linux messages.
    status: completed
  - id: test-boot-sequence
    content: Test that BIOS and Linux boot correctly with the new initialization sequence, and text output appears properly on terminal
    status: completed
    dependencies:
      - align-emulator-init
      - verify-device-init
      - verify-gui-init
---

# Align Hardware Initialization with Original Bochs

## Overview

The Rust emulator's initialization sequence in `rusty_box/src/emulator.rs` needs to exactly match the original Bochs `bx_init_hardware()` function from `cpp_orig/bochs/main.cc:1192-1401`. The current implementation has some differences in order and missing steps.

## Current Issues

1. **CPU Initialization Order**: In Rust, `sanity_checks()` is called inside `initialize()`, but in original Bochs they are separate calls in sequence: `initialize()` → `sanity_checks()` → `register_state()`

2. **Initialization Sequence**: The order in `emulator.rs::initialize()` doesn't exactly match `bx_init_hardware()`:

   - Original: PC system init → Memory init → BIOS load → CPU init → CPU sanity → CPU register_state → DEV_init_devices → register_state → Reset → GUI init → start_timers
   - Current: PC system init → Memory init → CPU init (includes sanity) → CPU register_state → Device init → register_state

3. **Missing Steps**: Some initialization steps from the original are missing or in wrong order

4. **GUI Initialization**: Text GUI (TermGui) needs to be properly initialized to display BIOS and Linux text output. The original calls `bx_gui->init_signal_handlers()` after reset (line 1383), and GUI should be set up before hardware initialization begins.

## Implementation Plan

### 1. Fix CPU Initialization Sequence

**File**: `rusty_box/src/cpu/init.rs`

- Remove `sanity_checks()` call from inside `initialize()` (line 58)
- Make `sanity_checks()` a separate public method that can be called after `initialize()`
- Ensure `register_state()` is called separately after `sanity_checks()`

**Original order** (from `main.cc:1337-1339`):

```cpp
BX_CPU(0)->initialize();
BX_CPU(0)->sanity_checks();
BX_CPU(0)->register_state();
```

### 2. Align Emulator Initialization Sequence

**File**: `rusty_box/src/emulator.rs`

Update `initialize()` method to match exact order from `bx_init_hardware()`:

1. **PC System Initialize** (line 1201): `bx_pc_system.initialize(IPS)` ✓ Already done
2. **Memory Initialize** (line 1312): `BX_MEM(0)->init_memory(...)` ✓ Already done (in `new()`)
3. **BIOS Load** (line 1315-1316): `BX_MEM(0)->load_ROM(...)` - Should be done AFTER memory init, before CPU init
4. **Optional ROM Load** (line 1319-1325): Loop through optional ROMs - Add if not present
5. **Optional RAM Load** (line 1328-1334): Loop through optional RAM images - Add if not present
6. **CPU Initialize** (line 1337): `BX_CPU(0)->initialize()` - Separate from sanity_checks
7. **CPU Sanity Checks** (line 1338): `BX_CPU(0)->sanity_checks()` - Separate call
8. **CPU Register State** (line 1339): `BX_CPU(0)->register_state()` - Already done
9. **Device Init** (line 1353): `DEV_init_devices()` ✓ Already done via `device_manager.init()`
10. **PC System Register State** (line 1356): `bx_pc_system.register_state()` ✓ Already done
11. **Device Register State** (line 1357): `DEV_register_state()` - Ensure this is called
12. **Reset** (line 1363): `bx_pc_system.Reset(BX_RESET_HARDWARE)` - Should be called AFTER device init
13. **GUI Init Signal Handlers** (line 1383): `bx_gui->init_signal_handlers()` - Must be called AFTER reset, before start_timers
14. **Start Timers** (line 1384): `bx_pc_system.start_timers()` - Should be called AFTER reset and GUI signal handlers

### 3. GUI Initialization for Text Output

**Files**: `rusty_box/src/emulator.rs`, `rusty_box/src/gui/term.rs`

Ensure text GUI (TermGui) is properly initialized to display BIOS and Linux text output:

1. **GUI Setup** (before hardware init): GUI should be set via `emu.set_gui()` before calling `initialize()`
2. **GUI Specific Init** (during hardware init): Call `gui.specific_init()` to set up terminal raw mode and clear screen
3. **GUI Signal Handlers** (after reset, line 1383): Call `gui.init_signal_handlers()` after hardware reset
4. **Text Update Integration**: Ensure `emu.update_gui()` is called during execution to refresh text display
5. **VGA Text Mode**: Verify VGA device properly updates text memory that GUI reads from

**Original sequence** (from `main.cc`):

- GUI is loaded/selected before `bx_init_hardware()` (in main flow)
- `bx_gui->init_signal_handlers()` called at line 1383 (after reset, before start_timers)
- GUI receives text updates via `text_update()` callback from VGA device

### 4. Update Example to Match Sequence

**File**: `rusty_box/examples/dlxlinux.rs`

Ensure the example follows the correct initialization order:

1. Create emulator
2. **Set up GUI** (TermGui) - must be done BEFORE initialize()
3. Initialize hardware (calls `emu.initialize()`)
4. Load BIOS (should be done in `initialize()` or right after)
5. Load VGA BIOS (optional ROM)
6. Attach disk
7. Initialize GUI (calls `emu.init_gui()` which calls `specific_init()`)
8. Reset hardware (enables A20, resets CPU/devices)
9. **GUI signal handlers** (should be called in `init_gui()` or after reset)
10. Start execution (GUI updates happen during `run_interactive()`)

### 5. Verify Device Initialization

**File**: `rusty_box/src/iodev/devices.rs`

Ensure `DeviceManager::init()` matches the device loading order from `cpp_orig/bochs/iodev/devices.cc:116-315`:

- I/O handler registration
- Removable devices (keyboard/mouse) initialization
- Timer devices (virt_timer, slowdown_timer)
- Core devices (CMOS, DMA, PIC, PIT)
- Keyboard controller
- Hard drive controller
- VGA (if present)

### 6. Check Memory Initialization

**File**: `rusty_box/src/memory/mod.rs`

Verify `init_memory()` is called at the right time (should be in `initialize()`, not `new()`). The original calls it in `bx_init_hardware()` after PC system init.

## Files to Modify

1. `rusty_box/src/cpu/init.rs` - Separate `sanity_checks()` from `initialize()`
2. `rusty_box/src/emulator.rs` - Reorder initialization sequence to match original, ensure GUI signal handlers called after reset
3. `rusty_box/src/gui/term.rs` - Verify text GUI properly initializes terminal and handles text updates
4. `rusty_box/examples/dlxlinux.rs` - Update example to use correct sequence with GUI setup before initialize()

## Testing

After changes, verify:

- BIOS boots correctly
- **Text output appears on terminal** (TermGui displays BIOS messages)
- **Linux boot messages are visible** (text GUI updates properly)
- **Keyboard input works** (GUI handles events and sends scancodes)
- Linux boots properly
- All devices are initialized in correct order
- GUI signal handlers are set up correctly
- No regressions in existing functionality