use std::fs;

fn main() {
    let bios = fs::read("cpp_orig/bochs/bios/BIOS-bochs-latest").expect("Failed to read BIOS");
    
    // _start is at address 0xE0000
    // ROM offset: 0xE0000 - 0xFFFE0000 + 0x100000000 
    // In 128KB ROM (0x20000 bytes): offset = 0xE0000 & 0x1FFFF = 0x0
    
    println!("_start function at ROM offset 0x0 (address 0xE0000):");
    println!();
    
    // Print first 64 bytes
    for i in 0..64 {
        if i % 16 == 0 {
            print!("{:05x}: ", i);
        }
        print!("{:02x} ", bios[i]);
        if i % 16 == 15 {
            println!();
        }
    }
    println!();
    
    println!("Expected _start (from rombios32start.S):");
    println!("  31 c0                xor eax, eax");
    println!("  bf 10 07 00 00       mov edi, 0x710      ; __bss_start");
    println!("  b9 68 07 00 00       mov ecx, 0x768      ; __bss_end");
    println!("  29 f9                sub ecx, edi");
    println!("  f3 aa                rep stosb");
    println!();
    println!("  be 6f 41 0e 00       mov esi, 0xE416F    ; _end");
    println!("  bf 0c 07 00 00       mov edi, 0x70C      ; __data_start");
}
