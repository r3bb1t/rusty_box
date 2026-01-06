# Accessing Decoded Instruction Data

This document explains how to access register operands and immediate values from a decoded `BxInstructionGenerated` structure.

## Decoder Functions

### Runtime Decoders

```rust
use rusty_box::cpu::decoder::fetchdecode32::fetch_decode32_chatgpt_generated_instr;
use rusty_box::cpu::decoder::fetchdecode64::fetch_decode64;

// 32-bit mode decoder (returns Result)
let result = fetch_decode32_chatgpt_generated_instr(&bytes, is_32_bit_mode);

// 64-bit mode decoder (returns Result)  
let result = fetch_decode64(&bytes);
```

### Const Decoders

For compile-time instruction decoding (useful for static analysis, compile-time tables, etc.):

```rust
use rusty_box::cpu::decoder::const_fetchdecode32::const_fetch_decode32;
use rusty_box::cpu::decoder::const_fetchdecode64::const_fetch_decode64;

// 32-bit/16-bit mode const decoder
// is_32: true for 32-bit mode, false for 16-bit mode
const DECODED_32: BxInstructionGenerated = const_fetch_decode32(&[0x90], true);

// 64-bit mode const decoder
const DECODED_64: BxInstructionGenerated = const_fetch_decode64(&[0x90]);
```

**Key differences:**
- Const decoders return `BxInstructionGenerated` directly (not `Result`)
- Const decoders can be used in `const` contexts
- Invalid instructions return with `Opcode::IaError`

## Register Operands

Registers are stored in the `meta_data` array with the following indices:

```rust
// Access register operands
let dst_reg = instr.dst();        // meta_data[0] - destination register
let src1_reg = instr.src1();      // meta_data[1] - source register 1
let src2_reg = instr.src2();      // meta_data[2] - source register 2
let src3_reg = instr.src3();      // meta_data[3] - source register 3
let seg_reg = instr.seg();        // meta_data[4] - segment register
let base_reg = instr.base();      // meta_data[5] - base register (for addressing)
let index_reg = instr.index();   // meta_data[6] - index register (for addressing)
let scale = instr.scale();        // meta_data[7] - scale factor (for addressing)
```

### Example: MOV instruction
```rust
// MOV r32, r/m32
// dst_reg = destination register (r32)
// src1_reg = source register from ModRM (r/m32)
let dst = instr.dst();
let src = instr.src1();
```

## Immediate Values

Immediate values are stored in `modrm_form.operand_data` and can be accessed using type-specific methods:

```rust
// 8-bit immediate (Ib)
let imm8 = instr.ib();            // operand_data.ib()[0]

// 16-bit immediate (Iw)
let imm16 = instr.iw();           // operand_data.iw()[0]

// 32-bit immediate (Id)
let imm32 = instr.id();           // operand_data.id()

// 64-bit immediate (Iq) - x86-64 only
let imm64 = instr.iq();           // modrm_form (union with IqForm)

// Second 8-bit immediate (Ib2) - for instructions like ENTER
let imm8_2 = instr.ib2();          // displacement.ib2()[0]

// Second 16-bit immediate (Iw2)
let imm16_2 = instr.iw2();        // displacement.iw2()[0]

// Second 32-bit immediate (Id2)
let imm32_2 = instr.id2();         // displacement.id2()
```

### Example: JNB instruction (8-bit relative jump)
```rust
// JNB rel8 (0x73 0x68)
// The immediate value 0x68 is stored as an 8-bit signed offset
let offset = instr.ib() as i8;    // Get 8-bit immediate, cast to signed
let target_address = current_ip + offset as i32 + 2;  // +2 for instruction length
```

### Example: ADD EAX, imm32
```rust
// ADD EAX, 0x12345678
let imm32 = instr.id();            // Get 32-bit immediate value
```

## Displacements

Displacements are stored in `modrm_form.displacement`:

```rust
// 16-bit displacement
let disp16 = instr.displ16u();     // displacement.displ16u()

// 32-bit displacement
let disp32 = instr.displ32u();     // displacement.displ32u()

// Signed displacements
let disp32s = instr.displ32s();   // displacement.displ32u() as i32
let disp16s = instr.displ16s();   // displacement.displ16u() as i16
```

## Instruction Metadata

```rust
// Get the decoded opcode
let opcode = instr.get_ia_opcode();

// Get instruction length in bytes
let len = instr.ilen();

// Check prefix flags
let has_lock = instr.get_lock();      // LOCK prefix present
let has_rep = instr.get_rep();        // REP/REPNE prefix present

// Check addressing mode
let is_reg_mode = instr.mod_c0();     // ModRM mod=11 (register mode)

// Operand/address size
let os32 = instr.os32();              // 32-bit operand size
let as32 = instr.as32();              // 32-bit address size
let os64 = instr.os64();              // 64-bit operand size (x86-64)
let as64 = instr.as64();              // 64-bit address size (x86-64)
```

## Complete Example (Runtime)

```rust
use rusty_box::cpu::decoder::fetchdecode32::fetch_decode32_chatgpt_generated_instr;

let bytes = [0x73, 0x68];  // JNB rel8
let result = fetch_decode32_chatgpt_generated_instr(&bytes, false);

match result {
    Ok(instr) => {
        // Access opcode
        let opcode = instr.get_ia_opcode();
        println!("Opcode: {:?}", opcode);  // JnbJbw
        
        // Access instruction length
        let len = instr.ilen();
        println!("Length: {}", len);  // Should be 2
        
        // Access 8-bit immediate (branch offset)
        let offset = instr.ib() as i8;
        println!("Offset: {} (0x{:02x})", offset, instr.ib());
        
        // Calculate target address (assuming current IP)
        let current_ip = 0x1000;
        let target = current_ip.wrapping_add(offset as i32 as u32).wrapping_add(len as u32);
        println!("Target address: 0x{:x}", target);
    }
    Err(e) => {
        println!("Decode error: {:?}", e);
    }
}
```

## Complete Example (Const)

```rust
use rusty_box::cpu::decoder::const_fetchdecode64::const_fetch_decode64;
use rusty_box::cpu::decoder::ia_opcodes::Opcode;

// Decode at compile time
const NOP_INSTR: BxInstructionGenerated = const_fetch_decode64(&[0x90]);
const RET_INSTR: BxInstructionGenerated = const_fetch_decode64(&[0xC3]);
const INT3_INSTR: BxInstructionGenerated = const_fetch_decode64(&[0xCC]);

// Use in const context
const fn is_nop(bytes: &[u8]) -> bool {
    let instr = const_fetch_decode64(bytes);
    matches!(instr.meta_info.ia_opcode, Opcode::Nop)
}

// Verify at compile time
const _: () = assert!(is_nop(&[0x90]));
```

## Notes

- Register indices are 5-bit values (0-31) that map to actual registers based on the instruction context
- Immediate values are stored in little-endian format
- For branch instructions, the immediate is a **signed** relative offset from the end of the instruction
- The instruction length (`ilen`) includes all bytes: opcode, prefixes, ModRM, SIB, displacement, and immediate
- Const decoders do not support tracing/logging (removed for const compatibility)
- Invalid/unsupported instructions in const decoders set `ia_opcode` to `Opcode::IaError`
