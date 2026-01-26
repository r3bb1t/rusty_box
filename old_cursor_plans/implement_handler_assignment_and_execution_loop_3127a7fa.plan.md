---
name: Implement Handler Assignment and Execution Loop
overview: Implement handler assignment logic (like assignHandler in original) and execution loop (like cpu.cc:177-192) in the Rust emulator, mirroring the original C++ structure while maintaining no_std compatibility and Rust safety practices.
todos:
  - id: "1"
    content: Create opcodes_table.rs with OpFlags bitflags and BxOpcodeEntry structure (use pub(super) visibility)
    status: completed
  - id: "2"
    content: Extract handler functions from execute_instruction match statement into separate functions organized in files matching original structure (bit32.rs, shift32.rs, stack32.rs, etc.) with pub(super) visibility
    status: completed
  - id: "3"
    content: Populate BX_OPCODES_TABLE with all opcodes mapping to their handlers (use pub(super) static)
    status: in_progress
  - id: "4"
    content: Implement assign_handler method in cpu.rs matching original assignHandler logic (use pub(super) visibility)
    status: completed
  - id: "5"
    content: Add handler storage mechanism (wrapper struct or icache entry modification) with appropriate visibility
    status: completed
  - id: "6"
    content: Implement error handler functions (bx_error, bx_no_fpu, etc.) with pub(super) visibility
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
  - id: "10"
    content: Verify file structure matches original C++ structure (handler functions in correct files matching original organization)
    status: pending
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
pub(super) struct BxOpcodeEntry {
    pub(super) execute1: fn(&mut BxCpuC, &BxInstructionGenerated) -> Result<()>,
    pub(super) execute2: Option<fn(&mut BxCpuC, &BxInstructionGenerated) -> Result<()>>,
    pub(super) opflags: OpFlags,
}

pub(super) static BX_OPCODES_TABLE: &[BxOpcodeEntry] = [...];
```

**Visibility Guidelines**:

- `pub(super)` for items only needed by parent module (`cpu` module)
- `pub(crate)` for items needed across the crate but not externally
- `pub` only for items that must be accessible to end users (minimize this)

**OpFlags** (bitflags) - must support ALL features:

- `BX_PREPARE_FPU` - FPU (x87) instruction
- `BX_PREPARE_MMX` - MMX instruction
- `BX_PREPARE_SSE` - SSE instruction (requires CPU_LEVEL >= 6)
- `BX_PREPARE_AVX` - AVX instruction (requires BX_SUPPORT_AVX)
- `BX_PREPARE_EVEX` - EVEX instruction (requires BX_SUPPORT_EVEX)
- `BX_PREPARE_OPMASK` - Opmask register instruction (requires BX_SUPPORT_EVEX)
- `BX_PREPARE_AMX` - AMX instruction (requires BX_SUPPORT_AMX)
- `BX_TRACE_END` - End of trace marker
- `BX_PREPARE_EVEX_NO_BROADCAST` - EVEX instruction that doesn't support broadcast
- `BX_PREPARE_EVEX_NO_SAE` - EVEX instruction that doesn't support SAE in register form
- `BX_EVEX_VL_IGNORE` - EVEX instruction that ignores vector length
- Any other opflags from the original codebase

**FetchModeMask** flags (for feature availability checks):

- `BX_FETCH_MODE_FPU_MMX_OK` - FPU/MMX available
- `BX_FETCH_MODE_SSE_OK` - SSE available (CPU_LEVEL >= 6)
- `BX_FETCH_MODE_AVX_OK` - AVX available (BX_SUPPORT_AVX)
- `BX_FETCH_MODE_OPMASK_OK` - Opmask available (BX_SUPPORT_EVEX)
- `BX_FETCH_MODE_EVEX_OK` - EVEX available (BX_SUPPORT_EVEX)
- `BX_FETCH_MODE_AMX_OK` - AMX available (BX_SUPPORT_AMX)

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

4. Check opflags against `fetchModeMask` (matching original logic exactly):

   - Check `BX_PREPARE_FPU` / `BX_PREPARE_MMX` against `BX_FETCH_MODE_FPU_MMX_OK`
   - Check `BX_PREPARE_SSE` against `BX_FETCH_MODE_SSE_OK` (if CPU_LEVEL >= 6)
   - Check `BX_PREPARE_AVX` against `BX_FETCH_MODE_AVX_OK` (if BX_SUPPORT_AVX)
   - Check `BX_PREPARE_OPMASK` against `BX_FETCH_MODE_OPMASK_OK` (if BX_SUPPORT_EVEX)
   - Check `BX_PREPARE_EVEX` against `BX_FETCH_MODE_EVEX_OK` (if BX_SUPPORT_EVEX)
   - Check `BX_PREPARE_AMX` against `BX_FETCH_MODE_AMX_OK` (if BX_SUPPORT_AMX)
   - For EVEX instructions, check `getEvexb()` and handle:
     - `BX_PREPARE_EVEX_NO_BROADCAST` in memory form
     - `BX_PREPARE_EVEX_NO_SAE` in register form
   - If feature not available, set handler to appropriate error handler
   - Return early (1) if feature check fails

5. Return whether to stop trace:

   - Return 1 if `BX_TRACE_END` flag is set
   - Return 1 if handler is `BxError`
   - Return 0 otherwise

**Special Cases**:

- `BX_IA_MOV_Op32_GdEd` with SS segment → use `MOV32S_GdEdM`
- `BX_IA_MOV_Op32_EdGd` with SS segment → use `MOV32S_EdGdM`

### 3. Store Handler in Instruction

**Location**: `rusty_box/src/cpu/icache.rs` and instruction structure

**Original C++ Structure** (from `icache.h` and `instr.h`):

- `bxICacheEntry_c` contains:
  - `bx_phy_address pAddr` - Physical address of instruction
  - `Bit32u traceMask` - Trace mask (fetch mode)
  - `Bit32u tlen` - Trace length in instructions
  - `bxInstruction_c *i` - Pointer to instruction array (from memory pool)
- `bxInstruction_c` (in `instr.h`) contains:
  - `BxExecutePtr_tR execute1` - Function pointer for memory form or primary handler
  - `BxExecutePtr_tR execute2` - Function pointer for register form or secondary handler
  - Instruction metadata (opcode, operands, etc.)

**Rust Implementation Approach**:

- Keep `BxInstructionGenerated` in decoder unchanged (no_std compatible)
- In `rusty_box`, extend `BxIcacheEntry` to store handler function pointers:
  ```rust
  pub(super) struct BxIcacheEntry {
      pub(super) p_addr: BxPhyAddress,
      pub(super) trace_mask: u32,
      pub(super) tlen: u32,
      pub(super) instructions: &'static [BxInstructionGenerated], // From memory pool
      pub(super) handlers: Vec<(fn(&mut BxCpuC, &BxInstructionGenerated) -> Result<()>, Option<fn(&mut BxCpuC, &BxInstructionGenerated) -> Result<()>>)>, // execute1, execute2 pairs
  }
  ```

- Alternative (simpler): Store handlers in instruction wrapper when assigning, or look up from opcodes table on each execution
- **Recommended**: Store handlers in icache entry (matches original structure) - handlers are assigned once per trace and reused

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

**Thread Safety**:

- Execution loop operates on `&mut BxCpuC` - exclusive mutable access ensures thread safety
- Each CPU instance should have its own execution context
- Handler functions should use safe Rust methods (no `unsafe` unless necessary)
- If shared state is needed (e.g., SMP), ensure proper synchronization

### 5. Error Handlers

**Location**: `rusty_box/src/cpu/cpu.rs`

Implement error handlers matching original (all must be implemented):

**Visibility**: `pub(super)` - only needed within cpu module

- `bx_error` - Invalid instruction (BxError)
- `bx_no_fpu` - FPU not available (BxNoFPU)
- `bx_no_mmx` - MMX not available (BxNoMMX)
- `bx_no_sse` - SSE not available (BxNoSSE) - only if CPU_LEVEL >= 6
- `bx_no_avx` - AVX not available (BxNoAVX) - only if BX_SUPPORT_AVX
- `bx_no_evex` - EVEX not available (BxNoEVEX) - only if BX_SUPPORT_EVEX
- `bx_no_opmask` - Opmask not available (BxNoOpMask) - only if BX_SUPPORT_EVEX
- `bx_no_amx` - AMX not available (BxNoAMX) - only if BX_SUPPORT_AMX

All error handlers should generate appropriate exceptions or handle the error condition as in the original code.

### 6. File Structure

**CRITICAL**: Ensure files mirror original structure exactly:

**Original C++ Structure**:

- Handler functions organized by instruction type and operand size:
  - `bit.cc`, `bit16.cc`, `bit32.cc`, `bit64.cc` - Bit manipulation (BSF, BSR, BT, BTS, BTR, BTC, POPCNT, TZCNT, LZCNT)
  - `shift8.cc`, `shift16.cc`, `shift32.cc`, `shift64.cc` - Shift/rotate (SHL, SHR, SAR, ROL, ROR, RCL, RCR, SHLD, SHRD)
  - `stack16.cc`, `stack32.cc`, `stack64.cc` - Stack operations (PUSH, POP, PUSHA, POPA, ENTER, LEAVE)
  - `arith8.cc`, `arith16.cc`, `arith32.cc`, `arith64.cc` - Arithmetic (ADD, SUB, ADC, SBB, INC, DEC, CMP)
  - `logical8.cc`, `logical16.cc`, `logical32.cc`, `logical64.cc` - Logical (AND, OR, XOR, NOT, TEST)
  - `mult8.cc`, `mult16.cc`, `mult32.cc`, `mult64.cc` - Multiply/divide (MUL, IMUL, DIV, IDIV)
  - `data_xfer8.cc`, `data_xfer16.cc`, `data_xfer32.cc`, `data_xfer64.cc` - Data transfer (MOV, MOVSX, MOVZX)
  - `ctrl_xfer16.cc`, `ctrl_xfer32.cc`, `ctrl_xfer64.cc` - Control transfer (JMP, CALL, RET, IRET)
  - Handler functions named like: `BSF_GdEdR`, `BT_EdGdM`, `SHLD_EdGdR`, `POP_EdR`, `PUSH_EdM`, etc.

**Rust Structure Must Match**:

- Handler functions should be in files matching original:
  - Functions from `bit32.cc` → should be in `bit32.rs` (currently missing - needs to be created)
  - Functions from `shift32.cc` → should be in `shift32.rs` (currently in `shift.rs` - needs splitting)
  - Functions from `stack32.cc` → should be in `stack32.rs` (currently in `stack.rs` - needs splitting)
  - Functions already correctly placed: `logical8.rs`, `logical16.rs`, `logical32.rs`, `logical64.rs`, `mult8.rs`, `mult16.rs`, `mult32.rs`, `mult64.rs`, `arith/arith8.rs`, `arith/arith16.rs`, `arith/arith32.rs`, `data_xfer/data_xfer8.rs`, etc.

**Handler Function Naming**:

- Match original C++ function names exactly:
  - `BSF_GdEdR` → `bsf_gd_ed_r` (snake_case in Rust)
  - `BT_EdGdM` → `bt_ed_gd_m`
  - `SHLD_EdGdR` → `shld_ed_gd_r`
  - `POP_EdR` → `pop_ed_r`
  - `PUSH_EdM` → `push_ed_m`
- Use `pub(super)` visibility for all handler functions

**Opcodes Table Organization**:

- `opcodes_table.rs` ↔ opcodes table in `fetchdecode32.cc` (lines 91-98)
- Handler assignment in `cpu.rs` ↔ `assignHandler` in `fetchdecode32.cc` (lines 2041-2139)
- Execution loop in `cpu.rs` ↔ execution loop in `cpu.cc` (lines 177-192)
- Table entries reference handler functions from their respective files (bit32.rs, shift32.rs, etc.)

**File Structure Checklist**:

- ✅ `logical8.rs`, `logical16.rs`, `logical32.rs`, `logical64.rs` - Correct
- ✅ `mult8.rs`, `mult16.rs`, `mult32.rs`, `mult64.rs` - Correct
- ✅ `arith/arith8.rs`, `arith/arith16.rs`, `arith/arith32.rs` - Correct (missing arith64.rs?)
- ✅ `data_xfer/data_xfer8.rs`, `data_xfer16.rs`, `data_xfer32.rs`, `data_xfer64.rs` - Correct
- ✅ `ctrl_xfer16.rs`, `ctrl_xfer32.rs`, `ctrl_xfer64.rs` - Correct
- ❌ `bit8.rs`, `bit16.rs`, `bit32.rs`, `bit64.rs` - Missing (need to create)
- ⚠️ `shift.rs` - Should be `shift8.rs`, `shift16.rs`, `shift32.rs`, `shift64.rs` (needs splitting)
- ⚠️ `stack.rs` - Should be `stack16.rs`, `stack32.rs`, `stack64.rs` (needs splitting)

## Implementation Steps

1. **Create opcodes table** (`rusty_box/src/cpu/opcodes_table.rs`):

   - Define `OpFlags` bitflags with ALL flags from original (FPU, MMX, SSE, AVX, EVEX, OPMASK, AMX, TRACE_END, EVEX_NO_BROADCAST, EVEX_NO_SAE, EVEX_VL_IGNORE, BX_LOCKABLE, etc.)
   - Define `FetchModeMask` bitflags for feature availability
   - Define `BxOpcodeEntry` structure with execute1, execute2, and opflags (matching `bxIAOpcodeTable` structure)
   - Populate table with ALL opcodes from decoder, mapping to handler functions from their respective files:
     - Functions from `bit32.cc` → reference handlers in `bit32.rs`
     - Functions from `shift32.cc` → reference handlers in `shift32.rs`
     - Functions from `stack32.cc` → reference handlers in `stack32.rs`
     - Functions from `logical8.cc` → reference handlers in `logical8.rs`
     - etc.
   - Table format matches `bx_define_opcode` macro: `(opcode, execute1, execute2, isa, src1, src2, src3, src4, opflags)`
   - Extract handler functions from current `execute_instruction` match statement
   - Ensure table covers all opcodes including EVEX, AVX, SSE, etc.
   - Handler function names must match original C++ names (converted to snake_case)
   - **Note**: Handler functions may use macros (like `impl_eflag!` generated methods) or helper methods (like `read_8bit_regx`) - ensure these are thread-safe (they operate on `&mut self` which provides exclusive access)

2. **Implement assign_handler** (`rusty_box/src/cpu/cpu.rs`):

   - Add `assign_handler` method with signature: `pub(super) fn assign_handler(&mut self, instr: &mut BxInstructionGenerated, fetch_mode_mask: u32) -> bool`
   - Match `assignHandler` from `fetchdecode32.cc:2041-2139` exactly
   - Implement modrm check logic (modC0 check) - if `!modC0()` use execute1/execute2, else use execute2 only
   - Implement ALL opflags checking (FPU, MMX, SSE, AVX, EVEX, OPMASK, AMX)
   - Handle EVEX-specific checks (getEvexb, NO_BROADCAST, NO_SAE) - matching lines 2067-2084
   - Handle special cases (MOV with SS segment override) - matching lines 2049-2056
   - Return bool: true if trace should end, false otherwise (matching return values)
   - Match original logic exactly including all conditional compilation checks
   - Handler assignment should store function pointer in instruction wrapper or icache entry
   - **Thread Safety**: Method operates on `&mut self` providing exclusive access - thread-safe if each CPU instance is separate

3. **Modify instruction structure** (matching original `icache.h` and `instr.h`):

   - **Recommended**: Store handlers in icache entry (matches original C++ structure)
   - Extend `BxIcacheEntry` in `rusty_box/src/cpu/icache.rs` to include handler function pointers
   - Handlers are assigned once per trace during `get_icache_entry()` or `alloc_trace()`
   - Each instruction in the trace has its `execute1` and `execute2` handlers stored
   - Memory pool (`BX_ICACHE_MEM_POOL`) stores instruction structures (already exists in Rust)
   - Trace linking: `execute2` can be union with `next` pointer for trace chaining optimization (future enhancement)

4. **Refactor execution loop** (matching `cpu.cc:177-192`):

   - Replace `execute_instruction` match with handler function pointer call
   - Handler assignment happens in `get_icache_entry()` or during trace allocation (not in execution loop)
   - Execution loop:

     1. Get icache entry (contains decoded instructions with assigned handlers)
     2. Loop through trace (up to `BX_MAX_TRACE_LENGTH` instructions):

        - Call `before_execution()` hook
        - Advance RIP by instruction length
        - Call `execute1` handler function pointer
        - If `execute2` is set and needed, call it
        - Check `async_event`, break if set

     1. Get next trace from icache

   - Implement handler chaining support (trace linking via `execute2` union with `next` pointer - future optimization)

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
- **Support ALL features**: EVEX, AVX, SSE, MMX, FPU, OPMASK, AMX - ensure all opflags and feature checks are implemented
- Ensure `fetchModeMask` is properly maintained and passed to `assign_handler`
- All conditional compilation features (BX_SUPPORT_AVX, BX_SUPPORT_EVEX, BX_SUPPORT_AMX, etc.) should be supported in Rust

## Macros and Thread Safety

### Macros in Original C++ Code

The original C++ code uses extensive macros for register access and instruction execution:

**Register Access Macros** (from `cpu.h`):

- `BX_READ_8BIT_REG(index)`, `BX_READ_16BIT_REG(index)`, `BX_READ_32BIT_REG(index)`, `BX_READ_64BIT_REG(index)`
- `BX_WRITE_8BIT_REG(index, val)`, `BX_WRITE_16BIT_REG(index, val)`, `BX_WRITE_32BIT_REG(index, val)`, `BX_WRITE_32BIT_REGZ(index, val)`
- `BX_READ_8BIT_REGx(index, extended)` - handles extended 8-bit registers (AH, BH, CH, DH)
- `BX_WRITE_8BIT_REGx(index, extended, val)` - handles extended 8-bit registers
- `BX_CPU_THIS_PTR` - pointer to current CPU instance (not thread-safe - direct pointer access)
- Register shortcuts: `AL`, `CL`, `EAX`, `RAX`, `RIP`, etc. (all expand to `BX_CPU_THIS_PTR gen_reg[...]`)

**Instruction Execution Macros**:

- `BX_NEXT_INSTR(i)` - advances to next instruction (sets RIP, handles trace linking)
- `BX_CPU_RESOLVE_ADDR(i)` - resolves effective address from ModRM
- `BX_CPU_RESOLVE_ADDR_32(i)`, `BX_CPU_RESOLVE_ADDR_64(i)` - address resolution with size

**Memory Access Macros**:

- `read_virtual_byte/word/dword/qword(seg, addr)` - read from virtual address
- `write_virtual_byte/word/dword/qword(seg, addr, val)` - write to virtual address
- `read_RMW_virtual_byte/word/dword/qword(seg, addr)` - read-modify-write read
- `write_RMW_linear_byte/word/dword/qword(val)` - read-modify-write write

### Macros in Rust Code

The Rust code uses macros for code generation:

**Code Generation Macros**:

- `impl_eflag!` (in `cpu_macros.rs`) - generates EFLAGS accessor methods (`get_cf`, `set_cf`, `assert_cf`, `clear_cf`, etc.)
- `impl_crreg_accessors!` (in `crregs.rs`) - generates control register bit accessors
- `impl_drreg_accessors!` (in `crregs.rs`) - generates debug register accessors

**Register Access Methods** (not macros, but equivalent functionality):

- `get_gpr8(index)`, `get_gpr16(index)`, `get_gpr32(index)`, `get_gpr64(index)`
- `set_gpr8(index, val)`, `set_gpr16(index, val)`, `set_gpr32(index, val)`, `set_gpr64(index, val)`
- `read_8bit_regx(index, extended)` - handles extended 8-bit registers (matches `BX_READ_8BIT_REGx`)
- `write_8bit_regx(index, extended, val)` - handles extended 8-bit registers (matches `BX_WRITE_8BIT_REGx`)

### Thread Safety Requirements

**Original C++ Code**: Not thread-safe

- Uses `BX_CPU_THIS_PTR` which is a direct pointer to CPU instance
- No synchronization mechanisms
- Assumes single-threaded execution per CPU instance
- In SMP mode, each CPU has its own instance, but no cross-CPU synchronization

**Rust Code Requirements**: Must be thread-safe

- Handler functions receive `&mut BxCpuC` - exclusive mutable reference (Rust's ownership system provides thread safety)
- Use `UnsafeCell` only when necessary for interior mutability (already used in `cpu.rs`)
- Handler functions should NOT use `unsafe` blocks unless absolutely necessary
- If shared state is needed (e.g., for SMP), use proper synchronization (Arc, Mutex, RwLock) but avoid unnecessary overhead
- Handler assignment and execution should be safe for concurrent access if CPU instances are separate

**Thread Safety Considerations for Handler Assignment**:

- Handler function pointers are static (immutable) - safe to share across threads
- Opcodes table is static (immutable) - safe to share across threads
- Instruction structures from decoder are immutable - safe to share
- CPU state (`BxCpuC`) should have exclusive mutable access (`&mut`) during execution
- If multiple CPU instances exist (SMP), each should have its own `BxCpuC` instance
- Handler assignment modifies instruction wrapper or icache entry - ensure this is thread-safe if icache is shared

**Implementation Guidelines**:

- Handler functions should use safe Rust methods for register/memory access (no `unsafe` unless necessary)
- If `UnsafeCell` is used, document why and ensure proper synchronization
- Avoid global mutable state - prefer passing state through function parameters
- Handler function pointers are `Send + Sync` (function pointers are thread-safe)
- Opcodes table is `Send + Sync` (static immutable data)

## Visibility Guidelines

**Critical**: Minimize use of `pub` keyword. Follow this hierarchy:

1. **`pub(super)`** - Use for items only needed by parent module (`cpu` module):

   - `BxOpcodeEntry` struct
   - `BX_OPCODES_TABLE` static
   - `assign_handler` method
   - Error handler functions (bx_error, bx_no_fpu, etc.)
   - OpFlags and FetchModeMask bitflags
   - Helper functions for handler assignment

2. **`pub(crate)`** - Use only when `pub(super)` isn't sufficient (needed across crate but not externally):

   - Only if other modules in `rusty_box` crate need access
   - Should be rare - most things should be `pub(super)`

3. **`pub`** - Use ONLY for items that must be accessible to end users:

   - Should be extremely rare
   - Only for public API that external code needs

**General Rule**: Start with `pub(super)`, only escalate to `pub(crate)` or `pub` if absolutely necessary. Most implementation details should be `pub(super)`.

## File Structure Requirements

**CRITICAL**: Handler functions must be organized in files matching the original C++ structure:

1. **Handler functions should be in files matching original**:

   - Functions from `bit32.cc` (BSF, BSR, BT, BTS, BTR, BTC, POPCNT, TZCNT, LZCNT) → `bit32.rs`
   - Functions from `shift32.cc` (SHL, SHR, SAR, ROL, ROR, RCL, RCR, SHLD, SHRD) → `shift32.rs`
   - Functions from `stack32.cc` (PUSH, POP, PUSHA, POPA, ENTER, LEAVE) → `stack32.rs`
   - Functions already correctly placed should remain in their files

2. **Handler function naming**:

   - Match original C++ names exactly (converted to snake_case)
   - Example: `BX_CPU_C::BSF_GdEdR` → `pub(super) fn bsf_gd_ed_r`
   - Example: `BX_CPU_C::BT_EdGdM` → `pub(super) fn bt_ed_gd_m`

3. **Opcodes table references**:

   - Table entries reference handler functions from their respective files
   - Example: `Opcode::BsfGdEd` → `bit32::bsf_gd_ed_r` (from `bit32.rs`)

4. **Missing files** (noted but may be created separately):

   - `bit8.rs`, `bit16.rs`, `bit32.rs`, `bit64.rs` - Currently missing
   - `shift8.rs`, `shift16.rs`, `shift32.rs`, `shift64.rs` - Currently consolidated in `shift.rs`
   - `stack16.rs`, `stack32.rs`, `stack64.rs` - Currently consolidated in `stack.rs`

**Note**: For this implementation, focus on creating the opcodes table and handler assignment system. File splitting (shift.rs, stack.rs) and creation of missing bit*.rs files can be done as a separate task, but handler functions should be organized correctly when extracted from the match statement.

**Macro Usage in Handlers**:

- Handler functions may use macro-generated methods (e.g., `impl_eflag!` generated `get_cf()`, `set_cf()`, etc.)
- Handler functions may use helper methods that replace C++ macros (e.g., `read_8bit_regx()` replaces `BX_READ_8BIT_REGx()`)
- All macro-generated code should maintain thread-safety (operate on `&mut self` for exclusive access)
- When extracting handlers from match statement, preserve any macro usage or helper method calls

## Instruction Cache and Trace Management

**Original C++ Structure** (from `icache.h`):

- **ICache Entry**: `bxICacheEntry_c` contains:
  - Physical address (`pAddr`)
  - Trace mask (`traceMask`) - fetch mode for feature checks
  - Trace length (`tlen`) - number of instructions in trace (max `BX_MAX_TRACE_LENGTH = 32`)
  - Instruction array pointer (`bxInstruction_c *i`) - from memory pool
- **Memory Pool**: `BX_ICACHE_MEM_POOL` (576KB) stores instruction structures
- **ICache Size**: `BxICacheEntries` (64K entries, power of 2)
- **Handler Storage**: Each `bxInstruction_c` has `execute1` and `execute2` function pointers

**Rust Implementation**:

- **ICache Entry**: Extend `BxIcacheEntry` in `rusty_box/src/cpu/icache.rs`:
  - Store handler function pointers for each instruction in trace
  - Handlers assigned during trace allocation (in `get_icache_entry()` or `alloc_trace()`)
  - Trace mask used for feature availability checks in `assign_handler`
- **Memory Pool**: Already exists (`BX_ICACHE_MEM_POOL`) - stores `BxInstructionGenerated` structures
- **Trace Execution**: Loop through trace executing handlers, checking `async_event` after each instruction
- **SMC (Self-Modifying Code) Handling**: Original uses `bxPageWriteStampTable` to detect code modifications - ensure Rust implementation handles this correctly