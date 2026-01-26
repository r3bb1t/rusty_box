---
name: Implement Initialization Logic
overview: Implement the hardware initialization sequence from Bochs main.cc lines 1300-1363, including PC system initialization, device initialization, A20 line control, and system reset, plus create an example demonstrating the complete flow.
todos:
  - id: pc-system-init
    content: Add initialize(ips), set_enable_a20(), timer fields to BxPcSystemC
    status: pending
  - id: io-handlers
    content: Implement I/O port handler infrastructure in iodev module
    status: pending
    dependencies:
      - pc-system-init
  - id: port-92h
    content: Implement Port 0x92 System Control handler (A20, soft reset)
    status: pending
    dependencies:
      - io-handlers
  - id: device-init
    content: Complete BxDevicesC::init() with IO handler setup
    status: pending
    dependencies:
      - io-handlers
  - id: reset-flow
    content: Update reset flow to properly sequence A20/CPU/devices
    status: pending
    dependencies:
      - pc-system-init
      - device-init
  - id: example
    content: Create hardware_init.rs example demonstrating full init sequence
    status: pending
    dependencies:
      - reset-flow
---

# Implement Bochs Hardware Initialization Sequence

This plan implements the initialization logic from `main.cc:1300-1363` following the analysis in `BOCHS_DEVICE_INITIALIZATION_ANALYSIS.md`.

## Current State Analysis

**Already implemented:**

- CPU: `initialize()`, `reset()`, `cpu_loop()` in [`src/cpu/init.rs`](rusty_box/src/cpu/init.rs)
- Memory: `init_memory()`, `load_ROM()` in [`src/memory/misc_mem.rs`](rusty_box/src/memory/misc_mem.rs)
- PC System: Basic structure with `reset()` in [`src/pc_system.rs`](rusty_box/src/pc_system.rs) (incomplete)
- Devices: Stub methods in [`src/iodev/devices.rs`](rusty_box/src/iodev/devices.rs)

**Needs implementation:**

- PC system `initialize(ips)` method
- PC system A20 line control (`set_enable_a20`)
- Enhanced `BxDevicesC::init()` with I/O port handler infrastructure
- I/O port read/write handlers
- `register_state()` methods for state save/restore
- Proper reset flow for devices

## Implementation Tasks

### 1. Enhance PC System (`src/pc_system.rs`)

Add:

- `initialize(ips: u32)` method - timer array setup, IPS configuration
- `set_enable_a20(value: bool)` - A20 line mask control
- `register_state()` - state registration for save/restore
- Timer fields for null timer and timer array
- `start_timers()` method

### 2. Enhance Device System (`src/iodev/mod.rs` and `src/iodev/devices.rs`)

Add:

- I/O port handler arrays (65536 ports)
- `register_io_read_handler()` / `register_io_write_handler()`
- `inp()` / `outp()` methods for port I/O
- Port 0x92 handler (System Control Port - A20 line enable, soft reset)
- Default I/O handlers that return 0xFF

### 3. Update Reset Flow

Modify `BxPcSystemC::reset()` to:

- Call `set_enable_a20(true)` first
- Reset CPU
- Reset devices (hardware reset only)

### 4. Create Initialization Example

Create [`examples/hardware_init.rs`](rusty_box/examples/hardware_init.rs):

```rust
// Demonstrates full initialization sequence:
// 1. bx_pc_system.initialize(ips)
// 2. Memory init + ROM loading
// 3. CPU initialize
// 4. DEV_init_devices()
// 5. register_state() for all components  
// 6. bx_pc_system.Reset(HARDWARE)
// 7. cpu_loop()
```



## Initialization Flow (per analysis doc)

```mermaid
flowchart TD
    A[Start] --> B[bx_pc_system.initialize]
    B --> C[Memory init_memory]
    C --> D[Load BIOS ROM]
    D --> E[CPU initialize]
    E --> F[DEV_init_devices]
    F --> G[Setup IO handlers]
    G --> H[Register Port 0x92]
    H --> I[register_state all]
    I --> J[bx_pc_system.Reset HARDWARE]
    J --> K[set_enable_a20 true]
    K --> L[CPU reset]
    L --> M[Devices reset]
    M --> N[cpu_loop]
```