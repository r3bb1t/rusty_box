//! Check the size of emulator structs to diagnose stack overflow

use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, BxCpuC},
    emulator::Emulator,
    memory::BxMemC,
};

fn main() {
    println!("╔══════════════════════════════════════════════╗");
    println!("║         Struct Size Analysis                 ║");
    println!("╚══════════════════════════════════════════════╝");
    println!();

    // Check size of main emulator struct
    println!("Emulator<Corei7SkylakeX>: {} bytes ({:.2} KB)",
        std::mem::size_of::<Emulator<Corei7SkylakeX>>(),
        std::mem::size_of::<Emulator<Corei7SkylakeX>>() as f64 / 1024.0
    );

    // Check size of CPU
    println!("BxCpuC<Corei7SkylakeX>: {} bytes ({:.2} KB)",
        std::mem::size_of::<BxCpuC<Corei7SkylakeX>>(),
        std::mem::size_of::<BxCpuC<Corei7SkylakeX>>() as f64 / 1024.0
    );

    // Check size of Memory
    println!("BxMemC: {} bytes ({:.2} KB)",
        std::mem::size_of::<BxMemC>(),
        std::mem::size_of::<BxMemC>() as f64 / 1024.0
    );

    println!();
    println!("Analysis:");
    println!("--------");

    let total_size = std::mem::size_of::<Emulator<Corei7SkylakeX>>();

    if total_size > 1024 * 1024 {
        println!("⚠️  WARNING: Emulator struct is > 1 MB!");
        println!("   This is too large to safely allocate on the stack.");
        println!("   Recommendation: Box the Emulator or its large components.");
    } else if total_size > 100 * 1024 {
        println!("⚠️  CAUTION: Emulator struct is > 100 KB");
        println!("   This may cause stack issues with deep call chains.");
    } else {
        println!("✓ Emulator struct size is reasonable for stack allocation.");
    }
}
