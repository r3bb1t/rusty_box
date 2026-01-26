---
name: BIOS Loading and Device Initialization
overview: Implement memory initialization with ROM loading, device initialization, and create a working BIOS execution example. The implementation will support both file-based and compile-time embedded BIOS, use UnsafeCell for thread-safety, and follow the original Bochs initialization sequence.
todos: []
---

# BIOS Loading and Device Initialization Implementation

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
3. CMOS RTC - register ports 0x0070-0x0071
4. DMA controller - register ports 0x0000-0x000F, 0x0080-0x008F, 0x00C0-0x00DF
5. PIC (interrupt controller) - register ports 0x0020-0x003F, 0x00A0-0x00BF
6. PIT (timer) - register ports 0x0040-0x005F
7. Keyboard controller - register ports 0x0060, 0x0064
8. Floppy controller - register ports 0x03F0-0x03F7
9. IDE controller - register ports 0x01F0-0x01F7, 0x03F6-0x03F7

- For minimal implementation, create stub handlers that return appropriate defaults
- Thread-safe: Each device instance should be independent (no shared mutable state)

**Device handler structure:**

- Use trait objects or function pointers for I/O handlers
- Store handlers in arrays indexed by port number
- Support handler chaining for ports with multiple devices

### 4. System Reset (`rusty_box/src/pc_system.rs`)

**Add `Reset` method to `BxPcSystemC`:**

- Enable A20 line (set `a20_mask` to 0xFFFFFFFFFFFFFFFF)
- Call CPU reset for all CPUs
- Call device reset for all devices
- Thread-safe: Reset should be per-instance, not global

### 5. Memory Initialization Integration (`rusty_box/src/memory/mod.rs`)

**Add `init_memory` method to `BxMemC`:**

- Already exists in `BxMemoryStubC::create_and_init`, but may need wrapper
- Ensure proper initialization of ROM areas
- Set up memory handler chains for ROM regions

### 6. BIOS Execution Example (`rusty_box/examples/bios_execution.rs`)

**Create new example file:**

- Initialize PC system with `bx_pc_system().initialize(15000000)` (15 MIPS)
- Create memory: 32MB guest, 32MB host, 128KB block size
- Load BIOS from file or embedded:
- Try file path first: `"../binaries/bios/BIOS-bochs-latest"` or similar
- Fallback to embedded BIOS if file not found
- Initialize CPU with `Corei7SkylakeX` or `Corei7Haswell4770` (matching bochsout.txt)
- Initialize devices: `DEV_init_devices()` or equivalent
- Reset system: `bx_pc_system().Reset(ResetReason::Hardware)`
- Execute CPU loop: `cpu.cpu_loop(&mut mem, &[&cpu])`
- Add proper logging to match bochsout.txt output
- Remove artificial iteration limit, let BIOS execute naturally

**BIOS loading approach:**

```rust
// Try file first
let bios_data = if let Ok(data) = std::fs::read("path/to/bios") {
    data
} else {
    // Fallback to embedded
    include_bytes!("../../binaries/bios/BIOS-bochs-latest").to_vec()
};
mem.load_ROM(bios_data, 0xfffe0000, 0)?; // type=0 for system BIOS
```



### 7. CPU Instruction Execution Fixes (`rusty_box/src/cpu/cpu.rs`)

**Fix instruction fetching and execution:**

- Ensure `get_icache_entry` properly fetches from BIOS ROM addresses (0xfff0 area)
- Fix `prefetch` method to handle ROM memory correctly
- Ensure instruction decoder is called for BIOS instructions
- Remove artificial iteration limit in `cpu_loop`
- Properly handle instruction pointer increment

**Key fixes:**

- `prefetch`: Should use `get_host_mem_addr` with `BX_EXECUTE` access type
- `get_icache_entry`: Should decode actual BIOS instructions, not return NOP
- Instruction execution: Ensure all BIOS-relevant instructions are implemented

### 8. Thread Safety Considerations

**Use UnsafeCell for:**

- Memory handler chains (already using UnsafeCell in `BxMemoryStubC`)
- Device I/O port handlers (each instance has its own handlers)
- Timer state in `BxPcSystemC` (if mutable state needed)

**Avoid Mutex where possible:**

- Memory access: Already using UnsafeCell for block offsets
- CPU state: Each CPU instance is independent
- Device state: Each device instance is independent

## Files to Modify

1. `rusty_box/src/memory/misc_mem.rs` - Add `load_ROM` method
2. `rusty_box/src/pc_system.rs` - Add `initialize` and `Reset` methods
3. `rusty_box/src/iodev/devices.rs` - Implement device initialization
4. `rusty_box/src/iodev/mod.rs` - Add device handler structures
5. `rusty_box/src/memory/mod.rs` - Add `init_memory` wrapper if needed
6. `rusty_box/src/cpu/cpu.rs` - Fix instruction fetching from ROM
7. `rusty_box/examples/bios_execution.rs` - New example file

## Testing Strategy

1. Test ROM loading with file-based BIOS
2. Test ROM loading with embedded BIOS
3. Verify BIOS ROM is accessible at correct addresses (0xe0000-0xfffff)
4. Test device initialization doesn't panic
5. Test system reset properly initializes CPU state
6. Verify BIOS execution starts at 0xfff0 (CS:IP = 0xf000:0xfff0)
7. Compare execution trace with bochsout.txt logs

## Success Criteria

- BIOS loads successfully from file or embedded data
- CPU starts execution at 0xfff0 after reset
- First BIOS instructions are fetched and decoded correctly
- Memory reads from BIOS ROM return correct data
- Device initialization completes without errors
- System reset properly initializes all components