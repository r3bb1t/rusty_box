use std::fs;

fn main() {
    let bios = fs::read("cpp_orig/bochs/bios/BIOS-bochs-latest").expect("Failed to read BIOS");

    // BIOS-bochs-latest is 128KB starting at 0xFFFE0000
    // Address 0xF9E5F maps to ROM offset: 0xF9E5F - 0xFFFE0000 + 0x100000000
    // Actually simpler: 0xF9E5F in 32-bit wraps, so offset = 0xF9E5F & 0x1FFFF = 0x19E5F

    let offset_19e5f = 0x19E5F;

    println!("Checking rombios32_05 at ROM offset 0x{:x} (address 0xF9E5F):", offset_19e5f);
    println!();

    // Print 64 bytes starting from rombios32_05
    for i in 0..64 {
        if i % 16 == 0 {
            print!("{:05x}: ", offset_19e5f + i);
        }
        if offset_19e5f + i < bios.len() {
            print!("{:02x} ", bios[offset_19e5f + i]);
        }
        if i % 16 == 15 {
            println!();
        }
    }
    println!();

    // The sequence should be (from rombios.c):
    // push 0x4B0
    // push 0x4B2
    // mov eax, 0xE0000
    // call eax

    println!("Expected sequence:");
    println!("  68 b0 04 00 00       push 0x4B0");
    println!("  68 b2 04 00 00       push 0x4B2");
    println!("  b8 00 00 0e 00       mov eax, 0xE0000");
    println!("  ff d0                call eax");
}
