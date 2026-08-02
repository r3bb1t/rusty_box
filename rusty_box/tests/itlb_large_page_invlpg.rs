//! Regression test: INVLPG must drop EVERY cached ITLB translation of a
//! large (2 MiB / 4 MiB / 1 GiB) code page, not just the exact 4 KiB frame.
//!
//! Bochs paging.cc `translate_linear` stores the walk's true `lpf_mask` in
//! the (I)TLB entry and sets `ITLB.split_large` when a large-page execute
//! translation is cached; `bx_TLB_c::invlpg` then takes the scan path that
//! matches `(laddr & ~lpf_mask) == (lpf & ~lpf_mask)`, killing all 4 KiB
//! frames of the huge page. Filling ITLB entries with a hardcoded
//! `lpf_mask = 0xFFF` (the ported bug) leaves sibling frames alive: after
//! the guest remaps a huge page and INVLPGs it, instruction fetch keeps
//! resolving to the OLD physical frame while data reads see the new one —
//! the "executed bytes differ from memory" class behind the Ubuntu `logger`
//! segfault.
//!
//! The test warms an ITLB translation inside a 2 MiB code page, repoints the
//! PD entry at different backing, INVLPGs a *different* 4 KiB frame of the
//! same huge page (exactly what Linux's PMD-stride flush emits), and calls
//! back into the page. Which `mov rbx` executes tells us which physical
//! page the fetch used.

#![cfg(feature = "std")]

use rusty_box::cpu::{core_i7_skylake::Corei7SkylakeX, CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

/// Same sizing rationale as fp_vex_scalar_ops.rs.
const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;

/// FlatLong64 identity-maps with 2 MiB pages: PD0 @ 0x3000, entry 1 maps
/// virtual [0x20_0000, 0x40_0000) to the same physical range.
const PD0_ENTRY1: u64 = 0x3008;
/// Victim function inside the huge page (identity phys = 0x30_0000).
const V_FN: u64 = 0x0030_0000;
/// After the remap, the huge page is backed by phys 0x0080_0000, so V_FN
/// resolves to 0x0080_0000 + (V_FN - 0x20_0000) = 0x0090_0000.
const NEW_BACKING: u64 = 0x0080_0000;
const V_FN_NEW_PHYS: u64 = NEW_BACKING + (V_FN - 0x0020_0000);
/// Driver + stack live in the NEXT 2 MiB page, untouched by the remap.
const DRIVER: u64 = 0x0040_0000;
const STACK_TOP: u64 = 0x0058_0000;

#[test]
fn invlpg_flushes_every_itlb_frame_of_a_large_code_page() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");

            // Old bytes (identity phys of V_FN): mov rbx, 0x11111111 ; ret
            emu.mem_write(V_FN, &[0x48, 0xC7, 0xC3, 0x11, 0x11, 0x11, 0x11, 0xC3])
                .expect("write old fn");
            // New bytes (phys the remapped huge page exposes at V_FN):
            // mov rbx, 0x22222222 ; ret
            emu.mem_write(
                V_FN_NEW_PHYS,
                &[0x48, 0xC7, 0xC3, 0x22, 0x22, 0x22, 0x22, 0xC3],
            )
            .expect("write new fn");

            // Driver:
            //   mov rax, V_FN ; call rax                 (warm ITLB + icache)
            //   mov rax, NEW_BACKING|P|RW|PS ; mov [PD0_ENTRY1], rax
            //   invlpg [0x20_0000]                       (a DIFFERENT 4K frame)
            //   mov rax, V_FN ; call rax                 (must see the remap)
            //   jmp $
            let mut code: Vec<u8> = Vec::new();
            code.extend_from_slice(&[0x48, 0xB8]);
            code.extend_from_slice(&V_FN.to_le_bytes());
            code.extend_from_slice(&[0xFF, 0xD0]);
            code.extend_from_slice(&[0x48, 0xB8]);
            code.extend_from_slice(&(NEW_BACKING | 0x83).to_le_bytes());
            // mov [PD0_ENTRY1], rax — moffs64 form (REX.W A3).
            code.extend_from_slice(&[0x48, 0xA3]);
            code.extend_from_slice(&PD0_ENTRY1.to_le_bytes());
            // invlpg [0x0020_0000] — 0F 01 /7 with SIB disp32 addressing.
            code.extend_from_slice(&[0x0F, 0x01, 0x3C, 0x25, 0x00, 0x00, 0x20, 0x00]);
            code.extend_from_slice(&[0x48, 0xB8]);
            code.extend_from_slice(&V_FN.to_le_bytes());
            code.extend_from_slice(&[0xFF, 0xD0]);
            let park = DRIVER + code.len() as u64;
            code.extend_from_slice(&[0xEB, 0xFE]);
            emu.mem_write(DRIVER, &code).expect("write driver");

            emu.reg_write(X86Reg::Rsp, STACK_TOP);
            emu.reg_write(X86Reg::Rbx, 0);
            match emu.emu_start(DRIVER, Some(park), None, Some(32)) {
                Ok(_) | Err(_) => {}
            }

            assert_eq!(
                emu.reg_read(X86Reg::Rbx),
                0x2222_2222,
                "the second call fetched through a stale ITLB large-page \
                 translation — INVLPG on a sibling 4 KiB frame must flush the \
                 whole 2 MiB code page (Bochs paging.cc translate_linear \
                 lpf_mask + ITLB.split_large)"
            );
        })
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}
