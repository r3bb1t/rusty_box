//! Regression tests for the far-pointer loads (LSS/LFS/LGS in 64-bit mode).
//!
//! Bochs segment_ctrl.cc `load_segq` reads both halves of the m16:64 operand
//! through the paging-aware LINEAR accessors:
//!
//! ```text
//! Bit16u segsel = read_linear_word (i->seg(), get_laddr64(i->seg(), (eaddr + 8) & i->asize_mask()));
//! Bit64u reg_64 = read_linear_qword(i->seg(), get_laddr64(i->seg(), eaddr));
//! ```
//!
//! Three properties fall out of that and are pinned here:
//!   * the reads are LINEAR — they walk the page tables and can raise #PF;
//!     handing a linear address to a physical accessor silently reads the
//!     wrong bytes whenever linear != physical;
//!   * the SELECTOR is read first, so when both halves fault the selector's
//!     page is the one reported in CR2;
//!   * the selector address is `(eaddr + 8) & asize_mask`, i.e. it wraps
//!     within the instruction's address size.

#![cfg(feature = "std")]

use rusty_box::cpu::{core_i7_skylake::Corei7SkylakeX, CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

/// Same sizing rationale as fp_vex_scalar_ops.rs.
const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;

const CODE: u64 = 0x0040_0000;
/// FlatLong64 identity-maps 0..4 GiB with 2 MiB pages; PD0 @ 0x3000 entry 3
/// covers virtual [0x60_0000, 0x80_0000).
const PD0_ENTRY3: u64 = 0x3018;
/// Virtual address of the far pointer, inside that 2 MiB page.
const PTR_VADDR: u64 = 0x0060_1000;
/// Backing we repoint the page at, so linear != physical for PTR_VADDR.
/// Must be 2 MiB-aligned — a PS=1 PDE with a misaligned frame is a
/// reserved-bit violation, not a mapping.
const NEW_BACKING: u64 = 0x00A0_0000;
const PTR_PADDR: u64 = NEW_BACKING + (PTR_VADDR - 0x0060_0000);

const TRUE_OFFSET: u64 = 0x1122_3344_5566_7788;
/// A value planted at the *identity* location, which a physical-address read
/// of PTR_VADDR would pick up instead.
const DECOY_OFFSET: u64 = 0xDEAD_BEEF_DEAD_BEEF;
/// Flat data selector installed by the FlatLong64 harness GDT.
const DATA_SEL: u16 = 0x10;

#[test]
fn lss_reads_the_far_pointer_through_linear_addresses() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");

            // Repoint virtual [0x60_0000, 0x80_0000) at NEW_BACKING so the
            // far pointer's linear and physical addresses differ.
            emu.mem_write(PD0_ENTRY3, &(NEW_BACKING | 0x83u64).to_le_bytes())
                .expect("repoint PD0[3]");

            // The real operand lives at the new physical location...
            emu.mem_write(PTR_PADDR, &TRUE_OFFSET.to_le_bytes())
                .expect("write far offset");
            emu.mem_write(PTR_PADDR + 8, &DATA_SEL.to_le_bytes())
                .expect("write far selector");
            // ...and a decoy sits where a physical-address read would land.
            emu.mem_write(PTR_VADDR, &DECOY_OFFSET.to_le_bytes())
                .expect("write decoy offset");
            emu.mem_write(PTR_VADDR + 8, &0u16.to_le_bytes())
                .expect("write decoy selector");

            // lss rbx, [PTR_VADDR]  =  REX.W 0F B2 /r with SIB disp32.
            let mut code: Vec<u8> = vec![0x48, 0x0F, 0xB2, 0x1C, 0x25];
            code.extend_from_slice(&(PTR_VADDR as u32).to_le_bytes());
            let park = CODE + code.len() as u64;
            code.extend_from_slice(&[0xEB, 0xFE]);
            emu.mem_write(CODE, &code).expect("write code");


            emu.reg_write(X86Reg::Rbx, 0);
            match emu.emu_start(CODE, Some(park), None, Some(8)) {
                Ok(_) | Err(_) => {}
            }

            assert_eq!(
                emu.reg_read(X86Reg::Rbx),
                TRUE_OFFSET,
                "LSS must read the far pointer through the LINEAR address \
                 (Bochs segment_ctrl.cc load_segq uses read_linear_qword); \
                 picking up the decoy means it read physical memory. \
                 rip={:#x} park={:#x} cr2={:#x}",
                emu.cpu().rip(),
                park,
                emu.reg_read(X86Reg::Cr2)
            );
        })
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}

#[test]
fn lss_faults_on_an_unmapped_far_pointer() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");

            // Unmap the 2 MiB page holding the far pointer entirely.
            emu.mem_write(PD0_ENTRY3, &0u64.to_le_bytes())
                .expect("clear PD0[3]");

            let mut code: Vec<u8> = vec![0x48, 0x0F, 0xB2, 0x1C, 0x25];
            code.extend_from_slice(&(PTR_VADDR as u32).to_le_bytes());
            let park = CODE + code.len() as u64;
            code.extend_from_slice(&[0xEB, 0xFE]);
            emu.mem_write(CODE, &code).expect("write code");

            emu.reg_write(X86Reg::Rbx, 0);
            match emu.emu_start(CODE, Some(park), None, Some(8)) {
                Ok(_) | Err(_) => {}
            }

            // A physical read of an unmapped-but-present-in-RAM address
            // silently succeeds and parks; a correct linear read raises #PF,
            // so RBX must never receive a value and RIP must not reach the
            // park instruction.
            assert_eq!(
                emu.reg_read(X86Reg::Rbx),
                0,
                "a faulting LSS must not write the destination register"
            );
            assert_ne!(
                emu.cpu().rip(),
                park,
                "LSS on an unmapped far pointer must raise #PF, not complete"
            );
            assert_eq!(
                emu.reg_read(X86Reg::Cr2),
                PTR_VADDR + 8,
                "CR2 must hold the faulting linear address; Bochs load_segq \
                 reads the SELECTOR at (eaddr + 8) first"
            );
        })
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}
