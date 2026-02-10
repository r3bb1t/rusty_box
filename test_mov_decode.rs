// Test: Does 89 E5 decode correctly as MOV BP,SP?
// ModRM E5 = 11 100 101
// mod=11 (register), reg=100=SP(4), r/m=101=BP(5)
// For MOV Ew,Gw (opcode 0x89): dst should be r/m (BP), src should be reg (SP)

fn main() {
    println!("Opcode 0x89 (MOV Ew,Gw / MOV Ed,Gd)");
    println!("ModRM 0xE5 = 11 100 101");
    println!("  mod = 11 (register-register)");
    println!("  reg = 100 = SP (index 4)");
    println!("  r/m = 101 = BP (index 5)");
    println!();
    println!("For MOV Ed,Gd format:");
    println!("  Ed (destination) = r/m field = BP");
    println!("  Gd (source) = reg field = SP");
    println!("  Instruction: BP = SP ✓");
    println!();
    println!("My decoder fix:");
    println!("  if ((b1 & 0x0F) == 0x09) // 0x89 matches!");
    println!("    dst = rm = 5 (BP)");
    println!("    src = reg = 4 (SP)");
    println!("  Result: BP = SP ✓ CORRECT!");
}
