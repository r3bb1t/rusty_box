#![cfg(feature = "alloc")]

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::instrumentation::CpuSetupMode;
use rusty_box::cpu::memory_session::{EntryMode, SessionStop};
use rusty_box::cpu::registers::*;
use rusty_box::cpu::{CodeBreakpoint, CpuEmulator, CpuError, ProcessorActivity, X86Reg};
use rusty_box::{Emulator, EmulatorConfig};

const CODE: BxPhyAddress = BxPhyAddress::new(0x1000);

#[test]
fn typed_subregisters_and_full_system_execution_observe_the_same_cpu() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut pc =
                Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
                    .unwrap();
            let mut regs = pc.regs();
            regs.write(RAX, 0x1122_3344_5566_7788);
            regs.write(RSP, 0x1234);
            regs.write(AH, 0x99);
            assert_eq!(regs.read(RAX), 0x1122_3344_5566_9988);
            assert_eq!(regs.read(AX), 0x9988);
            assert_eq!(regs.read(SPL), 0x34);
            regs.write(SPL, 0xF0);
            assert_eq!(regs.read(RSP), 0x12F0);
            assert_eq!(regs.read(AH), 0x99);
            regs.write(AX, 0x2211);
            assert_eq!(regs.read(RAX), 0x1122_3344_5566_2211);
            regs.write(EAX, 0xAA55_F00D);
            assert_eq!(regs.read(RAX), 0xAA55_F00D);
            regs.write(R11, u64::MAX);
            regs.write(R11D, 0x1020_3040);
            assert_eq!(regs.read(R11), 0x1020_3040);

            let root = regs.read(CR3);
            assert_eq!(
                regs.set_page_table_root(BxPhyAddress::new(0x9000)),
                Err(PageTableRootError::Mode)
            );
            assert_eq!(regs.read(CR3), root);

            // mov rbx,rax — confirm the guest sees the typed host edit.
            pc.mem_write(BxPhyAddress::new(0x20_0000), &[0x48, 0x89, 0xC3])
                .unwrap();
            pc.emu_start(0x20_0000, None, None, Some(1)).unwrap();
            assert_eq!(pc.reg_read(X86Reg::Rbx), 0xAA55_F00D);
            pc.reg_write(X86Reg::Rbx, 0xDEAD_BEEF);
            assert_eq!(pc.regs().read(RBX), 0xDEAD_BEEF);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn simd_views_share_state_dynamic_scalars_are_explicit_and_flags_keep_reserved_bits() {
    let mut emu = CpuEmulator::builder().build().unwrap();
    let mut regs = emu.regs();
    regs.write(ZMM0, [0x5A; 64]);
    regs.write(XMM0, [0xAA; 16]);
    assert_eq!(&regs.read(ZMM0)[..16], &[0xAA; 16]);
    assert_eq!(&regs.read(ZMM0)[16..], &[0x5A; 48]);
    regs.write(YMM0, [0x77; 32]);
    assert_eq!(&regs.read(ZMM0)[..32], &[0x77; 32]);
    assert_eq!(&regs.read(ZMM0)[32..], &[0; 32]);
    let last = zmm(31).unwrap();
    regs.write(last, [0xCC; 64]);
    assert_eq!(regs.read(xmm(31).unwrap()), [0xCC; 16]);
    assert!(xmm(32).is_none() && ymm(32).is_none() && zmm(32).is_none());

    regs.write_dynamic(X86Reg::Eax, 0x89AB_CDEF).unwrap();
    assert_eq!(regs.read(EAX), 0x89AB_CDEF);
    assert_eq!(regs.read_dynamic(X86Reg::Ah), Ok(0xCD));
    assert_eq!(
        regs.read_dynamic(X86Reg::Xmm0),
        Err(RegisterRefusal::Unsupported(X86Reg::Xmm0))
    );
    assert_eq!(
        regs.write_dynamic(X86Reg::Cr3, 0),
        Err(RegisterRefusal::ReadOnly(X86Reg::Cr3))
    );
    regs.write(EFLAGS, EFlags::CF | EFlags::IOPL_MASK);
    assert!(regs.read(EFLAGS).contains(EFlags::CF));
    assert!(!regs.read(EFLAGS).intersects(EFlags::IOPL_MASK));
}

#[test]
fn port_exit_survives_reads_and_is_restarted_after_successful_edits() {
    let mut emu = CpuEmulator::builder().build().unwrap();
    // out 0x80,al; hlt
    emu.memory().write_phys(CODE, &[0xE6, 0x80, 0xF4]).unwrap();
    emu.regs().write(AL, 0x11);
    let exit = emu.step();
    assert_eq!(exit.instructions, 0);
    assert!(matches!(
        exit.stop,
        SessionStop::Exit(CpuError::IoUnavailable { .. })
    ));
    emu.regs().write(AL, 0x22);
    assert!(
        emu.service_scalar_io(|_| panic!("edit must discard pending I/O"))
            .is_err()
    );

    assert!(matches!(
        emu.step().stop,
        SessionStop::Exit(CpuError::IoUnavailable { .. })
    ));
    assert_eq!(emu.regs().read(EIP), 0x1000);
    let mut code = [0; 3];
    emu.memory().read_phys(CODE, &mut code).unwrap();
    assert_eq!(code, [0xE6, 0x80, 0xF4]);
    assert!(
        !emu.service_scalar_io(|request| {
            assert_eq!(request.output, Some(0x22));
            None
        })
        .unwrap()
    );
    assert!(
        emu.service_scalar_io(|request| {
            assert_eq!(
                (request.port.get(), request.len, request.output),
                (0x80, 1, Some(0x22))
            );
            Some(0)
        })
        .unwrap()
    );
    assert_eq!(emu.regs().read(EIP), 0x1002);
    assert!(
        emu.service_scalar_io(|_| panic!("already retired"))
            .is_err()
    );
    assert!(matches!(
        emu.step().stop,
        SessionStop::Inactive(ProcessorActivity::Halted)
    ));
}

#[test]
fn instruction_pointer_edit_rearms_a_breakpoint_and_reset_restarts_halted_owner() {
    let mut emu = CpuEmulator::builder().build().unwrap();
    emu.memory().write_phys(CODE, &[0x90, 0xF4]).unwrap();
    let trap = CodeBreakpoint::Linear(0x1000);
    emu.set_code_breakpoints(&[trap]).unwrap();
    assert!(matches!(emu.step().stop, SessionStop::Breakpoint(hit) if hit.breakpoint == trap));
    emu.regs().write(EIP, 0x1000);
    assert!(matches!(emu.step().stop, SessionStop::Breakpoint(hit) if hit.breakpoint == trap));
    assert!(matches!(emu.step().stop, SessionStop::Budget));
    assert!(matches!(
        emu.step().stop,
        SessionStop::Inactive(ProcessorActivity::Halted)
    ));
    emu.regs().write(EIP, 0x1001);
    assert_eq!(emu.step().instructions, 0);
    emu.set_code_breakpoints(&[]).unwrap();
    emu.reset_at(EntryMode::Protected32, 0x1000, 0x3000)
        .unwrap();
    assert_eq!(emu.step().instructions, 1);
}

#[test]
fn builder_refuses_unbacked_or_truncated_entry_layouts() {
    assert!(CpuEmulator::builder().ram(4096).build().is_err());
    assert!(
        CpuEmulator::builder()
            .mode(EntryMode::Real16)
            .ram(0x11000)
            .entry(0x10000)
            .build()
            .is_err()
    );
    assert!(
        CpuEmulator::builder()
            .ram(0x2000)
            .stack(0x2001)
            .build()
            .is_err()
    );
}
