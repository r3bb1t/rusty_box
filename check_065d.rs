use std::fs;

fn main() {
    let bios = fs::read("cpp_orig/bochs/bios/BIOS-bochs-latest").expect("Failed to read BIOS");
    
    // F000:065D = ROM offset 0x1065D
    let offset = 0x1065D;
    
    println!("Instructions around F000:065D (ROM offset {:#x}):", offset);
    for addr in (offset-10..=offset+10).step_by(1) {
        if addr % 16 == 0 {
            println!();
            print!("{:05x}: ", addr);
        }
        print!("{:02x} ", bios[addr]);
    }
    println!("\n");
}
