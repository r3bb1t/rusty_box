use std::fs;

fn main() {
    let bios = fs::read("cpp_orig/bochs/bios/BIOS-bochs-latest").expect("Failed to read BIOS");
    
    // F000:0019 in real mode = linear 0xF0019
    // ROM offset: 0xF0019 & 0x1FFFF = 0x10019
    let offset = 0x10019;
    
    println!("Instruction at F000:0019 (ROM offset {:#x}):", offset);
    for i in 0..16 {
        print!("{:02x} ", bios[offset + i]);
    }
    println!();
    
    // Also check 0x0, 0x15
    println!("\nInstruction at F000:0000 (ROM offset {:#x}):", 0x10000);
    for i in 0..16 {
        print!("{:02x} ", bios[0x10000 + i]);
    }
    println!();
    
    println!("\nInstruction at F000:0015 (ROM offset {:#x}):", 0x10015);
    for i in 0..16 {
        print!("{:02x} ", bios[0x10015 + i]);
    }
    println!();
}
