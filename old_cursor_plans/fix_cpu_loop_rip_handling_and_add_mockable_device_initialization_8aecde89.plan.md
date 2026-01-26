---
name: Fix CPU Loop RIP Handling and Add Mockable Device Initialization
overview: ""
todos: []
---

# Fix CPU Loop RIP Handling and Add Mockable Device Initialization

## Overview

Fix the `cpu_loop` method to ensure RIP (instruction pointer) is correctly incremented, and add mockable device initialization infrastructure to support BIOS simulation.

## Analysis

### RIP Increment Issue

Looking at the code:

- In Rust (`src/cpu/cpu.rs:917-918`): RIP is incremented BEFORE instruction execution
- In C++ (`cpp_orig/bochs/cpu/cpu.cc:202`): RIP is also incremented BEFORE execution (`RIP += i->ilen()`)
- Jump instructions use RIP after it's been incremented (which is correct - x86 jumps are relative to the instruction AFTER the jump)

However, the user reports RIP isn't being incremented correctly. The current code DOES increment RIP, but we should verify the logic matches Bochs exactly and ensure it works correctly for all instruction types.

### Device Initialization

- Original C++ code (`cpp_orig/bochs/iodev/devices.cc:116`) has comprehensive device initialization in `bx_devices_c::init()`
- Current Rust code (`src/iodev/mod.rs`, `src/iodev/devices.rs`) has only stub implementations
- Device initialization should be mockable so BIOS simulation can work without full device implementations

## Implementation Plan

### 1. Review and Fix CPU Loop RIP Handling

**Files to modify:**

- `src/cpu/cpu.rs` - `cpu_loop` method (lines 857-939)

**Changes:**

- Verify RIP increment timing matches C++ original exactly
- Ensure RIP is correctly committed after instruction execution
- Add comments explaining the RIP increment semantics
- Verify jump instructions use the correct RIP value (after increment)

**Key observation:** The C++ code increments RIP before execution (line 202), and jump instructions use EIP/RIP after increment (which is correct for x86 relative jumps). The Rust code should match this behavior.

### 2. Add Mockable Device Initialization Infrastructure

**Files to create/modify:**

- `src/iodev/mod.rs` - Add device initialization trait and mock implementation
- `src/iodev/devices.rs` - Implement basic device initialization structure
- `src/iodev/mock.rs` (new) - Mock device implementations for testing/BIOS simulation

**Design:**

- Create a trait `DeviceInitializer` that can be implemented by real or mock devices
- Use Rust's trait system to make device initialization mockable
- Keep existing stub implementations as default/mock implementations
- Structure similar to C++ plugin system but using Rust traits instead

**Key components:**

- `BxDevicesC` struct with initialization method
- Mock device implementations that satisfy the device trait requirements
- Ability to enable/disable specific devices (like C++ plugin system)
- I/O port handler registration (stub for now, can be expanded later)

### 3. Integration

**Files to modify:**

- `src/lib.rs` - Export device initialization
- `examples/init_and_run.rs` - Add device initialization call (optional/mockable)

**Integration points:**

- Device initialization should be callable but optional
- Default to mock/stub devices
- Allow enabling real device implementations when available

## Implementation Details

### Device Initialization Trait Design

```rust
pub trait DeviceInitializer {
    fn init(&mut self, mem: &mut BxMemC) -> Result<()>;
    fn reset(&mut self, reset_type: ResetReason) -> Result<()>;
}
```



### Mock Device Implementation

- Provide no-op implementations for all device methods
- Return appropriate default values for I/O port reads
- Ignore I/O port writes
- Support BIOS simulation without full device emulation

## Testing Strategy

- Verify RIP increments correctly for sequential instructions
- Verify jump instructions calculate targets correctly using incremented RIP