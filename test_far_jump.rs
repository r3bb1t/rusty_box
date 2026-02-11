// Quick test for FAR JMP decoder fix
use rusty_box_decoder::fetch_decode32;

fn main() {
    // Test 16-bit FAR JMP: EA 5B E0 00 F0
    // Opcode: EA, Offset: 0xE05B, Segment: 0xF000
    let bytes = [0xEA, 0x5B, 0xE0, 0x00, 0xF0];

    match fetch_decode32(&bytes, false) {  // is_32 = false for 16-bit mode
        Ok(instr) => {
            println!("✅ SUCCESS! FAR JMP decoded:");
            println!("   Opcode: {:?}", instr.get_ia_opcode());
            println!("   Length: {} bytes", instr.ilen());
            println!("   Offset (iw): {:#x}", instr.iw());
            println!("   Segment (iw2): {:#x}", instr.iw2());

            // Verify values
            assert_eq!(instr.ilen(), 5, "Instruction length should be 5 bytes");
            assert_eq!(instr.iw(), 0xE05B, "Offset should be 0xE05B");
            assert_eq!(instr.iw2(), 0xF000, "Segment should be 0xF000");

            println!("\n✅ All assertions passed!");
        }
        Err(e) => {
            println!("❌ FAILED: {:?}", e);
            std::process::exit(1);
        }
    }
}
