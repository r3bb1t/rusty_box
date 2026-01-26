---
name: Implement Missing CPU Instructions
overview: Implement all missing instruction handlers from the specified C++ files into Rust, ensuring they're added to the execute_instruction match statement. This includes control transfer, data transfer, logical, multiplication, and infrastructure instructions.
todos:
  - id: ctrl_xfer_16
    content: "Implement missing 16-bit control transfer instructions: far calls (CALL16_Ap, CALL16_Ep), far jumps (JMP16_Ap, JMP16_Ep), far returns (RETfar16_Iw), IRET16, conditional jumps with 16-bit displacement, JECXZ"
    status: completed
  - id: ctrl_xfer_32
    content: "Implement missing 32-bit control transfer instructions: far calls (CALL32_Ap, CALL32_Ep), far jumps (JMP32_Ap, JMP32_Ep), far returns (RETfar32_Iw), IRET32, conditional jumps with 32-bit displacement, loop instructions (32-bit)"
    status: completed
  - id: ctrl_xfer_64
    content: "Implement missing 64-bit control transfer instructions: all 64-bit variants of calls, jumps, returns, IRET64, conditional jumps, loops, JRCXZ"
    status: completed
  - id: jmp_far_helpers
    content: "Implement far jump helpers from jmp_far.cc: jump_protected, task_gate, jmp_call_gate, jmp_call_gate64"
    status: pending
  - id: ret_far_helpers
    content: "Implement far return helpers from ret_far.cc: return_protected (for protected mode far returns)"
    status: pending
  - id: data_xfer_8
    content: "Implement missing 8-bit data transfer: MOV memory forms, XCHG, XLAT"
    status: completed
  - id: data_xfer_16
    content: "Implement missing 16-bit data transfer: MOV memory forms, MOVZX, MOVSX, XCHG, CMOV variants, segment register MOV"
    status: completed
  - id: data_xfer_32
    content: "Implement missing 32-bit data transfer: MOV memory forms, MOVZX, MOVSX, XCHG, CMOV variants (with BX_CLEAR_64BIT_HIGH)"
    status: completed
  - id: data_xfer_64
    content: "Create data_xfer64.rs and implement all 64-bit data transfer instructions: MOV, MOVZX, MOVSX, XCHG, CMOV variants"
    status: pending
  - id: logical_8
    content: "Implement missing 8-bit logical: OR, AND, XOR memory forms, NOT, TEST variants"
    status: pending
  - id: logical_16
    content: "Implement missing 16-bit logical: OR, AND, XOR memory forms, NOT, TEST variants, ZERO_IDIOM"
    status: pending
  - id: logical_32
    content: "Implement missing 32-bit logical: OR, AND, XOR memory forms, NOT, TEST variants, ZERO_IDIOM"
    status: pending
  - id: logical_64
    content: Implement all 64-bit logical instruction variants
    status: pending
  - id: mult_all
    content: "Implement all multiplication instructions in mult.rs: MUL, IMUL, DIV, IDIV for 8/16/32/64-bit with proper exception handling"
    status: pending
  - id: lazy_flags
    content: "Complete lazy_flags.rs implementation: all flag getters/setters, SET_FLAGS macros, match C++ lazy flags behavior exactly"
    status: pending
  - id: scalar_arith
    content: "Create scalar_arith.rs with helper functions: parity_byte, tzcnt*, lzcnt*, popcnt*, bextr*, rol*, ror*"
    status: pending
  - id: infrastructure_rao
    content: Check and implement RAO instructions (AADD, etc.) if opcodes exist
    status: pending
  - id: infrastructure_rdrand
    content: Implement RDRAND instructions (RDRAND_Ew, RDRAND_Ed, RDRAND_Eq) with hardware random generator
    status: pending
  - id: infrastructure_mwait
    content: Check and implement MONITOR/MWAIT instructions if opcodes exist
    status: pending
  - id: load_instructions
    content: Check if LOAD_* helpers from load.cc are needed and implement if used
    status: pending
  - id: execute_match
    content: Add all missing opcodes to execute_instruction match statement in cpu.rs, ensuring proper error handling and trace breaking behavior
    status: pending
---

# Implementation Plan: Missing CPU Instructions

## Overview

Implement missing instruction handlers from C++ Bochs files into Rust, ensuring exact behavioral parity while leveraging Rust's type safety. All implementations must be added to the `execute_instruction` match statement in `rusty_box/src/cpu/cpu.rs`.

## File Structure Mapping

### Control Transfer Instructions

- **C++**: `ctrl_xfer16.cc`, `ctrl_xfer32.cc`, `ctrl_xfer64.cc`
- **Rust**: `rusty_box/src/cpu/ctrl_xfer.rs` (exists, needs expansion)

### Data Transfer Instructions  

- **C++**: `data_xfer8.cc`, `data_xfer16.cc`, `data_xfer32.cc`, `data_xfer64.cc`
- **Rust**: `rusty_box/src/cpu/data_xfer/data_xfer8.rs`, `data_xfer16.rs`, `data_xfer32.rs` (exist, need expansion)
- **New**: Create `rusty_box/src/cpu/data_xfer/data_xfer64.rs` for 64-bit variants

### Logical Instructions

- **C++**: `logical8.cc`, `logical16.cc`, `logical32.cc`, `logical64.cc`
- **Rust**: `rusty_box/src/cpu/logical.rs` (exists, needs expansion)
- **New**: May need separate files for 64-bit variants

### Multiplication Instructions

- **C++**: `mult8.cc`, `mult16.cc`, `mult32.cc`, `mult64.cc`
- **Rust**: `rusty_box/src/cpu/mult.rs` (exists but empty, needs full implementation)

### Far Jump/Call/Return

- **C++**: `jmp_far.cc`, `ret_far.cc`
- **Rust**: `rusty_box/src/cpu/ctrl_xfer.rs` (partially implemented, needs expansion)

### Helper Functions

- **C++**: `lazy_flags.h`, `load.cc`, `scalar_arith.h`
- **Rust**: `rusty_box/src/cpu/lazy_flags.rs` (basic structure exists), create new modules as needed

### Infrastructure

- **C++**: `rao.cc`, `rdrand.cc`, `mwait.cc`, `paging.cc`, `proc_ctrl.cc`, `protect_ctrl.cc`
- **Rust**: Check existing files, implement missing helpers

## Implementation Tasks

### 1. Control Transfer Instructions (`ctrl_xfer.rs`)

**Missing from C++ `ctrl_xfer16.cc`:**

- `CALL16_Ap` - Far call with absolute pointer (16-bit)
- `CALL16_Ep` - Far call indirect (16-bit)
- `JMP16_Ap` - Far jump with absolute pointer (16-bit)  
- `JMP16_Ep` - Far jump indirect (16-bit)
- `RETfar16_Iw` - Far return with immediate (16-bit)
- `IRET16` - Interrupt return (16-bit)
- All conditional jumps with 16-bit displacement (Jw variants)
- `JECXZ_Jb` - Jump if ECX zero (32-bit mode)

**Missing from C++ `ctrl_xfer32.cc`:**

- `CALL32_Ap` - Far call with absolute pointer (32-bit)
- `CALL32_Ep` - Far call indirect (32-bit)
- `JMP32_Ap` - Far jump with absolute pointer (32-bit)
- `JMP32_Ep` - Far jump indirect (32-bit)
- `RETfar32_Iw` - Far return with immediate (32-bit)
- `IRET32` - Interrupt return (32-bit)
- All conditional jumps with 32-bit displacement (Jd variants)
- `JECXZ_Jb` - Jump if ECX zero (32-bit mode)
- `LOOPNE32_Jb`, `LOOPE32_Jb`, `LOOP32_Jb` - Loop instructions (32-bit)

**Missing from C++ `ctrl_xfer64.cc`:**

- `RETnear64_Iw` - Near return (64-bit)
- `RETfar64_Iw` - Far return (64-bit)
- `CALL_Jq` - Near call (64-bit)
- `CALL_EqR` - Near call indirect (64-bit)
- `CALL64_Ep` - Far call indirect (64-bit)
- `JMP_Jq` - Near jump (64-bit)
- `JMP_EqR` - Near jump indirect (64-bit)
- `JMP64_Ep` - Far jump indirect (64-bit)
- `IRET64` - Interrupt return (64-bit)
- All conditional jumps with 64-bit displacement (Jq variants)
- `JRCXZ_Jb` - Jump if RCX zero (64-bit)
- `LOOPNE64_Jb`, `LOOPE64_Jb`, `LOOP64_Jb` - Loop instructions (64-bit)

**Implementation approach:**

- Extend `ctrl_xfer.rs` with missing functions
- Match C++ function signatures and behavior exactly
- Use `branch_near16`, `branch_near32`, `branch_near64` helpers
- Implement `call_far16`, `call_far32`, `jmp_far16`, `jmp_far32` from `jmp_far.cc`
- Implement `return_protected` helper from `ret_far.cc` for protected mode returns

### 2. Data Transfer Instructions

**Missing from C++ `data_xfer8.cc`:**

- `MOV_EbIbR` - MOV r/m8, imm8 (register form)
- `MOV_EbIbM` - MOV r/m8, imm8 (memory form)
- `MOV_EbGbM` - MOV r/m8, r8 (memory form)
- `MOV_GbEbM` - MOV r8, r/m8 (memory form)
- `MOV_GbEbR` - MOV r8, r8 (register form)
- `XLAT` - Table lookup translation

**Missing from C++ `data_xfer16.cc`:**

- `MOV_EwIwM` - MOV r/m16, imm16 (memory form)
- `MOV_EwIwR` - MOV r/m16, imm16 (register form)
- `MOV_EwGwM` - MOV r/m16, r16 (memory form)
- `MOV_GwEwM` - MOV r16, r/m16 (memory form)
- `MOV_GwEwR` - MOV r16, r16 (register form)
- `MOV_EwSwR` - MOV r/m16, Sreg (register form)
- `MOV_EwSwM` - MOV r/m16, Sreg (memory form)
- `MOV_SwEw` - MOV Sreg, r/m16
- `LEA_GwM` - LEA r16, m16
- `MOVZX_GwEbM`, `MOVZX_GwEbR` - MOVZX r16, r/m8
- `MOVSX_GwEbM`, `MOVSX_GwEbR` - MOVSX r16, r/m8
- `XCHG_EwGwM`, `XCHG_EwGwR` - XCHG r16, r/m16
- All `CMOV` conditional move instructions (16-bit)

**Missing from C++ `data_xfer32.cc`:**

- `MOV_EdIdM` - MOV r/m32, imm32 (memory form)
- `MOV_EdIdR` - MOV r/m32, imm32 (register form)
- `MOV32_EdGdM` - MOV r/m32, r32 (memory form, 32-bit addressing)
- `MOV32_GdEdM` - MOV r32, r/m32 (memory form, 32-bit addressing)
- `MOV32S_EdGdM`, `MOV32S_GdEdM` - Stack variants
- `LEA_GdM` - LEA r32, m32
- `MOV_EAXOd`, `MOV_OdEAX` - MOV EAX, moffs32
- `MOVZX_GdEbM`, `MOVZX_GdEbR` - MOVZX r32, r/m8
- `MOVZX_GdEwM`, `MOVZX_GdEwR` - MOVZX r32, r/m16
- `MOVSX_GdEbM`, `MOVSX_GdEbR` - MOVSX r32, r/m8
- `MOVSX_GdEwM`, `MOVSX_GdEwR` - MOVSX r32, r/m16
- `XCHG_EdGdM`, `XCHG_EdGdR` - XCHG r32, r/m32
- All `CMOV` conditional move instructions (32-bit) - note: these call `BX_CLEAR_64BIT_HIGH`

**Missing from C++ `data_xfer64.cc`:**

- All 64-bit MOV variants
- `MOV_RRXIq` - MOV r64, imm64
- `MOV64_GdEdM`, `MOV64_EdGdM` - 32-bit MOV in 64-bit mode
- `MOV_EqGqM`, `MOV_GqEqM`, `MOV_GqEqR` - 64-bit MOV
- `MOV_EqIdM`, `MOV_EqIdR` - MOV r/m64, imm32 (sign-extended)
- `LEA_GqM` - LEA r64, m64
- All MOVZX/MOVSX 64-bit variants
- `XCHG_EqGqM`, `XCHG_EqGqR` - XCHG r64, r/m64
- All `CMOV` conditional move instructions (64-bit)

**Implementation approach:**

- Extend existing `data_xfer8.rs`, `data_xfer16.rs`, `data_xfer32.rs`
- Create new `data_xfer64.rs` for 64-bit variants
- Use memory access helpers (`mem_read_byte`, `mem_write_byte`, etc.)
- For CMOV instructions, match C++ behavior: always clear high 64 bits for 32-bit operations
- Implement `BX_CLEAR_64BIT_HIGH` equivalent for register clearing

### 3. Logical Instructions

**Missing from C++ `logical8.cc`:**

- `OR_EbIbM`, `OR_EbIbR` - OR r/m8, imm8
- `OR_EbGbM`, `OR_GbEbM`, `OR_GbEbR` - OR r/m8, r8
- `AND_EbIbM`, `AND_EbIbR` - AND r/m8, imm8
- `AND_EbGbM`, `AND_GbEbM`, `AND_GbEbR` - AND r/m8, r8
- `XOR_EbIbM`, `XOR_EbIbR` - XOR r/m8, imm8
- `XOR_EbGbM`, `XOR_GbEbM`, `XOR_GbEbR` - XOR r/m8, r8
- `NOT_EbM`, `NOT_EbR` - NOT r/m8
- `TEST_EbIbM`, `TEST_EbIbR` - TEST r/m8, imm8
- `TEST_EbGbM`, `TEST_EbGbR` - TEST r/m8, r8

**Missing from C++ `logical16.cc`:**

- `ZERO_IDIOM_GwR` - XOR r16, r16 (zero idiom optimization)
- `XOR_EwIwM`, `XOR_EwIwR` - XOR r/m16, imm16
- `XOR_EwGwM`, `XOR_GwEwM`, `XOR_GwEwR` - XOR r/m16, r16
- `OR_EwIwM`, `OR_EwIwR` - OR r/m16, imm16
- `OR_EwGwM`, `OR_GwEwM`, `OR_GwEwR` - OR r/m16, r16
- `AND_EwIwM`, `AND_EwIwR` - AND r/m16, imm16
- `AND_EwGwM`, `AND_GwEwM`, `AND_GwEwR` - AND r/m16, r16
- `NOT_EwM`, `NOT_EwR` - NOT r/m16
- `TEST_EwIwM`, `TEST_EwIwR` - TEST r/m16, imm16
- `TEST_EwGwM`, `TEST_EwGwR` - TEST r/m16, r16

**Missing from C++ `logical32.cc`:**

- `ZERO_IDIOM_GdR` - XOR r32, r32 (zero idiom optimization)
- `XOR_EdIdM`, `XOR_EdIdR` - XOR r/m32, imm32
- `XOR_EdGdM`, `XOR_GdEdM`, `XOR_GdEdR` - XOR r/m32, r32
- `OR_EdIdM`, `OR_EdIdR` - OR r/m32, imm32
- `OR_EdGdM`, `OR_GdEdM`, `OR_GdEdR` - OR r/m32, r32
- `AND_EdIdM`, `AND_EdIdR` - AND r/m32, imm32
- `AND_EdGdM`, `AND_GdEdM`, `AND_GdEdR` - AND r/m32, r32
- `NOT_EdM`, `NOT_EdR` - NOT r/m32
- `TEST_EdIdM`, `TEST_EdIdR` - TEST r/m32, imm32
- `TEST_EdGdM`, `TEST_EdGdR` - TEST r/m32, r32

**Missing from C++ `logical64.cc`:**

- All 64-bit variants of above instructions

**Implementation approach:**

- Extend `logical.rs` with missing functions
- Use `SET_FLAGS_OSZAPC_LOGIC_8/16/32/64` equivalents from lazy flags
- Match C++ flag update behavior exactly
- For memory forms, use RMW (Read-Modify-Write) pattern matching C++

### 4. Multiplication Instructions (`mult.rs`)

**From C++ `mult8.cc`:**

- `MUL_ALEbR` - MUL AL, r/m8 (unsigned multiply)
- `IMUL_ALEbR` - IMUL AL, r/m8 (signed multiply)
- `DIV_ALEbR` - DIV AL, r/m8 (unsigned divide)
- `IDIV_ALEbR` - IDIV AL, r/m8 (signed divide)

**From C++ `mult16.cc`:**

- `MUL_AXEwR` - MUL AX, r/m16
- `IMUL_AXEwR` - IMUL AX, r/m16
- `DIV_AXEwR` - DIV AX, r/m16
- `IDIV_AXEwR` - IDIV AX, r/m16

**From C++ `mult32.cc`:**

- `MUL_EAXEdR` - MUL EAX, r/m32
- `IMUL_EAXEdR` - IMUL EAX, r/m32
- `DIV_EAXEdR` - DIV EAX, r/m32
- `IDIV_EAXEdR` - IDIV EAX, r/m32

**From C++ `mult64.cc`:**

- All 64-bit variants

**Implementation approach:**

- Implement all functions in `mult.rs`
- Match C++ exception handling (divide by zero, overflow)
- Use lazy flags for flag updates (`SET_FLAGS_OSZAPC_LOGIC_8` for MUL)
- For IMUL, check for overflow and set OF/CF accordingly
- For DIV/IDIV, check for divide by zero and quotient overflow

### 5. Far Jump/Call/Return Helpers

**From C++ `jmp_far.cc`:**

- `jump_protected` - Protected mode far jump handler
- `task_gate` - Task gate handling
- `jmp_call_gate` - Call gate handling for jumps
- `jmp_call_gate64` - 64-bit call gate handling

**From C++ `ret_far.cc`:**

- `return_protected` - Protected mode far return handler
- Shadow stack support (if `BX_SUPPORT_CET` is enabled)

**Implementation approach:**

- Add to `ctrl_xfer.rs` or create separate module
- Match C++ protected mode logic exactly
- Handle descriptor validation, privilege checks
- Support real mode, protected mode, and long mode variants

### 6. Lazy Flags Implementation

**From C++ `lazy_flags.h`:**

- Complete `bx_lazyflags_entry` struct implementation
- All flag getter/setter methods
- `SET_FLAGS_OSZAPC_*` macros
- `SET_FLAGS_OSZAP_*` macros
- `SET_FLAGS_OSZAxC_LOGIC_*` macros

**Implementation approach:**

- Extend `lazy_flags.rs` with full implementation
- Match C++ bit manipulation exactly
- Use Rust's bit manipulation operators
- Implement all helper macros as functions

### 7. Load Instructions

**From C++ `load.cc`:**

- `LOAD_Eb`, `LOAD_Ew`, `LOAD_Ed`, `LOAD_Eq` - Load operands for two-stage instructions
- XMM/AVX load variants (if needed)

**Implementation approach:**

- Check if these are used in the Rust codebase
- If used, implement matching helpers
- These are typically used for instructions with memory operands that need two-stage execution

### 8. Scalar Arithmetic Helpers

**From C++ `scalar_arith.h`:**

- `parity_byte` - Calculate parity
- `tzcntw`, `tzcntd`, `tzcntq` - Count trailing zeros
- `lzcntw`, `lzcntd`, `lzcntq` - Count leading zeros
- `popcntb`, `popcntw`, `popcntd`, `popcntq` - Population count
- `bextrd`, `bextrq` - Bit field extract
- `rol8`, `rol16`, `rol32`, `rol64` - Rotate left
- `ror8`, `ror16`, `ror32`, `ror64` - Rotate right

**Implementation approach:**

- Create `rusty_box/src/cpu/scalar_arith.rs` module
- Use Rust's built-in functions where available (`count_zeros`, `count_ones`, etc.)
- Match C++ bit manipulation exactly

### 9. Infrastructure Instructions

**From C++ `rao.cc`:**

- `AADD_EdGdM`, `AADD_EqGqM` - Atomic ADD instructions
- Other RAO (Restricted Transactional Memory) instructions

**From C++ `rdrand.cc`:**

- `RDRAND_Ew`, `RDRAND_Ed`, `RDRAND_Eq` - Random number generation
- Hardware random generator helpers

**From C++ `mwait.cc`:**

- `MONITOR` - Monitor instruction
- `MWAIT` - Monitor wait instruction
- `UMONITOR`, `UMWAIT` - User mode variants

**Implementation approach:**

- Check for existing implementations
- Implement as instruction handlers if opcodes exist
- Add helper functions for infrastructure support

### 10. Integration into `execute_instruction`

**Add all missing opcodes to match statement:**

- Map each C++ function to corresponding Rust function
- Ensure proper error handling (return `Result<()>`)
- Match C++ `BX_NEXT_INSTR`, `BX_NEXT_TRACE`, `BX_LINK_TRACE` behavior
- For instructions that break traces, set `BX_ASYNC_EVENT_STOP_TRACE`

**Key opcodes to add:**

- All `Callf*`, `Jmpf*`, `Retf*` variants
- All `MOV*` memory forms
- All `XCHG*` variants
- All `MOVZX*`, `MOVSX*` variants
- All `CMOV*` variants
- All `NOT*` variants
- All `MUL*`, `IMUL*`, `DIV*`, `IDIV*` variants
- All conditional jump variants (Jw, Jd, Jq)
- All loop variants (32-bit and 64-bit)
- `JECXZ`, `JRCXZ`
- `IRET*` variants
- `XLAT`
- Infrastructure opcodes (RDRAND, MONITOR, MWAIT, RAO)

## Implementation Guidelines

1. **Exact C++ Parity**: Match C++ behavior exactly, including:

- Flag updates
- Exception handling
- Memory access patterns
- Register updates
- Trace breaking behavior

2. **Rust Benefits**: Use Rust features where appropriate:

- Pattern matching instead of switch statements
- `Result` types for error handling
- Type safety for register indices
- Safe memory access abstractions

3. **Code Organization**:

- Keep file structure matching C++ (separate files for 8/16/32/64-bit)
- Use modules appropriately
- Follow existing Rust code style

4. **Testing**:

- Ensure all implementations compile
- Verify no unused code warnings
- Match C++ instruction behavior

5. **Documentation**:

- Add comments matching C++ function names
- Document opcode mappings
- Note any deviations from C++ (if necessary)

## Files to Create/Modify

**New Files:**

- `rusty_box/src/cpu/data_xfer/data_xfer64.rs`
- `rusty_box/src/cpu/scalar_arith.rs` (if not exists)

**Files to Extend:**

- `rusty_box/src/cpu/ctrl_xfer.rs`
- `rusty_box/src/cpu/data_xfer/data_xfer8.rs`
- `rusty_box/src/cpu/data_xfer/data_xfer16.rs`
- `rusty_box/src/cpu/data_xfer/data_xfer32.rs`
- `rusty_box/src/cpu/logical.rs`
- `rusty_box/src/cpu/mult.rs`
- `rusty_box/src/cpu/lazy_flags.rs`
- `rusty_box/src/cpu/cpu.rs` (execute_instruction match statement)

**Files to Check:**

- `rusty_box/src/cpu/mwait.rs` (may already exist)
- `rusty_box/src/cpu/paging.rs` (infrastructure)
- `rusty_box/src/cpu/proc_ctrl.rs` (infrastructure)
- `rusty_box/src/cpu/protect_ctrl.rs` (infrastructure)