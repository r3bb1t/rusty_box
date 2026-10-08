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
// Operand directions and encodings pinned against Bochs
// =============================================================================

// -- SHRD/SHLD Ed,Gd direction --
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

// -- MOVQ 66 0F D6 Ed,Gd direction --
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

// -- F3 0F 7E MOVQ Vq,Wq exclusion from Ed,Gd --
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

/// A VEX map-7 instruction: `C4 E7 <W vvvv L pp> <opcode> <ModRM> imm32`,
/// with R/X/B clear (no extension), so `pp` selects the form.
fn vex_map7(vex2: u8, opcode: u8, modrm: u8, imm: u32) -> std::vec::Vec<u8> {
    let mut bytes = std::vec![0xC4, 0xE7, vex2, opcode, modrm];
    bytes.extend_from_slice(&imm.to_le_bytes());
    bytes
}

/// VEX byte 2 with W0, vvvv unused (1111), L0, and the given `pp`.
const VEX2_F3: u8 = 0x7A;
const VEX2_F2: u8 = 0x7B;

/// The register forms of URDMSR and UWRMSR, `F2/F3 0F 38 F8 /r` with a
/// register operand (Bochs fetchdecode_opmap_0f38.cc `BxOpcodeTable0F38F8`).
/// URDMSR writes ModRM.rm with the MSR named by ModRM.reg (`Eq, Gq`); UWRMSR
/// reads the MSR index from ModRM.reg and the value from ModRM.rm (`Gq, Eq`).
#[test]
fn urdmsr_and_uwrmsr_register_forms_decode_in_64_bit_mode() {
    // ModRM 0xC1: mod 11, reg = rax, rm = rcx.
    let read = fetch_decode64(&[0xF2, 0x0F, 0x38, 0xF8, 0xC1]).unwrap();
    assert_eq!(read.get_ia_opcode(), Opcode::UrdmsrEqGq);
    assert_eq!((read.dst(), read.src1()), (1, 0), "rcx <- MSR[rax]");

    let write = fetch_decode64(&[0xF3, 0x0F, 0x38, 0xF8, 0xC1]).unwrap();
    assert_eq!(write.get_ia_opcode(), Opcode::UwrmsrGqEq);
    assert_eq!((write.dst(), write.src1()), (0, 1), "MSR[rax] <- rcx");

    // A memory operand is MOVDIR64B's encoding space only with 66; with F2
    // or F3 it is no instruction.
    assert!(fetch_decode64(&[0xF2, 0x0F, 0x38, 0xF8, 0x01]).is_err());
    // 64-bit mode only.
    assert!(fetch_decode32(&[0xF2, 0x0F, 0x38, 0xF8, 0xC1], true).is_err());
}

/// What a decoded SSE4A immediate form names.
#[derive(Debug, PartialEq, Eq)]
struct Sse4aImmediateForm {
    opcode: Opcode,
    dst: u8,
    src1: u8,
    length: u8,
    position: u8,
    instruction_length: u8,
}

fn sse4a_immediate_form(instr: &Instruction) -> Sse4aImmediateForm {
    Sse4aImmediateForm {
        opcode: instr.get_ia_opcode(),
        dst: instr.dst(),
        src1: instr.src1(),
        length: instr.ib(),
        position: instr.ib2(),
        instruction_length: instr.ilen(),
    }
}

/// SSE4A's immediate forms carry two immediates, the field length and then
/// its position (Bochs ia_opcodes.def `EXTRQ_UdqIbIb` and
/// `INSERTQ_VdqUqIbIb`, `OP_Ib, OP_Ib2`). Both are fetched, so each is six
/// bytes long, and the second lands where `ib2` reads it. EXTRQ's one
/// register is ModRM.rm; INSERTQ writes ModRM.reg from ModRM.rm.
#[test]
fn sse4a_immediate_forms_fetch_both_immediates() {
    // ModRM 0xC1: mod 11, reg = xmm0, rm = xmm1; length 4, position 8.
    let extrq = [0x66, 0x0F, 0x78, 0xC1, 0x04, 0x08];
    let insertq = [0xF2, 0x0F, 0x78, 0xC1, 0x04, 0x08];
    for decoded in [fetch_decode64(&extrq).unwrap(), fetch_decode32(&extrq, true).unwrap()] {
        assert_eq!(
            sse4a_immediate_form(&decoded),
            Sse4aImmediateForm {
                opcode: Opcode::ExtrqUdqIbIb,
                dst: 1,
                src1: decoded.src1(),
                length: 4,
                position: 8,
                instruction_length: 6,
            }
        );
    }
    for decoded in [fetch_decode64(&insertq).unwrap(), fetch_decode32(&insertq, true).unwrap()] {
        assert_eq!(
            sse4a_immediate_form(&decoded),
            Sse4aImmediateForm {
                opcode: Opcode::InsertqVdqUqIbIb,
                dst: 0,
                src1: 1,
                length: 4,
                position: 8,
                instruction_length: 6,
            }
        );
    }
    // A memory operand is no SSE4A form, and fetches no immediate for one.
    assert!(fetch_decode64(&[0x66, 0x0F, 0x78, 0x01]).is_err());
}

/// VEX map 7 holds the immediate-index forms of RDMSR, WRMSRNS, URDMSR and
/// UWRMSR (Bochs fetchdecode_opmap_avx.cc `BxOpcodeGroup_VEX_MAP7_F6` and
/// `_F8`). Each takes a 32-bit immediate and one register operand in
/// ModRM.rm — the destination of a read, the source of a write.
#[test]
fn vex_map_7_decodes_the_msr_immediate_forms() {
    // ModRM 0xC2: mod 11, reg 0, rm = rdx.
    for (vex2, opcode, expected, reads) in [
        (VEX2_F2, 0xF6, Opcode::RdmsrEqId, true),
        (VEX2_F3, 0xF6, Opcode::WrmsrnsIdEq, false),
        (VEX2_F2, 0xF8, Opcode::UrdmsrEqId, true),
        (VEX2_F3, 0xF8, Opcode::UwrmsrIdEq, false),
    ] {
        let bytes = vex_map7(vex2, opcode, 0xC2, 0x0000_001C);
        let i = fetch_decode64(&bytes).unwrap();
        assert_eq!(i.get_ia_opcode(), expected);
        assert_eq!(i.ilen() as usize, bytes.len(), "{expected:?} consumes its imm32");
        assert_eq!(i.id(), 0x1C, "{expected:?} index");
        let register = if reads { i.dst() } else { i.src1() };
        assert_eq!(register, 2, "{expected:?} names rdx");
    }
}

/// What Bochs's map-7 groups do not accept is #UD: a memory operand, a
/// nonzero ModRM.reg, VEX.L or VEX.W set, `vvvv` naming a register, an
/// opcode byte with no group, and anything outside 64-bit mode.
#[test]
fn vex_map_7_rejects_what_bochs_rejects() {
    let imm = 0x1C;
    assert!(fetch_decode64(&vex_map7(VEX2_F2, 0xF8, 0x02, imm)).is_err(), "memory form");
    assert!(fetch_decode64(&vex_map7(VEX2_F2, 0xF8, 0xCA, imm)).is_err(), "reg != 0");
    assert!(fetch_decode64(&vex_map7(VEX2_F2 | 0x04, 0xF8, 0xC2, imm)).is_err(), "VEX.L");
    assert!(fetch_decode64(&vex_map7(VEX2_F2 | 0x80, 0xF8, 0xC2, imm)).is_err(), "VEX.W");
    assert!(fetch_decode64(&vex_map7(VEX2_F2 & !0x08, 0xF8, 0xC2, imm)).is_err(), "vvvv");
    assert!(fetch_decode64(&vex_map7(VEX2_F2, 0xF7, 0xC2, imm)).is_err(), "no group");
    assert!(fetch_decode32(&vex_map7(VEX2_F2, 0xF8, 0xC2, imm), true).is_err(), "32-bit");
}

/// `X86Feature` lists Bochs's ISA features, every one and in Bochs's order,
/// so the generated ISA table's numbers mean what Bochs's do. The reference
/// is the list `scripts/gen_opcode_isa.py` copied out of Bochs
/// `cpu/decoder/features.h` (which itself refuses to generate on a
/// mismatch); this catches `features.rs` edited afterwards. Names compare
/// with the underscores dropped and case folded, as the script matches them.
#[test]
fn x86_feature_lists_the_bochs_features_in_order() {
    fn normalized(name: &str) -> std::string::String {
        name.replace('_', "").to_lowercase()
    }
    let bochs: Vec<_> = crate::opcode_isa::BOCHS_ISA_FEATURES
        .iter()
        .map(|name| normalized(name))
        .collect();
    let ours: Vec<_> = include_str!("features.rs")
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("Isa"))
        .filter_map(|rest| rest.strip_suffix(','))
        .map(normalized)
        .collect();
    assert_eq!(ours, bochs);
}

/// Every `X86Feature` has a bit in the `[u32; BX_ISA_EXTENSIONS_ARRAY_SIZE]`
/// ISA-extension bitmask, which `enable_extension` and
/// `bx_cpuid_support_isa_extension` index as `[feature / 32]`. Bochs
/// `cpu/decoder/decoder.h` refuses to build when `BX_ISA_EXTENSION_LAST >=
/// BX_ISA_EXTENSIONS_ARRAY_SIZE*32`; this is that check, over the feature list
/// `scripts/gen_opcode_isa.py` copied out of `features.h`. The order test above
/// reads `features.rs` as text, so the compiled enum is tied to the list here:
/// its last discriminant is the list's last index.
#[test]
fn every_x86_feature_fits_the_isa_extensions_bitmask() {
    use crate::features::X86Feature;
    use crate::opcode_isa::BOCHS_ISA_FEATURES;
    use crate::BX_ISA_EXTENSIONS_ARRAY_SIZE;

    assert_eq!(
        X86Feature::IsaUserMsr as usize + 1,
        BOCHS_ISA_FEATURES.len(),
        "X86Feature's last variant must number the last Bochs feature"
    );
    assert!(
        BOCHS_ISA_FEATURES.len() < BX_ISA_EXTENSIONS_ARRAY_SIZE * 32,
        "ISA extensions array limit exceeded! (Bochs cpu/decoder/decoder.h): \
         {} features, {} bitmask words",
        BOCHS_ISA_FEATURES.len(),
        BX_ISA_EXTENSIONS_ARRAY_SIZE
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
        X86Feature::IsaAesPclmulqdq as u16,
        "the 128-bit form needs AES+PCLMULQDQ (its AVX state is BX_PREPARE_AVX's); \
         only the 256-bit form needs VPCLMULQDQ"
    );
}

#[test]
fn opcode_prepare_table_is_in_sync_with_the_opcode_enum() {
    use crate::opcode_isa::{
        opcode_state, CpuState, OPCODE_STATE, OPCODE_VARIANT_COUNT, STATE_AVX_OPCODE_COUNT,
        STATE_EVEX_OPCODE_COUNT,
    };

    assert_eq!(OPCODE_STATE.len(), OPCODE_VARIANT_COUNT);
    assert_eq!(
        OPCODE_STATE.iter().filter(|c| **c == CpuState::Avx).count(),
        STATE_AVX_OPCODE_COUNT
    );
    assert_eq!(
        OPCODE_STATE.iter().filter(|c| **c == CpuState::Evex).count(),
        STATE_EVEX_OPCODE_COUNT
    );

    // Spot-checks against Bochs ia_opcodes.def field 10.
    assert_eq!(opcode_state(Opcode::VtestpsVpsWps), CpuState::Avx);
    assert_eq!(opcode_state(Opcode::V256VpaddbVdqHdqWdq), CpuState::Avx);
    assert_eq!(opcode_state(Opcode::PaddbVdqWdq), CpuState::Sse);

    // VEX encoding does NOT imply AVX state: the BMI instructions are
    // VEX-encoded but operate on GPRs, and Bochs gives them
    // BX_PROTECTED_MODE_ONLY / 0 rather than BX_PREPARE_AVX. Gating them on
    // XCR0 would #UD them on a CPU that never enables AVX.
    assert_eq!(opcode_state(Opcode::AndnGdBdEd), CpuState::Base);
    assert_eq!(opcode_state(Opcode::TzcntGdEd), CpuState::Base);

    // The fault sentinels must never be gated, or resolving one would
    // substitute another and loop.
    assert_eq!(opcode_state(Opcode::NoAvxState), CpuState::Base);
    assert_eq!(opcode_state(Opcode::NoEvexState), CpuState::Base);
    assert_eq!(opcode_state(Opcode::IaError), CpuState::Base);

    // Nothing that is actually a vector encoding may be left ungated: the
    // icache state gate would wave it through for a guest that never enabled
    // AVX. The generator enforces this too, but hand-edits bypass the
    // generator and this catches them.
    for index in 0..OPCODE_VARIANT_COUNT {
        let opcode = Opcode::from_u16_const(index as u16);
        let name = std::format!("{opcode:?}");
        let vector = name.starts_with("Evex")
            || name.starts_with("V128")
            || name.starts_with("V256")
            || name.starts_with("V512");
        if vector {
            assert_ne!(
                opcode_state(opcode),
                CpuState::Base,
                "{name} is a vector encoding but requires no CPU state — add it \
                 to STATE_OVERRIDES in scripts/gen_opcode_isa.py and regenerate"
            );
        }
    }
}

/// The opcode counts `scripts/gen_opcode_isa.py` writes into opcode_isa.rs
/// (`GATED_OPCODE_COUNT`, `EVEX_FLAGGED_OPCODE_COUNT`, `STATE_AVX_OPCODE_COUNT`,
/// `STATE_EVEX_OPCODE_COUNT`), pinned to what Bochs's ia_opcodes.def,
/// ia_opcodes_evex.def and fetchdecode.h give against this enum. A
/// regeneration that moves one must move it here too. `OPCODE_VARIANT_COUNT`
/// is held to the enum by `opcode_isa_table_is_in_sync_with_the_opcode_enum`.
#[test]
fn opcode_isa_counts_are_pinned_to_the_reference_build() {
    use crate::opcode_isa::{
        EVEX_FLAGGED_OPCODE_COUNT, GATED_OPCODE_COUNT, OPCODE_EVEX_FLAGS,
        STATE_AVX_OPCODE_COUNT, STATE_EVEX_OPCODE_COUNT,
    };

    assert_eq!(
        OPCODE_EVEX_FLAGS.iter().filter(|f| **f != 0).count(),
        EVEX_FLAGGED_OPCODE_COUNT,
        "EVEX_FLAGGED_OPCODE_COUNT must count the table it describes"
    );
    assert_eq!(
        GATED_OPCODE_COUNT, 2964,
        "the reference build's ia_opcodes*.def gates 2964 of this enum's opcodes \
         on a BX_ISA_* feature; regenerate with scripts/gen_opcode_isa.py if \
         upstream changed"
    );
    assert_eq!(
        EVEX_FLAGGED_OPCODE_COUNT, 1437,
        "the reference build's ia_opcodes*.def gives 1437 of this enum's opcodes \
         a BX_PREPARE_EVEX* bit (BX_PREPARE_OPMASK included); regenerate with \
         scripts/gen_opcode_isa.py if upstream changed"
    );
    assert_eq!(
        STATE_AVX_OPCODE_COUNT, 676,
        "the reference build's ia_opcodes*.def marks 676 of this enum's opcodes \
         BX_PREPARE_AVX; regenerate with scripts/gen_opcode_isa.py if upstream \
         changed"
    );
    assert_eq!(
        STATE_EVEX_OPCODE_COUNT, 1406,
        "the reference build's ia_opcodes*.def marks 1406 of this enum's opcodes \
         BX_PREPARE_EVEX* without BX_PREPARE_AMX; regenerate with \
         scripts/gen_opcode_isa.py if upstream changed"
    );
}

#[test]
fn test_vex_legacy_shared_forms_enforce_bochs_encoding_limits() {
    // These VEX forms share a legacy SSE table entry, so nothing in the table
    // itself constrains them. Bochs constrains them in its separate VEX groups
    // (fetchdecode_opmap_avx.cc). The decoded results are the same; what this
    // checks is the reserved-encoding #UD.

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
// -- PUSH imm8 sign-extension --
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

// -- REX byte register mapping --
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
//    single-byte (b1 & 0x0F) == 0x01/0x09 matching --

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
fn evex_vpbroadcastb_from_gpr_decodes() {
    // 62 E2 7D 28 7A C6 = VPBROADCASTB ymm0, esi (EVEX.256.66.0F38.W0 7A /r).
    // glibc's AVX-512 strlen/memchr IFUNC emits exactly this. When the EVEX
    // maps were hand-written this slot was empty, so Ubuntu's init took #UD
    // and the kernel panicked with "Attempted to kill init!".
    let i = crate::decoder::decode64::fetch_decode64(&[0x62, 0xE2, 0x7D, 0x28, 0x7A, 0xC6])
        .expect("EVEX VPBROADCASTB Vdq, Eb must decode");
    assert_eq!(
        i.get_ia_opcode(),
        crate::opcode::Opcode::EvexVpbroadcastbVdqEb,
        "EVEX.66.0F38.W0 7A must resolve to VPBROADCASTB from a GPR"
    );
}

// ════════════════════════════════════════════════════════════════════════
// EVEX opcode-map coverage.
//
// The maps are generated from Bochs's own tables by
// scripts/gen_opmap_evex.py. These tests pin the result so a regeneration
// that silently drops entries fails loudly — the previous hand-written
// maps covered 268 of 1333 opcodes and nothing caught it until a guest
// kernel panicked.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn evex_master_table_has_every_slot_bochs_defines() {
    use crate::decoder::opmap_evex::EVEX_TABLE;
    let defined = EVEX_TABLE.iter().filter(|g| !g.is_empty()).count();
    assert_eq!(
        defined, 396,
        "BxOpcodeTableEVEX defines 396 non-ERR slots in the reference build \
         (BX_SUPPORT_AMX 0); regenerate with scripts/gen_opmap_evex.py if \
         upstream changed"
    );
    assert_eq!(EVEX_TABLE.len(), 256 * 5, "Bochs BxOpcodeTableEVEX[256*5]");
}

/// The reference Bochs build has BX_SUPPORT_AMX 0, so the EVEX tile-row
/// encodings sit in `BxOpcodeGroup_ERR` slots and decode to #UD.
#[test]
fn evex_amx_encodings_are_undefined() {
    // TCVTROWD2PS zmm0, tmm1, ecx: EVEX.512.F3.0F38.W0 4A /r, mod = 11.
    let decoded =
        crate::decoder::decode64::fetch_decode64(&[0x62, 0xF2, 0x7E, 0x48, 0x4A, 0xC1]);
    assert!(
        matches!(
            decoded,
            Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode))
        ),
        "expected an illegal opcode, got {decoded:?}"
    );
}

#[test]
fn evex_encodings_glibc_emits_all_decode() {
    use crate::opcode::Opcode;
    // Real encodings taken from glibc's AVX-512 string/memory IFUNCs — the
    // family that panicked Ubuntu's init when these slots were empty.
    let cases: &[(&[u8], Opcode)] = &[
        // VPBROADCASTB ymm0, esi          EVEX.256.66.0F38.W0 7A
        (&[0x62, 0xE2, 0x7D, 0x28, 0x7A, 0xC6], Opcode::EvexVpbroadcastbVdqEb),
        // VPCMPEQB k0, ymm0, [rdi]        EVEX.256.66.0F.W0 74
        (&[0x62, 0xF1, 0x7D, 0x28, 0x74, 0x07], Opcode::EvexVpcmpeqbKgqHdqWdq),
        // VPMINUB ymm1, ymm0, ymm2        EVEX.256.66.0F.W0 DA
        (&[0x62, 0xF1, 0x7D, 0x28, 0xDA, 0xCA], Opcode::EvexVpminubVdqHdqWdq),
        // VMOVDQU64 zmm0, [rdi]           EVEX.512.F3.0F.W1 6F
        (&[0x62, 0xF1, 0xFE, 0x48, 0x6F, 0x07], Opcode::EvexVmovdqu64VdqWdq),
        // VPTESTMB k1, ymm0, ymm1         EVEX.256.66.0F38.W0 26
        (&[0x62, 0xF2, 0x7D, 0x28, 0x26, 0xC9], Opcode::EvexVptestmbKgqHdqWdq),
    ];
    for (bytes, want) in cases {
        let i = crate::decoder::decode64::fetch_decode64(bytes)
            .unwrap_or_else(|e| panic!("{bytes:02X?} must decode, got {e:?}"));
        assert_eq!(i.get_ia_opcode(), *want, "for encoding {bytes:02X?}");
    }
}

#[test]
fn evex_disp8_is_scaled_by_the_tuple_size() {
    use crate::opcode::Opcode;
    // 62 E1 FD 28 6F 4E 01 = VMOVDQA64 ymm1, [rsi+0x20]
    // EVEX.256.66.0F.W1 6F, mod=01 disp8=1, full-vector tuple at VL256 so
    // N=32. This is the encoding glibc emits right after `and rsi,-32`; left
    // unscaled it addresses rsi+1 and the aligned move takes a spurious #GP.
    let i = crate::decoder::decode64::fetch_decode64(&[0x62, 0xE1, 0xFD, 0x28, 0x6F, 0x4E, 0x01])
        .expect("VMOVDQA64 with disp8 must decode");
    assert_eq!(i.get_ia_opcode(), Opcode::EvexVmovdqa64VdqWdq);
    assert_eq!(
        i.displacement, 32,
        "disp8=1 with a full-vector tuple at VL256 scales to 32, not 1"
    );

    // Same opcode at VL512 (L'L=10) scales by 64 instead.
    let i = crate::decoder::decode64::fetch_decode64(&[0x62, 0xE1, 0xFD, 0x48, 0x6F, 0x4E, 0x01])
        .expect("VMOVDQA64 zmm form must decode");
    assert_eq!(i.displacement, 64, "full-vector tuple at VL512 scales by 64");

    // A scalar-qword tuple ignores the vector length: VMOVSD zmm/xmm form,
    // EVEX.LIG.F2.0F.W1 10 with mod=01 -> N=8.
    let i = crate::decoder::decode64::fetch_decode64(&[0x62, 0xF1, 0xFF, 0x48, 0x10, 0x4E, 0x03])
        .expect("VMOVSD load must decode");
    assert_eq!(i.displacement, 24, "scalar qword scales by 8, so disp8=3 is 24");

    // Legacy and VEX encodings must be untouched: MOVDQA xmm1, [rsi+1].
    let i = crate::decoder::decode64::fetch_decode64(&[0x66, 0x0F, 0x6F, 0x4E, 0x01])
        .expect("legacy MOVDQA must decode");
    assert_eq!(i.displacement, 1, "non-EVEX displacements are never scaled");
}

#[test]
fn vvvv_destination_opcodes_reach_vvvv() {
    // Upstream declares 72 opcodes whose first operand (the destination) is
    // BX_SRC_VVV: the shift/rotate-by-immediate groups 0F 71/72/73 in both
    // their VEX and EVEX forms, and the BMI1/TBM group at 0F38 F3 and
    // 0F38 01/02.
    //
    // rusty reaches vvvv through two different accessors, so both have to
    // hold. `src2` is unconditionally vvvv for any VEX/EVEX encoding, and the
    // BMI handlers write their result there (bmi32.rs blsr_bd_ed uses
    // `instr.src2()`). The shift handlers instead write `instr.dst()`
    // (avx512.rs evex_vprord_imm), so for those the decoder must also place
    // vvvv in dst — which is what the 0F 71/72/73 branch in decode64 does.
    //
    // Encodings give nnn, rm and vvvv distinct values so a wrong pick shows.
    let cases: &[(&str, &[u8], u8, u8, bool)] = &[
        // name, bytes, vvvv, rm, dst-must-also-be-vvvv
        // BLSR eax, ebx = VEX.NDD.LZ.0F38.W0 F3 /1 — handler reads src2
        ("BLSR", &[0xC4, 0xE2, 0x78, 0xF3, 0xCB], 0, 3, false),
        ("BLSMSK", &[0xC4, 0xE2, 0x78, 0xF3, 0xD3], 0, 3, false),
        ("BLSI", &[0xC4, 0xE2, 0x78, 0xF3, 0xDB], 0, 3, false),
        // VPSRLD xmm1, xmm2, 8 = VEX.NDD.128.66.0F.W0 72 /2 — handler reads dst
        ("VEX VPSRLD", &[0xC5, 0xF1, 0x72, 0xD2, 0x08], 1, 2, true),
        // VPRORD zmm1, zmm2, 8 = EVEX.512.66.0F.W0 72 /0 — handler reads dst
        ("EVEX VPRORD", &[0x62, 0xF1, 0x75, 0x48, 0x72, 0xC2, 0x08], 1, 2, true),
    ];
    for (name, bytes, vvvv, rm, dst_is_vvvv) in cases {
        let i = crate::decoder::decode64::fetch_decode64(bytes)
            .unwrap_or_else(|e| panic!("{name} must decode: {e:?}"));
        assert_eq!(
            i.operands.src2, *vvvv,
            "{name}: src2 must carry VEX.vvvv for every VEX/EVEX encoding"
        );
        if *dst_is_vvvv {
            assert_eq!(
                i.operands.dst, *vvvv,
                "{name}: this handler reads dst(), so dst must be VEX.vvvv"
            );
            assert_eq!(
                i.operands.src1, *rm,
                "{name}: the shifted value comes from the rm field"
            );
        }
    }
}

#[test]
fn vl512_entries_are_reachable() {
    use crate::opcode::Opcode;
    // The decmask vector-length field is a thermometer code (0 / 1 / 3), not
    // the raw L'L bits (0 / 1 / 2). Feeding the raw value made every table
    // entry carrying ATTR_VL512 or ATTR_VL256_512 unreachable at 512-bit, so
    // a large family of instructions decoded as #UD only at zmm width.
    let cases: &[(&str, &[u8], Opcode)] = &[
        // VEXTRACTF32X4 xmm2, zmm1, 1 — ATTR_VL256_512
        (
            "VEXTRACTF32X4 512",
            &[0x62, 0xF3, 0x7D, 0x48, 0x19, 0xCA, 0x01],
            Opcode::EvexVextractf32x4WpsVpsIb,
        ),
        // Same opcode at 256-bit must keep working.
        (
            "VEXTRACTF32X4 256",
            &[0x62, 0xF3, 0x7D, 0x28, 0x19, 0xCA, 0x01],
            Opcode::EvexVextractf32x4WpsVpsIb,
        ),
        // VINSERTF32X8 zmm0, zmm1, ymm2, 0 — ATTR_VL512, 512-bit only
        (
            "VINSERTF32X8",
            &[0x62, 0xF3, 0x75, 0x48, 0x1A, 0xC2, 0x00],
            Opcode::EvexVinsertf32x8VpsHpsWpsIb,
        ),
    ];
    for (name, bytes, want) in cases {
        let i = crate::decoder::decode64::fetch_decode64(bytes)
            .unwrap_or_else(|e| panic!("{name} must decode, got {e:?}"));
        assert_eq!(i.get_ia_opcode(), *want, "for {name}");
    }
}

// ════════════════════════════════════════════════════════════════════════
// Decode round-trip over every EVEX opcode-map entry.
//
// The maps say, per entry, which encoding selects it: SSE prefix, EVEX.W,
// vector length, whether an opmask is required, register or memory form,
// and the ModRM reg field for the /digit groups. decode64 independently
// builds a decmask from a real instruction and matches it against those
// attributes. Two pieces of code describing one encoding, so they can be
// checked against each other — synthesise the encoding each entry demands
// and decode it back.
//
// This is the check that would have caught the vector-length defect by
// itself: 70 entries carrying ATTR_VL512 / ATTR_VL256_512 decoded at xmm and
// ymm width but #UD'd at zmm, because the decmask field is a thermometer
// code (0/1/3) while the raw L'L bits (0/1/2) were being shifted in.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn every_evex_map_entry_decodes() {
    use crate::decoder::opmap_evex::EVEX_TABLE;
    use crate::decoder::tables::{
        MASK_K0_OFFSET, MODC0_OFFSET, NNN_OFFSET, SSE_PREFIX_OFFSET, VEX_VL_128_256_OFFSET,
        VEX_W_OFFSET,
    };
    use crate::decoder::OpcodeTableEntry;
    use crate::opcode::Opcode;

    let mut checked = 0usize;
    let mut skipped_mem = 0usize;
    let mut failures: std::vec::Vec<std::string::String> = std::vec::Vec::new();

    for (idx, group) in EVEX_TABLE.iter().enumerate() {
        // Block index -> EVEX.mm. The table has no block for map 4, so the
        // last two blocks are MAP5 and MAP6 (Bochs folds 5 and 6 down by one
        // when indexing). Synthesise the mm the encoding must actually carry.
        let map = [1usize, 2, 3, 5, 6][idx / 256];
        let opcode = (idx % 256) as u8;
        // An entry whose own opcode is IaError names an opcode the enum lacks
        // (the generator reports each one); #UD is the correct outcome for
        // it. If such an entry comes first it also
        // shadows the rest of the group for any encoding it matches, so those
        // cannot be reached by synthesis either.
        let group_shadowed = group
            .first()
            .map(|r| OpcodeTableEntry::new(*r).opcode() == Opcode::IaError)
            .unwrap_or(false);
        if group_shadowed {
            continue;
        }
        for raw in group.iter() {
            let entry = OpcodeTableEntry::new(*raw);
            if entry.opcode() == Opcode::IaError {
                continue;
            }
            // value_bits() carries the packed opcode in its upper bits;
            // only the low 24 are decmask fields.
            let value = entry.value_bits() & 0x00FF_FFFF;
            let mask = entry.mask_bits();
            let field = |offset: u32, width: u32| -> Option<u32> {
                let m = ((1u32 << width) - 1) << offset;
                if mask & m == 0 {
                    None
                } else {
                    Some((value & m) >> offset)
                }
            };

            // Memory forms need mod != 11 and a different operand layout;
            // they are covered by the targeted disp8 tests instead.
            if field(MODC0_OFFSET, 1) == Some(0) {
                skipped_mem += 1;
                continue;
            }

            let pp = field(SSE_PREFIX_OFFSET, 2).unwrap_or(0) as u8;
            let w = field(VEX_W_OFFSET, 1).unwrap_or(0) as u8;
            // Thermometer code: 0 = 128, 1 = 256, 3 = 512. Choose the
            // narrowest width the entry accepts, then turn it back into the
            // L'L bits an encoding actually carries.
            let ll: u8 = match field(VEX_VL_128_256_OFFSET, 2) {
                Some(3) => 2,
                Some(1) => 1,
                Some(0) => 0,
                Some(_) => 1,
                None => match field(VEX_VL_128_256_OFFSET, 1) {
                    Some(1) => 1,
                    _ => 0,
                },
            };
            // MASK_K0 means "no opmask"; otherwise a non-zero one is required.
            let aaa: u8 = match field(MASK_K0_OFFSET, 1) {
                Some(1) => 0,
                Some(_) => 1,
                None => 0,
            };
            let nnn = field(NNN_OFFSET, 3).unwrap_or(0) as u8;

            let p0 = 0xF0u8 | (map as u8 & 0x07);
            let p1 = (w << 7) | (0x0F << 3) | 0x04 | pp;
            let p2 = (ll << 5) | 0x08 | aaa;
            let modrm = 0xC0u8 | ((nnn & 0x07) << 3) | 0x02;
            let bytes = [0x62, p0, p1, p2, opcode, modrm, 0x00];

            checked += 1;
            match crate::decoder::decode64::fetch_decode64(&bytes) {
                Ok(i) if i.get_ia_opcode() != Opcode::IaError => {}
                other => failures.push(std::format!(
                    "map{map} {opcode:02X} {bytes:02X?} wanted {:?} got {}",
                    entry.opcode(),
                    match other {
                        Ok(_) => std::string::String::from("IaError"),
                        Err(e) => std::format!("{e:?}"),
                    }
                )),
            }
        }
    }

    // Guard against the exclusions above quietly hollowing the harness out.
    assert!(
        checked > 1150,
        "harness should still cover the bulk of the maps, only checked {checked}"
    );
    assert!(
        failures.is_empty(),
        "{} of {checked} EVEX map entries do not decode ({skipped_mem} memory forms skipped):\n  {}",
        failures.len(),
        failures
            .iter()
            .take(12)
            .cloned()
            .collect::<std::vec::Vec<_>>()
            .join("\n  ")
    );
}

// ============================================================================
// Encoding rules that decode64 got wrong, found while porting decoder_vex32 /
// decoder_evex32. Each of these is a Bochs behaviour the 64-bit path missed.
// ============================================================================

/// EVEX map 5 lands in the same internal `opcode_map` slot as the legacy
/// `0F 0F` 3DNow! escape, which reads a trailing suffix byte. An EVEX
/// instruction has no such byte, so reading one stole a byte from the next
/// instruction. Bochs never conflates them: 3DNow! is `decoder32_3dnow` off the
/// one-byte table and map 5 is reached only through `decoder_evex64`.
#[test]
fn evex_map5_does_not_consume_a_3dnow_suffix_byte() {
    // VADDPH zmm1, zmm2, zmm3 = 62 F5 6C 48 58 CB (EVEX.512.NP.MAP5.W0 58 /r)
    let i = fetch_decode64(&[0x62, 0xF5, 0x6C, 0x48, 0x58, 0xCB]).unwrap();
    assert_eq!(i.get_ia_opcode(), Opcode::EvexVaddphVphHphWph);
    assert_eq!(i.ilen(), 6, "map 5 has no suffix byte");
    assert_eq!(i.dst(), 1);
    assert_eq!(i.src2(), 2);
    assert_eq!(i.src1(), 3);
}

/// `EVEX.L'L = 11b` is reserved. Bochs closes both EVEX decoders with
/// `if (i->getVL() > BX_VL512) ia_opcode = BX_IA_ERROR;`.
#[test]
fn evex_reserved_vector_length_is_ud() {
    // Baseline with L'L = 00.
    assert!(fetch_decode64(&[0x62, 0xF1, 0x6D, 0x08, 0xFE, 0xCB]).is_ok());
    // Same encoding with L'L = 11.
    assert!(fetch_decode64(&[0x62, 0xF1, 0x6D, 0x68, 0xFE, 0xCB]).is_err());
    // EVEX.b in register form replaces the length outright, so L'L is free to
    // carry a rounding mode — including 11b (round-to-zero).
    let i = fetch_decode64(&[0x62, 0xF1, 0x6C, 0x78, 0x58, 0xCB]).unwrap();
    assert_eq!(i.get_vl(), 2);
    assert_eq!(i.get_rc(), 3);
}

/// The opmask groups constrain the ModRM form and, for the qword GPR moves, the
/// mode — Bochs `BxOpcodeGroup_VEX_0F91` is ATTR_MOD_MEM throughout, and
/// `_0F92`/`_0F93` are ATTR_MODC0 with ATTR_IS64 on their VEX.W1 entries.
#[test]
fn vex_opmask_group_form_constraints() {
    // 0F 91 stores an opmask to memory; there is no register form.
    let i = fetch_decode64(&[0xC4, 0xE1, 0x78, 0x91, 0x08]).unwrap();
    assert_eq!(i.get_ia_opcode(), Opcode::KmovwKewKgw);
    assert!(fetch_decode64(&[0xC4, 0xE1, 0x78, 0x91, 0xCA]).is_err());

    // 0F 92 moves a GPR into an opmask; there is no memory form.
    let i = fetch_decode64(&[0xC4, 0xE1, 0x78, 0x92, 0xC8]).unwrap();
    assert_eq!(i.get_ia_opcode(), Opcode::KmovwKgwEw);
    assert!(fetch_decode64(&[0xC4, 0xE1, 0x78, 0x92, 0x08]).is_err());

    // 0F 93 goes the other way, same constraint.
    let i = fetch_decode64(&[0xC4, 0xE1, 0x78, 0x93, 0xC8]).unwrap();
    assert_eq!(i.get_ia_opcode(), Opcode::KmovwGdKew);
    assert!(fetch_decode64(&[0xC4, 0xE1, 0x78, 0x93, 0x08]).is_err());

    // The VEX.W1 qword GPR forms stay available in 64-bit mode.
    let i = fetch_decode64(&[0xC4, 0xE1, 0xFB, 0x92, 0xC8]).unwrap();
    assert_eq!(i.get_ia_opcode(), Opcode::KmovqKgqEq);

    // KORTEST/KTEST are register-only.
    assert!(fetch_decode64(&[0xC4, 0xE1, 0x78, 0x98, 0x08]).is_err());
    assert!(fetch_decode64(&[0xC4, 0xE1, 0x78, 0x99, 0x08]).is_err());
}

/// VEX.L0.66.0F3A 30..33 are the opmask shifts. Bochs
/// `BxOpcodeGroup_VEX_0F3A30`..`_0F3A33` (fetchdecode_opmap_avx.cc) lets the
/// opcode byte choose direction and width pair and VEX.W choose the member:
/// 30 = KSHIFTRB/KSHIFTRW, 31 = KSHIFTRD/KSHIFTRQ, 32 = KSHIFTLB/KSHIFTLW,
/// 33 = KSHIFTLD/KSHIFTLQ. None of the eight carries ATTR_IS64, so all of them
/// decode identically in 32-bit mode.
#[test]
fn vex_kshift_encodings_select_the_bochs_opcode() {
    // C4 E3 = 3-byte VEX, R/X/B clear, map 0F3A. Byte 2 = W vvvv=1111 L=0
    // pp=01 (66): 0x79 for W0, 0xF9 for W1. ModRM CA = mod 11, reg k1, rm k2.
    const W0: u8 = 0x79;
    const W1: u8 = 0xF9;
    let cases: [(u8, u8, Opcode); 8] = [
        (0x30, W0, Opcode::KshiftrbKgbKebIb),
        (0x30, W1, Opcode::KshiftrwKgwKewIb),
        (0x31, W0, Opcode::KshiftrdKgdKedIb),
        (0x31, W1, Opcode::KshiftrqKgqKeqIb),
        (0x32, W0, Opcode::KshiftlbKgbKebIb),
        (0x32, W1, Opcode::KshiftlwKgwKewIb),
        (0x33, W0, Opcode::KshiftldKgdKedIb),
        (0x33, W1, Opcode::KshiftlqKgqKeqIb),
    ];
    for (byte, vex2, expected) in cases {
        let enc = [0xC4u8, 0xE3, vex2, byte, 0xCA, 0x03];
        let w = vex2 >> 7;
        for (mode, decoded) in [
            ("64-bit", fetch_decode64(&enc)),
            ("32-bit", fetch_decode32(&enc, true)),
        ] {
            let i = decoded.unwrap_or_else(|e| {
                panic!("{mode}: VEX.66.0F3A.W{w} {byte:02X} must decode, got {e:?}")
            });
            assert_eq!(
                i.get_ia_opcode(),
                expected,
                "{mode}: VEX.66.0F3A.W{w} {byte:02X}"
            );
            assert_eq!(i.ilen(), 6, "{mode}: {expected:?} length");
            assert_eq!(i.dst(), 1, "{mode}: {expected:?} writes ModRM.reg (k1)");
            assert_eq!(i.src(), 2, "{mode}: {expected:?} reads ModRM.rm (k2)");
            assert_eq!(i.ib(), 3, "{mode}: {expected:?} shift count");
        }
    }

    // Every group is ATTR_VL128: VEX.L1 (byte 2 = 0x7D / 0xFD) is reserved.
    for byte in 0x30u8..=0x33 {
        for vex2 in [0x7Du8, 0xFD] {
            let enc = [0xC4u8, 0xE3, vex2, byte, 0xCA, 0x03];
            assert!(
                fetch_decode64(&enc).is_err(),
                "VEX.256.66.0F3A {byte:02X} must #UD"
            );
        }
        // No legacy (non-VEX) encoding exists at 66 0F 3A 30..33.
        assert!(
            fetch_decode64(&[0x66, 0x0F, 0x3A, byte, 0xCA, 0x03]).is_err(),
            "legacy 66 0F 3A {byte:02X} must #UD"
        );
    }
}

/// SETcc has no VEX encoding, so a VEX-prefixed 0F 90..9F that no opmask entry
/// claims must be #UD rather than falling through to the shared SETcc entry,
/// whose attribute mask constrains nothing.
#[test]
fn vex_setcc_bytes_do_not_fall_through() {
    // VEX.F3 0F 90 matches no entry in BxOpcodeGroup_VEX_0F90.
    assert!(fetch_decode64(&[0xC4, 0xE1, 0x7A, 0x90, 0xC8]).is_err());
    // 0F 9F has no opmask meaning at all.
    assert!(fetch_decode64(&[0xC4, 0xE1, 0x78, 0x9F, 0xC8]).is_err());
    // The legacy encodings are untouched.
    let i = fetch_decode64(&[0x0F, 0x90, 0xC0]).unwrap();
    assert_eq!(i.get_ia_opcode(), Opcode::SetoEb);
}

/// The is4 register of VBLENDV* is four bits wide in 64-bit mode.
#[test]
fn vex64_is4_operand_is_four_bits() {
    let i = fetch_decode64(&[0xC4, 0xE3, 0x69, 0x4C, 0xCB, 0xF0]).unwrap();
    assert_eq!(i.get_ia_opcode(), Opcode::V128VpblendvbVdqHdqWdqIb);
    assert_eq!(i.src3(), 0xF);
}

/// A VEX-prefixed byte with no VEX form must not have the *legacy* immediate
/// rules applied to it before the table lookup rejects it. `0F 8x` is a rel32
/// Jcc without a prefix, so the legacy rules would consume four bytes the
/// instruction does not own — a real over-read at a page boundary. Bochs
/// derives the VEX immediate size from the opcode number alone.
#[test]
fn vex_immediate_size_follows_the_vex_rule_not_the_legacy_one() {
    // VEX.0F 80 is not a Jcc and is not a VEX opcode: #UD, and it must reach
    // that verdict without asking for a rel32 that is not there.
    assert!(fetch_decode64(&[0xC4, 0xE1, 0x78, 0x80, 0xC0]).is_err());
    assert!(fetch_decode32(&[0xC4, 0xE1, 0x78, 0x80, 0xC0], true).is_err());

    // The bytes that really do carry an imm8 under VEX still consume it.
    // VPSHUFD xmm1, xmm2, 0x1B = C5 F9 70 CA 1B
    let i = fetch_decode64(&[0xC5, 0xF9, 0x70, 0xCA, 0x1B]).unwrap();
    assert_eq!(i.ilen(), 5);
    assert_eq!(i.ib(), 0x1B);
    let i = fetch_decode32(&[0xC5, 0xF9, 0x70, 0xCA, 0x1B], true).unwrap();
    assert_eq!(i.ilen(), 5);
    assert_eq!(i.ib(), 0x1B);

    // VCMPPS xmm1, xmm2, xmm3, 0 = C5 E8 C2 CB 00
    let i = fetch_decode64(&[0xC5, 0xE8, 0xC2, 0xCB, 0x00]).unwrap();
    assert_eq!(i.ilen(), 5);
    // Map 3 always carries one: VPALIGNR xmm1,xmm2,xmm3,4
    let i = fetch_decode64(&[0xC4, 0xE3, 0x69, 0x0F, 0xCB, 0x04]).unwrap();
    assert_eq!(i.ilen(), 6);
    assert_eq!(i.ib(), 4);
}

/// A VEX prefix on an opcode byte Bochs leaves as `BxOpcodeGroup_ERR` is a
/// guest #UD, whatever the shared legacy table holds for that byte.
///
/// Upstream never consults the legacy tables from the VEX path — it resolves
/// against `BxOpcodeTableVEX` alone. Sharing the tables meant `VEX.0F 80`
/// resolved to `JO rel32` and the guest *took the branch*; CPUID, RDTSC, MOV
/// CR/DR, CMPXCHG, the BT group and the SHLD/SHRD pair were all reachable the
/// same way.
#[test]
fn vex_unpopulated_opcode_slots_are_ud() {
    // VEX.128.NP.0F <byte> /r, register form. Every byte below is
    // BxOpcodeGroup_ERR in BxOpcodeTableVEX.
    for byte in [
        0x00u8, 0x01, 0x0B, 0x20, 0x22, 0x31, 0x80, 0x82, 0xA2, 0xA3, 0xA4, 0xAB, 0xAC, 0xB0,
        0xB6, 0xBA, 0xBC, 0xC0, 0xC1, 0xC3, 0xC7,
    ] {
        let enc = [0xC4u8, 0xE1, 0x78, byte, 0xC1, 0x00, 0x00, 0x00, 0x00];
        assert!(
            fetch_decode64(&enc).is_err(),
            "VEX.0F {byte:02X} has no VEX form and must be #UD in 64-bit mode"
        );
        assert!(
            fetch_decode32(&enc, true).is_err(),
            "VEX.0F {byte:02X} has no VEX form and must be #UD in 32-bit mode"
        );
    }

    // 0F38 F0/F1 (MOVBE) and 0F3A FF have no VEX form either.
    for enc in [
        [0xC4u8, 0xE2, 0x78, 0xF0, 0xC1, 0x00],
        [0xC4, 0xE2, 0x78, 0xF1, 0xC1, 0x00],
        [0xC4, 0xE3, 0x78, 0xFF, 0xC1, 0x00],
    ] {
        assert!(fetch_decode64(&enc).is_err());
        assert!(fetch_decode32(&enc, true).is_err());
    }

    // The neighbouring populated slots still decode, in both modes.
    assert!(fetch_decode64(&[0xC4, 0xE2, 0x69, 0x00, 0xCB]).is_ok()); // VPSHUFB
    assert!(fetch_decode32(&[0xC4, 0xE2, 0x69, 0x00, 0xCB], true).is_ok());
    assert!(fetch_decode64(&[0xC4, 0xE3, 0x69, 0x44, 0xCB, 0x00]).is_ok()); // VPCLMULQDQ
    assert!(fetch_decode32(&[0xC4, 0xE3, 0x69, 0x44, 0xCB, 0x00], true).is_ok());
    // ...and so do the legacy encodings of the same bytes.
    assert!(fetch_decode64(&[0x0F, 0xA2]).is_ok()); // CPUID
    assert!(fetch_decode32(&[0x0F, 0x31], true).is_ok()); // RDTSC
}

/// `LOCK MOV CR0` is AMD's ALT_MOV_CR8 alias and must decode as an access to
/// CR8, not as #UD.
///
/// A LOCK prefix on a register-form operand is illegal for every opcode
/// except these four. Bochs carves them out in the tail of
/// `fetchDecode64` / `fetchDecode32`, extending CR0 to CR8. Whether the CPU is
/// actually allowed to do that is a feature question the decoder cannot answer,
/// so it always extends and the MOV CR handlers veto it; see
/// `check_alt_mov_cr8` in cpu/crregs.rs.
///
/// The CR index lives in operand slot 0 (`dst()`) for both directions in this
/// decoder, unlike Bochs, which uses slot 1 for the read form.
#[test]
fn lock_mov_cr0_decodes_as_the_cr8_alias() {
    // MOV CR0, rax  =  0F 22 C0   (ModRM C0: mod=11, reg=CR0, rm=rax)
    let plain = fetch_decode64(&[0x0F, 0x22, 0xC0]).unwrap();
    assert_eq!(plain.get_ia_opcode(), Opcode::MovCr0rq);
    assert_eq!(plain.dst(), 0, "without LOCK the destination stays CR0");

    // LOCK MOV CR0, rax  =  F0 0F 22 C0
    let locked = fetch_decode64(&[0xF0, 0x0F, 0x22, 0xC0]).unwrap();
    assert_eq!(locked.get_ia_opcode(), Opcode::MovCr0rq);
    assert_eq!(locked.dst(), 8, "LOCK redirects the write to CR8");
    assert!(locked.get_lock(), "the handler needs the prefix to gate on");

    // LOCK MOV rax, CR0  =  F0 0F 20 C0 — the read direction.
    let locked_read = fetch_decode64(&[0xF0, 0x0F, 0x20, 0xC0]).unwrap();
    assert_eq!(locked_read.get_ia_opcode(), Opcode::MovRqCr0);
    assert_eq!(locked_read.dst(), 8, "LOCK redirects the read to CR8");

    // Every other register-form opcode still rejects LOCK.
    // LOCK ADD eax, ecx  =  F0 01 C8
    assert!(
        fetch_decode64(&[0xF0, 0x01, 0xC8]).is_err(),
        "LOCK on a register destination is #UD for everything else"
    );

    // 32-bit mode is where this actually matters: CR8 has no other encoding
    // there, because REX.R does not exist.
    let i32 = fetch_decode32(&[0xF0, 0x0F, 0x22, 0xC0], true).unwrap();
    assert_eq!(i32.get_ia_opcode(), Opcode::MovCr0rd);
    assert_eq!(i32.dst(), 8, "32-bit LOCK MOV CR0 must reach CR8 too");
}

// ════════════════════════════════════════════════════════════════════════
// VEX.W1 / EVEX.W1 outside 64-bit mode, and the groups whose rows Bochs ends
// with `last_opcode`.
// ════════════════════════════════════════════════════════════════════════

/// One encoding, and the opcodes each decoder mode may make of it: every name
/// that one operand-width form goes by.
struct ModeCase {
    what: &'static str,
    bytes: &'static [u8],
    in_32_bit: &'static [Opcode],
    in_64_bit: &'static [Opcode],
}

/// One encoding that decodes, and the opcode it must decode to.
struct DecodeCase {
    what: &'static str,
    bytes: &'static [u8],
    decodes_to: Opcode,
}

/// One encoding that no row of its group admits, so it must be #UD.
struct UndefinedCase {
    what: &'static str,
    bytes: &'static [u8],
}

fn assert_decodes_64(case: &DecodeCase) {
    match fetch_decode64(case.bytes) {
        Ok(i) => assert_eq!(
            i.get_ia_opcode(),
            case.decodes_to,
            "{}: {:02X?}",
            case.what,
            case.bytes
        ),
        Err(e) => panic!("{}: {:02X?} must decode, got {e:?}", case.what, case.bytes),
    }
}

fn assert_undefined_64(case: &UndefinedCase) {
    let decoded = fetch_decode64(case.bytes).map(|i| i.get_ia_opcode());
    assert!(
        decoded.is_err(),
        "{}: {:02X?} must be #UD, decoded {decoded:?}",
        case.what,
        case.bytes
    );
}

/// Bochs fetchdecode_opmap_avx.cc `BxOpcodeGroup_VEX_0F2A` (and 0F2C, 0F2D,
/// 0F6E, 0F7E, 0F3A16, 0F3A22) and fetchdecode_opmap_evex.cc
/// `BxOpcodeGroup_EVEX_0F2A` (and the same siblings plus MAP5 2A/2C/2D/78/79/
/// 7B): the 64-bit form is `ATTR_VEX_W1 | ATTR_IS64` and comes first; the
/// 32-bit form that follows carries no W attribute. Outside 64-bit mode W1
/// is therefore ignored and selects the 32-bit form; in 64-bit mode W1
/// selects the 64-bit form and W0 the 32-bit one.
#[test]
fn w1_selects_the_64_bit_form_only_in_64_bit_mode() {
    // A width form counts under the port's legacy SSE name or Bochs's VEX name.
    let cases = [
        ModeCase {
            what: "VEX.W1 F3 0F 2A VCVTSI2SS",
            bytes: &[0xC4, 0xE1, 0xFA, 0x2A, 0xC0],
            in_32_bit: &[Opcode::Vcvtsi2ssVssEd],
            in_64_bit: &[Opcode::Vcvtsi2ssVssEq],
        },
        ModeCase {
            what: "VEX.W0 F3 0F 2A VCVTSI2SS",
            bytes: &[0xC4, 0xE1, 0x7A, 0x2A, 0xC0],
            in_32_bit: &[Opcode::Vcvtsi2ssVssEd],
            in_64_bit: &[Opcode::Vcvtsi2ssVssEd],
        },
        ModeCase {
            what: "VEX.W1 F3 0F 2C VCVTTSS2SI",
            bytes: &[0xC4, 0xE1, 0xFA, 0x2C, 0xC0],
            in_32_bit: &[Opcode::Cvttss2siGdWss, Opcode::Vcvttss2siGdWss],
            in_64_bit: &[Opcode::Cvttss2siGqWss, Opcode::Vcvttss2siGqWss],
        },
        ModeCase {
            what: "VEX.W1 F2 0F 2D VCVTSD2SI",
            bytes: &[0xC4, 0xE1, 0xFB, 0x2D, 0xC0],
            in_32_bit: &[Opcode::Cvtsd2siGdWsd, Opcode::Vcvtsd2siGdWsd],
            in_64_bit: &[Opcode::Cvtsd2siGqWsd, Opcode::Vcvtsd2siGqWsd],
        },
        ModeCase {
            what: "VEX.W1 66 0F 6E VMOVD/VMOVQ",
            bytes: &[0xC4, 0xE1, 0xF9, 0x6E, 0xC0],
            in_32_bit: &[Opcode::MovdVdqEd, Opcode::V128VmovdVdqEd],
            in_64_bit: &[Opcode::MovqVdqEq, Opcode::V128VmovqVdqEq],
        },
        ModeCase {
            what: "VEX.W1 66 0F 7E VMOVD/VMOVQ",
            bytes: &[0xC4, 0xE1, 0xF9, 0x7E, 0xC0],
            in_32_bit: &[Opcode::MovdEdVd, Opcode::V128VmovdEdVd],
            in_64_bit: &[Opcode::MovqEqVq, Opcode::V128VmovqEqVq],
        },
        ModeCase {
            what: "VEX.W1 66 0F3A 16 VPEXTRD/VPEXTRQ",
            bytes: &[0xC4, 0xE3, 0xF9, 0x16, 0xC0, 0x01],
            in_32_bit: &[Opcode::V128VpextrdEdVdqIb],
            in_64_bit: &[Opcode::V128VpextrqEqVdqIb],
        },
        ModeCase {
            what: "VEX.W1 66 0F3A 22 VPINSRD/VPINSRQ",
            bytes: &[0xC4, 0xE3, 0xF9, 0x22, 0xC0, 0x01],
            in_32_bit: &[Opcode::V128VpinsrdVdqEdIb],
            in_64_bit: &[Opcode::V128VpinsrqVdqEqIb],
        },
        ModeCase {
            what: "EVEX.W1 F3 0F 2A VCVTSI2SS",
            bytes: &[0x62, 0xF1, 0xFE, 0x08, 0x2A, 0xC0],
            in_32_bit: &[Opcode::EvexVcvtsi2ssVssEd],
            in_64_bit: &[Opcode::EvexVcvtsi2ssVssEq],
        },
        ModeCase {
            what: "EVEX.W0 F3 0F 2A VCVTSI2SS",
            bytes: &[0x62, 0xF1, 0x7E, 0x08, 0x2A, 0xC0],
            in_32_bit: &[Opcode::EvexVcvtsi2ssVssEd],
            in_64_bit: &[Opcode::EvexVcvtsi2ssVssEd],
        },
        ModeCase {
            what: "EVEX.W1 F3 0F 2C VCVTTSS2SI",
            bytes: &[0x62, 0xF1, 0xFE, 0x08, 0x2C, 0xC0],
            in_32_bit: &[Opcode::EvexVcvttss2siGdWss],
            in_64_bit: &[Opcode::EvexVcvttss2siGqWss],
        },
        ModeCase {
            what: "EVEX.W1 F2 0F 2D VCVTSD2SI",
            bytes: &[0x62, 0xF1, 0xFF, 0x08, 0x2D, 0xC0],
            in_32_bit: &[Opcode::EvexVcvtsd2siGdWsd],
            in_64_bit: &[Opcode::EvexVcvtsd2siGqWsd],
        },
        ModeCase {
            what: "EVEX.W1 66 0F 6E VMOVD/VMOVQ",
            bytes: &[0x62, 0xF1, 0xFD, 0x08, 0x6E, 0xC0],
            in_32_bit: &[Opcode::EvexVmovdVdqEd],
            in_64_bit: &[Opcode::EvexVmovqVdqEq],
        },
        ModeCase {
            what: "EVEX.W1 66 0F 7E VMOVD/VMOVQ",
            bytes: &[0x62, 0xF1, 0xFD, 0x08, 0x7E, 0xC0],
            in_32_bit: &[Opcode::EvexVmovdEdVd],
            in_64_bit: &[Opcode::EvexVmovqEqVq],
        },
        ModeCase {
            what: "EVEX.W1 66 0F3A 16 VPEXTRD/VPEXTRQ",
            bytes: &[0x62, 0xF3, 0xFD, 0x08, 0x16, 0xC0, 0x01],
            in_32_bit: &[Opcode::EvexVpextrdEdVdqIb],
            in_64_bit: &[Opcode::EvexVpextrqEqVdqIb],
        },
        ModeCase {
            what: "EVEX.W1 66 0F3A 22 VPINSRD/VPINSRQ",
            bytes: &[0x62, 0xF3, 0xFD, 0x08, 0x22, 0xC0, 0x01],
            in_32_bit: &[Opcode::EvexVpinsrdVdqEdIb],
            in_64_bit: &[Opcode::EvexVpinsrqVdqEqIb],
        },
    ];
    for case in &cases {
        match fetch_decode32(case.bytes, true) {
            Ok(i) => assert!(
                case.in_32_bit.contains(&i.get_ia_opcode()),
                "{} in 32-bit mode: {:02X?} decoded {:?}, not the 32-bit form {:?}",
                case.what,
                case.bytes,
                i.get_ia_opcode(),
                case.in_32_bit
            ),
            Err(e) => panic!("{} must decode in 32-bit mode, got {e:?}", case.what),
        }
        match fetch_decode64(case.bytes) {
            Ok(i) => assert!(
                case.in_64_bit.contains(&i.get_ia_opcode()),
                "{} in 64-bit mode: {:02X?} decoded {:?}, not the expected form {:?}",
                case.what,
                case.bytes,
                i.get_ia_opcode(),
                case.in_64_bit
            ),
            Err(e) => panic!("{} must decode in 64-bit mode, got {e:?}", case.what),
        }
    }
}

/// Bochs fetchdecode_opmap_evex.cc `BxOpcodeGroup_EVEX_0F3819`: every row
/// carries `ATTR_VL256_512`, so VBROADCASTSD and VBROADCASTF32X2 have no
/// 128-bit form.
#[test]
fn evex_128_bit_vbroadcastsd_and_vbroadcastf32x2_are_undefined() {
    for case in [
        UndefinedCase {
            what: "EVEX.128.66.0F38.W1 19 VBROADCASTSD",
            bytes: &[0x62, 0xF2, 0xFD, 0x08, 0x19, 0xC0],
        },
        UndefinedCase {
            what: "EVEX.128.66.0F38.W1 19 VBROADCASTSD {k1}",
            bytes: &[0x62, 0xF2, 0xFD, 0x09, 0x19, 0xC0],
        },
        UndefinedCase {
            what: "EVEX.128.66.0F38.W0 19 VBROADCASTF32X2",
            bytes: &[0x62, 0xF2, 0x7D, 0x08, 0x19, 0xC0],
        },
        UndefinedCase {
            what: "EVEX.128.66.0F38.W0 19 VBROADCASTF32X2 {k1}",
            bytes: &[0x62, 0xF2, 0x7D, 0x09, 0x19, 0xC0],
        },
    ] {
        assert_undefined_64(&case);
    }
    for case in [
        DecodeCase {
            what: "EVEX.256.66.0F38.W1 19 VBROADCASTSD",
            bytes: &[0x62, 0xF2, 0xFD, 0x28, 0x19, 0xC0],
            decodes_to: Opcode::EvexVbroadcastsdVpdWsd,
        },
        DecodeCase {
            what: "EVEX.512.66.0F38.W1 19 VBROADCASTSD {k1}",
            bytes: &[0x62, 0xF2, 0xFD, 0x49, 0x19, 0xC0],
            decodes_to: Opcode::EvexVbroadcastsdVpdWsdKmask,
        },
        DecodeCase {
            what: "EVEX.256.66.0F38.W0 19 VBROADCASTF32X2",
            bytes: &[0x62, 0xF2, 0x7D, 0x28, 0x19, 0xC0],
            decodes_to: Opcode::EvexVbroadcastf32x2VpsWq,
        },
        DecodeCase {
            what: "EVEX.512.66.0F38.W0 19 VBROADCASTF32X2 {k1}",
            bytes: &[0x62, 0xF2, 0x7D, 0x49, 0x19, 0xC0],
            decodes_to: Opcode::EvexVbroadcastf32x2VpsWqKmask,
        },
    ] {
        assert_decodes_64(&case);
    }
}

/// Bochs ends every opcode group with a `last_opcode` row; a group missing it
/// lets a lookup that matches no row run on into the next group's rows. The
/// port's groups are slices, so a lookup ends at the slice's end. These are
/// the groups Bochs's fetchdecode_opmap_evex.cc `BxOpcodeGroup_EVEX_0F381A`
/// and `BxOpcodeGroup_EVEX_0F381B` cover; each encoding below matches none of
/// their rows (memory only, VL256_512 or VL512), and the next group,
/// `BxOpcodeGroup_EVEX_0F381C`, holds VPABSB rows that would accept them.
#[test]
fn evex_broadcast_groups_end_at_their_last_row() {
    for case in [
        UndefinedCase {
            what: "EVEX.128.66.0F38.W0 1A VBROADCASTF32X4 m128",
            bytes: &[0x62, 0xF2, 0x7D, 0x08, 0x1A, 0x00],
        },
        UndefinedCase {
            what: "EVEX.128.66.0F38.W1 1A VBROADCASTF64X2 m128",
            bytes: &[0x62, 0xF2, 0xFD, 0x08, 0x1A, 0x00],
        },
        UndefinedCase {
            what: "EVEX.256.66.0F38.W0 1A register form",
            bytes: &[0x62, 0xF2, 0x7D, 0x28, 0x1A, 0xC0],
        },
        UndefinedCase {
            what: "EVEX.256.66.0F38.W0 1B VBROADCASTF32X8 m256",
            bytes: &[0x62, 0xF2, 0x7D, 0x28, 0x1B, 0x00],
        },
        UndefinedCase {
            what: "EVEX.256.66.0F38.W1 1B VBROADCASTF64X4 m256",
            bytes: &[0x62, 0xF2, 0xFD, 0x28, 0x1B, 0x00],
        },
        UndefinedCase {
            what: "EVEX.512.66.0F38.W0 1B register form",
            bytes: &[0x62, 0xF2, 0x7D, 0x48, 0x1B, 0xC0],
        },
    ] {
        assert_undefined_64(&case);
    }
    for case in [
        DecodeCase {
            what: "EVEX.256.66.0F38.W0 1A VBROADCASTF32X4",
            bytes: &[0x62, 0xF2, 0x7D, 0x28, 0x1A, 0x00],
            decodes_to: Opcode::EvexVbroadcastf32x4VpsWps,
        },
        DecodeCase {
            what: "EVEX.512.66.0F38.W1 1A VBROADCASTF64X2",
            bytes: &[0x62, 0xF2, 0xFD, 0x48, 0x1A, 0x00],
            decodes_to: Opcode::EvexVbroadcastf64x2VpdWpd,
        },
        DecodeCase {
            what: "EVEX.512.66.0F38.W0 1B VBROADCASTF32X8",
            bytes: &[0x62, 0xF2, 0x7D, 0x48, 0x1B, 0x00],
            decodes_to: Opcode::EvexVbroadcastf32x8VpsWps,
        },
        DecodeCase {
            what: "EVEX.512.66.0F38.W1 1B VBROADCASTF64X4",
            bytes: &[0x62, 0xF2, 0xFD, 0x48, 0x1B, 0x00],
            decodes_to: Opcode::EvexVbroadcastf64x4VpdWpd,
        },
    ] {
        assert_decodes_64(&case);
    }
}

/// Bochs fetchdecode_opmap_avx.cc `BxOpcodeGroup_VEX_0F2A`, `_0F2C`, `_0F2D`
/// (F3 and F2 rows only), `_0F6E` (66, VL128), `_0F7E` (66 VL128, F3 VL128),
/// `_0F3A16` and `_0F3A22` (66, VL128): an encoding outside those rows is #UD
/// in both modes. The port reaches these bytes through the shared legacy
/// table, whose no-prefix rows are the MMX forms.
#[test]
fn vex_groups_hold_only_their_bochs_rows() {
    let cases = [
        UndefinedCase { what: "VEX.NP 0F 2A (CVTPI2PS)", bytes: &[0xC4, 0xE1, 0x78, 0x2A, 0xC0] },
        UndefinedCase { what: "VEX.66 0F 2A (CVTPI2PD)", bytes: &[0xC4, 0xE1, 0x79, 0x2A, 0xC0] },
        UndefinedCase { what: "VEX.NP 0F 2C (CVTTPS2PI)", bytes: &[0xC4, 0xE1, 0x78, 0x2C, 0xC0] },
        UndefinedCase { what: "VEX.66 0F 2C (CVTTPD2PI)", bytes: &[0xC4, 0xE1, 0x79, 0x2C, 0xC0] },
        UndefinedCase { what: "VEX.NP 0F 2D (CVTPS2PI)", bytes: &[0xC4, 0xE1, 0x78, 0x2D, 0xC0] },
        UndefinedCase { what: "VEX.66 0F 2D (CVTPD2PI)", bytes: &[0xC4, 0xE1, 0x79, 0x2D, 0xC0] },
        UndefinedCase { what: "VEX.NP 0F 6E (MOVD mm)", bytes: &[0xC4, 0xE1, 0x78, 0x6E, 0xC0] },
        UndefinedCase { what: "VEX.W1 NP 0F 6E (MOVQ mm)", bytes: &[0xC4, 0xE1, 0xF8, 0x6E, 0xC0] },
        UndefinedCase { what: "VEX.F3 0F 6E", bytes: &[0xC4, 0xE1, 0x7A, 0x6E, 0xC0] },
        UndefinedCase { what: "VEX.F2 0F 6E", bytes: &[0xC4, 0xE1, 0x7B, 0x6E, 0xC0] },
        UndefinedCase { what: "VEX.256.66 0F 6E", bytes: &[0xC4, 0xE1, 0x7D, 0x6E, 0xC0] },
        UndefinedCase { what: "VEX.NP 0F 7E (MOVD mm)", bytes: &[0xC4, 0xE1, 0x78, 0x7E, 0xC0] },
        UndefinedCase { what: "VEX.W1 NP 0F 7E (MOVQ mm)", bytes: &[0xC4, 0xE1, 0xF8, 0x7E, 0xC0] },
        UndefinedCase { what: "VEX.F2 0F 7E", bytes: &[0xC4, 0xE1, 0x7B, 0x7E, 0xC0] },
        UndefinedCase { what: "VEX.256.66 0F 7E", bytes: &[0xC4, 0xE1, 0x7D, 0x7E, 0xC0] },
        UndefinedCase { what: "VEX.256.F3 0F 7E", bytes: &[0xC4, 0xE1, 0x7E, 0x7E, 0xC0] },
        UndefinedCase { what: "VEX.NP 0F3A 16", bytes: &[0xC4, 0xE3, 0x78, 0x16, 0xC0, 0x01] },
        UndefinedCase { what: "VEX.F3 0F3A 16", bytes: &[0xC4, 0xE3, 0x7A, 0x16, 0xC0, 0x01] },
        UndefinedCase { what: "VEX.256.66 0F3A 16", bytes: &[0xC4, 0xE3, 0x7D, 0x16, 0xC0, 0x01] },
        UndefinedCase { what: "VEX.NP 0F3A 22", bytes: &[0xC4, 0xE3, 0x78, 0x22, 0xC0, 0x01] },
        UndefinedCase { what: "VEX.F2 0F3A 22", bytes: &[0xC4, 0xE3, 0x7B, 0x22, 0xC0, 0x01] },
        UndefinedCase { what: "VEX.256.66 0F3A 22", bytes: &[0xC4, 0xE3, 0x7D, 0x22, 0xC0, 0x01] },
    ];
    for case in &cases {
        assert_undefined_64(case);
        let decoded = fetch_decode32(case.bytes, true).map(|i| i.get_ia_opcode());
        assert!(
            decoded.is_err(),
            "{} in 32-bit mode: {:02X?} must be #UD, decoded {decoded:?}",
            case.what,
            case.bytes
        );
    }
    // The rows those groups do hold still decode.
    for case in [
        DecodeCase {
            what: "VEX.F3 0F 7E VMOVQ",
            bytes: &[0xC4, 0xE1, 0x7A, 0x7E, 0xC0],
            decodes_to: Opcode::VmovqVqWq,
        },
        DecodeCase {
            what: "VEX.F2 0F 2A VCVTSI2SD",
            bytes: &[0xC4, 0xE1, 0x7B, 0x2A, 0xC0],
            decodes_to: Opcode::Vcvtsi2sdVsdEd,
        },
    ] {
        assert_decodes_64(&case);
    }
}

/// Bochs fetchdecode_opmap_avx.cc `BxOpcodeGroup_VEX_0F11` admits every
/// encoding of its byte: VMOVUPS (no prefix) and VMOVUPD (66) at VL128 and
/// VL256, VMOVSS (F3) and VMOVSD (F2) as a register or a memory store at
/// any VEX.L. No encoding matches none of its rows, so none can reach the
/// group's end.
#[test]
fn vex_0f11_group_admits_every_encoding() {
    for pp in 0u8..4 {
        for vex_l in 0u8..2 {
            for modrm in [0xC0u8, 0x00] {
                let bytes = [0xC4, 0xE1, 0x78 | (vex_l << 2) | pp, 0x11, modrm];
                let opcode = match fetch_decode64(&bytes) {
                    Ok(i) => i.get_ia_opcode(),
                    Err(e) => panic!("VEX 0F 11 {bytes:02X?} must decode, got {e:?}"),
                };
                let admitted = match pp {
                    0 if vex_l == 0 => opcode == Opcode::V128VmovupsWpsVps,
                    0 => opcode == Opcode::V256VmovupsWpsVps,
                    1 if vex_l == 0 => opcode == Opcode::V128VmovupdWpdVpd,
                    1 => opcode == Opcode::V256VmovupdWpdVpd,
                    // The port folds Bochs's register and memory rows into one
                    // opcode whose handler branches on mod; either is VMOVSS.
                    2 => matches!(
                        opcode,
                        Opcode::V128VmovssWssHpsVss | Opcode::V128VmovssWssVss
                    ),
                    _ => matches!(
                        opcode,
                        Opcode::V128VmovsdWsdHpdVsd | Opcode::V128VmovsdWsdVsd
                    ),
                };
                assert!(admitted, "VEX 0F 11 {bytes:02X?} decoded {opcode:?}");
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════
// The ACE EVEX slots outside `#if BX_SUPPORT_AMX`.
// ════════════════════════════════════════════════════════════════════════

/// Bochs fetchdecode_opmap_evex.cc `BxOpcodeGroup_EVEX_0F3841`,
/// `BxOpcodeGroup_EVEX_0F3A3D` and `BxOpcodeGroup_EVEX_MAP5_36` .. `_3E`:
/// each row is pinned by its SSE prefix and EVEX.W, so a row that moves to
/// another prefix or W decodes something else or nothing.
#[test]
fn evex_ace_slots_select_by_prefix_and_w() {
    let decodes = [
        DecodeCase {
            what: "EVEX.F3.0F38.W0 41",
            bytes: &[0x62, 0xF2, 0x7E, 0x08, 0x41, 0xC0],
            decodes_to: Opcode::EvexVpmovssdbWdqVdq,
        },
        DecodeCase {
            what: "EVEX.F3.0F38.W0 41 {k1}",
            bytes: &[0x62, 0xF2, 0x7E, 0x09, 0x41, 0xC0],
            decodes_to: Opcode::EvexVpmovssdbWdqVdqKmask,
        },
        DecodeCase {
            what: "EVEX.NP.0F3A.W0 3D",
            bytes: &[0x62, 0xF3, 0x7C, 0x08, 0x3D, 0xC0, 0x00],
            decodes_to: Opcode::EvexVunpackbVdqWdqIbKmask,
        },
        DecodeCase {
            what: "EVEX.NP.MAP5.W1 36",
            bytes: &[0x62, 0xF5, 0xFC, 0x08, 0x36, 0xC0],
            decodes_to: Opcode::EvexVcvtbf82psVpsWf8Kmask,
        },
        DecodeCase {
            what: "EVEX.NP.MAP5.W0 36",
            bytes: &[0x62, 0xF5, 0x7C, 0x08, 0x36, 0xC0],
            decodes_to: Opcode::EvexVcvthf82psVpsWf8Kmask,
        },
        DecodeCase {
            what: "EVEX.NP.MAP5.W0 37",
            bytes: &[0x62, 0xF5, 0x7C, 0x08, 0x37, 0xC0],
            decodes_to: Opcode::EvexVcvtbf42hf8Vf8Wf4Kmask,
        },
        DecodeCase {
            what: "EVEX.66.MAP5.W1 37 (register)",
            bytes: &[0x62, 0xF5, 0xFD, 0x08, 0x37, 0xC0],
            decodes_to: Opcode::EvexVcvtbf62hf8Vf8Wf6Kmask,
        },
        DecodeCase {
            what: "EVEX.66.MAP5.W0 37 (register)",
            bytes: &[0x62, 0xF5, 0x7D, 0x08, 0x37, 0xC0],
            decodes_to: Opcode::EvexVcvthf62hf8Vf8Wf6Kmask,
        },
        DecodeCase {
            what: "EVEX.F3.MAP5.W0 38",
            bytes: &[0x62, 0xF5, 0x7E, 0x08, 0x38, 0xC0],
            decodes_to: Opcode::EvexVcvtps2hf8Vf8WpsKmask,
        },
        DecodeCase {
            what: "EVEX.66.MAP5.W0 38",
            bytes: &[0x62, 0xF5, 0x7D, 0x08, 0x38, 0xC0],
            decodes_to: Opcode::EvexVcvtrops2hf8Vf8WpsKmask,
        },
        DecodeCase {
            what: "EVEX.NP.MAP5.W0 38",
            bytes: &[0x62, 0xF5, 0x7C, 0x08, 0x38, 0xC0],
            decodes_to: Opcode::EvexVcvtbiasps2hf8Vf8HdqWpsKmask,
        },
        DecodeCase {
            what: "EVEX.F3.MAP5.W0 39",
            bytes: &[0x62, 0xF5, 0x7E, 0x08, 0x39, 0xC0],
            decodes_to: Opcode::EvexVcvtps2bf8Vf8WpsKmask,
        },
        DecodeCase {
            what: "EVEX.NP.MAP5.W0 39",
            bytes: &[0x62, 0xF5, 0x7C, 0x08, 0x39, 0xC0],
            decodes_to: Opcode::EvexVcvtbiasps2bf8Vf8HdqWpsKmask,
        },
        DecodeCase {
            what: "EVEX.F3.MAP5.W0 3A",
            bytes: &[0x62, 0xF5, 0x7E, 0x08, 0x3A, 0xC0],
            decodes_to: Opcode::EvexVcvtps2hf8sVf8WpsKmask,
        },
        DecodeCase {
            what: "EVEX.66.MAP5.W0 3A",
            bytes: &[0x62, 0xF5, 0x7D, 0x08, 0x3A, 0xC0],
            decodes_to: Opcode::EvexVcvtrops2hf8sVf8WpsKmask,
        },
        DecodeCase {
            what: "EVEX.NP.MAP5.W0 3A",
            bytes: &[0x62, 0xF5, 0x7C, 0x08, 0x3A, 0xC0],
            decodes_to: Opcode::EvexVcvtbiasps2hf8sVf8HdqWpsKmask,
        },
        DecodeCase {
            what: "EVEX.F3.MAP5.W0 3B",
            bytes: &[0x62, 0xF5, 0x7E, 0x08, 0x3B, 0xC0],
            decodes_to: Opcode::EvexVcvtps2bf8sVf8WpsKmask,
        },
        DecodeCase {
            what: "EVEX.NP.MAP5.W0 3B",
            bytes: &[0x62, 0xF5, 0x7C, 0x08, 0x3B, 0xC0],
            decodes_to: Opcode::EvexVcvtbiasps2bf8sVf8HdqWpsKmask,
        },
        DecodeCase {
            what: "EVEX.F3.MAP5.W0 3C (register, k0)",
            bytes: &[0x62, 0xF5, 0x7E, 0x08, 0x3C, 0xC0],
            decodes_to: Opcode::EvexVcvthf82hf6sVf6Wf8,
        },
        DecodeCase {
            what: "EVEX.F3.MAP5.W0 3D (k0)",
            bytes: &[0x62, 0xF5, 0x7E, 0x08, 0x3D, 0xC0],
            decodes_to: Opcode::EvexVcvthf82bf4sWf4Vdq,
        },
        DecodeCase {
            what: "EVEX.F3.MAP5.W1 3D (k0)",
            bytes: &[0x62, 0xF5, 0xFE, 0x08, 0x3D, 0xC0],
            decodes_to: Opcode::EvexVcvtbf82bf4sWf4Vdq,
        },
        DecodeCase {
            what: "EVEX.F3.MAP5.W1 3E (register, k0)",
            bytes: &[0x62, 0xF5, 0xFE, 0x08, 0x3E, 0xC0],
            decodes_to: Opcode::EvexVcvtbf82bf6sVf6Wf8,
        },
    ];
    for case in &decodes {
        assert_decodes_64(case);
    }

    let undefined = [
        UndefinedCase { what: "EVEX.F3.0F38.W1 41", bytes: &[0x62, 0xF2, 0xFE, 0x08, 0x41, 0xC0] },
        UndefinedCase { what: "EVEX.66.0F38.W0 41", bytes: &[0x62, 0xF2, 0x7D, 0x08, 0x41, 0xC0] },
        UndefinedCase {
            what: "EVEX.NP.0F3A.W1 3D",
            bytes: &[0x62, 0xF3, 0xFC, 0x08, 0x3D, 0xC0, 0x00],
        },
        UndefinedCase {
            what: "EVEX.66.0F3A.W0 3D",
            bytes: &[0x62, 0xF3, 0x7D, 0x08, 0x3D, 0xC0, 0x00],
        },
        UndefinedCase { what: "EVEX.66.MAP5.W0 36", bytes: &[0x62, 0xF5, 0x7D, 0x08, 0x36, 0xC0] },
        UndefinedCase { what: "EVEX.F3.MAP5.W0 36", bytes: &[0x62, 0xF5, 0x7E, 0x08, 0x36, 0xC0] },
        UndefinedCase { what: "EVEX.NP.MAP5.W1 37", bytes: &[0x62, 0xF5, 0xFC, 0x08, 0x37, 0xC0] },
        UndefinedCase {
            what: "EVEX.66.MAP5.W0 37 (memory)",
            bytes: &[0x62, 0xF5, 0x7D, 0x08, 0x37, 0x00],
        },
        UndefinedCase { what: "EVEX.F3.MAP5.W1 38", bytes: &[0x62, 0xF5, 0xFE, 0x08, 0x38, 0xC0] },
        UndefinedCase { what: "EVEX.F2.MAP5.W0 38", bytes: &[0x62, 0xF5, 0x7F, 0x08, 0x38, 0xC0] },
        UndefinedCase { what: "EVEX.66.MAP5.W0 39", bytes: &[0x62, 0xF5, 0x7D, 0x08, 0x39, 0xC0] },
        UndefinedCase { what: "EVEX.F2.MAP5.W0 3A", bytes: &[0x62, 0xF5, 0x7F, 0x08, 0x3A, 0xC0] },
        UndefinedCase { what: "EVEX.66.MAP5.W0 3B", bytes: &[0x62, 0xF5, 0x7D, 0x08, 0x3B, 0xC0] },
        UndefinedCase {
            what: "EVEX.F3.MAP5.W0 3C {k1}",
            bytes: &[0x62, 0xF5, 0x7E, 0x09, 0x3C, 0xC0],
        },
        UndefinedCase {
            what: "EVEX.F3.MAP5.W0 3C (memory)",
            bytes: &[0x62, 0xF5, 0x7E, 0x08, 0x3C, 0x00],
        },
        UndefinedCase { what: "EVEX.F3.MAP5.W1 3C", bytes: &[0x62, 0xF5, 0xFE, 0x08, 0x3C, 0xC0] },
        UndefinedCase {
            what: "EVEX.F3.MAP5.W0 3D {k1}",
            bytes: &[0x62, 0xF5, 0x7E, 0x09, 0x3D, 0xC0],
        },
        UndefinedCase { what: "EVEX.NP.MAP5.W0 3D", bytes: &[0x62, 0xF5, 0x7C, 0x08, 0x3D, 0xC0] },
        UndefinedCase { what: "EVEX.F3.MAP5.W0 3E", bytes: &[0x62, 0xF5, 0x7E, 0x08, 0x3E, 0xC0] },
        UndefinedCase {
            what: "EVEX.F3.MAP5.W1 3E (memory)",
            bytes: &[0x62, 0xF5, 0xFE, 0x08, 0x3E, 0x00],
        },
    ];
    for case in &undefined {
        assert_undefined_64(case);
    }
}

// ════════════════════════════════════════════════════════════════════════
// Opcode::category and the TILEMOV operand fields.
// ════════════════════════════════════════════════════════════════════════

/// `category()` classifies by discriminant range, so a range bound left
/// behind when variants are inserted would quietly reclassify them. Every
/// `Evex*` variant is EVEX, and nothing else is.
#[test]
fn evex_category_is_exactly_the_evex_named_opcodes() {
    use crate::opcode_isa::OPCODE_VARIANT_COUNT;
    use crate::typed::OpcodeCategory;
    for index in 0..OPCODE_VARIANT_COUNT {
        let opcode = Opcode::from_u16_const(index as u16);
        let name = std::format!("{opcode:?}");
        assert_eq!(
            name.starts_with("Evex"),
            opcode.category() == OpcodeCategory::Evex,
            "{name} is categorised {:?}",
            opcode.category()
        );
    }
}

/// Bochs fetchdecode.h: `OP_Trm = BX_FORM_SRC(BX_TMM_REG, BX_SRC_RM)` and
/// `OP_Wdq = BX_FORM_SRC(BX_VMM_FULL_VECTOR, BX_SRC_VECTOR_RM)`, so
/// TILEMOVROW/TILEMOVCOL `Trm, Wdq` read their vector from rm, the field the
/// tile is written to; `OP_Bd = BX_FORM_SRC(BX_GPR32, BX_SRC_VVV)` names the
/// GPR in vvvv.
#[test]
fn tilemov_trm_wdq_reads_its_vector_from_rm() {
    use crate::decoder::evex_operands::{evex_dst, EvexDst};
    use crate::instruction::{GprIndex, InstructionFlags, Operands};
    use crate::typed::TypedInstruction as T;

    for opcode in [
        Opcode::EvexTilemovrowTrmWdqIb,
        Opcode::EvexTilemovrowTrmWdqBd,
        Opcode::EvexTilemovcolTrmWdqIb,
        Opcode::EvexTilemovcolTrmWdqBd,
    ] {
        assert_eq!(evex_dst(opcode), EvexDst::RmSourceRm, "{opcode:?}");
    }

    // The decoder's operands for `EvexDst::RmSourceRm` with rm = 5 and
    // vvvv = 3: dst and src1 both rm, src2 vvvv.
    let decoded = |opcode: Opcode| Instruction {
        opcode,
        length: 7,
        flags: InstructionFlags::ModC0,
        operands: Operands {
            dst: 5,
            src1: 5,
            src2: 3,
            src3: 0,
            segment: 0,
            base: 0,
            index: 0,
            scale: 0,
        },
        immediate: 0x2A,
        displacement: 0,
    };
    match decoded(Opcode::EvexTilemovrowTrmWdqBd).typed() {
        T::EvexTilemovrowTrmWdqBdR { dst, src, ctrl } => {
            assert_eq!(dst, 5, "the tile is rm");
            assert_eq!(src, 5, "the vector is rm too");
            assert_eq!(ctrl, GprIndex::Rbx, "the row GPR is vvvv");
        }
        other => panic!("TILEMOVROW Trm, Wdq, Bd typed as {other:?}"),
    }
    match decoded(Opcode::EvexTilemovcolTrmWdqBd).typed() {
        T::EvexTilemovcolTrmWdqBdR { dst, src, ctrl } => {
            assert_eq!(dst, 5, "the tile is rm");
            assert_eq!(src, 5, "the vector is rm too");
            assert_eq!(ctrl, GprIndex::Rbx, "the column GPR is vvvv");
        }
        other => panic!("TILEMOVCOL Trm, Wdq, Bd typed as {other:?}"),
    }
    match decoded(Opcode::EvexTilemovrowTrmWdqIb).typed() {
        T::EvexTilemovrowTrmWdqIbR { dst, src, imm } => {
            assert_eq!(dst, 5, "the tile is rm");
            assert_eq!(src, 5, "the vector is rm too");
            assert_eq!(imm, 0x2A);
        }
        other => panic!("TILEMOVROW Trm, Wdq, Ib typed as {other:?}"),
    }
    match decoded(Opcode::EvexTilemovcolTrmWdqIb).typed() {
        T::EvexTilemovcolTrmWdqIbR { dst, src, imm } => {
            assert_eq!(dst, 5, "the tile is rm");
            assert_eq!(src, 5, "the vector is rm too");
            assert_eq!(imm, 0x2A);
        }
        other => panic!("TILEMOVCOL Trm, Wdq, Ib typed as {other:?}"),
    }
}

// ════════════════════════════════════════════════════════════════════════
// Instruction::typed() for every EVEX opcode, held to its Bochs def entry.
// ════════════════════════════════════════════════════════════════════════

/// ModRM.reg, ModRM.rm and EVEX.vvvv in the instructions the typed-view test
/// builds: three different registers, so a field's value shows which one it
/// was read from.
const TYPED_NNN: u8 = 5;
const TYPED_RM: u8 = 6;
const TYPED_VVVV: u8 = 7;

/// The ModRM form an instruction is typed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModrmForm {
    Register,
    Memory,
}

/// The two register fields the decoder fills from `evex_dst`.
struct EvexRegisterFields {
    dst: u8,
    src1: u8,
}

/// An EVEX instruction with the fields the decoder assigns for
/// `evex_dst(opcode)` (the `evex_dst` match in decode64.rs and decode32.rs),
/// vvvv in `src2`, and `[rsi + 0x40]` as the memory form's address.
fn evex_instruction(opcode: Opcode, form: ModrmForm) -> Instruction {
    use crate::decoder::evex_operands::{evex_dst, EvexDst};
    use crate::instruction::{InstructionFlags, Operands};

    let fields = match evex_dst(opcode) {
        EvexDst::Nnn => EvexRegisterFields {
            dst: TYPED_NNN,
            src1: TYPED_RM,
        },
        EvexDst::Rm => EvexRegisterFields {
            dst: TYPED_RM,
            src1: TYPED_NNN,
        },
        EvexDst::RmSourceRm => EvexRegisterFields {
            dst: TYPED_RM,
            src1: TYPED_RM,
        },
        EvexDst::Vvvv => EvexRegisterFields {
            dst: TYPED_VVVV,
            src1: TYPED_RM,
        },
    };
    let flags = match form {
        ModrmForm::Register => InstructionFlags::ModC0,
        ModrmForm::Memory => InstructionFlags::empty(),
    };
    Instruction {
        opcode,
        length: 7,
        flags,
        operands: Operands {
            dst: fields.dst,
            src1: fields.src1,
            src2: TYPED_VVVV,
            src3: 0,
            segment: crate::BxSegregs::Ds as u8,
            base: TYPED_RM,
            index: 4,
            scale: 0,
        },
        immediate: 0,
        displacement: 0x40,
    }
}

/// One top-level field of a `TypedInstruction` variant: its name and the
/// `Debug` text of its value.
struct TypedField {
    name: std::string::String,
    value: std::string::String,
}

/// A `TypedInstruction` as its `Debug` text spells it.
struct TypedShape {
    text: std::string::String,
    fields: Vec<TypedField>,
}

impl TypedShape {
    /// Splits `Name { a: 1, b: MemoryOperand { .. } }` at its top-level
    /// commas; a unit variant has no fields.
    fn of(instr: &Instruction) -> Self {
        let text = std::format!("{:?}", instr.typed());
        let mut fields = Vec::new();
        if let Some(open) = text.find(" { ") {
            let inner = &text[open + 3..];
            let body = inner.strip_suffix(" }").unwrap_or(inner);
            let mut depth = 0i32;
            let mut start = 0;
            for (at, c) in body.char_indices() {
                match c {
                    '{' | '(' | '[' => depth += 1,
                    '}' | ')' | ']' => depth -= 1,
                    ',' if depth == 0 => {
                        fields.push(TypedField::parse(&body[start..at]));
                        start = at + 1;
                    }
                    _ => {}
                }
            }
            fields.push(TypedField::parse(&body[start..]));
        }
        Self { text, fields }
    }

    fn field(&self, name: &str) -> Option<&TypedField> {
        self.fields.iter().find(|field| field.name == name)
    }

    /// The names of the fields that hold a `MemoryOperand`.
    fn memory_fields(&self) -> Vec<&str> {
        self.fields
            .iter()
            .filter(|field| field.value.starts_with("MemoryOperand {"))
            .map(|field| field.name.as_str())
            .collect()
    }

    /// Whether field `name` holds the register index `register`.
    fn holds(&self, name: &str, register: u8) -> bool {
        self.field(name)
            .is_some_and(|field| field.value == std::format!("{register}"))
    }

    /// How many fields are of `kind`.
    fn count(&self, kind: TypedFieldKind) -> usize {
        self.fields
            .iter()
            .filter(|field| field.kind() == kind)
            .count()
    }
}

/// What a `TypedInstruction` field of an EVEX variant names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypedFieldKind {
    /// A register or memory operand: `dst`, `src`, `src1`, `src2`, and `ctrl`
    /// (a GPR named by EVEX.vvvv).
    Operand,
    /// An immediate: `imm`.
    Immediate,
}

impl TypedField {
    fn parse(piece: &str) -> Self {
        let piece = piece.trim();
        let colon = piece
            .find(": ")
            .unwrap_or_else(|| panic!("field {piece:?} has no `: `"));
        Self {
            name: piece[..colon].into(),
            value: piece[colon + 2..].into(),
        }
    }

    /// Every field name an EVEX variant uses; a new one stops the test until
    /// it is classified here.
    fn kind(&self) -> TypedFieldKind {
        match self.name.as_str() {
            "dst" | "src" | "src1" | "src2" | "ctrl" => TypedFieldKind::Operand,
            "imm" => TypedFieldKind::Immediate,
            other => panic!("EVEX variant field `{other}` is not classified in TypedField::kind"),
        }
    }
}

/// A rule of `evex_typed_view_matches_the_bochs_operand_lists` that a typed
/// variant breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypedRule {
    /// (a) Operand fields that do not number the def entry's distinct
    /// register and memory operands.
    OperandFields,
    /// (a) A `src1` field where the def entry lists fewer than three register
    /// or memory operands.
    PhantomSrc1,
    /// (b) Immediate fields that do not number the def entry's immediates.
    ImmediateFields,
    /// (c) A store's memory variant whose only `MemoryOperand` is not `dst`.
    StoreMemoryNotInDst,
    /// (c) A load's memory variant that does not hold exactly one
    /// `MemoryOperand`, in a source field.
    LoadMemoryNotInSource,
    /// (d) A store's register variant whose source is not ModRM.reg.
    StoreSourceNotReg,
    /// (e) A TILEMOV `Trm, Wdq` register variant that does not write and read
    /// ModRM.rm.
    TileOperandsNotRm,
}

/// One broken rule: the opcode, the form, the rule, what was counted where
/// the rule counts, and the variant as typed.
struct TypedDriftFinding {
    opcode: Opcode,
    form: ModrmForm,
    rule: TypedRule,
    detail: std::string::String,
    typed: std::string::String,
}

/// An EVEX opcode excused from the named rules of the typed-view test, and
/// why. It is held to every other rule.
struct TypedViewException {
    opcode: Opcode,
    rules: &'static [TypedRule],
    reason: &'static str,
}

const fn excepted(
    opcode: Opcode,
    rules: &'static [TypedRule],
    reason: &'static str,
) -> TypedViewException {
    TypedViewException {
        opcode,
        rules,
        reason,
    }
}

const ALIASED_STORE: &str = "Bochs's first operand is OP_Mb/OP_Mw, an alias of OP_Eb/OP_Ew \
     that gen_evex_operands.py's destination table does not follow, so `evex_dst` reads Nnn \
     for this store (Task 4c)";

/// Exceptions to `evex_typed_view_matches_the_bochs_operand_lists`. Each one
/// excuses only the rules it names, and each named rule must still fire on
/// its opcode, so an entry is removed with the defect it names.
const TYPED_VIEW_EXCEPTIONS: &[TypedViewException] = &[
    excepted(
        Opcode::EvexVpextrbMbVdqIbM,
        &[TypedRule::LoadMemoryNotInSource],
        ALIASED_STORE,
    ),
    excepted(
        Opcode::EvexVpextrwMwVdqIbM,
        &[TypedRule::LoadMemoryNotInSource],
        ALIASED_STORE,
    ),
];

/// Every rule the typed view of `opcode` in `form` breaks.
fn typed_view_drift(opcode: Opcode, form: ModrmForm) -> Vec<TypedDriftFinding> {
    use crate::decoder::evex_operands::{
        evex_dst, evex_immediate_count, evex_operand_count, EvexDst,
    };

    let shape = TypedShape::of(&evex_instruction(opcode, form));
    let operands = evex_operand_count(opcode);
    let immediates = evex_immediate_count(opcode);
    let mut findings = Vec::new();
    let mut broken = |rule: TypedRule, detail: std::string::String| {
        findings.push(TypedDriftFinding {
            opcode,
            form,
            rule,
            detail,
            typed: shape.text.clone(),
        });
    };

    let typed_operands = shape.count(TypedFieldKind::Operand);
    if typed_operands != usize::from(operands) {
        broken(
            TypedRule::OperandFields,
            std::format!(" ({typed_operands} fields, {operands} in the def)"),
        );
    }
    if shape.field("src1").is_some() && operands < 3 {
        broken(TypedRule::PhantomSrc1, std::string::String::new());
    }
    let typed_immediates = shape.count(TypedFieldKind::Immediate);
    if typed_immediates != usize::from(immediates) {
        broken(
            TypedRule::ImmediateFields,
            std::format!(" ({typed_immediates} fields, {immediates} in the def)"),
        );
    }
    match form {
        ModrmForm::Memory => {
            let memory = shape.memory_fields();
            match evex_dst(opcode) {
                EvexDst::Rm => {
                    if memory != ["dst"] {
                        broken(TypedRule::StoreMemoryNotInDst, std::string::String::new());
                    }
                }
                EvexDst::Nnn | EvexDst::Vvvv | EvexDst::RmSourceRm => {
                    let in_one_source = match memory.as_slice() {
                        [field] => matches!(*field, "src" | "src1" | "src2"),
                        _ => false,
                    };
                    if !in_one_source {
                        broken(TypedRule::LoadMemoryNotInSource, std::string::String::new());
                    }
                }
            }
        }
        ModrmForm::Register => match evex_dst(opcode) {
            EvexDst::Rm => {
                let source = if operands >= 3 { "src2" } else { "src" };
                if !shape.holds(source, TYPED_NNN) {
                    broken(TypedRule::StoreSourceNotReg, std::string::String::new());
                }
            }
            EvexDst::RmSourceRm => {
                if !(shape.holds("dst", TYPED_RM) && shape.holds("src", TYPED_RM)) {
                    broken(TypedRule::TileOperandsNotRm, std::string::String::new());
                }
            }
            EvexDst::Nnn | EvexDst::Vvvv => {}
        },
    }
    findings
}

/// `Instruction::typed()` against Bochs's ia_opcodes_evex.def, for every
/// EVEX opcode in each ModRM form its def entry executes in
/// (`evex_forms`). A form whose handler is `NULL` or `BxError` is not judged:
/// the decoder never produces it.
///
/// (a) a variant's operand fields (`TypedField::kind`: `dst`, `src`, `src1`,
///     `src2`, and `ctrl`, a GPR named by vvvv) number exactly the entry's
///     distinct register and memory operands (`evex_operand_count`), and a
///     `src1` field appears only when there are three or more. Immediates do
///     not count. Bochs fetchdecode32.cc `assign_srcs` rejects an encoding
///     whose unused vvvv is not zero, so a two-operand entry has no second
///     source to name.
/// (b) a variant's `imm` fields number exactly the entry's immediates
///     (`evex_immediate_count`).
/// (c) a memory variant holds exactly one `MemoryOperand`: in `dst` when the
///     entry is a store (`evex_dst` is `EvexDst::Rm`: its first operand is
///     ModRM.rm and its source ModRM.reg), and otherwise in a source field
///     (`src`, `src1` or `src2`), so a register-only shape on an entry with a
///     memory form fails.
/// (d) a store's register variant reads ModRM.reg: in `src`, or in `src2`
///     when the entry lists three operands (`W, H, V`).
/// (e) a TILEMOVROW/TILEMOVCOL `Trm, Wdq` register variant
///     (`EvexDst::RmSourceRm`) writes and reads ModRM.rm.
#[test]
fn evex_typed_view_matches_the_bochs_operand_lists() {
    use crate::decoder::evex_operands::{evex_forms, evex_operand_count, EvexForms};
    use crate::opcode_isa::OPCODE_VARIANT_COUNT;
    use crate::typed::OpcodeCategory;

    let mut findings = Vec::new();
    let mut judged = 0usize;
    for index in 0..OPCODE_VARIANT_COUNT {
        let opcode = Opcode::from_u16_const(index as u16);
        if opcode.category() != OpcodeCategory::Evex {
            continue;
        }
        assert_ne!(
            evex_operand_count(opcode),
            0,
            "{opcode:?} has no EVEX def entry"
        );
        let forms: &[ModrmForm] = match evex_forms(opcode) {
            EvexForms::RegisterAndMemory => &[ModrmForm::Register, ModrmForm::Memory],
            EvexForms::Register => &[ModrmForm::Register],
            EvexForms::Memory => &[ModrmForm::Memory],
        };
        for &form in forms {
            findings.extend(typed_view_drift(opcode, form));
        }
        judged += 1;
    }
    assert!(judged > 1000, "only {judged} EVEX opcodes judged");

    let excused = |finding: &TypedDriftFinding| {
        TYPED_VIEW_EXCEPTIONS.iter().any(|exception| {
            exception.opcode == finding.opcode && exception.rules.contains(&finding.rule)
        })
    };
    let drifted: Vec<&TypedDriftFinding> = findings
        .iter()
        .filter(|finding| !excused(finding))
        .collect();

    let mut report = std::format!("{} typed-view findings:\n", drifted.len());
    for finding in &drifted {
        report.push_str(&std::format!(
            "  {:?} {:?} {:?}{}: {}\n",
            finding.opcode,
            finding.form,
            finding.rule,
            finding.detail,
            finding.typed
        ));
    }
    let mut stale = 0usize;
    for (index, exception) in TYPED_VIEW_EXCEPTIONS.iter().enumerate() {
        if exception.rules.is_empty() {
            stale += 1;
            report.push_str(&std::format!(
                "  exception {:?} names no rule, so it excuses nothing ({}); remove it\n",
                exception.opcode, exception.reason
            ));
        }
        for (rule_index, rule) in exception.rules.iter().enumerate() {
            let named_again = exception.rules[..rule_index].contains(rule)
                || TYPED_VIEW_EXCEPTIONS[..index].iter().any(|earlier| {
                    earlier.opcode == exception.opcode && earlier.rules.contains(rule)
                });
            if named_again {
                stale += 1;
                report.push_str(&std::format!(
                    "  exception {:?} names {:?} a second time ({}); remove the duplicate\n",
                    exception.opcode, rule, exception.reason
                ));
            }
        }
        for rule in exception.rules {
            let fires = findings
                .iter()
                .any(|finding| finding.opcode == exception.opcode && finding.rule == *rule);
            if !fires {
                stale += 1;
                report.push_str(&std::format!(
                    "  exception {:?} names {:?}, which it no longer breaks ({}); remove it\n",
                    exception.opcode,
                    rule,
                    exception.reason
                ));
            }
        }
    }
    assert!(drifted.is_empty() && stale == 0, "{report}");
}

/// The EVEX features Bochs's Skylake-X model enables (cpudb/intel/
/// corei7_skylake-x.cc `enable_cpu_extension`): AVX512F, AVX512DQ, AVX512CD
/// and AVX512BW. The decoder itself takes no feature mask; an opcode's gate
/// is `opcode_isa_feature`.
const SKYLAKE_X_EVEX_FEATURES: [crate::features::X86Feature; 4] = [
    crate::features::X86Feature::IsaAvx512,
    crate::features::X86Feature::IsaAvx512Dq,
    crate::features::X86Feature::IsaAvx512Cd,
    crate::features::X86Feature::IsaAvx512Bw,
];

/// ModRM.reg, ModRM.rm and EVEX.vvvv in the encodings of
/// `evex_typed_view_reads_each_operand_from_its_field`. All three are below
/// 8, so each encoding is valid in 32-bit protected mode too.
const ENC_REG: u8 = 1;
const ENC_RM: u8 = 2;
const ENC_VVVV: u8 = 3;
/// The immediate those encodings carry.
const ENC_IMM: u8 = 0x2A;

/// `bytes` decoded in 64-bit mode and in 32-bit protected mode, each required
/// to be all of `expected`, an opcode Skylake-X executes, then typed.
fn decode_evex(bytes: &[u8], expected: Opcode) -> [crate::typed::TypedInstruction; 2] {
    use crate::opcode_isa::opcode_isa_feature;

    assert!(
        SKYLAKE_X_EVEX_FEATURES
            .iter()
            .any(|feature| *feature as u16 == opcode_isa_feature(expected)),
        "{expected:?} is gated on a feature Skylake-X does not enable"
    );
    [fetch_decode64(bytes), fetch_decode32(bytes, true)].map(|decoded| {
        let instr = decoded.unwrap_or_else(|error| panic!("{bytes:02X?}: {error:?}"));
        assert_eq!(instr.get_ia_opcode(), expected, "{bytes:02X?}");
        assert_eq!(usize::from(instr.length), bytes.len(), "{bytes:02X?}");
        instr.typed()
    })
}

/// `operand` is `[esi/rsi + displacement]`, the scaled disp8 of the encodings.
fn assert_rsi_plus(operand: crate::typed::MemoryOperand, displacement: i32) {
    use crate::instruction::GprIndex;
    assert_eq!(operand.base, GprIndex::Rsi, "{operand:?}");
    assert_eq!(operand.index, GprIndex::Rsp, "no index: {operand:?}");
    assert_eq!(operand.displacement, displacement, "{operand:?}");
}

/// The typed view reads each operand from the field the decoder put it in,
/// on real encodings decoded in 64-bit and 32-bit mode: one per macro shape,
/// each with the operands Bochs ia_opcodes_evex.def lists. The encodings use
/// reg = 1, rm = 2, vvvv = 3 and imm 0x2A; a memory form is `[rsi + 1*N]`,
/// disp8 1 scaled by the opcode's tuple size N.
///
/// The GPR-from-vvvv shape (`ctrl`, Bochs `OP_Bd`) exists only on the AMX
/// row ops, which Bochs's EVEX opcode map emits under `#if BX_SUPPORT_AMX`
/// and this port's decode tables do not emit at all, so that one is typed
/// from a hand-built instruction (`evex_instruction`).
#[test]
fn evex_typed_view_reads_each_operand_from_its_field() {
    use crate::instruction::GprIndex;
    use crate::typed::TypedInstruction as T;

    // simd!: VMOVDQU32 zmm1{k1}, zmm2/m512 (EVEX.512.F3.0F.W0 6F).
    for typed in decode_evex(
        &[0x62, 0xF1, 0x7E, 0x49, 0x6F, 0xCA],
        Opcode::EvexVmovdqu32VdqWdqKmask,
    ) {
        match typed {
            T::EvexVmovdqu32VdqWdqKmaskR { dst, src } => {
                assert_eq!(dst, ENC_REG, "the destination is reg");
                assert_eq!(src, ENC_RM, "the source is rm");
            }
            other => panic!("VMOVDQU32 load typed as {other:?}"),
        }
    }
    for typed in decode_evex(
        &[0x62, 0xF1, 0x7E, 0x49, 0x6F, 0x4E, 0x01],
        Opcode::EvexVmovdqu32VdqWdqKmask,
    ) {
        match typed {
            T::EvexVmovdqu32VdqWdqKmaskM { dst, src } => {
                assert_eq!(dst, ENC_REG, "the destination is reg");
                assert_rsi_plus(src, 64);
            }
            other => panic!("VMOVDQU32 load from memory typed as {other:?}"),
        }
    }
    // simd_st!: VMOVDQU32 zmm2/m512{k1}, zmm1 (EVEX.512.F3.0F.W0 7F).
    for typed in decode_evex(
        &[0x62, 0xF1, 0x7E, 0x49, 0x7F, 0xCA],
        Opcode::EvexVmovdqu32WdqVdqKmask,
    ) {
        match typed {
            T::EvexVmovdqu32WdqVdqKmaskR { dst, src } => {
                assert_eq!(dst, ENC_RM, "the destination is rm");
                assert_eq!(src, ENC_REG, "the source is reg");
            }
            other => panic!("VMOVDQU32 store typed as {other:?}"),
        }
    }
    for typed in decode_evex(
        &[0x62, 0xF1, 0x7E, 0x49, 0x7F, 0x4E, 0x01],
        Opcode::EvexVmovdqu32WdqVdqKmask,
    ) {
        match typed {
            T::EvexVmovdqu32WdqVdqKmaskM { dst, src } => {
                assert_rsi_plus(dst, 64);
                assert_eq!(src, ENC_REG, "the source is reg");
            }
            other => panic!("VMOVDQU32 store to memory typed as {other:?}"),
        }
    }
    // simd_dst_gpr!: VPBROADCASTD zmm1{k1}, edx (EVEX.512.66.0F38.W0 7C).
    for typed in decode_evex(
        &[0x62, 0xF2, 0x7D, 0x49, 0x7C, 0xCA],
        Opcode::EvexVpbroadcastdVdqEdKmask,
    ) {
        match typed {
            T::EvexVpbroadcastdVdqEdKmaskR { dst, src } => {
                assert_eq!(dst, ENC_REG, "the destination is reg");
                assert_eq!(src, GprIndex::Rdx, "the GPR is rm");
            }
            other => panic!("VPBROADCASTD from a GPR typed as {other:?}"),
        }
    }
    // simd_ib!: VPSHUFD zmm1{k1}, zmm2/m512, imm8 (EVEX.512.66.0F.W0 70).
    for typed in decode_evex(
        &[0x62, 0xF1, 0x7D, 0x49, 0x70, 0xCA, ENC_IMM],
        Opcode::EvexVpshufdVdqWdqIbKmask,
    ) {
        match typed {
            T::EvexVpshufdVdqWdqIbKmaskR { dst, src, imm } => {
                assert_eq!(dst, ENC_REG, "the destination is reg");
                assert_eq!(src, ENC_RM, "the source is rm");
                assert_eq!(imm, ENC_IMM);
            }
            other => panic!("VPSHUFD typed as {other:?}"),
        }
    }
    for typed in decode_evex(
        &[0x62, 0xF1, 0x7D, 0x49, 0x70, 0x4E, 0x01, ENC_IMM],
        Opcode::EvexVpshufdVdqWdqIbKmask,
    ) {
        match typed {
            T::EvexVpshufdVdqWdqIbKmaskM { dst, src, imm } => {
                assert_eq!(dst, ENC_REG, "the destination is reg");
                assert_rsi_plus(src, 64);
                assert_eq!(imm, ENC_IMM);
            }
            other => panic!("VPSHUFD from memory typed as {other:?}"),
        }
    }
    // Shift by immediate: VPSRLD zmm3, zmm2/m512, imm8 (EVEX.512.66.0F.W0
    // 72 /2), whose destination is vvvv (Bochs `OP_Hdq`).
    for typed in decode_evex(
        &[0x62, 0xF1, 0x65, 0x48, 0x72, 0xD2, ENC_IMM],
        Opcode::EvexVpsrldUdqIb,
    ) {
        match typed {
            T::EvexVpsrldUdqIbR { dst, src, imm } => {
                assert_eq!(dst, ENC_VVVV, "the destination is vvvv");
                assert_eq!(src, ENC_RM, "the source is rm");
                assert_eq!(imm, ENC_IMM);
            }
            other => panic!("VPSRLD by immediate typed as {other:?}"),
        }
    }
    for typed in decode_evex(
        &[0x62, 0xF1, 0x65, 0x48, 0x72, 0x56, 0x01, ENC_IMM],
        Opcode::EvexVpsrldUdqIb,
    ) {
        match typed {
            T::EvexVpsrldUdqIbM { dst, src, imm } => {
                assert_eq!(dst, ENC_VVVV, "the destination is vvvv");
                assert_rsi_plus(src, 64);
                assert_eq!(imm, ENC_IMM);
            }
            other => panic!("VPSRLD by immediate from memory typed as {other:?}"),
        }
    }
    for typed in decode_evex(
        &[0x62, 0xF1, 0x65, 0x49, 0x72, 0xD2, ENC_IMM],
        Opcode::EvexVpsrldUdqIbKmask,
    ) {
        match typed {
            T::EvexVpsrldUdqIbKmaskR { dst, src, imm } => {
                assert_eq!(dst, ENC_VVVV, "the destination is vvvv");
                assert_eq!(src, ENC_RM, "the source is rm");
                assert_eq!(imm, ENC_IMM);
            }
            other => panic!("masked VPSRLD by immediate typed as {other:?}"),
        }
    }
    // simd_st_ib!: VEXTRACTF32X4 xmm2/m128{k1}, zmm1, imm8
    // (EVEX.512.66.0F3A.W0 19); N is 16.
    for typed in decode_evex(
        &[0x62, 0xF3, 0x7D, 0x49, 0x19, 0xCA, 0x01],
        Opcode::EvexVextractf32x4WpsVpsIbKmask,
    ) {
        match typed {
            T::EvexVextractf32x4WpsVpsIbKmaskR { dst, src, imm } => {
                assert_eq!(dst, ENC_RM, "the destination is rm");
                assert_eq!(src, ENC_REG, "the source is reg");
                assert_eq!(imm, 1);
            }
            other => panic!("VEXTRACTF32X4 typed as {other:?}"),
        }
    }
    for typed in decode_evex(
        &[0x62, 0xF3, 0x7D, 0x49, 0x19, 0x4E, 0x01, 0x01],
        Opcode::EvexVextractf32x4WpsVpsIbKmask,
    ) {
        match typed {
            T::EvexVextractf32x4WpsVpsIbKmaskM { dst, src, imm } => {
                assert_rsi_plus(dst, 16);
                assert_eq!(src, ENC_REG, "the source is reg");
                assert_eq!(imm, 1);
            }
            other => panic!("VEXTRACTF32X4 to memory typed as {other:?}"),
        }
    }
    for typed in decode_evex(
        &[0x62, 0xF3, 0x7D, 0x48, 0x19, 0x4E, 0x01, 0x01],
        Opcode::EvexVextractf32x4WpsVpsIb,
    ) {
        match typed {
            T::EvexVextractf32x4WpsVpsIbM { dst, src, imm } => {
                assert_rsi_plus(dst, 16);
                assert_eq!(src, ENC_REG, "the source is reg");
                assert_eq!(imm, 1);
            }
            other => panic!("unmasked VEXTRACTF32X4 to memory typed as {other:?}"),
        }
    }
    // simd3_ib!: VCMPPS k1, zmm3, zmm2/m512, imm8 (EVEX.512.0F.W0 C2).
    for typed in decode_evex(
        &[0x62, 0xF1, 0x64, 0x48, 0xC2, 0xCA, ENC_IMM],
        Opcode::EvexVcmppsKgwHpsWpsIb,
    ) {
        match typed {
            T::EvexVcmppsKgwHpsWpsIbR {
                dst,
                src1,
                src2,
                imm,
            } => {
                assert_eq!(dst, ENC_REG, "the k register is reg");
                assert_eq!(src1, ENC_VVVV, "the first source is vvvv");
                assert_eq!(src2, ENC_RM, "the second source is rm");
                assert_eq!(imm, ENC_IMM);
            }
            other => panic!("VCMPPS typed as {other:?}"),
        }
    }
    for typed in decode_evex(
        &[0x62, 0xF1, 0x64, 0x48, 0xC2, 0x4E, 0x01, ENC_IMM],
        Opcode::EvexVcmppsKgwHpsWpsIb,
    ) {
        match typed {
            T::EvexVcmppsKgwHpsWpsIbM {
                dst,
                src1,
                src2,
                imm,
            } => {
                assert_eq!(dst, ENC_REG, "the k register is reg");
                assert_eq!(src1, ENC_VVVV, "the first source is vvvv");
                assert_rsi_plus(src2, 64);
                assert_eq!(imm, ENC_IMM);
            }
            other => panic!("VCMPPS with memory typed as {other:?}"),
        }
    }
    // simd3_ib! on a masked form: VPTERNLOGD zmm1{k1}, zmm3, zmm2/m512, imm8
    // (EVEX.512.66.0F3A.W0 25).
    for typed in decode_evex(
        &[0x62, 0xF3, 0x65, 0x49, 0x25, 0xCA, ENC_IMM],
        Opcode::EvexVpternlogdVdqHdqWdqIbKmask,
    ) {
        match typed {
            T::EvexVpternlogdVdqHdqWdqIbKmaskR {
                dst,
                src1,
                src2,
                imm,
            } => {
                assert_eq!(dst, ENC_REG, "the destination is reg");
                assert_eq!(src1, ENC_VVVV, "the first source is vvvv");
                assert_eq!(src2, ENC_RM, "the second source is rm");
                assert_eq!(imm, ENC_IMM);
            }
            other => panic!("VPTERNLOGD typed as {other:?}"),
        }
    }
    // simd3!: VCVTSS2SD xmm1, xmm3, xmm2/m32 (EVEX.LIG.F3.0F.W0 5A); N is 4.
    for typed in decode_evex(
        &[0x62, 0xF1, 0x66, 0x08, 0x5A, 0x4E, 0x01],
        Opcode::EvexVcvtss2sdVsdWss,
    ) {
        match typed {
            T::EvexVcvtss2sdVsdWssM { dst, src1, src2 } => {
                assert_eq!(dst, ENC_REG, "the destination is reg");
                assert_eq!(src1, ENC_VVVV, "the merged vector is vvvv");
                assert_rsi_plus(src2, 4);
            }
            other => panic!("VCVTSS2SD typed as {other:?}"),
        }
    }
    // simd3_gpr!: VCVTSI2SS xmm1, xmm3, edx (EVEX.LIG.F3.0F.W0 2A).
    for typed in decode_evex(
        &[0x62, 0xF1, 0x66, 0x08, 0x2A, 0xCA],
        Opcode::EvexVcvtsi2ssVssEd,
    ) {
        match typed {
            T::EvexVcvtsi2ssVssEdR { dst, src1, src2 } => {
                assert_eq!(dst, ENC_REG, "the destination is reg");
                assert_eq!(src1, ENC_VVVV, "the merged vector is vvvv");
                assert_eq!(src2, GprIndex::Rdx, "the GPR is rm");
            }
            other => panic!("VCVTSI2SS typed as {other:?}"),
        }
    }
    // simd3_gpr_ib!: VPINSRD xmm1, xmm3, edx, imm8 (EVEX.128.66.0F3A.W0 22).
    for typed in decode_evex(
        &[0x62, 0xF3, 0x65, 0x08, 0x22, 0xCA, ENC_IMM],
        Opcode::EvexVpinsrdVdqEdIb,
    ) {
        match typed {
            T::EvexVpinsrdVdqEdIbR {
                dst,
                src1,
                src2,
                imm,
            } => {
                assert_eq!(dst, ENC_REG, "the destination is reg");
                assert_eq!(src1, ENC_VVVV, "the merged vector is vvvv");
                assert_eq!(src2, GprIndex::Rdx, "the GPR is rm");
                assert_eq!(imm, ENC_IMM);
            }
            other => panic!("VPINSRD typed as {other:?}"),
        }
    }
    // VPEXTRW ecx, xmm2, imm8 (EVEX.128.66.0F.WIG C5): the GPR is reg.
    for typed in decode_evex(
        &[0x62, 0xF1, 0x7D, 0x08, 0xC5, 0xCA, ENC_IMM],
        Opcode::EvexVpextrwGdUdqIb,
    ) {
        match typed {
            T::EvexVpextrwGdUdqIb { dst, src, imm } => {
                assert_eq!(dst, GprIndex::Rcx, "the GPR is reg");
                assert_eq!(src, ENC_RM, "the vector is rm");
                assert_eq!(imm, ENC_IMM);
            }
            other => panic!("VPEXTRW to a GPR typed as {other:?}"),
        }
    }
    // simd_ctrl_gpr!, hand-built (see above): TILEMOVROW Vdq, Trm, Bd with
    // reg = 5, rm = 6, vvvv = 7; the row GPR is vvvv.
    match evex_instruction(Opcode::EvexTilemovrowVdqTrmBd, ModrmForm::Register).typed() {
        T::EvexTilemovrowVdqTrmBdR { dst, src, ctrl } => {
            assert_eq!(dst, TYPED_NNN, "the vector is reg");
            assert_eq!(src, TYPED_RM, "the tile is rm");
            assert_eq!(ctrl, GprIndex::Rdi, "the row GPR is vvvv");
        }
        other => panic!("TILEMOVROW Vdq, Trm, Bd typed as {other:?}"),
    }
}
