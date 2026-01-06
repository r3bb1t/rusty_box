#[cfg(test)]
mod tests {
    extern crate alloc;
    use alloc::{format, vec::Vec};

    use crate::cpu::decoder::{
        fetchdecode32::fetch_decode32_chatgpt_generated_instr, fetchdecode64::fetch_decode64, ia_opcodes::Opcode, instr_generated::BxInstructionGenerated
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
    fn format_instruction(address: u64, instr: &BxInstructionGenerated) -> alloc::string::String {
        let opcode_name = format!("{:?}", instr.get_ia_opcode());
        let length = instr.meta_info.ilen;

        // Format address as 16 hex digits
        format!("{:016X}  {} (len={})", address, opcode_name, length)
    }

    /// Disassemble a sequence of instructions from a byte buffer
    ///
    /// Similar to Zydis example: loops over instructions in buffer and prints them
    fn disassemble_sequence(
        data: &[u8],
        runtime_address: u64,
        is_32: bool,
    ) -> Vec<(u64, BxInstructionGenerated)> {
        let mut offset = 0;
        let mut current_address = runtime_address;
        let mut instructions = Vec::new();

        while offset < data.len() {
            let remaining = &data[offset..];

            match fetch_decode32_chatgpt_generated_instr(remaining, is_32) {
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
            0x48, 0x31, 0xff, 0x48, 0x31, 0xf6, 0x48, 0x31, 0xd2, 0x48, 0x31, 0xc0, 0x50, 0x48,
            0xbb, 0x2f, 0x62, 0x69, 0x6e, 0x2f, 0x2f, 0x73, 0x68, 0x53, 0x48, 0x89, 0xe7, 0xb0,
            0x3b, 0x0f, 0x05,
        ];

        let runtime_address = 0x007FFFFFFF400000;
        let instructions = disassemble_sequence(&data, runtime_address, false);

        for (len, instruction) in instructions {
            tracing::info!("{:?} {} {}", instruction.get_ia_opcode(), instruction.dst(), instruction.src());
        }
    }
    #[test]
    fn test_zydis_example_64bit() {
        init_tracing();
        let data = [
            0x48, 0x31, 0xff, 0x48, 0x31, 0xf6, 0x48, 0x31, 0xd2, 0x48, 0x31, 0xc0, 0x50, 0x48,
            0xbb, 0x2f, 0x62, 0x69, 0x6e, 0x2f, 0x2f, 0x73, 0x68, 0x53, 0x48, 0x89, 0xe7, 0xb0,
            0x3b, 0x0f, 0x05,
        ];

        let runtime_address = 0x007FFFFFFF400000;
        let instructions = disassemble_sequence_64bit(&data, runtime_address, false);

        for (len, instruction) in instructions {
            tracing::info!("{:?} {} {}", instruction.get_ia_opcode(), instruction.dst(), instruction.src());
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
        is_32: bool,
    ) -> Vec<(u64, BxInstructionGenerated)> {
        let mut offset = 0;
        let mut current_address = runtime_address;
        let mut instructions = Vec::new();

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
}
