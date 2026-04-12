// Test for FAR JMP decoder fix
use rusty_box_decoder::fetch_decode32;

#[test]
fn test_far_jmp_16bit() {
    // Test 16-bit FAR JMP: EA 5B E0 00 F0
    // Opcode: EA, Offset: 0xE05B, Segment: 0xF000
    let bytes = [0xEA, 0x5B, 0xE0, 0x00, 0xF0];

    let instr = fetch_decode32(&bytes, false).expect("Failed to decode FAR JMP");

    // Verify instruction properties
    assert_eq!(instr.ilen(), 5, "Instruction length should be 5 bytes");
    assert_eq!(instr.iw(), 0xE05B, "Offset should be 0xE05B");
    assert_eq!(instr.iw2(), 0xF000, "Segment should be 0xF000");

    println!("✅ 16-bit FAR JMP decoded correctly:");
    println!("   Offset: {:#x}, Segment: {:#x}", instr.iw(), instr.iw2());
}

#[test]
fn test_far_jmp_32bit() {
    // Test 32-bit FAR JMP: EA 5B E0 00 00 00 F0
    // Opcode: EA, Offset (4 bytes LE): 0x0000E05B, Segment (2 bytes LE): 0xF000
    let bytes = [0xEA, 0x5B, 0xE0, 0x00, 0x00, 0x00, 0xF0];

    let instr = fetch_decode32(&bytes, true).expect("Failed to decode 32-bit FAR JMP");

    // Verify instruction properties
    assert_eq!(instr.ilen(), 7, "Instruction length should be 7 bytes");
    assert_eq!(instr.id(), 0x0000E05B, "Offset should be 0x0000E05B");
    assert_eq!(instr.iw2(), 0xF000, "Segment should be 0xF000");

    println!("✅ 32-bit FAR JMP decoded correctly:");
    println!("   Offset: {:#x}, Segment: {:#x}", instr.id(), instr.iw2());
}
