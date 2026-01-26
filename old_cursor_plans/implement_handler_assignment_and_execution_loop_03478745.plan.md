---
name: Implement Handler Assignment and Execution Loop
overview: Implement handler assignment logic (like assignHandler in original) and execution loop (like cpu.cc:177-192) in the Rust emulator, mirroring the original C++ structure while maintaining no_std compatibility and Rust safety practices.
todos:
  - id: "1"
    content: Create opcodes_table.rs with OpFlags bitflags and BxOpcodeEntry structure
    status: completed
  - id: "2"
    content: Extract handler functions from execute_instruction match statement into separate functions
    status: completed
  - id: "3"
    content: Populate BX_OPCODES_TABLE with all opcodes mapping to their handlers
    status: in_progress
  - id: "4"
    content: Implement assign_handler method in cpu.rs matching original assignHandler logic
    status: completed
  - id: "5"
    content: Add handler storage mechanism (wrapper struct or icache entry modification)
    status: completed
  - id: "6"
    content: Implement error handler functions (bx_error, bx_no_fpu, etc.)
    status: completed
  - id: "7"
    content: Modify execution loop to use handler assignment and function pointer calls
    status: completed
  - id: "8"
    content: Handle special cases in assign_handler (MOV with SS segment override)
    status: completed
  - id: "9"
    content: Test compilation and verify all instructions still work correctly
    status: completed
isProject: false
---

# Implementation Plan: Handler Assignment and Execution Loop

## Overview

Implement the handler assignment system and execution loop from the original Bochs codebase, ensuring the Rust implementation mirrors the C++ structure while maintaining no_std compatibility and following Rust best practices.

## Key Components

### 1. Opcodes Table Structure

**Location**: `rusty_box/src/cpu/opcodes_table.rs` (new file)

Create a table structure similar to `BxOpcodesTable` in the original:

- Store handler function pointers for `execute1` and `execute2`
- Store opflags (feature requirements, trace end flags)
- Map from `Opcode` enum to handler information

**Structure**:

```rust
pub struct BxOpcodeEntry {
    pub execute1: fn(&mut BxCpuC, &BxInstructionGenerated) -> Result<()>,
    pub execute2: Option<fn(&mut BxCpuC, &BxInstructionGenerated) -> Result<()>>,
    pub opflags: OpFlags,
}

pub static BX_OPCODES_TABLE: &[BxOpcodeEntry] = [...];
```

**OpFlags** (bitflags):

- `BX_PREPARE_FPU`, `BX_PREPARE_MMX`, `BX_PREPARE_SSE`, `BX_PREPARE_AVX`, `BX_PREPARE_EVEX`, `BX_PREPARE_OPMASK`, `BX_PREPARE_AMX`
- `BX_TRACE_END`
- `BX_PREPARE_EVEX_NO_BROADCAST`, `BX_PREPARE_EVEX_NO_SAE`

### 2. Handler Assignment Function

**Location**: `rusty_box/src/cpu/cpu.rs`

Implement `assign_handler` method matching `assignHandler` from `fetchdecode32.cc:2041-2139`:

**Logic**:

1. Get opcode from instruction
2. If `!modC0()` (memory form):

   - Set `execute1` from table's `execute1`
   - Set `execute2` from table's `execute2`
   - Handle special cases (e.g., MOV with SS segment override)

3. Else (register form):

   - Set `execute1` from table's `execute2`
   - Clear `execute2`

4. Check opflags against `fetchModeMask`:

   - If feature not available, set handler to error handler
   - Return early if feature check fails

5. Return whether to stop trace (`BX_TRACE_END` flag or error handler)

**Special Cases**:

- `BX_IA_MOV_Op32_GdEd` with SS segment → use `MOV32S_GdEdM`
- `BX_IA_MOV_Op32_EdGd` with SS segment → use `MOV32S_EdGdM`

### 3. Store Handler in Instruction

**Location**: `rusty_box_decoder/src/instr_generated.rs`

Add optional handler field to `BxInstructionGenerated`:

- Since decoder should be no_std and independent, we can't store function pointers there
- **Alternative**: Store handler ID/index and look it up in `rusty_box`
- **Better Alternative**: Add handler field only in `rusty_box` wrapper, or use a separate structure

**Recommended Approach**:

- Keep `BxInstructionGenerated` in decoder unchanged (no_std compatible)
- In `rusty_box`, create a wrapper or extend the instruction with handler information when assigning
- Store handler in icache entry or instruction cache, not in the decoder structure

### 4. Execution Loop Modification

**Location**: `rusty_box/src/cpu/cpu.rs` (around line 1000-1300)

Modify the execution loop to match `cpu.cc:177-192`:

**Current**: Uses `execute_instruction` with large match statement

**New**:

1. Get instruction from icache
2. Call `assign_handler` to set handler
3. In loop:

   - Call `before_execution` hook
   - Advance RIP by instruction length
   - Call handler function pointer (execute1)
   - Check `async_event`, break if set
   - Get next instruction from trace

**Handler Chaining**:

- If `execute2` is set and needed, call it after `execute1`
- Support trace linking via `execute2` union with `next` pointer (for future optimization)

### 5. Error Handlers

**Location**: `rusty_box/src/cpu/cpu.rs`

Implement error handlers matching original:

- `bx_error` - Invalid instruction
- `bx_no_fpu` - FPU not available
- `bx_no_mmx` - MMX not available
- `bx_no_sse` - SSE not available
- `bx_no_avx` - AVX not available
- `bx_no_evex` - EVEX not available
- `bx_no_opmask` - Opmask not available
- `bx_no_amx` - AMX not available

### 6. File Structure

Ensure files mirror original structure:

- `opcodes_table.rs` ↔ opcodes table in `fetchdecode32.cc`
- Handler assignment in `cpu.rs` ↔ `assignHandler` in `fetchdecode32.cc`
- Execution loop in `cpu.rs` ↔ execution loop in `cpu.cc`

## Implementation Steps

1. **Create opcodes table** (`rusty_box/src/cpu/opcodes_table.rs`):

   - Define `OpFlags` bitflags
   - Define `BxOpcodeEntry` structure
   - Populate table with all opcodes, mapping current `execute_instruction` match arms to handler functions
   - Extract handler functions from current match statement

2. **Implement assign_handler** (`rusty_box/src/cpu/cpu.rs`):

   - Add `assign_handler` method
   - Implement modrm check logic
   - Implement opflags checking
   - Handle special cases (MOV with SS segment)

3. **Modify instruction structure**:

   - Option A: Add handler field to instruction wrapper in `rusty_box`
   - Option B: Store handler in icache entry
   - Option C: Look up handler on each execution (less efficient but simpler)

4. **Refactor execution loop**:

   - Replace `execute_instruction` match with handler call
   - Add handler assignment before execution
   - Implement handler chaining support

5. **Implement error handlers**:

   - Create error handler functions
   - Map them in opcodes table for error cases

6. **Testing**:

   - Ensure all instructions still work
   - Verify handler assignment logic matches original
   - Check that execution loop behavior matches original

## Notes

- Keep `rusty_box_decoder` no_std compatible - don't add handler storage there
- Use function pointers (`fn`) not closures to allow static storage
- Handler functions should have signature: `fn(&mut BxCpuC, &BxInstructionGenerated) -> Result<()>`
- Use `UnsafeCell` if needed for interior mutability, but prefer safe patterns
- Mirror original file structure - don't invent new abstractions
- Keep handler assignment in `rusty_box`, not `rusty_box_decoder` as requested