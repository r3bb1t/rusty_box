---
name: BIOS Loading and Device Initialization
overview: Implement memory initialization with ROM loading, device initialization, and system reset following Bochs logic. Create a working example that loads BIOS from binaries/bios/ directory and successfully emulates the first BIOS instructions.
todos:
  - id: rom-loading
    content: Implement load_ROM() in misc_mem.rs with support for SystemBios, VgaBios, and OptionalRom types
    status: pending
  - id: pc-system-reset
    content: Implement PC system Reset() and initialize() methods with A20 line control
    status: pending
  - id: device-init
    content: Implement BxDevicesC::init() with minimal device stubs (CMOS, PIC, etc.)
    status: pending
  - id: device-reset
    content: Implement BxDevicesC::reset() for hardware/software reset
    status: pending
    dependencies:
      - device-init
  - id: cpu-loop-fix
    content: Fix cpu_loop() to properly increment RIP after instruction execution and remove iteration limits
    status: pending
  - id: bios-example
    content: Create bios_boot.rs example that loads BIOS using include_bytes!() and runs first instructions
    status: pending
    dependencies:
      - rom-loading
      - pc-system-reset
      - device-init
      - cpu-loop-fix
---

# BIOS Loading and Device Initialization Implementation Plan

## Overview
Implement the missing pieces from `cpp_orig/bochs/main.cc:1299-1326` (memory/ROM initialization) and `cpp_orig/bochs/main.cc:1353-1363` (device initialization and reset), ensuring thread-safety with UnsafeCell where possible and supporting multiple emulator instances.

## Architecture Decisions

### Thread Safety Strategy
- Use `UnsafeCell` for internal mutable state within emulator instances (memory blocks, CPU state)
- Each emulator instance is independent (no shared global mutable state between instances)
- PC system singleton uses `OnceLock`/`Once` (already thread-safe for reads)
- Memory handlers use owned data structures, not shared references

### Memory and ROM Loading

#### 1. Implement `load_ROM()` in [rusty_box/src/memory/misc_mem.rs](rusty_box/src/memory/misc_mem.rs)
- Add function signature: `pub fn load_ROM(&mut self, rom_data: &[u8], rom_address: BxPhyAddress, rom_type: RomType) -> Result<()>`
- Handle three ROM types:
  - `RomType::SystemBios` (type 0): Must end at 0xfffff, mapped to 0xe0000-0xfffff
  - `RomType::VgaBios` (type 1): 128K max, address range 0xc0000-0xdffff
  - `RomType::OptionalRom` (type 2): 128K max, address range 0xc0000-0xdffff or 0xe0000+
- Validate alignment (512-byte multiples, 2KB-aligned for optional ROMs)
- Copy ROM data into `inherited_memory_stub.rom()` buffer at calculated offsets
- Update `rom_present` array to mark used 2KB blocks
- Handle address mapping: BIOS ROM uses `bios_map_last128k()` for 0xe0000-0xfffff, expansion ROMs use `(addr & EXROM_MASK) + BIOSROMSZ`

#### 2. Enhance Memory Initialization
- Update [rusty_box/src/memory/memory_stub.rs](rusty_box/src/memory/memory_stub.rs) `create_and_init()` to ensure ROM buffer is properly allocated (already done, verify)
- Fix ROM read path in [rusty_box/src/memory/misc_mem.rs](rusty_box/src/memory/misc_mem.rs) `get_host_mem_addr()` to handle ROM access correctly (partially implemented, verify mapping)

### Device Initialization

#### 3. Implement Device Initialization in [rusty_box/src/iodev/devices.rs](rusty_box/src/iodev/devices.rs)
- Add `BxDevicesC::init(mem: &mut BxMemC)` method
- Initialize devices in order (following BOCHS_DEVICE_INITIALIZATION_ANALYSIS.md):
  1. PCI controller (if enabled) - minimal stub for now
  2. PCI-to-ISA bridge - minimal stub
  3. CMOS RTC - implement basic CMOS read/write handlers
  4. DMA controller - stub for now
  5. PIC (Programmable Interrupt Controller) - implement basic interrupt handling
  6. PIT (Programmable Interval Timer) - stub for now
  7. Keyboard controller - stub for now
  8. Floppy controller - stub for now
  9. IDE controller - stub for now
  10. Hard drive - stub for now
  11. I/O APIC (if SMP enabled) - stub for now
- Each device registers I/O handlers via `register_io_read_handler` / `register_io_write_handler` (to be implemented)
- Store device state in `BxDevicesC` struct using `UnsafeCell` where needed for thread safety

#### 4. Implement Device Reset
- Add `BxDevicesC::reset(reset_type: ResetReason)` method
- For hardware reset: clear PCI config address, disable SMRAM, reset all devices
- For software reset: minimal cleanup

#### 5. Update PC System Reset
- Enhance [rusty_box/src/pc_system.rs](rusty_box/src/pc_system.rs) `BxPcSystemC::Reset(reset_type: ResetReason)`:
  - Enable A20 line: `set_enable_a20(1)` - add `set_enable_a20()` method
  - Reset all CPUs: call `cpu.reset(reset_type)` for each CPU
  - Reset all devices: call `devices.reset(reset_type)` (if hardware reset)
- Add `initialize(ips: u64)` method for PC system timer initialization

### CPU Loop Enhancement

#### 6. Fix Instruction Execution in [rusty_box/src/cpu/cpu.rs](rusty_box/src/cpu/cpu.rs)
- Remove hardcoded 10-iteration limit in `cpu_loop()`
- Fix RIP increment: ensure `set_rip()` is called AFTER instruction execution (currently happens before)
- Implement proper instruction prefetch: fix `prefetch()` to handle BIOS ROM reads correctly
- Ensure `get_icache_entry()` can read from ROM addresses (0xe0000-0xfffff)

### Example Implementation

#### 7. Create BIOS Loading Example [rusty_box/examples/bios_boot.rs](rusty_box/examples/bios_boot.rs)
- Create `binaries/bios/` directory structure (or document where BIOS files should be placed)
- Use `include_bytes!()` macro to embed BIOS ROM at compile time
- Implementation steps:
  1. Initialize PC system with `bx_pc_system().initialize(15000000)` (15M IPS)
  2. Create memory stub: `BxMemoryStubC::create_and_init(32MB, 32MB, 128KB)`
  3. Create memory: `BxMemC::new(mem_stub, false)` (PCI disabled for now)
  4. Load BIOS ROM: `mem.load_ROM(bios_bytes, 0xfffe0000, RomType::SystemBios)`
  5. Build CPU: `BxCpuBuilder<Corei7SkylakeX>::new().build()`
  6. Initialize CPU: `cpu.initialize(config)` with default params
  7. Initialize devices: `devices.init(&mut mem)`
  8. Register state: `bx_pc_system().register_state()` (stub for now)
  9. Reset system: `bx_pc_system().Reset(ResetReason::Hardware)`
  10. Run CPU loop: `cpu.cpu_loop(&mut mem, &[&cpu])`

### File Structure Changes

```
rusty_box/
├── src/
│   ├── memory/
│   │   ├── misc_mem.rs          [ADD: load_ROM(), RomType enum]
│   │   └── mod.rs                [UPDATE: export RomType]
│   ├── iodev/
│   │   ├── devices.rs            [IMPLEMENT: init(), reset(), device handlers]
│   │   └── mod.rs                [UPDATE: export BxDevicesC properly]
│   ├── pc_system.rs              [ADD: initialize(), Reset(), set_enable_a20()]
│   └── cpu/
│       └── cpu.rs                [FIX: cpu_loop RIP increment, remove limits]
├── examples/
│   └── bios_boot.rs              [NEW: Complete BIOS boot example]
└── binaries/
    └── bios/                     [NEW: Directory for BIOS ROM files]
        └── BIOS-bochs-latest     [User provides this file]
```

## Implementation Order

1. **ROM Loading** (misc_mem.rs) - Foundation for BIOS loading
2. **PC System Enhancements** (pc_system.rs) - A20 line, reset logic
3. **Device Initialization Stubs** (devices.rs) - Basic structure, minimal devices
4. **CPU Loop Fixes** (cpu.rs) - Proper instruction execution
5. **BIOS Boot Example** (examples/bios_boot.rs) - Integration test

## Testing Strategy

1. Unit tests for `load_ROM()` with various ROM sizes and addresses
2. Integration test: Load BIOS, verify first instruction fetch from 0xfff0
3. Manual verification: Run example, check trace logs match Bochs output format
4. Compare instruction execution with Bochs logs from `bochsout.txt`

## Thread Safety Notes

- All mutable state in `BxMemoryStubC` uses `UnsafeCell<Vec<Block>>` (already safe)
- `BxMemC` holds owned `BxMemoryStubC`, no shared mutable access
- Device I/O handlers use owned closures, no shared state
- Each emulator instance is independent (can run in separate threads)

## Open Questions / Assumptions

1. **BIOS Location**: Assumes `binaries/bios/BIOS-bochs-latest` exists or will be provided by user. Will document this requirement.
2. **Device Completeness**: Starts with minimal device implementations (CMOS, PIC stubs). Full device emulation can be added incrementally.
3. **PCI Support**: Initially disabled (`pci_enabled: false`) for simplicity. Can enable later when PCI devices are implemented.

## Success Criteria

- BIOS loads correctly at 0xfffe0000 (maps to 0xe0000-0xfffff)
- CPU reset sets RIP to 0xfff0, CS to 0xf000 with base 0xffff0000
- First instruction fetch succeeds from BIOS ROM
- CPU executes at least 10-20 BIOS instructions without errors
- Instruction pointer increments correctly after each instruction
- Trace logs show similar pattern to Bochs output (register states, instruction addresses)
