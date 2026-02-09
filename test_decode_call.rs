use rusty_box_decoder::*;

fn main() {
    // Test: FF D0 = CALL EAX (register indirect call)
    let bytes = [0xFF, 0xD0];

    match fetch_decode32_chatgpt_generated_instr(&bytes, true) {
        Ok(instr) => {
            println!("✅ Decoded successfully:");
            println!("  Opcode: {:?}", instr.get_ia_opcode());
            println!("  Length (ilen): {}", instr.ilen());
            println!("  Expected length: 2 bytes");
            println!();
            if instr.ilen() == 2 {
                println!("✓ Length is CORRECT");
            } else {
                println!("✗ Length is WRONG! Should be 2, got {}", instr.ilen());
            }
        }
        Err(e) => {
            println!("❌ Decode error: {:?}", e);
        }
    }

    // Also test: B8 00 00 0E 00 = MOV EAX, 0xE0000
    println!("\n---\n");
    let bytes2 = [0xB8, 0x00, 0x00, 0x0E, 0x00];
    match fetch_decode32_chatgpt_generated_instr(&bytes2, true) {
        Ok(instr) => {
            println!("✅ MOV EAX decoded:");
            println!("  Opcode: {:?}", instr.get_ia_opcode());
            println!("  Length (ilen): {}", instr.ilen());
            println!("  Immediate: {:#x}", instr.id());
            println!("  Expected length: 5 bytes");
            println!();
            if instr.ilen() == 5 {
                println!("✓ Length is CORRECT");
            } else {
                println!("✗ Length is WRONG! Should be 5, got {}", instr.ilen());
            }
        }
        Err(e) => {
            println!("❌ Decode error: {:?}", e);
        }
    }
}
