use rusty_box_decoder::fetch_decode32_chatgpt_generated_instr;

fn main() {
    // 0x66 0xEA <offset32> <selector16> - 32-bit far jump in 16-bit mode
    // From rombios.c: db 0x66, 0xea / dw rombios32_05 / dw 0x000f / dw 0x0010
    // Assuming rombios32_05 = 0x0BF1 (based on where we land)
    let bytes = [0x66, 0xEA, 0xF1, 0x0B, 0x0F, 0x00, 0x10, 0x00];
    
    match fetch_decode32_chatgpt_generated_instr(&bytes, false) {
        Ok(instr) => {
            println!("Opcode: {:?}", instr.get_ia_opcode());
            println!("Length: {}", instr.ilen());
            println!("Iw: {:#x}", instr.iw());
            println!("Iw2: {:#x}", instr.iw2());
            println!("Id: {:#x}", instr.id());
        }
        Err(e) => println!("Decode error: {:?}", e),
    }
}
