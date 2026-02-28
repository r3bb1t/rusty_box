#[cfg(test)]
extern crate std;

use crate::{
    fetchdecode32::fetch_decode32, fetchdecode64::fetch_decode64, ia_opcodes::Opcode,
    instr_generated::Instruction,
};

/// Initialize tracing for tests (similar to examples/init_and_run.rs)
fn init_tracing() {
    use tracing_subscriber::fmt;
    let _ = fmt()
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
}

/// Format an instruction for display (similar to Zydis output)
fn format_instruction(address: u64, instr: &Instruction) -> std::string::String {
    let opcode_name = std::format!("{:?}", instr.get_ia_opcode());
    let length = instr.meta_info.ilen;

    // Format address as 16 hex digits
    std::format!("{:016X}  {} (len={})", address, opcode_name, length)
}

/// Disassemble a sequence of instructions from a byte buffer
///
/// Similar to Zydis example: loops over instructions in buffer and prints them
fn disassemble_sequence(
    data: &[u8],
    runtime_address: u64,
    is_32: bool,
) -> std::vec::Vec<(u64, Instruction)> {
    let mut offset = 0;
    let mut current_address = runtime_address;
    let mut instructions = std::vec::Vec::new();

    while offset < data.len() {
        let remaining = &data[offset..];

        match fetch_decode32(remaining, is_32) {
            Ok(instr) => {
                let length = instr.meta_info.ilen as usize;

                if length == 0 || offset + length > data.len() {
                    // Invalid instruction or out of bounds
                    break;
                }

                instructions.push((current_address, instr));
                offset += length;
                current_address += length as u64;
            }
            Err(_) => {
                // Decode failed, stop
                break;
            }
        }
    }

    instructions
}

#[test]
fn test_disassemble_example_sequence() {
    // Example instruction sequence similar to Zydis example
    // This is a simple sequence: push rcx, push rax, etc.
    let data = [
        0x51, // push rcx
        0x50, // push rax
        0x48, 0x83, 0xC4, 0x08, // add rsp, 8
        0x48, 0x89, 0xC1, // mov rcx, rax
        0xC3, // ret
    ];

    let runtime_address = 0x007FFFFFFF400000;
    let instructions = disassemble_sequence(&data, runtime_address, false);

    // Print formatted output (similar to Zydis)
    init_tracing();
    tracing::info!("Disassembled instructions:");
    for (addr, instr) in &instructions {
        tracing::info!("{}", format_instruction(*addr, instr));
    }

    // Verify we decoded at least some instructions
    assert!(
        !instructions.is_empty(),
        "Should decode at least one instruction"
    );
}

#[test]
fn test_disassemble_32bit_sequence() {
    // 32-bit instruction sequence
    let data = [
        0x55, // push ebp
        0x89, 0xE5, // mov ebp, esp
        0x83, 0xEC, 0x10, // sub esp, 0x10
        0x8B, 0x45, 0x08, // mov eax, [ebp+8]
        0x5D, // pop ebp
        0xC3, // ret
    ];

    let runtime_address = 0x00400000;
    let instructions = disassemble_sequence(&data, runtime_address, true);

    init_tracing();
    tracing::info!("32-bit disassembled instructions:");
    for (addr, instr) in &instructions {
        tracing::info!("{}", format_instruction(*addr, instr));
    }

    assert!(
        !instructions.is_empty(),
        "Should decode at least one instruction"
    );
}

#[test]
fn test_disassemble_mov_instructions() {
    // Various MOV instructions
    let data = [
        0x48, 0x89, 0xC1, // mov rcx, rax
        0x48, 0x8B, 0x45, 0x10, // mov rax, [rbp+0x10]
        0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00, // mov rax, 1
        0x89, 0xD8, // mov eax, ebx
    ];

    let runtime_address = 0x10000000;
    let instructions = disassemble_sequence(&data, runtime_address, false);

    init_tracing();
    tracing::info!("MOV instructions:");
    for (addr, instr) in &instructions {
        tracing::info!("{}", format_instruction(*addr, instr));
    }

    assert!(!instructions.is_empty(), "Should decode MOV instructions");
}

#[test]
fn test_disassemble_arithmetic_instructions() {
    // Arithmetic instructions
    let data = [
        0x48, 0x01, 0xC1, // add rcx, rax
        0x48, 0x29, 0xD1, // sub rcx, rdx
        0x48, 0x83, 0xC1, 0x01, // add rcx, 1
        0x48, 0x83, 0xE9, 0x01, // sub rcx, 1
    ];

    let runtime_address = 0x20000000;
    let instructions = disassemble_sequence(&data, runtime_address, false);

    init_tracing();
    tracing::info!("Arithmetic instructions:");
    for (addr, instr) in &instructions {
        tracing::info!("{}", format_instruction(*addr, instr));
    }

    assert!(
        !instructions.is_empty(),
        "Should decode arithmetic instructions"
    );
}

#[test]
fn test_disassemble_with_relative_addressing() {
    // Instructions with relative addressing (similar to Zydis example)
    let data = [
        0x51, // push rcx
        0x8D, 0x45, 0xFF, // lea eax, [rbp-0x01]
        0x50, // push rax
        0xFF, 0x75, 0x0C, // push qword ptr [rbp+0x0C]
        0xFF, 0x75, 0x08, // push qword ptr [rbp+0x08]
    ];

    let runtime_address = 0x007FFFFFFF400000;
    let instructions = disassemble_sequence(&data, runtime_address, false);

    init_tracing();
    tracing::info!("Instructions with relative addressing:");
    for (addr, instr) in &instructions {
        tracing::info!("{}", format_instruction(*addr, instr));
    }

    assert!(
        !instructions.is_empty(),
        "Should decode instructions with addressing"
    );
}

#[test]
fn test_instruction_length_tracking() {
    // Test that instruction lengths are correctly tracked
    let data = [
        0x90, // nop (1 byte)
        0x48, 0x89, 0xC1, // mov rcx, rax (3 bytes)
        0x48, 0xC7, 0xC0, 0x00, 0x00, 0x00, 0x00, // mov rax, 0 (7 bytes)
    ];

    let runtime_address = 0x30000000;
    let instructions = disassemble_sequence(&data, runtime_address, false);

    init_tracing();
    let mut total_length = 0;
    for (addr, instr) in &instructions {
        let length = instr.meta_info.ilen as usize;
        total_length += length;
        tracing::info!("{} (len={})", format_instruction(*addr, instr), length);
    }

    // Verify total length matches data length
    assert_eq!(
        total_length,
        data.len(),
        "Total instruction length should match data length"
    );
}

#[test]
fn test_zydis_example() {
    init_tracing();
    let data = [
        0x48, 0x31, 0xff, 0x48, 0x31, 0xf6, 0x48, 0x31, 0xd2, 0x48, 0x31, 0xc0, 0x50, 0x48, 0xbb,
        0x2f, 0x62, 0x69, 0x6e, 0x2f, 0x2f, 0x73, 0x68, 0x53, 0x48, 0x89, 0xe7, 0xb0, 0x3b, 0x0f,
        0x05,
    ];

    let runtime_address = 0x007FFFFFFF400000;
    let instructions = disassemble_sequence(&data, runtime_address, false);

    for (_, instruction) in instructions {
        tracing::info!(
            "{:?} {} {}",
            instruction.get_ia_opcode(),
            instruction.dst(),
            instruction.src()
        );
    }
}
#[test]
fn test_zydis_example_64bit() {
    init_tracing();
    let data = [
        0x48, 0x31, 0xff, 0x48, 0x31, 0xf6, 0x48, 0x31, 0xd2, 0x48, 0x31, 0xc0, 0x50, 0x48, 0xbb,
        0x2f, 0x62, 0x69, 0x6e, 0x2f, 0x2f, 0x73, 0x68, 0x53, 0x48, 0x89, 0xe7, 0xb0, 0x3b, 0x0f,
        0x05,
    ];

    let runtime_address = 0x007FFFFFFF400000;
    let instructions = disassemble_sequence_64bit(&data, runtime_address, false);

    for (_, instruction) in instructions {
        tracing::info!(
            "{:?} {} {}",
            instruction.get_ia_opcode(),
            instruction.dst(),
            instruction.src()
        );
    }
}

#[test]
fn test_jmp_imm() {
    init_tracing();
    let data = [0x73, 0x68];

    let runtime_address = 0x007FFFFFFF400000;
    let jump_instruction = disassemble_sequence(&data, runtime_address, false)[0].1;

    assert_eq!(jump_instruction.meta_info.ilen, 2);
    assert_eq!(jump_instruction.get_ia_opcode(), Opcode::JnbJbw);
    tracing::info!("{:#x?}", jump_instruction)
}

fn disassemble_sequence_64bit(
    data: &[u8],
    runtime_address: u64,
    _is_32: bool,
) -> std::vec::Vec<(u64, Instruction)> {
    let mut offset = 0;
    let mut current_address = runtime_address;
    let mut instructions = std::vec::Vec::new();

    while offset < data.len() {
        let remaining = &data[offset..];

        match fetch_decode64(remaining) {
            Ok(instr) => {
                let length = instr.meta_info.ilen as usize;

                if length == 0 || offset + length > data.len() {
                    // Invalid instruction or out of bounds
                    tracing::error!("Invalid instruction length at offset {}", offset);
                    break;
                }

                instructions.push((current_address, instr));
                offset += length;
                current_address += length as u64;
            }
            Err(e) => {
                // Decode failed, stop
                tracing::error!("Decode error at offset {}: {:?}", offset, e);
                break;
            }
        }
    }

    instructions
}

// =============================================================================
// 3DNow! instruction tests
// =============================================================================

#[test]
fn test_3dnow_pi2fd() {
    init_tracing();
    // 0F 0F /r 0D = PI2FD mm, mm/m64
    // PI2FD MM0, MM1: 0F 0F C1 0D
    // ModRM C1 = 11 000 001 (mod=3, reg=0, rm=1)
    let data = [0x0F, 0x0F, 0xC1, 0x0D];
    let i = fetch_decode32(&data, true).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_eq!(i.get_ia_opcode(), Opcode::Pi2fdPqQq);
    tracing::info!("PI2FD: {:?}", i.get_ia_opcode());
}

#[test]
fn test_3dnow_pi2fw() {
    init_tracing();
    // 0F 0F /r 0C = PI2FW mm, mm/m64
    // PI2FW MM0, MM2: 0F 0F C2 0C
    let data = [0x0F, 0x0F, 0xC2, 0x0C];
    let i = fetch_decode32(&data, true).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_eq!(i.get_ia_opcode(), Opcode::Pi2fwPqQq);
}

#[test]
fn test_3dnow_pf2id() {
    init_tracing();
    // 0F 0F /r 1D = PF2ID mm, mm/m64
    let data = [0x0F, 0x0F, 0xC3, 0x1D];
    let i = fetch_decode32(&data, true).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_eq!(i.get_ia_opcode(), Opcode::Pf2idPqQq);
}

#[test]
fn test_3dnow_pf2iw() {
    init_tracing();
    // 0F 0F /r 1C = PF2IW mm, mm/m64
    let data = [0x0F, 0x0F, 0xC4, 0x1C];
    let i = fetch_decode32(&data, true).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_eq!(i.get_ia_opcode(), Opcode::Pf2iwPqQq);
}

#[test]
fn test_3dnow_pfadd() {
    init_tracing();
    // 0F 0F /r 9E = PFADD mm, mm/m64
    let data = [0x0F, 0x0F, 0xC5, 0x9E];
    let i = fetch_decode32(&data, true).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_eq!(i.get_ia_opcode(), Opcode::PfaddPqQq);
}

#[test]
fn test_3dnow_pfmul() {
    init_tracing();
    // 0F 0F /r B4 = PFMUL mm, mm/m64
    let data = [0x0F, 0x0F, 0xC6, 0xB4];
    let i = fetch_decode32(&data, true).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_eq!(i.get_ia_opcode(), Opcode::PfmulPqQq);
}

#[test]
fn test_3dnow_with_memory_operand() {
    init_tracing();
    // 3DNow! with memory operand and disp8
    // PI2FD MM0, [EBX+0x10]: 0F 0F 43 10 0D
    // ModRM 43 = 01 000 011 (mod=1, reg=0, rm=3=EBX, disp8)
    let data = [0x0F, 0x0F, 0x43, 0x10, 0x0D];
    let i = fetch_decode32(&data, true).unwrap();
    assert_eq!(i.ilen(), 5);
    assert_eq!(i.get_ia_opcode(), Opcode::Pi2fdPqQq);
    assert!(!i.mod_c0()); // Memory operand
    assert_eq!(i.modrm_form.displacement.displ32u(), 0x10);
}

#[test]
fn test_3dnow_invalid_suffix() {
    init_tracing();
    // 0F 0F /r 00 = Invalid (suffix 0x00 maps to IaError)
    let data = [0x0F, 0x0F, 0xC0, 0x00];
    let result = fetch_decode32(&data, true);
    // Should fail because suffix 0x00 is IaError in BX3_DNOW_OPCODE
    assert!(result.is_err());
}

#[test]
fn test_3dnow_64bit() {
    init_tracing();
    // 3DNow! in 64-bit mode (still valid)
    // PI2FD MM0, MM1: 0F 0F C1 0D
    let data = [0x0F, 0x0F, 0xC1, 0x0D];
    let i = fetch_decode64(&data).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_eq!(i.get_ia_opcode(), Opcode::Pi2fdPqQq);
}

// =============================================================================
// REX prefix interaction tests
// =============================================================================

#[test]
fn test_rex_w_sets_os64() {
    init_tracing();
    // REX.W alone should set Os64
    // 48 89 C0 = MOV RAX, RAX (64-bit)
    let i = fetch_decode64(&[0x48, 0x89, 0xC0]).unwrap();
    assert_eq!(i.ilen(), 3);
    assert_ne!(i.os64_l(), 0, "Os64 should be set with REX.W");
}

#[test]
fn test_rex_b_extends_rm() {
    init_tracing();
    // REX.B (0x41) extends the rm field to access R8-R15
    // 41 89 C0 = MOV R8D, EAX (REX.B extends rm from 0 to 8)
    let i = fetch_decode64(&[0x41, 0x89, 0xC0]).unwrap();
    assert_eq!(i.ilen(), 3);
    // The rm field (src1) should be extended by REX.B
    // rm is stored in src1/meta_data[1]
    assert_eq!(i.src1(), 8, "rm (src1) should be extended to R8 by REX.B");
}

#[test]
fn test_rex_r_extends_nnn() {
    init_tracing();
    // REX.R (0x44) extends the reg/nnn field
    // 44 89 C0 = MOV EAX, R8D (REX.R extends reg from 0 to 8)
    let i = fetch_decode64(&[0x44, 0x89, 0xC0]).unwrap();
    assert_eq!(i.ilen(), 3);
    // The nnn field (dst) should be extended by REX.R
    // nnn is stored in dst/meta_data[0]
    assert_eq!(i.dst(), 8, "nnn (dst) should be extended to R8 by REX.R");
}

#[test]
fn test_segment_prefix_before_rex() {
    init_tracing();
    // Segment override prefix (0x65 = GS:) BEFORE REX is valid and both apply
    // 65 48 8B 00 = MOV RAX, GS:[RAX]
    let i = fetch_decode64(&[0x65, 0x48, 0x8B, 0x00]).unwrap();
    assert_eq!(i.ilen(), 4);
    // REX.W should set Os64
    assert_ne!(i.os64_l(), 0, "Os64 should be set with REX.W");
    // GS segment override should be recorded
    assert_eq!(i.seg(), 5, "Segment should be GS (5)");
}

// =============================================================================
// RIP-relative addressing tests
// =============================================================================

#[test]
fn test_rip_relative_addressing() {
    init_tracing();
    // MOV EAX, [RIP+0x12345678]: 8B 05 78 56 34 12
    // ModRM 05 = 00 000 101 (mod=0, reg=0=EAX, rm=5=RIP-relative in 64-bit)
    let i = fetch_decode64(&[0x8B, 0x05, 0x78, 0x56, 0x34, 0x12]).unwrap();
    assert_eq!(i.ilen(), 6);
    assert_eq!(i.sib_base(), 17, "Base should be BX_64BIT_REG_RIP (17)");
    assert_eq!(i.modrm_form.displacement.displ32u(), 0x12345678);
}

#[test]
fn test_rip_relative_with_rex() {
    init_tracing();
    // MOV RAX, [RIP+0x10]: 48 8B 05 10 00 00 00
    let i = fetch_decode64(&[0x48, 0x8B, 0x05, 0x10, 0x00, 0x00, 0x00]).unwrap();
    assert_eq!(i.ilen(), 7);
    assert_eq!(i.sib_base(), 17, "Base should be BX_64BIT_REG_RIP (17)");
    assert_eq!(i.modrm_form.displacement.displ32u(), 0x10);
    assert_ne!(i.os64_l(), 0, "Should have 64-bit operand size");
}

#[test]
fn test_not_rip_relative_in_32bit() {
    init_tracing();
    // In 32-bit mode, mod=0 rm=5 is [disp32], not RIP-relative
    // MOV EAX, [0x12345678]: 8B 05 78 56 34 12
    let i = fetch_decode32(&[0x8B, 0x05, 0x78, 0x56, 0x34, 0x12], true).unwrap();
    assert_eq!(i.ilen(), 6);
    // In 32-bit mode, this should be BX_NIL_REGISTER (19), not RIP
    assert_eq!(
        i.sib_base(),
        19,
        "Base should be BX_NIL_REGISTER (19) in 32-bit mode"
    );
    assert_eq!(i.modrm_form.displacement.displ32u(), 0x12345678);
}
