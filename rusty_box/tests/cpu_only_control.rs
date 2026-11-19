//! Downstream exercise of the CPU-only control API.
#![cfg(feature = "alloc")]

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::builder::BxCpuBuilder;
use rusty_box::cpu::memory_session::{EntryMode, MemorySession, RegisterRefusal, SessionStop};
use rusty_box::cpu::{CodeBreakpoint, CpuError, ProcessorActivity, X86Reg};
use rusty_box::memory::flat::FlatMemory;
use rusty_box::memory::mapped::{MappedMemory, PhysicalRegion, RegionKind};

#[test]
fn real_engine_step_breakpoint_edit_and_io_exit() {
    let mut cpu = BxCpuBuilder::new().build().unwrap();
    let mut bytes = [0; 0x3000];
    let mut stamps = [0; 3];
    let mut memory = FlatMemory::new(&mut bytes, &mut stamps).unwrap();
    let mut session = MemorySession::new(&mut cpu, memory.loan());
    session
        .reset_at(EntryMode::Protected32, 0x1000, 0x2FFC)
        .unwrap();
    let mut code_descriptor = [0; 8];
    session
        .read_physical(BxPhyAddress::new(0x808), &mut code_descriptor)
        .unwrap();
    assert_eq!(code_descriptor[5], 0x9A);
    assert_eq!(session.read_register(X86Reg::Esp).unwrap(), 0x2FFC);
    assert_eq!(session.read_register(X86Reg::Cs).unwrap(), 8);
    assert_eq!(
        session.write_register(X86Reg::Cs, 0),
        Err(RegisterRefusal::ReadOnly(X86Reg::Cs))
    );
    assert_eq!(
        session.read_register(X86Reg::Xmm0),
        Err(RegisterRefusal::Unsupported(X86Reg::Xmm0))
    );

    // inc eax; out 0x80,al; hlt
    session
        .write_ram(BxPhyAddress::new(0x1000), &[0x40, 0xE6, 0x80, 0xF4])
        .unwrap();
    let bp = CodeBreakpoint::Linear(0x1001);
    session.set_code_breakpoints(&[bp]).unwrap();
    assert!(matches!(
        session.run_controlled(0).stop,
        SessionStop::Budget
    ));
    let stopped = session.step();
    assert_eq!(stopped.instructions, 1);
    assert!(matches!(stopped.stop, SessionStop::Breakpoint(hit) if hit.breakpoint == bp));
    assert_eq!(session.read_register(X86Reg::Eax).unwrap(), 1);

    session.write_register(X86Reg::Al, 42).unwrap();
    session.write_register(X86Reg::Eip, 0x1001).unwrap();
    let stopped = session.step();
    assert_eq!(stopped.instructions, 0);
    assert!(matches!(stopped.stop, SessionStop::Breakpoint(hit) if hit.breakpoint == bp));
    let stopped = session.run_controlled(4);
    assert_eq!(stopped.instructions, 0);
    assert!(matches!(
        stopped.stop,
        SessionStop::Exit(CpuError::IoUnavailable { .. })
    ));
    assert_eq!(session.read_register(X86Reg::Eip).unwrap(), 0x1001);
    assert_eq!(session.read_register(X86Reg::Al).unwrap(), 42);
    session.set_instruction_pointer(0x1003);
    assert!(matches!(
        session.step().stop,
        SessionStop::Inactive(ProcessorActivity::Halted)
    ));
    session.set_instruction_pointer(0x1000);
    let halted = session.step();
    assert_eq!(halted.instructions, 0);
    assert!(matches!(
        halted.stop,
        SessionStop::Inactive(ProcessorActivity::Halted)
    ));
}

#[test]
fn sparse_entry_and_memory_failures_preserve_state() {
    let mut cpu = BxCpuBuilder::new().build().unwrap();
    let mut bytes = [0; 8192];
    let mut stamps = [0; 2];
    bytes[4096] = 0xB7;
    let map = [
        PhysicalRegion {
            guest_start: BxPhyAddress::new(0x1000),
            arena_offset: 0,
            len: 4096,
            kind: RegionKind::Ram,
        },
        PhysicalRegion {
            guest_start: BxPhyAddress::new(0x3000),
            arena_offset: 4096,
            len: 4096,
            kind: RegionKind::Rom,
        },
    ];
    let mut memory = MappedMemory::new(&mut bytes, &mut stamps, &map).unwrap();
    let mut session = MemorySession::new(&mut cpu, memory.loan());

    session.reset_at(EntryMode::Real16, 0x1000, 0x1FF0).unwrap();
    session.write_register(X86Reg::Ax, 0x1234).unwrap();
    assert!(
        session
            .reset_at(EntryMode::Protected32, 0x2000, 0x2FF0)
            .is_err()
    );
    assert_eq!(session.read_register(X86Reg::Rip).unwrap(), 0x1000);
    assert_eq!(session.read_register(X86Reg::Ax).unwrap(), 0x1234);
    let mut byte = [0];
    session
        .read_physical(BxPhyAddress::new(0x3000), &mut byte)
        .unwrap();
    assert_eq!(byte, [0xB7]);
    assert!(
        session
            .write_ram(BxPhyAddress::new(0x3000), &[0x42])
            .is_err()
    );
    assert!(
        session
            .read_physical(BxPhyAddress::new(0x2000), &mut byte)
            .is_err()
    );
    assert_eq!(byte, [0xB7]);

    // mov al,[0x3000]; jmp near to the physical hole at 0x2000
    session
        .write_ram(
            BxPhyAddress::new(0x1000),
            &[0xA0, 0, 0x30, 0xE9, 0xFA, 0x0F],
        )
        .unwrap();
    let stopped = session.run_controlled(4);
    assert_eq!(stopped.instructions, 2);
    assert!(matches!(
        stopped.stop,
        SessionStop::Exit(CpuError::VetoedDirectRead { p_addr: 0x2000 })
    ));
    assert_eq!(session.read_register(X86Reg::Ax).unwrap(), 0x12B7);
}

#[test]
fn controlled_step_reports_a_static_observer_stop_at_the_budget_boundary() {
    use rusty_box::cpu::{HookCtx, HookMask, InstrAction, Instrumentation};

    #[derive(Default)]
    struct Observe;
    impl Instrumentation for Observe {
        fn active_hooks(&self) -> HookMask {
            HookMask::empty()
        }

        fn pre_syscall(&mut self, ctx: &mut HookCtx) -> InstrAction {
            assert_eq!(ctx.reg_read(X86Reg::Eip), 0x1010);
            ctx.reg_write(X86Reg::Eax, 0x1234);
            InstrAction::SkipAndStop
        }
    }

    let mut cpu = BxCpuBuilder::new().build_with_tracer(Observe).unwrap();
    let mut bytes = [0; 8192];
    let mut stamps = [0; 2];
    let mut memory = FlatMemory::new(&mut bytes, &mut stamps).unwrap();
    let mut session = MemorySession::new(&mut cpu, memory.loan());
    session
        .reset_at(EntryMode::Protected32, 0x1000, 0x1FF0)
        .unwrap();
    session
        .write_ram(
            BxPhyAddress::new(0x1000),
            &[
                0xB9, 0x74, 0x01, 0, 0, // mov ecx,IA32_SYSENTER_CS
                0xB8, 8, 0, 0, 0, // mov eax,8
                0x31, 0xD2, // xor edx,edx
                0x0F, 0x30, // wrmsr
                0x0F, 0x34, // sysenter
                0xF4, // hlt
            ],
        )
        .unwrap();
    assert!(matches!(
        session.run_controlled(4).stop,
        SessionStop::Budget
    ));
    let outcome = session.step();
    assert_eq!(outcome.instructions, 1);
    assert!(matches!(outcome.stop, SessionStop::Observer));
    assert_eq!(session.read_register(X86Reg::Eax).unwrap(), 0x1234);
    assert!(matches!(
        session.step().stop,
        SessionStop::Inactive(ProcessorActivity::Halted)
    ));
}
