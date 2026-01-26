---
name: BIOS Loading and Device Initialization
overview: Implement memory initialization with BIOS loading, device initialization, and create a working example that successfully emulates BIOS boot sequence. This includes implementing load_ROM, device initialization stubs, and fixing instruction execution flow.
todos:
  - id: load_rom_impl
    content: Implement load_ROM method in BxMemC to load BIOS and optional ROMs into memory
    status: completed
  - id: init_memory_method
    content: Add init_memory method to BxMemC that wraps memory stub initialization
    status: completed
  - id: device_init
    content: Implement DEV_init_devices with minimal device initialization stubs
    status: completed
  - id: device_reset
    content: Implement device reset functionality for hardware/software reset
    status: completed
    dependencies:
      - device_init
  - id: pc_system_reset
    content: Implement PC system reset method to enable A20 line
    status: completed
  - id: fix_instruction_execution
    content: Fix CPU instruction execution flow to properly fetch, decode, and execute BIOS instructions
    status: in_progress
  - id: bios_boot_example
    content: Create bios_boot.rs example that loads BIOS and runs initialization sequence
    status: completed
    dependencies:
      - load_rom_impl
      - init_memory_method
      - device_init
      - device_reset
      - pc_system_reset
  - id: verify_bios_access
    content: Verify BIOS ROM memory access works correctly for instruction fetching
    status: completed
    dependencies:
      - load_rom_impl
---

# BIOS Loading and Device Initialization Implementation

## Overview

This plan implements the memory initialization with BIOS loading (lines 1299-1326 from main.cc) and device initialization/reset (lines 1353-1363 from main.cc), creating a working example that successfully emulates the BIOS boot sequence.

## Implementation Tasks

### 1. Implement `load_ROM` in `BxMemC` ([rusty_box/src/memory/misc_mem.rs](rusty_box/src/memory/misc_mem.rs))

**Current State**: `load_ROM` is not implemented. The memory system has ROM space allocated but no way to load BIOS images.**Implementation**:

- Add `load_ROM(&mut self, rom_data: &[u8], rom_address: BxPhyAddress, rom_type: u8) -> Result<()>`
- Handle three ROM types:
- Type 0: System BIOS (must end at 0xfffff, loaded at 0xfffe0000)
- Type 1: VGA BIOS (optional, handled separately)
- Type 2: Optional ROM (0xc0000-0xdffff range, 2KB aligned)
- Validate ROM size and alignment constraints
- Copy ROM data into appropriate memory regions:
- System BIOS: `rom_offset + (rom_address & BIOS_MASK)`
- Optional ROM: `rom_offset + BIOSROMSZ + (rom_address & EXROM_MASK)`
- Update `rom_present` array to track loaded ROM blocks
- Set `bios_rom_addr` appropriately for system BIOS

**Key Logic** (from BIOS_ROM_MEMORY_MAPPING_ANALYSIS.md):

- System BIOS must be loaded ending at 0xfffff
- Optional ROMs must be 2KB aligned and multiples of 512 bytes
- ROM data is stored in the `rom()` region of `BxMemoryStubC`

### 2. Add `init_memory` method to `BxMemC` ([rusty_box/src/memory/mod.rs](rusty_box/src/memory/mod.rs))

**Current State**: Memory stub has `create_and_init`, but `BxMemC` doesn't have a matching `init_memory` method.**Implementation**:

- Add `pub fn init_memory(&mut self, guest_size: usize, host_size: usize, block_size: usize) -> Result<()>`
- This should wrap `BxMemoryStubC::create_and_init` and initialize `BxMemC` state
- Initialize `rom_present` array (65 elements for 2KB blocks in 0xc0000-0xfffff range)
- Set default `bios_rom_addr` to 0xffff0000
- Initialize `memory_type` array for PCI memory mapping

### 3. Implement Device Initialization Stub ([rusty_box/src/iodev/devices.rs](rusty_box/src/iodev/devices.rs))

**Current State**: `BxDevicesC` exists but `init` is empty. Need minimal device initialization for BIOS to boot.**Implementation**:

- Add `pub fn init(&mut self, mem: &mut BxMemC) -> Result<()>`
- For now, implement minimal stub that:
- Sets up default I/O port handlers (read returns 0xff, write no-op)
- Initializes basic device state
- Registers device state (placeholder for now)
- This allows BIOS to execute without crashing on I/O port accesses
- Use `UnsafeCell` for thread-safety where needed (avoid Mutex for performance)

**Future Enhancement**: Full device implementation (PIC, PIT, CMOS, DMA, etc.) can be added incrementally.

### 4. Implement Device Reset ([rusty_box/src/iodev/devices.rs](rusty_box/src/iodev/devices.rs))

**Current State**: No device reset implementation.**Implementation**:

- Add `pub fn reset(&mut self, reset_type: ResetReason) -> Result<()>`
- For hardware reset: clear device state, reset registers
- For software reset: minimal state clearing
- This is called after device initialization in the boot sequence

### 5. Fix Instruction Execution Flow ([rusty_box/src/cpu/cpu.rs](rusty_box/src/cpu/cpu.rs))

**Current State**: `cpu_loop` exists but may not properly increment RIP or handle instruction fetching.**Implementation**:

- Ensure `cpu_loop` properly:
- Fetches instructions from memory using `get_host_mem_addr` with `MemoryAccessType::Execute`
- Decodes instructions using the decoder
- Executes instructions and updates RIP appropriately
- Handles exceptions and interrupts
- Fix any issues with instruction pointer advancement
- Ensure BIOS ROM is accessible for instruction fetching (0xfffe0000-0xffffffff range)

### 6. Update PC System Reset ([rusty_box/src/pc_system.rs](rusty_box/src/pc_system.rs))

**Current State**: `BxPcSystemC` exists but `Reset` method is not implemented.**Implementation**:

- Add `pub fn reset(&mut self, reset_type: ResetReason) -> Result<()>`
- Enable A20 line: `self.a20_mask = 0xFFFFFFFFFFFFFFFFu64`
- This is called before CPU reset in the boot sequence

### 7. Create BIOS Loading Example ([rusty_box/examples/bios_boot.rs](rusty_box/examples/bios_boot.rs))

**Current State**: `init_and_run.rs` exists but doesn't load BIOS.**Implementation**:

- Create new example `bios_boot.rs` that:
- Loads BIOS from `C:\Users\Aslan\rusty_box_cursor\binaries\BIOS-bochs-latest` (or use `include_bytes!` macro for compile-time inclusion)
- Initializes memory with proper sizes (32MB guest, 32MB host, 128KB block size)
- Calls `mem.init_memory()` 
- Calls `mem.load_ROM()` with BIOS data
- Initializes CPU with `initialize()`
- Calls `DEV_init_devices()` (device initialization)
- Calls `bx_pc_system().reset(ResetReason::Hardware)`
- Calls `cpu.reset(ResetReason::Hardware)`
- Enters `cpu_loop()` to execute BIOS
- Use `std::fs::read` or `include_bytes!` macro to load BIOS
- Add proper error handling and logging
- Match the initialization sequence from `main.cc:1299-1363`

### 8. Fix Memory Access for BIOS ROM

**Current State**: Memory access may not properly handle BIOS ROM reads.**Implementation**:

- Ensure `get_host_mem_addr` in `BxMemC` correctly handles:
- BIOS ROM reads (0xfffe0000-0xffffffff) map to `rom()` buffer
- Last 128KB BIOS mapping (0xe0000-0xfffff) uses `bios_map_last128k`
- Instruction fetching uses `MemoryAccessType::Execute`
- Verify ROM data is accessible for both reads and instruction execution

## Thread Safety Considerations

- Use `UnsafeCell` for internal mutable state in devices (avoid Mutex for performance)
- Memory stub already uses `UnsafeCell` for `blocks_offsets`
- Device handlers should be designed for multi-instance support
- Avoid global mutable state where possible

## Testing Strategy

1. **Unit Tests**: Test `load_ROM` with various ROM sizes and addresses
2. **Integration Test**: Run `bios_boot` example and verify:

- BIOS loads correctly
- CPU starts at 0x0000FFF0 (after reset)
- First instructions execute successfully
- Instruction pointer advances correctly

3. **Compare with Bochs**: Match log output from `bochsout.txt` for initial BIOS execution

## Files to Modify

1. `rusty_box/src/memory/misc_mem.rs` - Add `load_ROM` implementation
2. `rusty_box/src/memory/mod.rs` - Add `init_memory` to `BxMemC`
3. `rusty_box/src/iodev/devices.rs` - Implement `init` and `reset`
4. `rusty_box/src/pc_system.rs` - Implement `reset` method
5. `rusty_box/src/cpu/cpu.rs` - Fix instruction execution flow if needed
6. `rusty_box/examples/bios_boot.rs` - New example file

## Dependencies

- BIOS file must exist at `C:\Users\Aslan\rusty_box_cursor\binaries\BIOS-bochs-latest` (or use `include_bytes!` for compile-time inclusion)
- All existing memory and CPU infrastructure
- Decoder crate for instruction decoding

## Success Criteria

- BIOS loads successfully into memory
- CPU resets to 0x0000FFF0