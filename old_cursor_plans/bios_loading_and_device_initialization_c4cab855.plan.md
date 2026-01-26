---
name: BIOS Loading and Device Initialization
overview: ""
todos: []
---

# BIOS

Loading and Device Initialization Implementation

## Overview

Implement the memory initialization (lines 1299-1326) and device initialization (lines 1353-1363) from Bochs main.cc, enabling successful BIOS emulation. The implementation will be thread-safe using UnsafeCell where appropriate and support multiple emulator instances.

## Implementation Tasks

### 1. Memory Initialization and ROM Loading (`rusty_box/src/memory/misc_mem.rs`)

**Add `load_ROM` method to `BxMemC`:**

- Support three ROM types: System BIOS (type=0), VGA BIOS (type=1), Optional ROM (type=2)

- System BIOS must end at 0xfffff, loaded at address 0xfffe0000 (mapped to 0xe0000-0xfffff)

- VGA/Optional ROMs in 0xc0000-0xdffff range, 2KB-aligned, 512-byte multiples

- Track ROM presence in `rom_present` array (65 entries for 2KB blocks)

- Calculate ROM buffer offsets: `BIOS_MAP_LAST128K(addr)` for system BIOS, `(addr & EXROM_MASK) + BIOSROMSZ` for expansion ROMs

- Support both file path loading and compile-time embedded BIOS via `include_bytes!`

- Validate ROM checksum (optional but recommended)
- Update `bios_rom_addr` and `bios_rom_access` fields

**Key implementation details:**

- File loading: Read BIOS file and copy to appropriate ROM buffer location

- Embedded BIOS: Use `include_bytes!` macro for compile-time embedding

- ROM address mapping: Handle 0xe0000-0xfffff (last 128K BIOS) and 0xc0000-0xdffff (expansion ROMs)

- Error handling: Validate size, alignment, and address ranges

### 2. PC System Initialization (`rusty_box/src/pc_system.rs`)

**Add `initialize` method to `BxPcSystemC`:**

- Accept IPS (instructions per second) parameter

- Initialize timer infrastructure

- Set up timer array (currently single timer, prepare for expansion)

- Initialize timing-related fields (`ticks_total`, `last_time_usec`, etc.)

- Thread-safe: Use UnsafeCell for mutable timer state if needed, or keep immutable where possible

**Key implementation:**

- Initialize `NullTimerInterval` to a safe default

- Set up timer synchronization method (realtime/slowdown)

- Prepare for timer callbacks (currently function pointers, may need trait objects)

### 3. Device Initialization (`rusty_box/src/iodev/devices.rs`)

**Implement `BxDevicesC::init` method:**

- Initialize I/O port handler tables (65536 ports)

- Register default I/O read/write handlers

- Initialize core devices in order:

1. PCI controller (if enabled) - register I/O ports 0x0CF8-0x0CFF

2. PCI-to-ISA bridge - register port 0x0092 (A20 line control)