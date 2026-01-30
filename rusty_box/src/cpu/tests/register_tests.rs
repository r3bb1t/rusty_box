// Test cases for register operations to investigate stack corruption bug

#[cfg(test)]
mod register_preservation_tests {
    use crate::cpu::{BxCpuC, cpuid::Corei7SkylakeX};
    use crate::memory::BxMemC;

    /// Helper: Create a minimal CPU for testing
    fn create_test_cpu() -> BxCpuC<Corei7SkylakeX> {
        // This is a simplified version - actual implementation would need proper initialization
        unimplemented!("TODO: Implement test CPU creation")
    }

    #[test]
    #[ignore] // Ignored until test infrastructure is set up
    fn test_set_gpr16_preserves_upper_bits() {
        let mut cpu = create_test_cpu();

        // Set EAX to 0xF0000000 (upper bits set)
        unsafe {
            cpu.gen_reg[0].rrx = 0xF0000000;
        }
        assert_eq!(cpu.get_gpr32(0), 0xF0000000, "Initial setup failed");

        // Set AX to 0xFF53 (should only modify lower 16 bits)
        cpu.set_gpr16(0, 0xFF53);

        // Verify: EAX should be 0xF000FF53 (upper 16 bits preserved)
        let eax = cpu.get_gpr32(0);
        assert_eq!(
            eax, 0xF000FF53,
            "Upper 16 bits NOT preserved! Got {:#010x}, expected 0xF000FF53",
            eax
        );

        // Also verify lower 16 bits are correct
        assert_eq!(cpu.get_gpr16(0), 0xFF53, "Lower 16 bits incorrect");
    }

    #[test]
    #[ignore]
    fn test_register_union_layout() {
        let mut cpu = create_test_cpu();

        // Test 1: Setting 64-bit value
        unsafe {
            cpu.gen_reg[0].rrx = 0x0123456789ABCDEF;
        }

        // Verify we can read it back correctly
        assert_eq!(unsafe { cpu.gen_reg[0].rrx }, 0x0123456789ABCDEF);

        // Test 2: Read as 32-bit (should get lower 32 bits)
        assert_eq!(
            unsafe { cpu.gen_reg[0].dword.erx },
            0x89ABCDEF,
            "32-bit lower dword read failed"
        );

        // Test 3: Read as 16-bit (should get lower 16 bits)
        assert_eq!(
            unsafe { cpu.gen_reg[0].word.rx },
            0xCDEF,
            "16-bit word read failed"
        );

        // Test 4: Write to 16-bit field, verify upper bits unchanged
        unsafe {
            cpu.gen_reg[0].word.rx = 0x5555;
        }

        assert_eq!(
            unsafe { cpu.gen_reg[0].rrx },
            0x0123456789AB5555,
            "Writing 16-bit modified upper bits!"
        );
    }

    #[test]
    #[ignore]
    fn test_shl_32bit_operand_16bit_mode() {
        let mut cpu = create_test_cpu();

        // Set EAX to 0x0000F000
        cpu.set_gpr32(0, 0x0000F000);

        // Manually create instruction: SHL EAX, 0x10
        // Bytes: 66 C1 E0 10
        // This would normally be created by the decoder
        // For now, just test the shift operation directly

        let initial = cpu.get_gpr32(0);
        let count = 0x10u32; // 16 bits
        let result = initial << count;

        cpu.set_gpr32(0, result);

        assert_eq!(
            cpu.get_gpr32(0),
            0xF0000000,
            "SHL EAX, 0x10 failed! Got {:#010x}",
            cpu.get_gpr32(0)
        );
    }
}

// Test cases specifically for the A124 function behavior
#[cfg(test)]
mod bios_a124_sequence_tests {
    use super::*;

    #[test]
    #[ignore]
    fn test_a124_instruction_sequence() {
        let mut cpu = create_test_cpu();

        // Simulate the exact sequence at F000:A124

        // 1. MOV AX, 0xF000
        cpu.set_gpr16(0, 0xF000);
        assert_eq!(cpu.get_gpr32(0), 0x0000F000, "Step 1 failed");

        // 2. SHL EAX, 0x10
        let eax = cpu.get_gpr32(0);
        cpu.set_gpr32(0, eax << 16);
        assert_eq!(cpu.get_gpr32(0), 0xF0000000, "Step 2 (SHL) failed");

        // 3. MOV AX, 0xFF53
        cpu.set_gpr16(0, 0xFF53);
        let final_eax = cpu.get_gpr32(0);

        assert_eq!(
            final_eax, 0xF000FF53,
            "Step 3 (MOV AX) failed! Got {:#010x}, expected 0xF000FF53",
            final_eax
        );

        // This is the value that should be written by STOSD
        println!("Final EAX value: {:#010x}", final_eax);
    }
}
