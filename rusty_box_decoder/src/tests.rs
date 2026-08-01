use crate::{
    decoder::{decode32::fetch_decode32, decode64::fetch_decode64, tables::BxDecodeError},
    error::DecodeError,
    instruction::Instruction,
    opcode::Opcode,
};
use std::vec::Vec;

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
    let length = instr.length;

    // Format address as 16 hex digits
    std::format!("{:016X}  {} (len={})", address, opcode_name, length)
}

/// Disassemble a sequence of instructions from a byte buffer
///
/// Similar to Zydis example: loops over instructions in buffer and prints them
fn disassemble_sequence(data: &[u8], runtime_address: u64, is_32: bool) -> Vec<(u64, Instruction)> {
    let mut offset = 0;
    let mut current_address = runtime_address;
    let mut instructions = Vec::new();

    while offset < data.len() {
        let remaining = &data[offset..];

        match fetch_decode32(remaining, is_32) {
            Ok(instr) => {
                let length = instr.length as usize;

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
        let length = instr.length as usize;
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

    assert_eq!(jump_instruction.length, 2);
    assert_eq!(jump_instruction.get_ia_opcode(), Opcode::JnbJbw);
    tracing::info!("{:#x?}", jump_instruction)
}

fn disassemble_sequence_64bit(
    data: &[u8],
    runtime_address: u64,
    _is_32: bool,
) -> Vec<(u64, Instruction)> {
    let mut offset = 0;
    let mut current_address = runtime_address;
    let mut instructions = Vec::new();

    while offset < data.len() {
        let remaining = &data[offset..];

        match fetch_decode64(remaining) {
            Ok(instr) => {
                let length = instr.length as usize;

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
    assert_eq!(i.displacement, 0x10);
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
    // 0x89 is in Ed,Gd branch: DST=rm, SRC1=nnn
    // REX.B extends rm, so dst should be R8 (8)
    assert_eq!(i.dst(), 8, "rm (dst) should be extended to R8 by REX.B");
}

#[test]
fn test_rex_r_extends_nnn() {
    init_tracing();
    // REX.R (0x44) extends the reg/nnn field
    // 44 89 C0 = MOV EAX, R8D (REX.R extends reg from 0 to 8)
    let i = fetch_decode64(&[0x44, 0x89, 0xC0]).unwrap();
    assert_eq!(i.ilen(), 3);
    // 0x89 is in Ed,Gd branch: DST=rm, SRC1=nnn
    // REX.R extends nnn, so src1 should be R8 (8)
    assert_eq!(i.src1(), 8, "nnn (src1) should be extended to R8 by REX.R");
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
// Regression tests for previously discovered decoder bugs
// =============================================================================

// -- SHRD/SHLD Ed,Gd direction (session 13 fix) --
// These opcodes (0F A4/A5/AC/AD) must be in the Ed,Gd branch:
// dst()=rm (destination register), src1()=nnn (shift source register).
// Bug: they were in the ELSE branch, swapping dst/src1, causing ext2
// "directory #12 contains a hole" errors during DLX boot.

#[test]
fn test_shld_reg_imm8_direction() {
    // SHLD EBX, ECX, 5 = 0F A4 CB 05
    // ModRM CB = 11 001 011: mod=3, reg=1(ECX), rm=3(EBX)
    // Ed,Gd: dst=rm=3(EBX), src1=nnn=1(ECX)
    let i = fetch_decode32(&[0x0F, 0xA4, 0xCB, 0x05], true).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_eq!(i.dst(), 3, "SHLD dst should be rm=EBX(3)");
    assert_eq!(i.src1(), 1, "SHLD src1 should be nnn=ECX(1)");
}

#[test]
fn test_shrd_reg_imm8_direction() {
    // SHRD EBX, ECX, 5 = 0F AC CB 05
    let i = fetch_decode32(&[0x0F, 0xAC, 0xCB, 0x05], true).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_eq!(i.dst(), 3, "SHRD dst should be rm=EBX(3)");
    assert_eq!(i.src1(), 1, "SHRD src1 should be nnn=ECX(1)");
}

#[test]
fn test_shld_reg_cl_direction() {
    // SHLD EBX, ECX, CL = 0F A5 CB
    let i = fetch_decode32(&[0x0F, 0xA5, 0xCB], true).unwrap();
    assert_eq!(i.ilen(), 3);
    assert_eq!(i.dst(), 3, "SHLD-CL dst should be rm=EBX(3)");
    assert_eq!(i.src1(), 1, "SHLD-CL src1 should be nnn=ECX(1)");
}

#[test]
fn test_shrd_reg_cl_direction() {
    // SHRD EBX, ECX, CL = 0F AD CB
    let i = fetch_decode32(&[0x0F, 0xAD, 0xCB], true).unwrap();
    assert_eq!(i.ilen(), 3);
    assert_eq!(i.dst(), 3, "SHRD-CL dst should be rm=EBX(3)");
    assert_eq!(i.src1(), 1, "SHRD-CL src1 should be nnn=ECX(1)");
}

#[test]
fn test_shld_64bit_direction() {
    // 64-bit: SHLD RBX, RCX, 5 = 48 0F A4 CB 05
    let i = fetch_decode64(&[0x48, 0x0F, 0xA4, 0xCB, 0x05]).unwrap();
    assert_eq!(i.ilen(), 5);
    assert_eq!(i.dst(), 3, "64-bit SHLD dst should be rm=RBX(3)");
    assert_eq!(i.src1(), 1, "64-bit SHLD src1 should be nnn=RCX(1)");
}

// -- MOVQ 66 0F D6 Ed,Gd direction (session 44-45 fix) --
// 66 0F D6 is MOVQ xmm/m64, xmm — a STORE instruction.
// Must be Ed,Gd: dst=rm (destination), src1=nnn (source XMM).

#[test]
fn test_movq_66_0f_d6_store_direction() {
    // 66 0F D6 D1 = MOVQ XMM1, XMM2
    // ModRM D1 = 11 010 001: reg=2(XMM2), rm=1(XMM1)
    // Store: dst=rm=1, src1=nnn=2
    let i = fetch_decode32(&[0x66, 0x0F, 0xD6, 0xD1], true).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_eq!(i.dst(), 1, "MOVQ store dst should be rm=XMM1(1)");
    assert_eq!(i.src1(), 2, "MOVQ store src1 should be nnn=XMM2(2)");
}

#[test]
fn test_movq_66_0f_d6_store_direction_64bit() {
    let i = fetch_decode64(&[0x66, 0x0F, 0xD6, 0xD1]).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_eq!(i.dst(), 1, "64-bit MOVQ store dst should be rm=XMM1(1)");
    assert_eq!(i.src1(), 2, "64-bit MOVQ store src1 should be nnn=XMM2(2)");
}

// -- F3 0F 7E MOVQ Vq,Wq exclusion from Ed,Gd (session 39 fix) --
// F3 0F 7E is MOVQ xmm, xmm/m64 — a LOAD instruction.
// Must NOT be Ed,Gd: dst=nnn (destination XMM), src1=rm (source).
// Bug: 0x17E was in Ed,Gd branch for ALL SSE prefix variants.

#[test]
fn test_movq_f3_0f_7e_load_direction() {
    // F3 0F 7E D1 = MOVQ XMM2, XMM1
    // ModRM D1 = 11 010 001: reg=2(XMM2), rm=1(XMM1)
    // Load: dst=nnn=2, src1=rm=1
    let i = fetch_decode32(&[0xF3, 0x0F, 0x7E, 0xD1], true).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_eq!(i.dst(), 2, "F3 0F 7E MOVQ load dst should be nnn=XMM2(2)");
    assert_eq!(i.src1(), 1, "F3 0F 7E MOVQ load src1 should be rm=XMM1(1)");
}

#[test]
fn test_movq_f3_0f_7e_load_direction_64bit() {
    let i = fetch_decode64(&[0xF3, 0x0F, 0x7E, 0xD1]).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_eq!(i.dst(), 2, "64-bit F3 0F 7E dst should be nnn=2");
    assert_eq!(i.src1(), 1, "64-bit F3 0F 7E src1 should be rm=1");
}

#[test]
fn test_final_entry_one_byte_os16_32bit_add() {
    let i = fetch_decode32(&[0x66, 0x01, 0xD8], true).unwrap();
    assert_eq!(i.ilen(), 3);
    assert_eq!(i.get_ia_opcode(), Opcode::AddEwGw);
}

#[test]
fn test_final_entry_one_byte_os16_64bit_add() {
    let i = fetch_decode64(&[0x66, 0x01, 0xD8]).unwrap();
    assert_eq!(i.ilen(), 3);
    assert_eq!(i.get_ia_opcode(), Opcode::AddEwGw);
}

#[test]
fn test_final_entry_0f_sse_f2_lookup() {
    let i32 = fetch_decode32(&[0xF2, 0x0F, 0xE6, 0xC1], true).unwrap();
    assert_eq!(i32.ilen(), 4);
    assert_eq!(i32.get_ia_opcode(), Opcode::Cvtpd2dqVqWpd);

    let i64 = fetch_decode64(&[0xF2, 0x0F, 0xE6, 0xC1]).unwrap();
    assert_eq!(i64.ilen(), 4);
    assert_eq!(i64.get_ia_opcode(), Opcode::Cvtpd2dqVqWpd);
}

#[test]
fn test_final_entry_0f38_sse66_lookup() {
    let i32 = fetch_decode32(&[0x66, 0x0F, 0x38, 0x00, 0xC1], true).unwrap();
    assert_eq!(i32.ilen(), 5);
    assert_eq!(i32.get_ia_opcode(), Opcode::PshufbVdqWdq);

    let i64 = fetch_decode64(&[0x66, 0x0F, 0x38, 0x00, 0xC1]).unwrap();
    assert_eq!(i64.ilen(), 5);
    assert_eq!(i64.get_ia_opcode(), Opcode::PshufbVdqWdq);
}

#[test]
fn test_final_entry_0f3a_sse66_lookup() {
    let i32 = fetch_decode32(&[0x66, 0x0F, 0x3A, 0x0F, 0xC1, 0x05], true).unwrap();
    assert_eq!(i32.ilen(), 6);
    assert_eq!(i32.get_ia_opcode(), Opcode::PalignrVdqWdqIb);
    assert_eq!(i32.ib(), 0x05);

    let i64 = fetch_decode64(&[0x66, 0x0F, 0x3A, 0x0F, 0xC1, 0x05]).unwrap();
    assert_eq!(i64.ilen(), 6);
    assert_eq!(i64.get_ia_opcode(), Opcode::PalignrVdqWdqIb);
    assert_eq!(i64.ib(), 0x05);
}

#[test]
fn test_vex_vperm2f128_decode_ubuntu_sha512_sequence() {
    // Ubuntu 26.04 kernel sha512_transform_rorx uses this AVX2 sequence when
    // CPUID advertises AVX2/BMI2. It must decode as VPERM2F128, not #UD.
    let instr = fetch_decode64(&[0xC4, 0xE3, 0x45, 0x06, 0xC6, 0x03]).unwrap();
    assert_eq!(instr.ilen(), 6);
    assert_eq!(instr.get_ia_opcode(), Opcode::V256Vperm2f128VdqHdqWdqIb);
    assert_eq!(instr.ib(), 0x03);
}

#[test]
fn test_vex_vpermq_decode_runtime_sequence() {
    // Captured from Ubuntu userspace boot: VEX.256.66.0F3A.W1 00 /r ib.
    let instr = fetch_decode64(&[0xC4, 0xE3, 0xFD, 0x00, 0xD8, 0xFF]).unwrap();
    assert_eq!(instr.ilen(), 6);
    assert_eq!(instr.get_ia_opcode(), Opcode::V256VpermqVdqWdqIb);
    assert_eq!(instr.dst(), 3);
    assert_eq!(instr.src1(), 0);
    assert_eq!(instr.ib(), 0xFF);
    assert_eq!(instr.get_vl(), 1);
}

#[test]
fn test_vex_vextractf128_decode_win10_setup_sequence() {
    // Captured from Windows 10 setup (VEX.256.66.0F3A.W0 19 /r ib).
    // Sibling of VEXTRACTI128 (0x39); must decode, not #UD.
    // c4 e3 7d 19 d8 01 = VEXTRACTF128 xmm0, ymm3, 1
    let instr = fetch_decode64(&[0xC4, 0xE3, 0x7D, 0x19, 0xD8, 0x01]).unwrap();
    assert_eq!(instr.ilen(), 6);
    assert_eq!(instr.get_ia_opcode(), Opcode::V256Vextractf128WdqVdqIb);
    assert_eq!(instr.dst(), 3); // nnn = source YMM (ymm3)
    assert_eq!(instr.src1(), 0); // rm = destination XMM (xmm0)
    assert_eq!(instr.ib(), 0x01);
    assert_eq!(instr.get_vl(), 1);
}

#[test]
fn test_vex_pinsr_family_decode() {
    // VEX integer-insert family — 3-operand (vvvv = base vector). These used to
    // mis-decode as the 2-operand SSE PINSR* (dropping vvvv → wrong data).

    // VPINSRW xmm0, xmm1, eax, 3 — VEX.128.66.0F.W0 C4 (2-byte VEX C5).
    // C5 F1: R=1(→0) vvvv=~1=1110 L=0 pp=01(66); modrm C0 = mod11 reg000 rm000.
    let w = fetch_decode64(&[0xC5, 0xF1, 0xC4, 0xC0, 0x03]).unwrap();
    assert_eq!(w.ilen(), 5);
    assert_eq!(w.get_ia_opcode(), Opcode::V128VpinsrwVdqEwIb);
    assert_eq!(w.dst(), 0);
    assert_eq!(w.src2(), 1); // vvvv = base vector (xmm1)
    assert_eq!(w.src1(), 0); // rm = GPR source (eax)
    assert_eq!(w.ib(), 0x03);

    // VPINSRB xmm0, xmm1, al, 5 — VEX.128.66.0F3A.W0 20 (3-byte VEX C4).
    let b = fetch_decode64(&[0xC4, 0xE3, 0x71, 0x20, 0xC0, 0x05]).unwrap();
    assert_eq!(b.ilen(), 6);
    assert_eq!(b.get_ia_opcode(), Opcode::V128VpinsrbVdqEbIb);
    assert_eq!(b.src2(), 1);
    assert_eq!(b.ib(), 0x05);

    // VPINSRD xmm0, xmm2, ecx, 2 — VEX.128.66.0F3A.W0 22.
    let d = fetch_decode64(&[0xC4, 0xE3, 0x69, 0x22, 0xC1, 0x02]).unwrap();
    assert_eq!(d.ilen(), 6);
    assert_eq!(d.get_ia_opcode(), Opcode::V128VpinsrdVdqEdIb);
    assert_eq!(d.src2(), 2); // vvvv = xmm2
    assert_eq!(d.src1(), 1); // rm = ecx

    // VPINSRQ xmm0, xmm2, rcx, 1 — VEX.128.66.0F3A.W1 22 (W1 → qword form).
    let q = fetch_decode64(&[0xC4, 0xE3, 0xE9, 0x22, 0xC1, 0x01]).unwrap();
    assert_eq!(q.ilen(), 6);
    assert_eq!(q.get_ia_opcode(), Opcode::V128VpinsrqVdqEqIb);

    // VEX.256 encoding of VPINSRW is illegal (VL128-only) → #UD.
    assert!(fetch_decode64(&[0xC5, 0xF5, 0xC4, 0xC0, 0x03]).is_err());

    // Legacy (non-VEX) PINSRW must still decode as the 2-operand SSE form.
    let legacy = fetch_decode64(&[0x66, 0x0F, 0xC4, 0xC1, 0x03]).unwrap();
    assert_eq!(legacy.get_ia_opcode(), Opcode::PinsrwVdqEwIb);
}

#[test]
fn test_vex_vtestps_vtestpd_decode() {
    // VTESTPS ymm1, ymm2 — VEX.256.66.0F38.W0 0E /r.
    // C4 E2: 3-byte VEX, mmmmm=02 (0F38). 7D: W=0 vvvv=1111 L=1 pp=01(66).
    let ps = fetch_decode64(&[0xC4, 0xE2, 0x7D, 0x0E, 0xCA]).unwrap();
    assert_eq!(ps.ilen(), 5);
    assert_eq!(ps.get_ia_opcode(), Opcode::VtestpsVpsWps);
    assert_eq!(ps.dst(), 1); // nnn
    assert_eq!(ps.src1(), 2); // rm
    assert_eq!(ps.get_vl(), 1);

    // VTESTPD xmm0, xmm1 — VEX.128.66.0F38.W0 0F /r.
    let pd = fetch_decode64(&[0xC4, 0xE2, 0x79, 0x0F, 0xC1]).unwrap();
    assert_eq!(pd.get_ia_opcode(), Opcode::VtestpdVpdWpd);
    assert_eq!(pd.dst(), 0);
    assert_eq!(pd.src1(), 1);
    assert_eq!(pd.get_vl(), 0);

    // Memory source form must decode too (Bochs LOAD_Vector path).
    let mem = fetch_decode64(&[0xC4, 0xE2, 0x7D, 0x0E, 0x08]).unwrap();
    assert_eq!(mem.get_ia_opcode(), Opcode::VtestpsVpsWps);
    assert!(!mem.mod_c0());

    // VEX.vvvv is reserved (must be 1111b) — Bochs "VEX.VVV #UD".
    assert!(fetch_decode64(&[0xC4, 0xE2, 0x75, 0x0E, 0xCA]).is_err());
    // VEX.W1 is reserved.
    assert!(fetch_decode64(&[0xC4, 0xE2, 0xFD, 0x0E, 0xCA]).is_err());
    // No legacy (non-VEX) encoding exists at 66 0F 38 0E/0F.
    assert!(fetch_decode64(&[0x66, 0x0F, 0x38, 0x0E, 0xCA]).is_err());
}

#[test]
fn test_vex_vpermil_decode() {
    // VPERMILPS ymm1, ymm2, ymm3 — VEX.256.66.0F38.W0 0C /r.
    // vvvv is the DATA operand, ModRM.rm is the per-element CONTROL.
    let ps = fetch_decode64(&[0xC4, 0xE2, 0x6D, 0x0C, 0xCB]).unwrap();
    assert_eq!(ps.get_ia_opcode(), Opcode::VpermilpsVpsHpsWps);
    assert_eq!(ps.dst(), 1);
    assert_eq!(ps.src2(), 2); // vvvv = data
    assert_eq!(ps.src1(), 3); // rm = control
    assert_eq!(ps.get_vl(), 1);

    // VPERMILPD xmm0, xmm1, xmm2 — VEX.128.66.0F38.W0 0D /r.
    let pd = fetch_decode64(&[0xC4, 0xE2, 0x71, 0x0D, 0xC2]).unwrap();
    assert_eq!(pd.get_ia_opcode(), Opcode::VpermilpdVpdHpdWpd);
    assert_eq!(pd.dst(), 0);
    assert_eq!(pd.src2(), 1);
    assert_eq!(pd.src1(), 2);
    assert_eq!(pd.get_vl(), 0);

    // VPERMILPS ymm1, ymm2, 0x1B — VEX.256.66.0F3A.W0 04 /r ib (no vvvv).
    let ps_ib = fetch_decode64(&[0xC4, 0xE3, 0x7D, 0x04, 0xCA, 0x1B]).unwrap();
    assert_eq!(ps_ib.get_ia_opcode(), Opcode::VpermilpsVpsWpsIb);
    assert_eq!(ps_ib.dst(), 1);
    assert_eq!(ps_ib.src1(), 2);
    assert_eq!(ps_ib.ib(), 0x1B);

    // VPERMILPD ymm1, ymm2, 0x05 — VEX.256.66.0F3A.W0 05 /r ib.
    let pd_ib = fetch_decode64(&[0xC4, 0xE3, 0x7D, 0x05, 0xCA, 0x05]).unwrap();
    assert_eq!(pd_ib.get_ia_opcode(), Opcode::VpermilpdVpdWpdIb);
    assert_eq!(pd_ib.ib(), 0x05);

    // VEX.W1 is reserved for all four.
    assert!(fetch_decode64(&[0xC4, 0xE2, 0xED, 0x0C, 0xCB]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE3, 0xFD, 0x04, 0xCA, 0x1B]).is_err());
    // The imm8 forms take no vvvv source.
    assert!(fetch_decode64(&[0xC4, 0xE3, 0x75, 0x04, 0xCA, 0x1B]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE3, 0x75, 0x05, 0xCA, 0x05]).is_err());
    // No legacy encoding exists at 66 0F 38 0C.
    assert!(fetch_decode64(&[0x66, 0x0F, 0x38, 0x0C, 0xCB]).is_err());
}

#[test]
fn test_vex_vpermps_vpermpd_decode() {
    // VPERMPS ymm1, ymm2, ymm3 — VEX.256.66.0F38.W0 16 /r (256-bit only).
    let ps = fetch_decode64(&[0xC4, 0xE2, 0x6D, 0x16, 0xCB]).unwrap();
    assert_eq!(ps.get_ia_opcode(), Opcode::V256VpermpsVpsHpsWps);
    assert_eq!(ps.dst(), 1);
    assert_eq!(ps.src2(), 2); // vvvv = index vector
    assert_eq!(ps.src1(), 3); // rm = source vector
    assert_eq!(ps.get_vl(), 1);

    // VPERMPD ymm1, ymm2, 0x1B — VEX.256.66.0F3A.W1 01 /r ib (256-bit only).
    let pd = fetch_decode64(&[0xC4, 0xE3, 0xFD, 0x01, 0xCA, 0x1B]).unwrap();
    assert_eq!(pd.get_ia_opcode(), Opcode::V256VpermpdVpdWpdIb);
    assert_eq!(pd.dst(), 1);
    assert_eq!(pd.src1(), 2);
    assert_eq!(pd.ib(), 0x1B);

    // VEX.128 forms are reserved.
    assert!(fetch_decode64(&[0xC4, 0xE2, 0x69, 0x16, 0xCB]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE3, 0xF9, 0x01, 0xCA, 0x1B]).is_err());
    // VPERMPS is W0-only, VPERMPD is W1-only.
    assert!(fetch_decode64(&[0xC4, 0xE2, 0xED, 0x16, 0xCB]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE3, 0x7D, 0x01, 0xCA, 0x1B]).is_err());
    // VPERMPD takes no vvvv source.
    assert!(fetch_decode64(&[0xC4, 0xE3, 0xF5, 0x01, 0xCA, 0x1B]).is_err());
}

#[test]
fn test_vex_variable_shift_decode() {
    // VPSRLVD/Q, VPSRAVD, VPSLLVD/Q — VEX.66.0F38 45/46/47, W-split.
    // vvvv holds the values to shift; ModRM.rm holds the per-element counts.
    let cases: &[(&[u8], Opcode)] = &[
        (&[0xC4, 0xE2, 0x6D, 0x45, 0xCB], Opcode::VpsrlvdVdqHdqWdq),
        (&[0xC4, 0xE2, 0xED, 0x45, 0xCB], Opcode::VpsrlvqVdqHdqWdq),
        (&[0xC4, 0xE2, 0x6D, 0x46, 0xCB], Opcode::VpsravdVdqHdqWdq),
        (&[0xC4, 0xE2, 0x6D, 0x47, 0xCB], Opcode::VpsllvdVdqHdqWdq),
        (&[0xC4, 0xE2, 0xED, 0x47, 0xCB], Opcode::VpsllvqVdqHdqWdq),
    ];
    for (bytes, want) in cases {
        let i = fetch_decode64(bytes).unwrap();
        assert_eq!(i.get_ia_opcode(), *want, "bytes {bytes:02X?}");
        assert_eq!(i.dst(), 1);
        assert_eq!(i.src2(), 2); // vvvv = values
        assert_eq!(i.src1(), 3); // rm = counts
        assert_eq!(i.get_vl(), 1);
    }

    // VPSRAVD has no VEX.W1 form (VPSRAVQ is AVX-512 only).
    assert!(fetch_decode64(&[0xC4, 0xE2, 0xED, 0x46, 0xCB]).is_err());
    // 128-bit forms decode too.
    let vl128 = fetch_decode64(&[0xC4, 0xE2, 0x69, 0x45, 0xCB]).unwrap();
    assert_eq!(vl128.get_ia_opcode(), Opcode::VpsrlvdVdqHdqWdq);
    assert_eq!(vl128.get_vl(), 0);
    // No legacy encoding exists at 66 0F 38 45.
    assert!(fetch_decode64(&[0x66, 0x0F, 0x38, 0x45, 0xCB]).is_err());
}

#[test]
fn test_vex_f16c_decode() {
    // VCVTPH2PS ymm1, xmm2 — VEX.256.66.0F38.W0 13 /r.
    let up = fetch_decode64(&[0xC4, 0xE2, 0x7D, 0x13, 0xCA]).unwrap();
    assert_eq!(up.get_ia_opcode(), Opcode::Vcvtph2psVpsWps);
    assert_eq!(up.dst(), 1);
    assert_eq!(up.src1(), 2);
    assert_eq!(up.get_vl(), 1);

    let up128 = fetch_decode64(&[0xC4, 0xE2, 0x79, 0x13, 0xCA]).unwrap();
    assert_eq!(up128.get_ia_opcode(), Opcode::Vcvtph2psVpsWps);
    assert_eq!(up128.get_vl(), 0);

    // VCVTPS2PH xmm1, ymm2, 0 — VEX.256.66.0F3A.W0 1D /r ib. The destination
    // is ModRM.rm, so nnn is the *source*: modrm D1 → nnn=2, rm=1.
    let down = fetch_decode64(&[0xC4, 0xE3, 0x7D, 0x1D, 0xD1, 0x00]).unwrap();
    assert_eq!(down.get_ia_opcode(), Opcode::Vcvtps2phWpsVpsIb);
    assert_eq!(down.dst(), 2); // nnn = source ymm2
    assert_eq!(down.src1(), 1); // rm = destination xmm1
    assert_eq!(down.ib(), 0x00);

    // Memory-destination form.
    let down_mem = fetch_decode64(&[0xC4, 0xE3, 0x7D, 0x1D, 0x10, 0x04]).unwrap();
    assert_eq!(down_mem.get_ia_opcode(), Opcode::Vcvtps2phWpsVpsIb);
    assert!(!down_mem.mod_c0());
    assert_eq!(down_mem.ib(), 0x04);

    // Both are VEX.W0-only and take no vvvv source.
    assert!(fetch_decode64(&[0xC4, 0xE2, 0xFD, 0x13, 0xCA]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE3, 0xFD, 0x1D, 0xD1, 0x00]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE2, 0x75, 0x13, 0xCA]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE3, 0x75, 0x1D, 0xD1, 0x00]).is_err());
    // No legacy encodings exist.
    assert!(fetch_decode64(&[0x66, 0x0F, 0x38, 0x13, 0xCA]).is_err());
    assert!(fetch_decode64(&[0x66, 0x0F, 0x3A, 0x1D, 0xD1, 0x00]).is_err());
}

#[test]
fn test_vex_maskmov_decode() {
    // VMASKMOVPS ymm1, ymm2, [rax] — VEX.256.66.0F38.W0 2C /r.
    // 6D: W=0 vvvv=~2 L=1 pp=66. modrm 08 = mod00 reg=1 rm=0(rax).
    let load = fetch_decode64(&[0xC4, 0xE2, 0x6D, 0x2C, 0x08]).unwrap();
    assert_eq!(load.get_ia_opcode(), Opcode::VmaskmovpsVpsHpsMps);
    assert_eq!(load.dst(), 1);
    assert_eq!(load.src2(), 2); // vvvv = mask
    assert!(!load.mod_c0());

    // VMASKMOVPS [rax], ymm2, ymm1 — VEX.256.66.0F38.W0 2E /r.
    let store = fetch_decode64(&[0xC4, 0xE2, 0x6D, 0x2E, 0x08]).unwrap();
    assert_eq!(store.get_ia_opcode(), Opcode::VmaskmovpsMpsHpsVps);
    assert_eq!(store.dst(), 1); // nnn = data source
    assert_eq!(store.src2(), 2); // vvvv = mask

    // The W-split AVX2 integer forms.
    let cases: &[(&[u8], Opcode)] = &[
        (&[0xC4, 0xE2, 0x6D, 0x2D, 0x08], Opcode::VmaskmovpdVpdHpdMpd),
        (&[0xC4, 0xE2, 0x6D, 0x2F, 0x08], Opcode::VmaskmovpdMpdHpdVpd),
        (&[0xC4, 0xE2, 0x6D, 0x8C, 0x08], Opcode::VmaskmovdVdqHdqMdq),
        (&[0xC4, 0xE2, 0xED, 0x8C, 0x08], Opcode::VmaskmovqVdqHdqMdq),
        (&[0xC4, 0xE2, 0x6D, 0x8E, 0x08], Opcode::VmaskmovdMdqHdqVdq),
        (&[0xC4, 0xE2, 0xED, 0x8E, 0x08], Opcode::VmaskmovqMdqHdqVdq),
    ];
    for (bytes, want) in cases {
        let i = fetch_decode64(bytes).unwrap();
        assert_eq!(i.get_ia_opcode(), *want, "bytes {bytes:02X?}");
        assert_eq!(i.src2(), 2);
    }

    // Every group is memory-only — the register form (mod=11) is #UD.
    assert!(fetch_decode64(&[0xC4, 0xE2, 0x6D, 0x2C, 0xCB]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE2, 0x6D, 0x2E, 0xCB]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE2, 0x6D, 0x8C, 0xCB]).is_err());
    // VMASKMOVPS/PD are VEX.W0-only.
    assert!(fetch_decode64(&[0xC4, 0xE2, 0xED, 0x2C, 0x08]).is_err());
    // No legacy encodings exist.
    assert!(fetch_decode64(&[0x66, 0x0F, 0x38, 0x2C, 0x08]).is_err());
}

#[test]
fn test_vex_gather_decode() {
    // VPGATHERDD xmm1, [rax+xmm2*4], xmm3 — VEX.128.66.0F38.W0 90 /r.
    // 61: W=0 vvvv=~3 L=0 pp=66. modrm 0C = mod00 reg=1 rm=4(SIB).
    // sib 90 = scale=2(*4) index=2 base=0(rax).
    let g = fetch_decode64(&[0xC4, 0xE2, 0x61, 0x90, 0x0C, 0x90]).unwrap();
    assert_eq!(g.get_ia_opcode(), Opcode::VgatherddVdqHdq);
    assert_eq!(g.dst(), 1);
    assert_eq!(g.src2(), 3); // vvvv = mask vector
    assert_eq!(g.sib_index(), 2); // VSIB index is a vector register
    assert_eq!(g.sib_scale(), 2);
    assert_eq!(g.sib_base(), 0);
    assert!(!g.mod_c0());

    // xmm4 is a legal VSIB index even though GPR index 4 means "no index"
    // in an ordinary SIB byte — sib A0 = scale=2 index=4 base=0.
    let idx4 = fetch_decode64(&[0xC4, 0xE2, 0x61, 0x90, 0x0C, 0xA0]).unwrap();
    assert_eq!(idx4.sib_index(), 4);

    let cases: &[(&[u8], Opcode)] = &[
        (&[0xC4, 0xE2, 0xE1, 0x90, 0x0C, 0x90], Opcode::VgatherdqVdqHdq),
        (&[0xC4, 0xE2, 0x61, 0x91, 0x0C, 0x90], Opcode::VgatherqdVdqHdq),
        (&[0xC4, 0xE2, 0xE1, 0x91, 0x0C, 0x90], Opcode::VgatherqqVdqHdq),
        (&[0xC4, 0xE2, 0x61, 0x92, 0x0C, 0x90], Opcode::VgatherdpsVpsHps),
        (&[0xC4, 0xE2, 0xE1, 0x92, 0x0C, 0x90], Opcode::VgatherdpdVpdHpd),
        (&[0xC4, 0xE2, 0x61, 0x93, 0x0C, 0x90], Opcode::VgatherqpsVpsHps),
        (&[0xC4, 0xE2, 0xE1, 0x93, 0x0C, 0x90], Opcode::VgatherqpdVpdHpd),
    ];
    for (bytes, want) in cases {
        let i = fetch_decode64(bytes).unwrap();
        assert_eq!(i.get_ia_opcode(), *want, "bytes {bytes:02X?}");
        assert_eq!(i.src2(), 3);
        assert_eq!(i.sib_index(), 2);
    }

    // Register form is #UD — every gather group is memory-only.
    assert!(fetch_decode64(&[0xC4, 0xE2, 0x61, 0x90, 0xCB]).is_err());
    // No legacy encoding exists.
    assert!(fetch_decode64(&[0x66, 0x0F, 0x38, 0x90, 0x0C, 0x90]).is_err());
}

#[test]
fn test_vex_vl128_only_legacy_shared_forms_decode() {
    // These VEX forms share the legacy SSE opcode's table entry. Bochs marks
    // every one of their VEX groups ATTR_VL128 and none of them takes a vvvv
    // source, so VEX.256 and VEX.vvvv != 1111b are both reserved.

    // VPEXTRB eax, xmm1, 3 — C4 E3 79 14 C8 03.
    let b = fetch_decode64(&[0xC4, 0xE3, 0x79, 0x14, 0xC8, 0x03]).unwrap();
    assert_eq!(b.get_ia_opcode(), Opcode::V128VpextrbEdVdqIbR);
    assert_eq!(b.dst(), 1); // nnn = xmm source
    assert_eq!(b.src1(), 0); // rm = GPR destination

    // VPEXTRW [rax], xmm1, 2 — C4 E3 79 15 08 02 (memory destination).
    let w = fetch_decode64(&[0xC4, 0xE3, 0x79, 0x15, 0x08, 0x02]).unwrap();
    assert_eq!(w.get_ia_opcode(), Opcode::V128VpextrwMwVdqIbM);
    assert!(!w.mod_c0());

    // 0F3A 16 keeps the W split: W0 -> VPEXTRD, W1 -> VPEXTRQ.
    let d = fetch_decode64(&[0xC4, 0xE3, 0x79, 0x16, 0xC8, 0x01]).unwrap();
    assert_eq!(d.get_ia_opcode(), Opcode::V128VpextrdEdVdqIb);
    let q = fetch_decode64(&[0xC4, 0xE3, 0xF9, 0x16, 0xC8, 0x01]).unwrap();
    assert_eq!(q.get_ia_opcode(), Opcode::V128VpextrqEqVdqIb);

    // The four VPCMPxSTRx forms.
    let cases: &[(&[u8], Opcode)] = &[
        (
            &[0xC4, 0xE3, 0x79, 0x60, 0xCA, 0x00],
            Opcode::V128VpcmpestrmVdqWdqIb,
        ),
        (
            &[0xC4, 0xE3, 0x79, 0x61, 0xCA, 0x00],
            Opcode::V128VpcmpestriVdqWdqIb,
        ),
        (
            &[0xC4, 0xE3, 0x79, 0x62, 0xCA, 0x00],
            Opcode::V128VpcmpistrmVdqWdqIb,
        ),
        (
            &[0xC4, 0xE3, 0x79, 0x63, 0xCA, 0x00],
            Opcode::V128VpcmpistriVdqWdqIb,
        ),
    ];
    for (bytes, want) in cases {
        assert_eq!(
            fetch_decode64(bytes).unwrap().get_ia_opcode(),
            *want,
            "bytes {bytes:02X?}"
        );
    }

    // VEX.256 is reserved for all of them.
    assert!(fetch_decode64(&[0xC4, 0xE3, 0x7D, 0x14, 0xC8, 0x03]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE3, 0x7D, 0x15, 0xC8, 0x03]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE3, 0x7D, 0x16, 0xC8, 0x01]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE3, 0x7D, 0x63, 0xCA, 0x00]).is_err());
    // So is a non-1111b VEX.vvvv.
    assert!(fetch_decode64(&[0xC4, 0xE3, 0x71, 0x14, 0xC8, 0x03]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE3, 0x71, 0x63, 0xCA, 0x00]).is_err());

    // Legacy (non-VEX) encodings must still resolve to the SSE opcodes.
    assert_eq!(
        fetch_decode64(&[0x66, 0x0F, 0x3A, 0x14, 0xC8, 0x03])
            .unwrap()
            .get_ia_opcode(),
        Opcode::PextrbEdVdqIbR
    );
    assert_eq!(
        fetch_decode64(&[0x66, 0x0F, 0x3A, 0x63, 0xCA, 0x00])
            .unwrap()
            .get_ia_opcode(),
        Opcode::PcmpistriVdqWdqIb
    );
}

#[test]
fn test_vex_vmovntdqa_and_vpclmulqdq_decode() {
    // VMOVNTDQA — VL-split, memory-only. C4 E2 79 2A 00 / C4 E2 7D 2A 00.
    let v128 = fetch_decode64(&[0xC4, 0xE2, 0x79, 0x2A, 0x00]).unwrap();
    assert_eq!(v128.get_ia_opcode(), Opcode::V128VmovntdqaVdqMdq);
    let v256 = fetch_decode64(&[0xC4, 0xE2, 0x7D, 0x2A, 0x00]).unwrap();
    assert_eq!(v256.get_ia_opcode(), Opcode::V256VmovntdqaVdqMdq);
    // No vvvv source.
    assert!(fetch_decode64(&[0xC4, 0xE2, 0x71, 0x2A, 0x00]).is_err());
    // Legacy form still decodes to the SSE4.1 opcode.
    assert_eq!(
        fetch_decode64(&[0x66, 0x0F, 0x38, 0x2A, 0x00])
            .unwrap()
            .get_ia_opcode(),
        Opcode::MovntdqaVdqMdq
    );

    // VPCLMULQDQ — 3-operand under VEX, VL-split.
    let c128 = fetch_decode64(&[0xC4, 0xE3, 0x71, 0x44, 0xC2, 0x11]).unwrap();
    assert_eq!(c128.get_ia_opcode(), Opcode::V128VpclmulqdqVdqHdqWdqIb);
    assert_eq!(c128.dst(), 0);
    assert_eq!(c128.src2(), 1, "vvvv is the first source");
    assert_eq!(c128.src1(), 2, "ModRM.rm is the second source");
    assert_eq!(c128.ib(), 0x11);
    let c256 = fetch_decode64(&[0xC4, 0xE3, 0x75, 0x44, 0xC2, 0x11]).unwrap();
    assert_eq!(c256.get_ia_opcode(), Opcode::V256VpclmulqdqVdqHdqWdqIb);
    // Legacy PCLMULQDQ is unchanged.
    assert_eq!(
        fetch_decode64(&[0x66, 0x0F, 0x3A, 0x44, 0xC1, 0x11])
            .unwrap()
            .get_ia_opcode(),
        Opcode::PclmulqdqVdqWdqIb
    );
}

#[test]
fn opcode_isa_table_is_in_sync_with_the_opcode_enum() {
    use crate::features::X86Feature;
    use crate::opcode_isa::{
        opcode_isa_feature, GATED_OPCODE_COUNT, ISA_ALWAYS, OPCODE_ISA, OPCODE_VARIANT_COUNT,
    };

    // The table is indexed by `Opcode as usize`, so a variant added without
    // rerunning `scripts/gen_opcode_isa.py` would index out of bounds or, worse,
    // read a neighbour's feature. `from_u16_const` returns IaError past the end
    // of the enum, which pins the variant count exactly.
    assert_eq!(OPCODE_ISA.len(), OPCODE_VARIANT_COUNT);
    assert_ne!(
        Opcode::from_u16_const((OPCODE_VARIANT_COUNT - 1) as u16),
        Opcode::IaError,
        "last table slot must correspond to a real Opcode variant — regenerate \
         with scripts/gen_opcode_isa.py"
    );
    assert_eq!(
        Opcode::from_u16_const(OPCODE_VARIANT_COUNT as u16),
        Opcode::IaError,
        "Opcode gained variants past the table — regenerate with \
         scripts/gen_opcode_isa.py"
    );

    assert_eq!(
        OPCODE_ISA.iter().filter(|f| **f != ISA_ALWAYS).count(),
        GATED_OPCODE_COUNT
    );

    // Spot-check the mapping against Bochs ia_opcodes.def. IaError itself must
    // never be gated, or a #UD would recurse into another #UD.
    assert_eq!(opcode_isa_feature(Opcode::IaError), ISA_ALWAYS);
    assert_eq!(opcode_isa_feature(Opcode::Nop), ISA_ALWAYS);
    assert_eq!(
        opcode_isa_feature(Opcode::V256VpermdVdqHdqWdq),
        X86Feature::IsaAvx2 as u16
    );
    assert_eq!(
        opcode_isa_feature(Opcode::Vcvtph2psVpsWps),
        X86Feature::IsaAvxF16c as u16
    );
    assert_eq!(
        opcode_isa_feature(Opcode::V256VpclmulqdqVdqHdqWdqIb),
        X86Feature::IsaVaesVpclmulqdq as u16
    );
    assert_eq!(
        opcode_isa_feature(Opcode::V128VpclmulqdqVdqHdqWdqIb),
        X86Feature::IsaAvx as u16,
        "the 128-bit form is plain AVX; only the 256-bit form needs VPCLMULQDQ"
    );
}

#[test]
fn test_vex_legacy_shared_forms_enforce_bochs_encoding_limits() {
    // These VEX forms share a legacy SSE table entry, so nothing in the table
    // itself constrains them. Bochs constrains them in its separate VEX groups
    // (fetchdecode_opmap_avx.cc), and the results are already correct — what
    // was missing is the reserved-encoding #UD.

    // --- VL128-only (Bochs marks each group ATTR_VL128) ---
    // VMOVLPS [rax], xmm1 — C5 F8 13 08 decodes; the VEX.256 form must not.
    assert_eq!(
        fetch_decode64(&[0xC5, 0xF8, 0x13, 0x08])
            .unwrap()
            .get_ia_opcode(),
        Opcode::MovlpsMqVps
    );
    assert!(fetch_decode64(&[0xC5, 0xFC, 0x13, 0x08]).is_err());
    // VMOVHPD [rax], xmm1 — 66-prefixed sibling at 0F 17.
    assert!(fetch_decode64(&[0xC5, 0xF9, 0x17, 0x08]).is_ok());
    assert!(fetch_decode64(&[0xC5, 0xFD, 0x17, 0x08]).is_err());
    // VMOVD/VMOVQ both directions.
    assert!(fetch_decode64(&[0xC5, 0xF9, 0x6E, 0xC0]).is_ok());
    assert!(fetch_decode64(&[0xC5, 0xFD, 0x6E, 0xC0]).is_err());
    assert!(fetch_decode64(&[0xC5, 0xF9, 0x7E, 0xC0]).is_ok());
    assert!(fetch_decode64(&[0xC5, 0xFD, 0x7E, 0xC0]).is_err());
    // VPEXTRW eax, xmm0, 1 — 0F C5, register source only.
    assert!(fetch_decode64(&[0xC5, 0xF9, 0xC5, 0xC0, 0x01]).is_ok());
    assert!(fetch_decode64(&[0xC5, 0xFD, 0xC5, 0xC0, 0x01]).is_err());
    // VMASKMOVDQU xmm0, xmm1 — 0F F7, register source only.
    assert!(fetch_decode64(&[0xC5, 0xF9, 0xF7, 0xC1]).is_ok());
    assert!(fetch_decode64(&[0xC5, 0xFD, 0xF7, 0xC1]).is_err());

    // --- ModRM form constraints ---
    // VPEXTRW and VMASKMOVDQU are ATTR_MODC0: no memory operand.
    assert!(fetch_decode64(&[0xC5, 0xF9, 0xC5, 0x08, 0x01]).is_err());
    assert!(fetch_decode64(&[0xC5, 0xF9, 0xF7, 0x08]).is_err());
    // VLDMXCSR/VSTMXCSR are ATTR_MOD_MEM: no register operand.
    assert!(fetch_decode64(&[0xC5, 0xF8, 0xAE, 0x10]).is_ok()); // /2 mem
    assert!(fetch_decode64(&[0xC5, 0xF8, 0xAE, 0x18]).is_ok()); // /3 mem
    assert!(fetch_decode64(&[0xC5, 0xF8, 0xAE, 0xD0]).is_err()); // /2 reg
    assert!(fetch_decode64(&[0xC5, 0xFC, 0xAE, 0x10]).is_err()); // VEX.256

    // --- 0F AE has no VEX form other than V(LD|ST)MXCSR ---
    // A VEX-prefixed FXSAVE / XSAVE / CLFLUSH / fences must all #UD.
    for modrm in [0x00u8 /* fxsave /0 */, 0x08 /* fxrstor /1 */, 0x20 /* xsave /4 */,
                  0x28 /* xrstor /5 */, 0x38 /* clflush /7 */] {
        assert!(
            fetch_decode64(&[0xC5, 0xF8, 0xAE, modrm]).is_err(),
            "VEX 0F AE /{} must #UD", modrm >> 3
        );
    }
    assert!(fetch_decode64(&[0xC5, 0xF8, 0xAE, 0xE8]).is_err(), "VEX LFENCE");
    assert!(fetch_decode64(&[0xC5, 0xF8, 0xAE, 0xF0]).is_err(), "VEX MFENCE");
    assert!(fetch_decode64(&[0xC5, 0xF8, 0xAE, 0xF8]).is_err(), "VEX SFENCE");

    // --- reserved VEX.vvvv (none of these take a vvvv source) ---
    assert!(fetch_decode64(&[0xC5, 0xF1, 0x13, 0x08]).is_err()); // VMOVLPS
    assert!(fetch_decode64(&[0xC5, 0xF1, 0x6E, 0xC0]).is_err()); // VMOVD
    assert!(fetch_decode64(&[0xC5, 0xF1, 0xC5, 0xC0, 0x01]).is_err()); // VPEXTRW
    assert!(fetch_decode64(&[0xC5, 0xF0, 0xAE, 0x10]).is_err()); // VLDMXCSR
    // VUCOMISS / VCOMISS / VCVTTSS2SI take no vvvv either — but Bochs puts NO
    // VL attribute on those groups, so VEX.256 stays legal for them.
    assert!(fetch_decode64(&[0xC5, 0xF0, 0x2E, 0xC1]).is_err()); // vvvv reserved
    assert!(fetch_decode64(&[0xC5, 0xFC, 0x2E, 0xC1]).is_ok(), "VUCOMISS has no VL limit");

    // --- the legacy (non-VEX) encodings are untouched ---
    assert_eq!(
        fetch_decode64(&[0x0F, 0xAE, 0x10]).unwrap().get_ia_opcode(),
        Opcode::Ldmxcsr
    );
    assert!(fetch_decode64(&[0x0F, 0xAE, 0xF8]).is_ok(), "legacy SFENCE");
    assert!(fetch_decode64(&[0x0F, 0xAE, 0x00]).is_ok(), "legacy FXSAVE");
    assert!(fetch_decode64(&[0x66, 0x0F, 0xF7, 0xC1]).is_ok(), "legacy MASKMOVDQU");
}

#[test]
fn test_vex_fma3_decode_runtime_sequences() {
    // Captured from Ubuntu userspace boot: VEX.128.66.0F38.W1 A9 /r m64.
    let a9 = fetch_decode64(&[0xC4, 0xE2, 0xF9, 0xA9, 0x51, 0x18]).unwrap();
    assert_eq!(a9.ilen(), 6);
    assert_eq!(a9.get_ia_opcode(), Opcode::Vfmadd213sdVpdHsdWsd);
    assert_eq!(a9.dst(), 2);
    assert_eq!(a9.src2(), 0);
    assert!(!a9.mod_c0());
    assert_eq!(a9.get_vl(), 0);

    // Captured from Ubuntu userspace boot: VEX.128.66.0F38.W1 B9 /r m64.
    let b9 = fetch_decode64(&[0xC4, 0xE2, 0xF1, 0xB9, 0x05, 0xD5, 0x54, 0x04, 0x00]).unwrap();
    assert_eq!(b9.ilen(), 9);
    assert_eq!(b9.get_ia_opcode(), Opcode::Vfmadd231sdVpdHsdWsd);
    assert_eq!(b9.dst(), 0);
    assert_eq!(b9.src2(), 1);
    assert!(!b9.mod_c0());
    assert_eq!(b9.get_vl(), 0);
}

#[test]
fn test_vex_fma3_decode_full_opcode_surface() {
    let cases: &[(u8, Opcode, Opcode)] = &[
        (
            0x96,
            Opcode::Vfmaddsub132psVpsHpsWps,
            Opcode::Vfmaddsub132pdVpdHpdWpd,
        ),
        (
            0x97,
            Opcode::Vfmsubadd132psVpsHpsWps,
            Opcode::Vfmsubadd132pdVpdHpdWpd,
        ),
        (
            0x98,
            Opcode::Vfmadd132psVpsHpsWps,
            Opcode::Vfmadd132pdVpdHpdWpd,
        ),
        (
            0x99,
            Opcode::Vfmadd132ssVpsHssWss,
            Opcode::Vfmadd132sdVpdHsdWsd,
        ),
        (
            0x9A,
            Opcode::Vfmsub132psVpsHpsWps,
            Opcode::Vfmsub132pdVpdHpdWpd,
        ),
        (
            0x9B,
            Opcode::Vfmsub132ssVpsHssWss,
            Opcode::Vfmsub132sdVpdHsdWsd,
        ),
        (
            0x9C,
            Opcode::Vfnmadd132psVpsHpsWps,
            Opcode::Vfnmadd132pdVpdHpdWpd,
        ),
        (
            0x9D,
            Opcode::Vfnmadd132ssVpsHssWss,
            Opcode::Vfnmadd132sdVpdHsdWsd,
        ),
        (
            0x9E,
            Opcode::Vfnmsub132psVpsHpsWps,
            Opcode::Vfnmsub132pdVpdHpdWpd,
        ),
        (
            0x9F,
            Opcode::Vfnmsub132ssVpsHssWss,
            Opcode::Vfnmsub132sdVpdHsdWsd,
        ),
        (
            0xA6,
            Opcode::Vfmaddsub213psVpsHpsWps,
            Opcode::Vfmaddsub213pdVpdHpdWpd,
        ),
        (
            0xA7,
            Opcode::Vfmsubadd213psVpsHpsWps,
            Opcode::Vfmsubadd213pdVpdHpdWpd,
        ),
        (
            0xA8,
            Opcode::Vfmadd213psVpsHpsWps,
            Opcode::Vfmadd213pdVpdHpdWpd,
        ),
        (
            0xA9,
            Opcode::Vfmadd213ssVpsHssWss,
            Opcode::Vfmadd213sdVpdHsdWsd,
        ),
        (
            0xAA,
            Opcode::Vfmsub213psVpsHpsWps,
            Opcode::Vfmsub213pdVpdHpdWpd,
        ),
        (
            0xAB,
            Opcode::Vfmsub213ssVpsHssWss,
            Opcode::Vfmsub213sdVpdHsdWsd,
        ),
        (
            0xAC,
            Opcode::Vfnmadd213psVpsHpsWps,
            Opcode::Vfnmadd213pdVpdHpdWpd,
        ),
        (
            0xAD,
            Opcode::Vfnmadd213ssVpsHssWss,
            Opcode::Vfnmadd213sdVpdHsdWsd,
        ),
        (
            0xAE,
            Opcode::Vfnmsub213psVpsHpsWps,
            Opcode::Vfnmsub213pdVpdHpdWpd,
        ),
        (
            0xAF,
            Opcode::Vfnmsub213ssVpsHssWss,
            Opcode::Vfnmsub213sdVpdHsdWsd,
        ),
        (
            0xB6,
            Opcode::Vfmaddsub231psVpsHpsWps,
            Opcode::Vfmaddsub231pdVpdHpdWpd,
        ),
        (
            0xB7,
            Opcode::Vfmsubadd231psVpsHpsWps,
            Opcode::Vfmsubadd231pdVpdHpdWpd,
        ),
        (
            0xB8,
            Opcode::Vfmadd231psVpsHpsWps,
            Opcode::Vfmadd231pdVpdHpdWpd,
        ),
        (
            0xB9,
            Opcode::Vfmadd231ssVpsHssWss,
            Opcode::Vfmadd231sdVpdHsdWsd,
        ),
        (
            0xBA,
            Opcode::Vfmsub231psVpsHpsWps,
            Opcode::Vfmsub231pdVpdHpdWpd,
        ),
        (
            0xBB,
            Opcode::Vfmsub231ssVpsHssWss,
            Opcode::Vfmsub231sdVpdHsdWsd,
        ),
        (
            0xBC,
            Opcode::Vfnmadd231psVpsHpsWps,
            Opcode::Vfnmadd231pdVpdHpdWpd,
        ),
        (
            0xBD,
            Opcode::Vfnmadd231ssVpsHssWss,
            Opcode::Vfnmadd231sdVpdHsdWsd,
        ),
        (
            0xBE,
            Opcode::Vfnmsub231psVpsHpsWps,
            Opcode::Vfnmsub231pdVpdHpdWpd,
        ),
        (
            0xBF,
            Opcode::Vfnmsub231ssVpsHssWss,
            Opcode::Vfnmsub231sdVpdHsdWsd,
        ),
    ];

    for &(opcode, w0_opcode, w1_opcode) in cases {
        let w0 = fetch_decode64(&[0xC4, 0xE2, 0x69, opcode, 0xCB]).unwrap();
        assert_eq!(w0.get_ia_opcode(), w0_opcode, "W0 opcode {opcode:02X}");
        assert_eq!(w0.get_vl(), 0, "W0 opcode {opcode:02X}");

        let w1 = fetch_decode64(&[0xC4, 0xE2, 0xE9, opcode, 0xCB]).unwrap();
        assert_eq!(w1.get_ia_opcode(), w1_opcode, "W1 opcode {opcode:02X}");
        assert_eq!(w1.get_vl(), 0, "W1 opcode {opcode:02X}");
    }
}

#[test]
fn test_vex_vpmin_vpmax_family_decode() {
    let cases: &[(&[u8], Opcode, u8)] = &[
        (&[0xC5, 0xE9, 0xDA, 0xCB], Opcode::V128VpminubVdqHdqWdq, 0),
        (&[0xC5, 0xED, 0xDA, 0xCB], Opcode::V256VpminubVdqHdqWdq, 1),
        (&[0xC5, 0xE9, 0xDE, 0xCB], Opcode::V128VpmaxubVdqHdqWdq, 0),
        (&[0xC5, 0xED, 0xDE, 0xCB], Opcode::V256VpmaxubVdqHdqWdq, 1),
        (&[0xC5, 0xE9, 0xEA, 0xCB], Opcode::V128VpminswVdqHdqWdq, 0),
        (&[0xC5, 0xED, 0xEA, 0xCB], Opcode::V256VpminswVdqHdqWdq, 1),
        (&[0xC5, 0xE9, 0xEE, 0xCB], Opcode::V128VpmaxswVdqHdqWdq, 0),
        (&[0xC5, 0xED, 0xEE, 0xCB], Opcode::V256VpmaxswVdqHdqWdq, 1),
        (
            &[0xC4, 0xE2, 0x69, 0x38, 0xCB],
            Opcode::V128VpminsbVdqHdqWdq,
            0,
        ),
        (
            &[0xC4, 0xE2, 0x6D, 0x38, 0xCB],
            Opcode::V256VpminsbVdqHdqWdq,
            1,
        ),
        (
            &[0xC4, 0xE2, 0x69, 0x39, 0xCB],
            Opcode::V128VpminsdVdqHdqWdq,
            0,
        ),
        (
            &[0xC4, 0xE2, 0x6D, 0x39, 0xCB],
            Opcode::V256VpminsdVdqHdqWdq,
            1,
        ),
        (
            &[0xC4, 0xE2, 0x69, 0x3A, 0xCB],
            Opcode::V128VpminuwVdqHdqWdq,
            0,
        ),
        (
            &[0xC4, 0xE2, 0x6D, 0x3A, 0xCB],
            Opcode::V256VpminuwVdqHdqWdq,
            1,
        ),
        (
            &[0xC4, 0xE2, 0x69, 0x3B, 0xCB],
            Opcode::V128VpminudVdqHdqWdq,
            0,
        ),
        (
            &[0xC4, 0xE2, 0x6D, 0x3B, 0xCB],
            Opcode::V256VpminudVdqHdqWdq,
            1,
        ),
        (
            &[0xC4, 0xE2, 0x69, 0x3C, 0xCB],
            Opcode::V128VpmaxsbVdqHdqWdq,
            0,
        ),
        (
            &[0xC4, 0xE2, 0x6D, 0x3C, 0xCB],
            Opcode::V256VpmaxsbVdqHdqWdq,
            1,
        ),
        (
            &[0xC4, 0xE2, 0x69, 0x3D, 0xCB],
            Opcode::V128VpmaxsdVdqHdqWdq,
            0,
        ),
        (
            &[0xC4, 0xE2, 0x6D, 0x3D, 0xCB],
            Opcode::V256VpmaxsdVdqHdqWdq,
            1,
        ),
        (
            &[0xC4, 0xE2, 0x69, 0x3E, 0xCB],
            Opcode::V128VpmaxuwVdqHdqWdq,
            0,
        ),
        (
            &[0xC4, 0xE2, 0x6D, 0x3E, 0xCB],
            Opcode::V256VpmaxuwVdqHdqWdq,
            1,
        ),
        (
            &[0xC4, 0xE2, 0x69, 0x3F, 0xCB],
            Opcode::V128VpmaxudVdqHdqWdq,
            0,
        ),
        (
            &[0xC4, 0xE2, 0x6D, 0x3F, 0xCB],
            Opcode::V256VpmaxudVdqHdqWdq,
            1,
        ),
    ];

    for (bytes, expected_opcode, expected_vl) in cases {
        let instr = fetch_decode64(bytes).unwrap();
        assert_eq!(
            instr.get_ia_opcode(),
            *expected_opcode,
            "bytes {bytes:02X?}"
        );
        assert_eq!(instr.get_vl(), *expected_vl, "bytes {bytes:02X?}");
    }
}

#[test]
fn test_group_error_table_entry_is_illegal_opcode() {
    let result32 = fetch_decode32(&[0x0F, 0x3A, 0x00, 0xC0, 0x00], true);
    assert!(
        matches!(
            result32,
            Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode))
        ),
        "32-bit group-error table should produce BxIllegalOpcode, got {result32:?}"
    );

    let result64 = fetch_decode64(&[0x0F, 0x3A, 0x00, 0xC0, 0x00]);
    assert!(
        matches!(
            result64,
            Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode))
        ),
        "64-bit group-error table should produce BxIllegalOpcode, got {result64:?}"
    );
}

#[test]
fn test_lock_prefix_memory_allowed_register_rejected_32bit() {
    let allowed = fetch_decode32(&[0xF0, 0x01, 0x18], true).unwrap();
    assert_eq!(allowed.ilen(), 3);
    assert_eq!(allowed.get_ia_opcode(), Opcode::AddEdGd);
    assert!(allowed.get_lock());
    assert!(!allowed.mod_c0());

    let rejected = fetch_decode32(&[0xF0, 0x01, 0xD8], true);
    assert!(
        matches!(
            rejected,
            Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode))
        ),
        "LOCK register form should be rejected, got {rejected:?}"
    );
}

#[test]
fn test_lock_prefix_memory_allowed_register_rejected_64bit_exact_error() {
    let allowed = fetch_decode64(&[0xF0, 0x01, 0x18]).unwrap();
    assert_eq!(allowed.ilen(), 3);
    assert_eq!(allowed.get_ia_opcode(), Opcode::AddEdGd);
    assert!(allowed.get_lock());
    assert!(!allowed.mod_c0());

    let rejected = fetch_decode64(&[0xF0, 0x01, 0xD8]);
    assert!(
        matches!(
            rejected,
            Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode))
        ),
        "LOCK register form should be rejected, got {rejected:?}"
    );
}
// -- PUSH imm8 sign-extension (session 10 fix) --
// Opcode 0x6A (PUSH imm8): the byte immediate must be SIGN-extended.
// Bug: was zero-extended, so PUSH 0xFF pushed 255 instead of -1,
// breaking wait4(-1) in Linux init.

#[test]
fn test_push_imm8_sign_extension() {
    // 6A FF = PUSH -1 (sign-extended from 0xFF to 0xFFFFFFFF)
    let i = fetch_decode32(&[0x6A, 0xFF], true).unwrap();
    assert_eq!(i.ilen(), 2);
    assert_eq!(
        i.id() as i32,
        -1,
        "PUSH imm8 0xFF should sign-extend to 0xFFFFFFFF (-1)"
    );
}

#[test]
fn test_push_imm8_positive() {
    // 6A 7F = PUSH 127
    let i = fetch_decode32(&[0x6A, 0x7F], true).unwrap();
    assert_eq!(i.ilen(), 2);
    assert_eq!(i.id(), 0x7F, "PUSH imm8 0x7F should remain 127");
}

#[test]
fn test_push_imm8_0x80() {
    // 6A 80 = PUSH -128
    let i = fetch_decode32(&[0x6A, 0x80], true).unwrap();
    assert_eq!(i.ilen(), 2);
    assert_eq!(
        i.id() as i32,
        -128,
        "PUSH imm8 0x80 should sign-extend to 0xFFFFFF80 (-128)"
    );
}

// -- Group 3 TEST immediate (0xF6/0xF7 nnn=0) --
// TEST r/m8, imm8 (F6 /0 ib) and TEST r/m32, imm32 (F7 /0 id) include
// an immediate that the decoder must account for in instruction length.

#[test]
fn test_group3_test_byte_immediate_length() {
    // F6 C3 42 = TEST BL, 0x42
    // ModRM C3 = 11 000 011: nnn=0(TEST), rm=3(BL)
    let i = fetch_decode32(&[0xF6, 0xC3, 0x42], true).unwrap();
    assert_eq!(i.ilen(), 3, "TEST BL, imm8 should be 3 bytes");
    assert_eq!(i.ib(), 0x42);
}

#[test]
fn test_group3_test_dword_immediate_length() {
    // F7 C3 78 56 34 12 = TEST EBX, 0x12345678
    // ModRM C3 = 11 000 011: nnn=0(TEST), rm=3(EBX)
    let i = fetch_decode32(&[0xF7, 0xC3, 0x78, 0x56, 0x34, 0x12], true).unwrap();
    assert_eq!(i.ilen(), 6, "TEST EBX, imm32 should be 6 bytes");
    assert_eq!(i.id(), 0x12345678);
}

// -- REX byte register mapping (session 25 fix) --
// Bare REX (0x40, no R/X/B/W bits) must still enable SPL/BPL/SIL/DIL
// register mapping by setting the Extend8bit flag.
// Bug: decoder stored rex_prefix = b & 0x0F, which gave 0 for 0x40,
// so Extend8bit was never set.

#[test]
fn test_bare_rex_enables_extend8bit() {
    // 40 C6 C6 00 = MOV SIL, 0 (bare REX enables SIL instead of DH)
    // REX=0x40, C6=MOV r/m8,imm8, ModRM C6=11 000 110: nnn=0, rm=6
    // With Extend8bit set, rm=6 maps to SIL (not DH)
    let i = fetch_decode64(&[0x40, 0xC6, 0xC6, 0x00]).unwrap();
    assert_eq!(i.ilen(), 4);
    assert_ne!(
        i.extend8bit_l(),
        0,
        "Bare REX (0x40) must set Extend8bit flag"
    );
    assert_eq!(i.dst(), 6, "rm should be 6 (SIL with Extend8bit)");
}

#[test]
fn test_no_rex_no_extend8bit() {
    // C6 C6 00 = MOV DH, 0 (no REX — register 6 is DH)
    let i = fetch_decode64(&[0xC6, 0xC6, 0x00]).unwrap();
    assert_eq!(i.ilen(), 3);
    assert_eq!(
        i.extend8bit_l(),
        0,
        "Without REX, Extend8bit should NOT be set"
    );
    assert_eq!(i.dst(), 6, "rm should be 6 (DH without Extend8bit)");
}

// -- Ed,Gd convention: two-byte opcodes should NOT be affected by
//    single-byte (b1 & 0x0F) == 0x01/0x09 matching (session 28 fix) --

#[test]
fn test_cmovno_not_swapped() {
    // 0F 41 C1 = CMOVNO ECX, ECX (two-byte 0F 41)
    // ModRM C1 = 11 000 001: reg=0(EAX), rm=1(ECX)
    // This is NOT in Ed,Gd: dst=nnn=0, src1=rm=1 (default convention)
    let i = fetch_decode32(&[0x0F, 0x41, 0xC1], true).unwrap();
    assert_eq!(i.ilen(), 3);
    // CMOVcc uses Gd,Ed convention: dst=nnn, src1=rm
    assert_eq!(i.dst(), 0, "CMOVNO dst should be nnn=EAX(0)");
    assert_eq!(i.src1(), 1, "CMOVNO src1 should be rm=ECX(1)");
}

// -- LEAVE (0xC9) must be in no-ModRM list --

#[test]
fn test_leave_no_modrm_32bit() {
    // C9 = LEAVE
    let i = fetch_decode32(&[0xC9], true).unwrap();
    assert_eq!(i.ilen(), 1, "LEAVE should be 1 byte (no ModRM)");
}

#[test]
fn test_leave_no_modrm_64bit() {
    let i = fetch_decode64(&[0xC9]).unwrap();
    assert_eq!(i.ilen(), 1, "64-bit LEAVE should be 1 byte (no ModRM)");
}

// -- Short jump sign-extension (session fix) --
// Opcodes 0x70-0x7F, 0xEB, 0xE0-0xE3: byte immediates must be sign-extended.

#[test]
fn test_short_jump_negative_displacement() {
    // EB FE = JMP -2 (infinite loop)
    let i = fetch_decode32(&[0xEB, 0xFE], true).unwrap();
    assert_eq!(i.ilen(), 2);
    assert_eq!(i.id() as i32, -2, "JMP short 0xFE should sign-extend to -2");
}

#[test]
fn test_conditional_jump_negative_displacement() {
    // 75 F0 = JNZ -16
    let i = fetch_decode32(&[0x75, 0xF0], true).unwrap();
    assert_eq!(i.ilen(), 2);
    assert_eq!(
        i.id() as i32,
        -16,
        "JNZ short 0xF0 should sign-extend to -16"
    );
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
    assert_eq!(i.sib_base(), 16, "Base should be BX_64BIT_REG_RIP (16)");
    assert_eq!(i.displacement, 0x12345678);
}

#[test]
fn test_rip_relative_with_rex() {
    init_tracing();
    // MOV RAX, [RIP+0x10]: 48 8B 05 10 00 00 00
    let i = fetch_decode64(&[0x48, 0x8B, 0x05, 0x10, 0x00, 0x00, 0x00]).unwrap();
    assert_eq!(i.ilen(), 7);
    assert_eq!(i.sib_base(), 16, "Base should be BX_64BIT_REG_RIP (16)");
    assert_eq!(i.displacement, 0x10);
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
    assert_eq!(i.displacement, 0x12345678);
}

/// `EVEX.b` means embedded broadcast on a memory operand and SAE / embedded
/// rounding on a register one. An opcode that supports neither must raise #UD
/// rather than quietly ignore the bit — Bochs applies these two tests in
/// `fetchdecode32.cc` right after resolving the EVEX opcode, redirecting
/// `execute1` to `BxError`.
///
/// EVEX layout used below: `62 P0 P1 P2 opcode modrm`, with
/// `P0 = 0xF0 | mm` (no REX extensions), `P1 = W<<7 | ~vvvv<<3 | 4 | pp`,
/// `P2 = z<<7 | L'L<<5 | b<<4 | ~V'<<3 | aaa`.
#[test]
fn evex_b_is_rejected_where_the_opcode_allows_neither_sae_nor_broadcast() {
    // VUNPCKLPS zmm1, zmm0, zmm2 — 0F map, no prefix, W0. Flagged NO_SAE, so
    // EVEX.b on the register form is illegal.
    assert!(
        fetch_decode64(&[0x62, 0xF1, 0x7C, 0x58, 0x14, 0xCA]).is_err(),
        "EVEX.b (SAE) on a register-form VUNPCKLPS must #UD"
    );
    // Same encoding with b=0 stays legal.
    assert!(
        fetch_decode64(&[0x62, 0xF1, 0x7C, 0x48, 0x14, 0xCA]).is_ok(),
        "VUNPCKLPS without EVEX.b must still decode"
    );

    // VADDSS xmm1, xmm0, [rax] — F3 prefix, 0F map, W0. Flagged NO_BROADCAST,
    // so EVEX.b on the memory form is illegal.
    assert!(
        fetch_decode64(&[0x62, 0xF1, 0x7E, 0x18, 0x58, 0x08]).is_err(),
        "EVEX.b (broadcast) on a memory-form VADDSS must #UD"
    );
    assert!(
        fetch_decode64(&[0x62, 0xF1, 0x7E, 0x08, 0x58, 0x08]).is_ok(),
        "VADDSS without EVEX.b must still decode"
    );

    // VADDPS carries plain BX_PREPARE_EVEX: SAE on the register form is
    // exactly what EVEX.b is for there, so it must be accepted.
    assert!(
        fetch_decode64(&[0x62, 0xF1, 0x7C, 0x58, 0x58, 0xCA]).is_ok(),
        "EVEX.b (SAE) on a register-form VADDPS is legal and must decode"
    );
}

#[test]
fn evex_vvvv_lands_in_src2_and_modrm_rm_in_src1() {
    // VADDPS zmm1, zmm2, zmm3  =  62 F1 6C 48 58 CB
    //   EVEX.vvvv encodes zmm2 (the first source), ModRM.rm encodes zmm3.
    // Bochs names those i->src1() and i->src2() respectively; this decoder
    // puts vvvv in src2() and rm in src1(), i.e. the two are swapped
    // relative to upstream. Handlers must account for that.
    let i = crate::decoder::decode64::fetch_decode64(&[0x62, 0xF1, 0x6C, 0x48, 0x58, 0xCB]).unwrap();
    assert_eq!(i.dst(), 1, "ModRM.reg is the destination");
    assert_eq!(i.src2(), 2, "EVEX.vvvv");
    assert_eq!(i.src1(), 3, "ModRM.rm");
}

#[test]
#[ignore = "EVEX opcode maps are incomplete: only 268 of 1333 EVEX opcodes have \
            a decoder table entry. This is the exact repro that blocks the \
            Skylake-X AVX-512 CPUID flip — see docs/evex-decoder-map-gap.md."]
fn evex_vpbroadcastb_from_gpr_decodes() {
    // 62 E2 7D 28 7A C6 = VPBROADCASTB ymm0, esi (EVEX.256.66.0F38.W0 7A /r).
    // glibc's AVX-512 strlen/memchr IFUNC emits exactly this. With AVX-512
    // advertised, Ubuntu's init took #UD on it and the kernel panicked with
    // "Attempted to kill init!". The handler exists and is dispatched; the
    // opcode map slot that would produce it does not.
    let i = crate::decoder::decode64::fetch_decode64(&[0x62, 0xE2, 0x7D, 0x28, 0x7A, 0xC6])
        .expect("EVEX VPBROADCASTB Vdq, Eb must decode");
    assert_ne!(
        i.get_ia_opcode(),
        crate::opcode::Opcode::IaError,
        "EVEX.66.0F38.W0 7A must not be an empty opcode-map slot"
    );
}
