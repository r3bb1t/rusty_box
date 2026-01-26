---
name: Enhance x86 decoder implementation
overview: Enhance the 32-bit instruction decoder, implement 64-bit decoder, fix instruction struct for execution support, and ensure thread-safety with no-std compatibility using Rust idioms.
todos:
  - id: fix_instr_struct
    content: "Fix BxInstructionGenerated struct: add IqForm for 64-bit immediates, replace unsafe unions with safe types, ensure Send+Sync"
    status: completed
  - id: verify_exec_dispatch
    content: Verify execution uses static dispatch (match on opcode) - no changes needed, just documentation
    status: pending
    dependencies:
      - fix_instr_struct
  - id: enhance_errors
    content: Enhance error types in error.rs to match all Bochs decode errors
    status: pending
  - id: create_common_utils
    content: Create fetchdecode_common.rs with shared prefix parsing, ModRM/SIB decoding, immediate fetching. Make pure computation helpers const fn where possible.
    status: pending
    dependencies:
      - enhance_errors
  - id: fix_decode32
    content: "Fix fetch_decode32: apply lock prefix, set segment override, validate prefixes, complete immediate fetching, integrate ModRM/SIB"
    status: pending
    dependencies:
      - create_common_utils
      - fix_instr_struct
  - id: implement_decode64_helpers
    content: "Implement 64-bit decoder helper functions: decoder64_modrm, decoder64, decoder_simple64, decoder_creg64, decoder64_fp_escape, decoder64_3dnow, decoder64_nop, decodeModrm64, parseModrm64"
    status: pending
    dependencies:
      - create_common_utils
      - fix_instr_struct
  - id: implement_fetch_decode64
    content: "Implement fetchDecode64() main entry point: handle REX prefix (0x40-0x4F), segment overrides (FS/GS only), lock prefix, 3-byte opcodes, route to appropriate decoder functions"
    status: pending
    dependencies:
      - implement_decode64_helpers
      - implement_decoder_vex
      - implement_decoder_evex
      - implement_decoder_xop
      - implement_decoder_fp_escape
      - implement_decoder_creg
  - id: add_tests
    content: Add unit and integration tests for decoders
    status: pending
    dependencies:
      - fix_decode32
      - implement_fetch_decode64
  - id: verify_thread_safety
    content: Verify thread-safety (Send+Sync) and no-std compatibility
    status: pending
    dependencies:
      - fix_decode32
      - implement_fetch_decode64
  - id: implement_fetch_immediate
    content: Implement fetch_immediate() - fetches immediate values from instruction stream based on opcode metadata
    status: completed
    dependencies:
      - create_common_utils
  - id: implement_assign_srcs
    content: Implement assign_srcs() - assigns source operands to instruction metadata based on opcode table
    status: completed
    dependencies:
      - create_common_utils
  - id: implement_assign_srcs_avx
    content: Implement assign_srcs_avx() - assigns source operands for AVX/VEX/EVEX instructions
    status: pending
    dependencies:
      - create_common_utils
  - id: implement_evex_displ8
    content: Implement evex_displ8_compression() - calculates displacement scale for EVEX instructions
    status: pending
    dependencies:
      - create_common_utils
  - id: implement_decoder_evex
    content: Implement decoder_evex32() - decodes EVEX-prefixed instructions (AVX-512)
    status: pending
    dependencies:
      - implement_assign_srcs_avx
      - implement_evex_displ8
  - id: implement_decoder_vex
    content: Implement decoder_vex32() - decodes VEX-prefixed instructions (AVX/AVX2)
    status: pending
    dependencies:
      - implement_assign_srcs_avx
  - id: implement_decoder_xop
    content: Implement decoder_xop32() - decodes XOP-prefixed instructions (AMD)
    status: pending
    dependencies:
      - implement_assign_srcs_avx
  - id: implement_decoder_fp_escape
    content: Implement decoder32_fp_escape() - decodes x87 FPU escape instructions (0xD8-0xDF)
    status: pending
    dependencies:
      - create_common_utils
  - id: implement_decoder_creg
    content: Implement decoder_creg32() - decodes control register access instructions (MOV CRx)
    status: pending
    dependencies:
      - create_common_utils
  - id: implement_disasm
    content: Implement disasm() function for instruction disassembly
    status: pending
    dependencies:
      - fix_decode32
  - id: implement_opflags_from
    content: Implement OpFlags::from(Opcode) conversion in ia_opcodes.rs
    status: pending
---

# Enhance x86 Instruction Decoder Implementation

## Overview

This plan addresses enhancing the 32-bit decoder (`fetch_decode32_chatgpt_generated_instr`), implementing a 64-bit decoder, fixing the instruction struct to support execution, and ensuring thread-safety with no-std compatibility.

## Key Issues Identified

1. **Instruction Struct Incomplete**: `BxInstructionGenerated` lacks execution method support
2. **32-bit Decoder Issues**: Incomplete prefix handling, lock prefix not applied, segment override not properly set
3. **Missing 64-bit Decoder**: No `fetch_decode64` function exists
4. **Execution Model**: Use static dispatch via match statements on opcode (no dynamic dispatch)

## Implementation Plan

### 1. Fix Instruction Struct (`src/cpu/decoder/instr_generated.rs`)

**Current Issues:**

- Union types not properly handled in Rust
- Missing fields for 64-bit immediate values (IqForm)
- Need to ensure `Send + Sync` for thread safety

**Changes:**

- Add `IqForm` field for 64-bit immediates (x86-64 MOV Rx,imm64)
- Replace unsafe unions with `enum` or `#[repr(C)]` structs where appropriate
- Ensure `Send + Sync` bounds (no heap allocations needed)
- Add helper methods for accessing operand data safely
- **No execution method stored** - execution uses static dispatch via match on opcode

**Structure:**

```rust
pub struct BxInstructionGenerated {
    pub opcode: Opcode,
    pub meta_info: BxInstructionMetaInfo,
    pub meta_data: [u8; 8],
    pub modrm_form: ModRmForm,
    // Add for x86-64:
    pub iq_form: Option<u64>, // For MOV Rx,imm64
    // No execute_method - use static dispatch via match on opcode
}
```

### 2. Execution via Static Dispatch (Already Implemented)

**Current Implementation:**

- Execution already uses static dispatch via match on opcode in `cpu.rs::execute_instruction`
- No function pointers or trait objects needed
- Compiler generates all branches at compile time (zero-cost abstraction)
- Thread-safe by design (no shared mutable state)
- Opcode is stored in instruction struct, execution matches on it

**Example:**

```rust
// In cpu.rs - already implemented
match instr.get_ia_opcode() {
    Opcode::MovOp32GdEd => { /* ... */ },
    Opcode::AddGdEd => { /* ... */ },
    // ... all opcodes handled via match
}
```

### 3. Enhance 32-bit Decoder (`src/cpu/decoder/fetchdecode32.rs`)

**Issues to Fix:**

1. Lock prefix (`0xF0`) not properly applied to instruction
2. Segment override not set correctly after prefix loop
3. Missing validation for illegal prefix combinations
4. Incomplete immediate fetching
5. Missing ModRM/SIB decoding integration
6. Error handling needs improvement
7. Missing comments and documentation from original code

**Enhancements:**

- Apply lock prefix to instruction meta_info
- Set segment override after prefix processing
- Validate prefix combinations (e.g., LOCK with non-lockable instructions)
- Complete immediate value fetching
- Integrate ModRM/SIB decoding properly
- Add proper error propagation
- Handle 0x0F escape sequence correctly
- Handle 0x38/0x3A three-byte opcodes
- **Preserve all comments from original C++ code** explaining:
  - Prefix processing order and logic
  - Segment override handling
  - ModRM/SIB decoding details
  - Special cases and edge conditions
  - Performance considerations

**Key Function Signature:**

```rust
pub fn fetch_decode32(
    iptr: &[u8],
    is_32: bool,
) -> DecodeResult<BxInstructionGenerated>
```

### 4. Implement 64-bit Decoder (`src/cpu/decoder/fetchdecode64.rs`)

**New File Structure:**

- Create `fetchdecode64.rs` following same structure as `fetchdecode32.rs`
- Implement main entry point `fetchDecode64()` (equivalent to C++ `fetchDecode64`)
- Implement helper decoder functions (decoder64_modrm, decoder64, decoder_simple64, etc.)
- Handle REX prefix (0x40-0x4F)
- Ignore CS/DS/ES/SS segment overrides in 64-bit mode (per x86-64 spec)
- Support extended registers (R8-R15)
- Handle 64-bit immediate values
- Support RIP-relative addressing

**Key Differences from 32-bit:**

- REX prefix processing (W, R, X, B bits) - must be handled before other prefixes
- Segment override handling (only FS/GS valid in 64-bit mode)
- Extended register encoding via REX bits
- 64-bit displacement/immediate support
- RIP-relative addressing mode (mod==00b, rm==5 or SIB base==5)
- Different segment register tables for 64-bit mode

**Main Entry Point:**

```rust
pub fn fetch_decode64(
    iptr: &[u8],
    remaining_in_page: usize,
) -> DecodeResult<BxInstructionGenerated>
```

**Helper Functions to Implement:**

- `decodeModrm64()` - Decodes ModRM/SIB for 64-bit mode with REX support
- `parseModrm64()` - Parses ModRM byte with REX bits
- `decoder64_modrm()` - Decodes instructions requiring ModRM
- `decoder64()` - Decodes simple instructions without ModRM
- `decoder_simple64()` - Decodes simple instructions (NOP, etc.)
- `decoder_creg64()` - Decodes control register instructions
- `decoder64_fp_escape()` - Decodes x87 FPU escape instructions
- `decoder64_3dnow()` - Decodes 3DNow! instructions
- `decoder64_nop()` - Special handling for NOP (0x90) with REX
- `decoder_vex64()` - Decodes VEX-prefixed instructions (64-bit)
- `decoder_evex64()` - Decodes EVEX-prefixed instructions (64-bit)
- `decoder_xop64()` - Decodes XOP-prefixed instructions (64-bit)
- `decoder_ud64()` - Returns undefined opcode error

**Implementation Notes:**

- REX prefix (0x40-0x4F) must be processed before other prefixes
- REX.W bit (0x48-0x4F) sets operand size to 64-bit
- REX.R, REX.X, REX.B bits extend register encoding
- Segment overrides CS/DS/ES/SS are ignored in 64-bit mode
- Only FS: and GS: segment overrides are valid
- RIP-relative addressing: mod==00b with rm==5 or SIB base==5
- Default address size is 64-bit (can be overridden with 0x67)
- Default operand size is 32-bit (can be overridden with 0x66 or REX.W)

**Documentation from Original Code:**

- Preserve comments explaining REX prefix processing
- Keep comments about segment override handling in 64-bit mode
- Document RIP-relative addressing special cases
- Include comments about default operand/address sizes
- Preserve license headers and copyright from original files

### 5. Create Common Decoder Utilities (`src/cpu/decoder/fetchdecode_common.rs`)

**Shared Functionality:**

- Prefix byte parsing (common to both 32/64)
- ModRM/SIB decoding
- Immediate value fetching
- Displacement handling
- Error types and handling

**Const Functions:**

- Make pure computation helpers `const fn` where possible:
  - `extract_modrm_fields()` - extracts mod, nnn, rm from byte
  - `extract_sib_fields()` - extracts scale, index, base from SIB byte
  - `calculate_rex_bits()` - calculates REX.R, REX.X, REX.B from REX prefix
  - `evex_displ8_compression()` - calculates displacement scale (if inputs are const)
  - Opcode table lookup helpers (if table and mask are const)

**Documentation:**

- Preserve all comments from original C++ helper functions
- Document bit manipulation operations
- Explain field extraction logic
- Include references to x86 architecture specifications where relevant

**Benefits:**

- Code reuse between 32/64 decoders
- Consistent error handling
- Easier maintenance
- Compile-time evaluation for static instruction analysis

### 6. Thread Safety & No-Std Compatibility

**Thread Safety:**

- All decoder tables should be `const` or `static` (immutable)
- Instruction struct implements `Send + Sync`
- No mutable global state in decoder
- Use `UnsafeCell` only where absolutely necessary (with proper documentation)

**No-Std Compatibility:**

- Remove `std` dependencies
- Use `core` and `alloc` only
- Ensure all collections use `alloc` versions
- No file I/O or other std features

### 7. Error Handling Enhancement (`src/cpu/decoder/error.rs`)

**Current Issues:**

- Error types may be incomplete
- Missing specific decode errors

**Enhancements:**

- Add all Bochs decode error types:
  - `BX_ILLEGAL_LOCK_PREFIX`
  - `BX_ILLEGAL_VEX_XOP_VVV`
  - `BX_EVEX_RESERVED_BITS_SET`
  - etc.
- Use `Result<T, DecodeError>` consistently
- Provide error context where possible

### 8. Implement Unimplemented Decoder Functions

**Functions to Implement:**

1. **`fetch_immediate()`** - Fetches immediate values from instruction stream

   - Handles various immediate types (Ib, Iw, Id, Iq, sign-extended variants)
   - Handles direct pointer addressing
   - Handles direct memory references
   - Returns updated instruction pointer and error status

2. **`assign_srcs()`** - Assigns source operands to instruction metadata

   - Reads source definitions from opcode table
   - Assigns registers based on ModRM fields (nnn, rm)
   - Handles EAX, NNN, RM, VVV sources
   - Sets meta_data array with register indices

3. **`assign_srcs_avx()`** - Assigns source operands for AVX/VEX/EVEX instructions

   - Handles vector register sources
   - Handles extended register encoding (REX)
   - Handles EVEX displacement compression
   - Handles mask registers

4. **`evex_displ8_compression()`** - Calculates displacement scale for EVEX

   - Returns byte size based on vector length and operand type
   - Handles broadcast semantics
   - Handles special cases (VMOVDDUP)

5. **`decoder_evex32()`** - Decodes EVEX-prefixed instructions

   - Parses EVEX prefix (4 bytes)
   - Handles AVX-512 instructions
   - Handles mask registers and zeroing/merging
   - Handles embedded rounding and SAE

6. **`decoder_vex32()`** - Decodes VEX-prefixed instructions

   - Parses VEX prefix (2 or 3 bytes)
   - Handles AVX/AVX2 instructions
   - Handles vector length encoding

7. **`decoder_xop32()`** - Decodes XOP-prefixed instructions

   - Parses XOP prefix (3 bytes)
   - Handles AMD XOP instructions

8. **`decoder32_fp_escape()`** - Decodes x87 FPU escape instructions

   - Handles opcodes 0xD8-0xDF
   - Routes to x87 opcode tables
   - Handles ModRM-based FPU instructions

9. **`decoder_creg32()`** - Decodes control register instructions

   - Handles MOV CRx, GPR instructions
   - Validates control register access permissions

10. **`disasm()`** - Instruction disassembler

    - Converts instruction bytes to assembly mnemonic
    - Supports Intel and AT&T syntax
    - Handles all instruction types

11. **`OpFlags::from(Opcode)`** - Opcode to flags conversion

    - Maps opcode enum to opflags from opcode table
    - Used for instruction metadata

### 9. Testing & Validation

**Add Tests:**

- Unit tests for prefix handling
- Unit tests for ModRM decoding
- Unit tests for immediate fetching
- Unit tests for source assignment
- Integration tests for common instructions
- Edge case testing (illegal prefixes, etc.)
- Tests for AVX/VEX/EVEX decoding
- Tests for x87 FPU decoding

## File Changes Summary

### Modified Files:

1. `src/cpu/decoder/instr_generated.rs` - Fix instruction struct (add IqForm, ensure Send+Sync)
2. `src/cpu/decoder/fetchdecode32.rs` - Enhance 32-bit decoder
3. `src/cpu/decoder/error.rs` - Enhance error types
4. `src/cpu/decoder/mod.rs` - Export new modules

### New Files:

1. `src/cpu/decoder/fetchdecode64.rs` - 64-bit decoder implementation
2. `src/cpu/decoder/fetchdecode_common.rs` - Shared decoder utilities

### Files with Unimplemented Functions:

1. `src/cpu/decoder/fetchdecode32.rs` - Multiple unimplemented decoder functions
2. `src/cpu/decoder/disasm.rs` - Disassembler function
3. `src/cpu/decoder/ia_opcodes.rs` - OpFlags conversion

## Implementation Order

1. Fix instruction struct (add IqForm, ensure Send+Sync)
2. Enhance error types
3. Create common decoder utilities (with const fn helpers and original comments)
4. Implement helper functions (fetch_immediate, assign_srcs, evex_displ8_compression) - preserve all original comments
5. Implement decoder functions (evex32, vex32, xop32, fp_escape32, creg32) - copy relevant comments
6. Fix and enhance 32-bit decoder main function (integrate helper functions, preserve comments)
7. Implement 64-bit decoder helper functions (decoder64_modrm, decoder64, etc.) - preserve original comments
8. Implement 64-bit decoder main function (fetchDecode64 - main entry point) - preserve all comments
9. Implement disassembler and opcode flags conversion
10. Add tests
11. Verify thread-safety and no-std compatibility

**Note:** The 64-bit decoder has a main entry point `fetchDecode64()` that handles prefix processing and routes to helper decoder functions, similar to how the 32-bit decoder works.

**Documentation Preservation:**

- Copy license headers from original files
- Preserve inline comments explaining logic
- Translate C++ comments to Rust documentation style
- Keep comments about performance considerations
- Document edge cases and special handling
- Include references to x86 architecture specs

**Note:** Execution uses static dispatch via match on opcode (already implemented in `cpu.rs`), so no execution trait or function pointers needed.

## Rust-Specific Considerations

- Use `enum` instead of unions where possible (type safety)
- Use `Option<T>` for optional fields instead of sentinel values
- Use `Result<T, E>` for error handling (Rust's equivalent to avoiding C++ exceptions)
- Implement `Send + Sync` for thread safety
- Use `const fn` where possible for compile-time evaluation
- Prefer pattern matching over if/else chains
- Use `#[repr(C)]` only where needed for compatibility

## Const Functions for Compile-Time Evaluation

**Goal:** Make decoder functions `const fn` where possible to enable compile-time decoding.

**What Can Be Const:**

1. **Opcode Table Lookups:**

   - `find_opcode()` - if opcode table and mask are const
   - Opcode table indexing operations

2. **Pure Computation Functions:**

   - `evex_displ8_compression()` - pure calculation based on const inputs
   - Prefix bit extraction and manipulation
   - ModRM field extraction (mod, nnn, rm bits)
   - Register encoding calculations

3. **Helper Functions:**

   - Functions that only do bit manipulation
   - Functions that only do arithmetic on const inputs
   - Validation functions that check const values

**What Cannot Be Const:**

1. **Main Decoder Functions:**

   - `fetch_decode32()` / `fetchDecode64()` - need mutable instruction struct
   - Functions that modify instruction state
   - Functions that read from mutable slices

2. **Memory Access:**

   - Reading instruction bytes from memory (runtime data)
   - Writing to instruction struct fields (mutable state)

3. **Error Handling:**

   - Functions returning `Result` with runtime errors
   - Functions that need to propagate errors

**Strategy:**

- Make pure computation helpers `const fn` (e.g., `extract_modrm_fields()`, `calculate_displacement_scale()`)
- Keep main decoder functions non-const (they need mutable state)
- Use `const fn` for compile-time opcode lookups when instruction bytes are known at compile time
- Consider providing `const` versions of lookup functions for static analysis/optimization

**Example:**

```rust
// Can be const - pure computation
pub const fn extract_modrm_fields(modrm_byte: u8) -> (u8, u8, u8) {
    let mod_field = (modrm_byte >> 6) & 0x3;
    let nnn = (modrm_byte >> 3) & 0x7;
    let rm = modrm_byte & 0x7;
    (mod_field, nnn, rm)
}

// Cannot be const - needs mutable state
pub fn fetch_decode32(
    iptr: &[u8],
    is_32: bool,
) -> DecodeResult<BxInstructionGenerated> {
    // ... modifies instruction struct
}
```

**Note:** If compile-time decoding proves too difficult or provides minimal benefit, focus on runtime performance instead. The main decoder functions will remain non-const as they need to handle dynamic input and mutable state.

**Documentation for Const Functions:**

- Document which functions are `const fn` and why
- Explain limitations (what cannot be const and why)
- Provide examples of compile-time usage where applicable
- Note performance benefits of const evaluation

## Code Documentation and Comments

**Preserve Original Bochs Comments:**

- Copy relevant comments from original C++ code explaining:
  - Why certain operations are done (not just what)
  - Performance considerations
  - Edge cases and special handling
  - Historical context or workarounds
  - References to x86 architecture specifications

- Translate C++ style comments to Rust documentation:
  - Use `///` for public API documentation
  - Use `//` for implementation comments
  - Preserve license headers and copyright notices
  - Keep inline comments explaining complex logic

- Document complex algorithms:
  - ModRM/SIB decoding logic
  - Prefix processing order
  - REX prefix bit manipulation
  - EVEX/VEX/XOP encoding details
  - Immediate value sign extension

**Example:**

```rust
/// Decodes ModRM byte for 64-bit mode with REX prefix support.
/// 
/// In 64-bit mode, the REX prefix extends register encoding:
/// - REX.R extends the reg field (nnn)
/// - REX.X extends the index field in SIB
/// - REX.B extends the base/rm field
/// 
/// Original Bochs comment: "note that mod==11b handled outside"
pub fn decode_modrm64(...) -> ... {
    // Initialize displacement to zero to include cases with no displacement
    // (from original C++: "initialize displ32 with zero to include cases with no diplacement")
}
```

## Bochs Performance Guidelines (Applied to Rust)

Based on Bochs development documentation, the emulator is **incredibly performance sensitive**. Apply these principles:

### Performance-Critical Decisions:

1. **Avoid Heap Allocations in Hot Paths:**

   - No `Box`, `Vec`, or other heap allocations in decoder functions
   - Use stack-allocated arrays with fixed sizes
   - Instruction struct should be `Copy` where possible (or at least `Clone`)

2. **Static Dispatch Only:**

   - Use match statements (zero-cost abstraction)
   - No trait objects (`dyn Trait`) in performance-critical paths
   - Compiler generates all branches at compile time

3. **Minimize Indirection:**

   - Prefer direct field access over getters/setters where performance matters
   - Use `#[inline]` for small, frequently-called functions
   - Avoid unnecessary abstractions

4. **Use Unsigned Types:**

   - Follow Bochs guideline: "Don't use signed ints where unsigned will do"
   - Use `u32`, `u64`, `u8`, `u16` instead of `i32`, `i64`, etc. where appropriate

5. **Const/Static Data:**

   - All decoder tables should be `const` (compile-time constants)
   - No runtime initialization of decoder tables
   - Use `const fn` for compile-time computations where possible

6. **Avoid Overhead:**

   - No unnecessary bounds checking (use `get_unchecked` in hot paths if safe)
   - Minimize function call overhead (inline small helpers)
   - Cache-friendly data structures (arrays over linked structures)

7. **Code Clarity vs Performance:**

   - While Bochs avoids "fancy features" in C++, Rust's zero-cost abstractions are fine
   - Pattern matching is encouraged (compiles to efficient code)
   - Traits with static dispatch are acceptable (zero-cost)

### Alignment with Bochs Architecture:

- **No exceptions**: Rust uses `Result<T, E>` instead (better than C++ exceptions)
- **Static methods**: Rust functions are naturally static (no `this` pointer overhead)
- **Performance-first**: Every design decision should consider performance impact
- **Portability**: Use only `core` and `alloc` (no `std`) for maximum portability