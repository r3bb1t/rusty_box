use std::fs;

fn main() {
    let bios = fs::read("cpp_orig/bochs/bios/BIOS-bochs-latest").expect("Failed to read BIOS");

    println!("BIOS size: {} bytes (0x{:x})", bios.len(), bios.len());
    println!();

    // The first bytes of the BIOS ROM map to address 0xE0000 (offset 0 in ROM)
    // This should contain the _start function
    println!("First 64 bytes at offset 0x0000 (maps to 0xE0000 - _start function):");
    for i in 0..64 {
        if i % 16 == 0 {
            print!("{:04x}: ", i);
        }
        print!("{:02x} ", bios[i]);
        if i % 16 == 15 {
            println!();
        }
    }
    println!();

    // Disassemble the key instructions
    println!("Expected _start function (from rombios32start.S):");
    println!("  31 c0                xor eax, eax");
    println!("  bf 10 07 00 00       mov edi, 0x710      ; __bss_start");
    println!("  b9 68 07 00 00       mov ecx, 0x768      ; __bss_end");
    println!("  29 f9                sub ecx, edi");
    println!("  f3 aa                rep stosb");
    println!();
    println!("  be 6f 41 0e 00       mov esi, 0xE416F    ; _end (data in ROM)");
    println!("  bf 0c 07 00 00       mov edi, 0x70C      ; __data_start (RAM)");
    println!("  b9 0c 07 00 00       mov ecx, 0x70C      ; size");
    println!("  29 f9                sub ecx, edi");
    println!("  f3 a4                rep movsb");
    println!();

    // Check what we actually have
    println!("Actual bytes in BIOS ROM:");
    if bios.len() >= 32 {
        let bytes = &bios[0..32];
        println!("  {:02x} {:02x}                xor eax, ...", bytes[0], bytes[1]);
        if bytes.len() >= 7 {
            println!("  {:02x} {:02x} {:02x} {:02x} {:02x}       mov edi, 0x{:02x}{:02x}{:02x}{:02x}",
                bytes[2], bytes[3], bytes[4], bytes[5], bytes[6],
                bytes[6], bytes[5], bytes[4], bytes[3]);
        }
        if bytes.len() >= 12 {
            println!("  {:02x} {:02x} {:02x} {:02x} {:02x}       mov ecx, 0x{:02x}{:02x}{:02x}{:02x}",
                bytes[7], bytes[8], bytes[9], bytes[10], bytes[11],
                bytes[11], bytes[10], bytes[9], bytes[8]);
        }
    }
}
