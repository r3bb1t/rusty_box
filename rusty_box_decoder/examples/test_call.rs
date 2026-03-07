use rusty_box_decoder::fetchdecode32::fetch_decode32;

fn main() {
    // Test: FF D0 = CALL EAX (register indirect call)
    let bytes = [0xFF, 0xD0, 0x90, 0x90]; // padding with NOPs

    match fetch_decode32(&bytes, true) {
        Ok(instr) => {
            println!("✅ FF D0 decoded successfully:");
            println!("  Opcode: {:?}", instr.get_ia_opcode());
            println!("  Length (ilen): {}", instr.ilen());
            println!("  Expected: 2 bytes");

            if instr.ilen() == 2 {
                println!("  ✓ CORRECT");
            } else {
                println!("  ✗ WRONG! Decoder bug!");
            }
        }
        Err(e) => {
            println!("❌ Decode error: {:?}", e);
        }
    }
}
