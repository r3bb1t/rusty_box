//! Regression test: the prefetch-window distance check must be 64-bit.
//!
//! Bochs cpu.cc `getICacheEntry` computes `bx_address eipBiased = RIP +
//! eipPageBias` and compares the FULL 64-bit value against
//! `eipPageWindowSize`; near indirect transfers (JMP/CALL r/m64, RET) rely on
//! that compare — they deliberately do not invalidate the prefetch window.
//! Truncating the distance to u32 before the compare (the ported bug) lets an
//! indirect transfer whose target lies a multiple of 4 GiB (+ sub-window
//! offset) away alias back into the stale window: the CPU then executes the
//! OLD page's bytes at the NEW RIP. Under guest ASLR (PIE exe <-> libc are
//! always > 4 GiB apart) this intermittently corrupted userspace — observed
//! as the Ubuntu live-boot `logger` segfault whose kernel Code: dump did not
//! match the reported RIP.
//!
//! The test builds the smallest such alias: V2 = V1 + 2^32 + 0x10 (bits
//! 12..31 identical), maps V2 to a DIFFERENT physical page, and puts a
//! distinguishable `mov rbx, imm` at each physical location. Which value
//! lands in RBX tells us which page's bytes executed after `jmp rax`.

#![cfg(feature = "std")]

use rusty_box::cpu::{core_i7_skylake::Corei7SkylakeX, CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

/// Same sizing rationale as fp_vex_scalar_ops.rs.
const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;

/// Branch source page (identity-mapped by FlatLong64).
const V1: u64 = 0x0030_0000;
/// Branch target: > 4 GiB away, but bits 12..31 match V1's page base.
const V2: u64 = V1 + (1 << 32) + 0x10;

/// FlatLong64 page-table layout (emulator_api.rs `setup_flat_long64`):
/// PML4 @ 0x1000, PDPT @ 0x2000 (entries 0..4 filled), PDs @ 0x3000..0x6FFF.
/// 0x7000 is free — it becomes the PD for the 4 GiB..5 GiB slot.
const PDPT: u64 = 0x2000;
const NEW_PD: u64 = 0x7000;
/// NEW_PD[1] maps VA [4G+2M, 4G+4M) as a 2 MiB page at phys 0x0060_0000.
const V2_BACKING: u64 = 0x0060_0000;
const V2_PHYS: u64 = V2_BACKING + (V2 & 0x1F_FFFF);

#[test]
fn far_indirect_jump_must_leave_the_stale_prefetch_window() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");

            // Map V2's 2 MiB page: PDPT[4] -> NEW_PD, NEW_PD[1] -> V2_BACKING (P|RW|PS).
            emu.mem_write(PDPT + 4 * 8, &(NEW_PD | 0x3).to_le_bytes())
                .expect("write PDPT[4]");
            emu.mem_write(NEW_PD + 8, &(V2_BACKING | 0x83).to_le_bytes())
                .expect("write NEW_PD[1]");

            // V1: mov rax, V2 ; jmp rax
            let mut code = vec![0x48, 0xB8];
            code.extend_from_slice(&V2.to_le_bytes());
            code.extend_from_slice(&[0xFF, 0xE0]);
            emu.mem_write(V1, &code).expect("write branch code");

            // Same in-page offset, two different physical pages:
            // stale-window bytes (phys V1+0x10):  mov rbx, 0x33333333 ; jmp $
            emu.mem_write(
                V1 + 0x10,
                &[0x48, 0xC7, 0xC3, 0x33, 0x33, 0x33, 0x33, 0xEB, 0xFE],
            )
            .expect("write old-page code");
            // correct-target bytes (phys V2_PHYS): mov rbx, 0x22222222 ; jmp $
            emu.mem_write(
                V2_PHYS,
                &[0x48, 0xC7, 0xC3, 0x22, 0x22, 0x22, 0x22, 0xEB, 0xFE],
            )
            .expect("write new-page code");

            emu.reg_write(X86Reg::Rbx, 0);
            // mov rax + jmp rax + mov rbx = 3 instructions; park at the jmp $.
            match emu.emu_start(V1, Some(V2 + 7), None, Some(8)) {
                Ok(_) | Err(_) => {}
            }

            assert_eq!(
                emu.reg_read(X86Reg::Rbx),
                0x2222_2222,
                "the indirect jump crossed 4 GiB with matching bits 12..31 — \
                 executing the stale window's bytes means the eipBiased compare \
                 truncated to u32 (Bochs cpu.cc getICacheEntry compares in 64-bit)"
            );
        })
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}
