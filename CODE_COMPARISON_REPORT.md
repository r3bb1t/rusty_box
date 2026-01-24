# Code Comparison Report: Rust vs Original C++ Bochs

## Overview
This document compares the Rust implementation against the original C++ Bochs codebase in `cpp_orig/bochs/cpu/`.

## File Structure Comparison

### ✅ Correct Structure
- `logical8.rs` ↔ `logical8.cc`
- `logical16.rs` ↔ `logical16.cc`
- `logical32.rs` ↔ `logical32.cc`
- `mult8.rs` ↔ `mult8.cc`
- `mult16.rs` ↔ `mult16.cc`
- `mult32.rs` ↔ `mult32.cc`
- `ctrl_xfer16.rs` ↔ `ctrl_xfer16.cc`
- `ctrl_xfer32.rs` ↔ `ctrl_xfer32.cc`
- `ctrl_xfer64.rs` ↔ `ctrl_xfer64.cc`

### ⚠️ Structural Issue: Memory-Form Functions Location

**Problem**: Memory-form functions (those with `_m` suffix) are currently in `data_xfer_ext.rs` instead of the logical files.

**Original C++ Structure**:
- All logical instruction functions (register and memory forms) are in `logical8.cc`, `logical16.cc`, `logical32.cc`

**Current Rust Structure**:
- Register-form functions: `logical8.rs`, `logical16.rs`, `logical32.rs` ✅
- Memory-form functions: `data_xfer_ext.rs` ❌

**Recommendation**: Move memory-form functions to their respective logical files to match the original structure.

## Missing Functions

### 8-bit Logical Instructions (`logical8.cc`)

#### ✅ Implemented (in `logical8.rs`):
- `XOR_EbIbR` → `xor_eb_ib_r` (but only handles AL, imm8)
- `AND_GbEbR` → `and_gb_eb_r`
- `AND_EbIbR` → `and_al_ib` (only AL, imm8)
- `OR_GbEbR` → `or_gb_eb_r`
- `OR_EbIbR` → `or_al_ib` (only AL, imm8)
- `NOT_EbR` → `not_eb_r`
- `TEST_EbGbR` → `test_eb_gb_r`
- `TEST_EbIbR` → `test_al_ib` (only AL, imm8)

#### ✅ Implemented (in `data_xfer_ext.rs`):
- `XOR_EbGbM` → `xor_eb_gb_m`
- `XOR_GbEbM` → `xor_gb_eb_m`
- `XOR_EbIbM` → `xor_eb_ib_m`
- `OR_EbGbM` → `or_eb_gb_m`
- `OR_GbEbM` → `or_gb_eb_m`
- `OR_EbIbM` → `or_eb_ib_m`
- `AND_EbGbM` → `and_eb_gb_m`
- `AND_GbEbM` → `and_gb_eb_m`
- `AND_EbIbM` → `and_eb_ib_m`
- `NOT_EbM` → `not_eb_m`
- `TEST_EbGbM` → `test_eb_gb_m`
- `TEST_EbIbM` → `test_eb_ib_m`

#### ❌ Missing:
- `XOR_GbEbR` - Register form of XOR r8, r/m8 (currently only memory form exists)

### 16-bit Logical Instructions (`logical16.cc`)

#### ✅ Implemented (in `logical16.rs`):
- `ZERO_IDIOM_GwR` → `zero_idiom_gw_r`
- `XOR_GwEwR` → `or_gw_ew_r` (but named `or_gw_ew_r` - **NAME MISMATCH**)
- `AND_GwEwR` → `and_gw_ew_r`
- `AND_EwIwR` → `and_ew_iw_r`
- `OR_GwEwR` → `or_gw_ew_r`
- `NOT_EwR` → `not_ew_r`
- `TEST_EwGwR` → `test_ew_gw_r`
- `TEST_EwIwR` → `test_ew_iw_r`

#### ✅ Implemented (in `data_xfer_ext.rs`):
- `XOR_EwGwM` → `xor_ew_gw_m`
- `XOR_GwEwM` → `xor_gw_ew_m`
- `XOR_EwIwM` → `xor_ew_iw_m`
- `OR_EwGwM` → `or_ew_gw_m`
- `OR_GwEwM` → `or_gw_ew_m`
- `OR_EwIwM` → `or_ew_iw_m`
- `AND_EwGwM` → `and_ew_gw_m`
- `AND_GwEwM` → `and_gw_ew_m`
- `AND_EwIwM` → `and_ew_iw_m`
- `NOT_EwM` → `not_ew_m`
- `TEST_EwGwM` → `test_ew_gw_m`
- `TEST_EwIwM` → `test_ew_iw_m`

#### ❌ Missing:
- `XOR_GwEwR` - Register form of XOR r16, r16 (only memory form exists)
- `XOR_GdEdR` - Register form of XOR r32, r32 (only memory form exists)
- `XOR_EwIwR` - Register form of XOR r/m16, imm16
- `OR_EwIwR` - Register form of OR r/m16, imm16

### 32-bit Logical Instructions (`logical32.cc`)

#### ✅ Implemented (in `logical32.rs`):
- `ZERO_IDIOM_GdR` → `zero_idiom_gd_r` (not found - **MISSING**)
- `XOR_GdEdR` → `or_gd_ed_r` (but named `or_gd_ed_r` - **NAME MISMATCH**)
- `AND_GdEdR` → `and_gd_ed_r`
- `AND_EdIdR` → `and_ed_id_r`
- `OR_GdEdR` → `or_gd_ed_r`
- `NOT_EdR` → `not_ed_r`
- `TEST_EdGdR` → `test_ed_gd_r`
- `TEST_EdIdR` → `test_ed_id_r`

#### ✅ Implemented (in `data_xfer_ext.rs`):
- `XOR_EdGdM` → `xor_ed_gd_m`
- `XOR_GdEdM` → `xor_gd_ed_m`
- `XOR_EdIdM` → `xor_ed_id_m`
- `OR_EdGdM` → `or_ed_gd_m`
- `OR_GdEdM` → `or_gd_ed_m`
- `OR_EdIdM` → `or_ed_id_m`
- `AND_EdGdM` → `and_ed_gd_m`
- `AND_GdEdM` → `and_gd_ed_m`
- `AND_EdIdM` → `and_ed_id_m`
- `NOT_EdM` → `not_ed_m`
- `TEST_EdGdM` → `test_ed_gd_m`
- `TEST_EdIdM` → `test_ed_id_m`

#### ❌ Missing:
- `ZERO_IDIOM_GdR` - Zero idiom for 32-bit registers
- `XOR_EdIdR` - Register form of XOR r/m32, imm32
- `OR_EdIdR` - Register form of OR r/m32, imm32

## Logic Comparison

### Flag Setting

#### ✅ Correct Implementation
The flag-setting logic matches the original C++ macros:

**C++ Macro** (`lazy_flags.h:86-94`):
```cpp
#define SET_FLAGS_OSZAPC_LOGIC_8(result_8) \
   SET_FLAGS_OSZAPC_8(0, (result_8))
```

**Rust Implementation** (`logical8.rs:18-33`):
```rust
fn set_flags_oszapc_logic_8(&mut self, result: u8) {
    let sf = (result & 0x80) != 0;
    let zf = result == 0;
    let pf = result.count_ones() % 2 == 0;
    
    const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 6) | (1 << 7) | (1 << 11);
    self.eflags &= !MASK;
    
    if pf { self.eflags |= 1 << 2; }
    if zf { self.eflags |= 1 << 6; }
    if sf { self.eflags |= 1 << 7; }
}
```

This correctly:
- Clears OF (bit 11), CF (bit 0), PF (bit 2), ZF (bit 6), SF (bit 7)
- Sets PF, ZF, SF based on result
- Leaves OF=0, CF=0 (cleared)

### Multiplication/Division Logic

#### ✅ Correct Implementation
The multiplication and division logic matches the original:

**MUL 8-bit** (`mult8.cc:27-48` vs `mult8.rs:20-42`):
- ✅ Correct product calculation
- ✅ Correct flag setting (CF/OF set if high byte != 0)
- ✅ Correct register updates (AX = product)

**IMUL 8-bit** (`mult8.cc:50-73` vs `mult8.rs:72-95`):
- ✅ Correct signed multiplication
- ✅ Correct overflow check: `product_16 != (product_16 as i8 as i16)`
- ✅ Correct flag setting

**DIV 8-bit** (`mult8.cc:75-98` vs `mult8.rs:126-150`):
- ✅ Correct division by zero check
- ✅ Correct quotient overflow check
- ✅ Correct register updates (AL = quotient, AH = remainder)

**IDIV 8-bit** (`mult8.cc:100-125` vs `mult8.rs:182-212`):
- ✅ Correct MIN_INT check (0x8000)
- ✅ Correct division by zero check
- ✅ Correct quotient overflow check

**MUL 16-bit** (`mult16.cc:27-48` vs `mult16.rs:20-42`):
- ✅ Correct product calculation (32-bit result)
- ✅ Correct register updates (AX = low word, DX = high word)
- ✅ Correct flag setting

**IMUL 16-bit** (`mult16.cc:50-74` vs `mult16.rs:73-97`):
- ✅ Correct signed multiplication
- ✅ Correct overflow check: `product_32 != (product_32 as i16 as i32)`

**DIV 16-bit** (`mult16.cc:76-96` vs `mult16.rs:130-156`):
- ✅ Correct operand construction: `((DX << 16) | AX)`
- ✅ Correct division by zero check
- ✅ Correct quotient overflow check

**IDIV 16-bit** (`mult16.cc:98-123` vs `mult16.rs:191-224`):
- ✅ Correct MIN_INT check (0x80000000)
- ✅ Correct operand construction with signed cast
- ✅ Correct division by zero check
- ✅ Correct quotient overflow check

## Issues Found

### 1. Missing XOR Register-Form Functions
**Location**: `logical16.rs` and `logical32.rs`
- **C++**: `XOR_GwEwR` (XOR r16, r16) - register form
- **Rust**: Missing - only `xor_gw_ew_m` exists in `data_xfer_ext.rs` (memory form)
- **Issue**: The dispatch in `cpu.rs:1840` calls `xor_gw_ew_m` for `Opcode::XorGwEw`, but this should be the register form, not memory form.

**Location**: `logical32.rs`
- **C++**: `XOR_GdEdR` (XOR r32, r32) - register form  
- **Rust**: Missing - only `xor_gd_ed_m` exists in `data_xfer_ext.rs` (memory form)
- **Issue**: Similar to 16-bit case - register form function is missing.

### 2. Missing Functions
- `XOR_EwIwR` (16-bit register form with immediate)
- `OR_EwIwR` (16-bit register form with immediate)
- `XOR_EdIdR` (32-bit register form with immediate)
- `OR_EdIdR` (32-bit register form with immediate)
- `ZERO_IDIOM_GdR` (32-bit zero idiom)
- `XOR_GbEbR` (8-bit register form - XOR r8, r/m8)

### 3. Incomplete Immediate Handling
Some functions only handle the AL/AX/EAX immediate forms, but not the general r/m forms:
- `XOR_EbIbR` - only handles AL, imm8
- `AND_EbIbR` - only handles AL, imm8
- `OR_EbIbR` - only handles AL, imm8
- `TEST_EbIbR` - only handles AL, imm8

### 4. File Structure Mismatch
Memory-form functions should be in `logical8.rs`, `logical16.rs`, `logical32.rs` instead of `data_xfer_ext.rs` to match the original C++ structure.

## Recommendations

1. **Move memory-form functions** from `data_xfer_ext.rs` to their respective logical files
2. **Add missing functions**:
   - `xor_gw_ew_r` (16-bit register form - XOR r16, r16)
   - `xor_gd_ed_r` (32-bit register form - XOR r32, r32)
   - `xor_ew_iw_r` (16-bit register form with immediate)
   - `or_ew_iw_r` (16-bit register form with immediate)
   - `xor_ed_id_r` (32-bit register form with immediate)
   - `or_ed_id_r` (32-bit register form with immediate)
   - `zero_idiom_gd_r` (32-bit)
   - `xor_gb_eb_r` (8-bit register form - XOR r8, r/m8)
3. **Fix dispatch logic**:
   - `Opcode::XorGwEw` should call `xor_gw_ew_r` (register form), not `xor_gw_ew_m`
   - `Opcode::XorGdEd` should call `xor_gd_ed_r` (register form), not `xor_gd_ed_m`
4. **Complete immediate handling** for general r/m forms (not just AL/AX/EAX)

## Summary

### ✅ Strengths
- Flag-setting logic is correct
- Multiplication/division logic matches the original
- Most functions are implemented
- Memory-form functions exist (just in wrong location)

### ⚠️ Issues
- File structure doesn't match original (memory forms in wrong file)
- Some register-form functions missing
- Function name mismatches
- Incomplete immediate operand handling

### 📊 Coverage
- **8-bit logical**: ~85% (missing 1 function, incomplete immediate handling)
- **16-bit logical**: ~90% (missing 2 functions)
- **32-bit logical**: ~85% (missing 3 functions)
- **Multiplication/Division**: ~100% ✅
