#[cfg(test)]
mod test_call_decode {
    use crate::fetchdecode32::fetch_decode32;
    use crate::ia_opcodes::Opcode;

    #[test]
    fn test_call_eax_length() {
        // FF D0 = CALL EAX (2 bytes: opcode + ModRM)
        let bytes = [0xFF, 0xD0, 0x90, 0x90]; // padding with NOPs

        match fetch_decode32(&bytes, true) {
            Ok(instr) => {
                println!("Opcode: {:?}", instr.get_ia_opcode());
                println!("Length: {}", instr.ilen());
                println!("Expected: 2 bytes");

                // Verify length is 2
                assert_eq!(instr.ilen(), 2, "CALL EAX should be 2 bytes, got {}", instr.ilen());
            }
            Err(e) => {
                panic!("Decode failed: {:?}", e);
            }
        }
    }

    #[test]
    fn test_mov_eax_imm32_length() {
        // B8 00 00 0E 00 = MOV EAX, 0xE0000 (5 bytes)
        let bytes = [0xB8, 0x00, 0x00, 0x0E, 0x00, 0x90];

        match fetch_decode32(&bytes, true) {
            Ok(instr) => {
                println!("Opcode: {:?}", instr.get_ia_opcode());
                println!("Length: {}", instr.ilen());
                println!("Immediate: {:#x}", instr.id());

                assert_eq!(instr.ilen(), 5, "MOV EAX, imm32 should be 5 bytes");
                assert_eq!(instr.id(), 0xE0000, "Immediate should be 0xE0000");
            }
            Err(e) => {
                panic!("Decode failed: {:?}", e);
            }
        }
    }
}
