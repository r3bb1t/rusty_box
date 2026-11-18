//! Real x86 vertical-slice regressions. Instruction semantics are those of
//! Bochs cpu.cc, data_xfer*.cc, io.cc, exception.cc and icache.cc. The new
//! CPU-only host exit is a host policy, not a replacement x86 instruction.

use alloc::boxed::Box;

use super::api_bridge::SegmentSize;
use super::builder::BxCpuBuilder;
use super::cpu::{BxCpuC, CpuActivityState};
use super::exec_ctx::{ExecCtx, Hardware};
use super::{CpuError, X86Reg};
use crate::config::{A20Mask, BxPhyAddress};
use crate::memory::{BxMemC, BxMemoryStubC};

const MIB: usize = 1 << 20;
const CODE: u64 = 0x2000;
const DATA: u64 = 0x6000;

/// Owns only an actual processor and memory. In particular no TestMachine,
/// BxDevicesC, DeviceManager or BxPcSystemC participates in this path.
struct Minimal {
    cpu: Box<BxCpuC<()>>,
    memory: BxMemC,
}

fn memory(guest: usize, host: usize) -> BxMemC {
    let mut mem = BxMemC::new(
        BxMemoryStubC::create_and_init(guest, host, 4096).unwrap(),
        false,
    );
    mem.set_a20_mask(A20Mask::OPEN);
    mem
}

impl Minimal {
    fn new(code: &[u8]) -> Self {
        let mut result = Self {
            cpu: BxCpuBuilder::new().build().unwrap(),
            memory: memory(MIB, MIB),
        };
        result.setup(SegmentSize::Bits32);
        if !code.is_empty() {
            result.write(CODE, code);
        }
        result
    }

    fn setup(&mut self, size: SegmentSize) {
        // As on the PC path, reset the real processor before selecting a
        // host-requested mode. Builder initialization alone does not reset
        // architectural MSRs or enable the LAPIC's physical decode.
        self.cpu.reset(super::ResetReason::Hardware);
        for reg in [
            X86Reg::Cs,
            X86Reg::Ds,
            X86Reg::Es,
            X86Reg::Ss,
            X86Reg::Fs,
            X86Reg::Gs,
        ] {
            self.cpu.set_seg_for_api(
                reg,
                if reg == X86Reg::Cs { 8 } else { 16 },
                0,
                if size == SegmentSize::Bits16 {
                    0xFFFF
                } else {
                    u32::MAX
                },
                size,
            );
        }
        if size == SegmentSize::Bits32 {
            self.cpu.enter_protected_mode_for_api();
        }
        self.cpu.set_rip(CODE);
        self.cpu.prev_rip = CODE;
        self.cpu.set_rsp(0x9000);
        self.cpu.set_rflags_for_api(2);
    }

    fn write(&mut self, addr: u64, bytes: &[u8]) {
        self.memory
            .load_RAM(bytes, BxPhyAddress::new(addr))
            .unwrap();
    }

    fn read<const N: usize>(&mut self, addr: u64) -> [u8; N] {
        let mut bytes = [0; N];
        self.memory
            .read_ram(BxPhyAddress::new(addr), &mut bytes)
            .unwrap();
        bytes
    }

    fn run(&mut self, instructions: u64) -> super::Result<u64> {
        let mut ctx = ExecCtx::cpu_only(&mut self.cpu, &mut self.memory)?;
        assert!(matches!(ctx.hardware, Hardware::CpuOnly));
        ctx.cpu_loop_n_slice(instructions, true, 1)
    }
}

const ARITHMETIC: &[u8] = &[
    0xB8, 7, 0, 0, 0, // mov eax,7
    0x83, 0xC0, 5, // add eax,5
    0xA3, 0, 0x60, 0, 0, // mov [0x6000],eax
    0x8B, 0x1D, 0, 0x60, 0, 0, // mov ebx,[0x6000]
    0x83, 0xEB, 2,    // sub ebx,2
    0xF4, // hlt
];

#[test]
fn cpu_only_real_fetch_arithmetic_load_store_and_halt() {
    let mut m = Minimal::new(ARITHMETIC);
    assert_eq!(m.run(6).unwrap(), 6);
    assert_eq!(m.cpu.eax(), 12);
    assert_eq!(m.cpu.ebx(), 10);
    assert_eq!(m.read::<4>(DATA), 12u32.to_le_bytes());
    assert_eq!(m.cpu.rip(), CODE + ARITHMETIC.len() as u64);
    assert_eq!(m.cpu.activity_state, CpuActivityState::Hlt);
    assert_eq!(m.run(1).unwrap(), 0, "HLT remains waiting without a board");
}

#[test]
fn cpu_only_and_pc_execute_the_same_real_instruction_sequence() {
    use super::CpuSetupMode;
    use crate::emulator::{Emulator, EmulatorConfig, MemorySize};
    let mut minimal = Minimal::new(ARITHMETIC);
    minimal.run(6).unwrap();
    let mut pc = Emulator::new_with_mode(
        EmulatorConfig {
            memory: MemorySize::bytes(MIB),
            pci_enabled: false,
            ..Default::default()
        },
        CpuSetupMode::FlatProtected32,
    )
    .unwrap();
    pc.mem_write(BxPhyAddress::new(CODE), ARITHMETIC).unwrap();
    pc.reg_write(X86Reg::Rip, CODE);
    pc.reg_write(X86Reg::Rflags, 2);
    pc.exec_ctx(0).cpu_loop_n_slice(6, true, 1).unwrap();
    for reg in [X86Reg::Rax, X86Reg::Rbx, X86Reg::Rip, X86Reg::Rflags] {
        assert_eq!(pc.reg_read(reg), minimal.cpu.api_reg_read(reg), "{reg:?}");
    }
    let mut bytes = [0; 4];
    pc.mem_read(BxPhyAddress::new(DATA), &mut bytes).unwrap();
    assert_eq!(bytes, minimal.read::<4>(DATA));
}

#[test]
fn cpu_only_scalar_io_exit_is_before_retirement_and_repeatable() {
    // Immediate and DX forms, all three operand widths, both directions.
    for (code, width, input) in [
        (&[0xE4, 0x21][..], 1, true),
        (&[0x66, 0xE5, 0x21], 2, true),
        (&[0xE5, 0x21], 4, true),
        (&[0xEC], 1, true),
        (&[0x66, 0xED], 2, true),
        (&[0xED], 4, true),
        (&[0xE6, 0x21], 1, false),
        (&[0x66, 0xE7, 0x21], 2, false),
        (&[0xE7, 0x21], 4, false),
        (&[0xEE], 1, false),
        (&[0x66, 0xEF], 2, false),
        (&[0xEF], 4, false),
    ] {
        let mut m = Minimal::new(code);
        m.cpu.set_eax(0x1234_ABCD);
        m.cpu.set_edx(0x21);
        for _ in 0..2 {
            let err = m.run(1).unwrap_err();
            assert!(
                matches!(err, CpuError::IoUnavailable { port, len, direction_in, string: false }
                if port.get() == 0x21 && len == width && direction_in == input),
                "{err:?}"
            );
            assert_eq!(m.cpu.rip(), CODE);
            assert_eq!(m.cpu.prev_rip, CODE);
            assert_eq!(m.cpu.icount, 0);
            assert_eq!(m.cpu.eax(), 0x1234_ABCD);
        }
    }
}

#[test]
fn cpu_only_io_exit_resumes_on_the_same_engine_when_hardware_is_supplied() {
    use super::CpuSetupMode;
    use crate::emulator::{Emulator, EmulatorConfig, MemorySize};
    // mov al,0xab; out 0x21,al; in al,0x21; hlt. Bochs PIC IMR round trip.
    let code = [0xB0, 0xAB, 0xE6, 0x21, 0xE4, 0x21, 0xF4];
    let mut m = Minimal::new(&code);
    assert!(matches!(m.run(4), Err(CpuError::IoUnavailable { .. })));
    assert_eq!(m.cpu.icount, 1);
    assert_eq!(m.cpu.rip(), CODE + 2);
    let mut pc = Emulator::new_with_mode(
        EmulatorConfig {
            memory: MemorySize::bytes(MIB),
            pci_enabled: false,
            ..Default::default()
        },
        CpuSetupMode::FlatProtected32,
    )
    .unwrap();
    pc.devices.init(&mut pc.memory).unwrap();
    pc.device_manager
        .init(&mut pc.devices, &mut pc.memory)
        .unwrap();
    // R3 already permits moved processor state; adoption must discard its
    // memory-dependent caches. Install the exact bytes in the new memory.
    pc.mem_write(BxPhyAddress::new(CODE), &code).unwrap();
    core::mem::swap(pc.cpu_mut(), &mut m.cpu);
    // Device access may end a slice for machine-wide effects. Resume through
    // the scheduler that owns those effects, not by bypassing its boundary.
    let outcome = pc
        .step(crate::emulator::RunBudget::Instructions(3))
        .unwrap();
    assert_eq!(outcome.progress, crate::emulator::Progress::Instructions(3));
    assert_eq!(pc.reg_read(X86Reg::Rax) as u8, 0xAB);
    assert_eq!(pc.cpu().icount, 4, "the stopped OUT retires exactly once");
    assert_eq!(pc.reg_read(X86Reg::Rip), CODE + code.len() as u64);
}

#[test]
fn cpu_only_string_io_refuses_before_any_iteration_and_empty_rep_completes() {
    for opcode in [0x6C, 0x6D, 0x6E, 0x6F] {
        let mut m = Minimal::new(&[0xF3, opcode, 0xF4]);
        m.cpu.set_ecx(3);
        m.cpu.set_esi(DATA as u32);
        m.cpu.set_edi(DATA as u32);
        m.cpu.set_edx(0x21);
        m.write(DATA, &[0xAA; 16]);
        for _ in 0..2 {
            assert!(matches!(
                m.run(1),
                Err(CpuError::IoUnavailable { string: true, .. })
            ));
            assert_eq!((m.cpu.rip(), m.cpu.icount, m.cpu.ecx()), (CODE, 0, 3));
            assert_eq!((m.cpu.esi(), m.cpu.edi()), (DATA as u32, DATA as u32));
            assert_eq!(m.read::<16>(DATA), [0xAA; 16]);
        }
        m.cpu.set_ecx(0);
        assert_eq!(m.run(1).unwrap(), 1);
        assert_eq!(m.cpu.rip(), CODE + 2);
        assert_eq!(m.cpu.icount, 1);
    }
}

#[test]
fn cpu_only_protected_store_takes_a_guest_fault_without_committing_the_store() {
    use super::MemPerms;
    use crate::memory::permissions::PagePermissions;
    let mut m = Minimal::new(&[]);
    m.cpu.reset(super::ResetReason::Hardware);
    m.setup(SegmentSize::Bits16);
    // Real-mode #GP vector points to HLT at 0x3000. The host permissions
    // API deliberately turns its denied writes into #GP (access.rs).
    m.write(CODE, &[0xB8, 0x34, 0x12, 0xA3, 0, 0x60, 0xF4]);
    m.write(13 * 4, &[0, 0x30, 0, 0]);
    m.write(0x3000, &[0xF4]);
    m.write(DATA, &[0xAA, 0xBB]);
    let mut permissions = PagePermissions::new(MIB as u64);
    permissions.set(BxPhyAddress::new(DATA), 4096, MemPerms::READ);
    m.cpu.page_permissions = Some(permissions);
    m.run(3).unwrap();
    assert_eq!(m.read::<2>(DATA), [0xAA, 0xBB]);
    assert_eq!(m.cpu.rip(), 0x3001, "execution entered the #GP handler");
    assert_eq!(
        m.read::<2>(0x8FFA),
        (CODE as u16 + 3).to_le_bytes(),
        "fault saves the issuing IP"
    );
    assert_eq!(m.cpu.activity_state, CpuActivityState::Hlt);
    // icount preserves the existing Bochs-loop convention: exception
    // delivery counts too. It is not a precise architectural-retirement API.
}

#[test]
fn cpu_only_host_code_write_invalidates_a_warm_trace() {
    let mut m = Minimal::new(&[0xB8, 1, 0, 0, 0, 0xEB, 0xF9]);
    m.run(2).unwrap();
    assert_eq!(m.cpu.eax(), 1);
    m.write(CODE + 1, &9u32.to_le_bytes());
    m.run(2).unwrap();
    assert_eq!(m.cpu.eax(), 9);
}

#[test]
fn cpu_only_guest_self_modification_stops_the_stale_trace_tail() {
    // mov byte [CODE+8],9; mov eax,1; hlt. The write changes the immediate
    // of the next instruction, already decoded in this very trace.
    let mut m = Minimal::new(&[0xC6, 0x05, 8, 0x20, 0, 0, 9, 0xB8, 1, 0, 0, 0, 0xF4]);
    m.run(3).unwrap();
    assert_eq!(m.cpu.eax(), 9);
}

#[test]
fn cpu_only_backing_replacement_reloads_fetch_and_data() {
    let code = [0xA1, 0, 0x60, 0, 0, 0xEB, 0xF9];
    let mut m = Minimal::new(&code);
    m.write(DATA, &11u32.to_le_bytes());
    m.run(2).unwrap();
    assert_eq!(m.cpu.eax(), 11);
    let old = core::mem::replace(&mut m.memory, memory(MIB, MIB));
    m.write(CODE, &code);
    m.write(DATA, &22u32.to_le_bytes());
    m.run(2).unwrap();
    assert_eq!(m.cpu.eax(), 22);
    // Keep the old allocation live, so the test cannot accidentally reuse it.
    drop(old);
}

#[test]
fn cpu_only_partial_residency_revalidates_after_host_accesses() {
    let code = [0xA1, 0, 0x60, 0, 0, 0xEB, 0xF9];
    let mut m = Minimal::new(&[]);
    m.memory = memory(4 * MIB, MIB);
    m.write(CODE, &code);
    m.write(DATA, &33u32.to_le_bytes());
    m.run(2).unwrap();
    assert_eq!(m.cpu.eax(), 33);
    let epoch = m.memory.swap_epoch();
    // More distinct blocks than fit: force the code/data backing slots to
    // be reused. The next context must not execute the replacement bytes.
    for page in 256..768 {
        m.write(page * 4096, &[0xF4; 16]);
    }
    assert!(m.memory.swap_epoch() > epoch);
    m.run(2).unwrap();
    assert_eq!(m.cpu.eax(), 33);
    assert_eq!(m.cpu.rip(), CODE);
}

#[test]
fn cpu_only_rejects_unserviceable_pc_mmio_at_assembly() {
    use crate::memory::mmio_map::MmioToken;
    let mut m = Minimal::new(ARITHMETIC);
    m.memory
        .mmio
        .map(
            MmioToken(0),
            BxPhyAddress::new(DATA),
            BxPhyAddress::new(DATA + 4095),
        )
        .unwrap();
    assert!(matches!(
        m.run(6),
        Err(CpuError::UnsupportedCpuOperation { .. })
    ));
    assert_eq!(m.cpu.icount, 0);
    assert_eq!(m.cpu.rip(), CODE);
}

#[test]
fn cpu_only_executes_from_caller_owned_aligned_storage() {
    // The provider lives in this separate test module, uses only the public
    // safe backing API, and prepares bytes before surrendering its loan.
    // This proves replaceable STORAGE, not an arbitrary memory-bus trait.
    #[repr(align(4096))]
    struct SuppliedStorage([u8; 6 * MIB]);
    std::thread::Builder::new()
        .stack_size(64 * MIB)
        .spawn(|| {
            let storage = Box::leak(Box::new(SuppliedStorage([0; 6 * MIB])));
            storage.0[CODE as usize..CODE as usize + ARITHMETIC.len()].copy_from_slice(ARITHMETIC);
            let stub = BxMemoryStubC::from_buffer(&mut storage.0, MIB, MIB, 4096).unwrap();
            let mut m = Minimal {
                cpu: BxCpuBuilder::new().build().unwrap(),
                memory: BxMemC::new_from_stub(stub, false),
            };
            m.memory.set_a20_mask(A20Mask::OPEN);
            m.setup(SegmentSize::Bits32);
            assert_eq!(m.run(6).unwrap(), 6);
            assert_eq!(m.cpu.ebx(), 10);
            assert_eq!(m.read::<4>(DATA), 12u32.to_le_bytes());
        })
        .unwrap()
        .join()
        .unwrap();
}

/// A diagnostic, not a timing assertion. Compare profiles over identical
/// 32-bit code and the same strict interpreter slice; construction excluded.
#[test]
#[ignore = "manual timing: run alone with --ignored --nocapture"]
fn cpu_only_profile_timing() {
    use std::time::Instant;

    use super::CpuSetupMode;
    use crate::emulator::{Emulator, EmulatorConfig, MemorySize};
    let code = [
        0x01, 0xC8, // add eax,ecx
        0x31, 0xD2, // xor edx,edx
        0xA3, 0, 0x60, 0, 0, // mov [0x6000],eax
        0x8B, 0x35, 0, 0x60, 0, 0, // mov esi,[0x6000]
        0x01, 0xF0, // add eax,esi
        0x49, // dec ecx
        0x75, 0xEC, // jnz loop
    ];
    let instructions = 50_000_000;
    let mut minimal = Minimal::new(&code);
    minimal.cpu.set_ecx(u32::MAX);
    let mut pc = Emulator::new_with_mode(
        EmulatorConfig {
            memory: MemorySize::bytes(MIB),
            pci_enabled: false,
            ..Default::default()
        },
        CpuSetupMode::FlatProtected32,
    )
    .unwrap();
    pc.mem_write(BxPhyAddress::new(CODE), &code).unwrap();
    pc.reg_write(X86Reg::Rip, CODE);
    pc.reg_write(X86Reg::Rcx, u32::MAX as u64);
    pc.reg_write(X86Reg::Rflags, 2);
    for round in 0..6 {
        // Alternate order to expose drift rather than bake it into one side.
        for bare in if round % 2 == 0 {
            [true, false]
        } else {
            [false, true]
        } {
            let start = Instant::now();
            let count = if bare {
                minimal.run(instructions).unwrap()
            } else {
                pc.exec_ctx(0)
                    .cpu_loop_n_slice(instructions, true, 1)
                    .unwrap()
            };
            let ns = start.elapsed().as_nanos() as f64 / count as f64;
            println!(
                "profile={} round={round} instructions={count} ns_per_instruction={ns:.4}",
                if bare { "cpu-only" } else { "pc" }
            );
        }
    }
    for reg in [
        X86Reg::Rax,
        X86Reg::Rcx,
        X86Reg::Rsi,
        X86Reg::Rip,
        X86Reg::Rflags,
    ] {
        assert_eq!(pc.reg_read(reg), minimal.cpu.api_reg_read(reg), "{reg:?}");
    }
}

#[test]
fn cpu_only_long64_paging_uses_the_real_engine_without_a_board() {
    let mut code = alloc::vec![0x48, 0xB8]; // mov rax,7
    code.extend_from_slice(&7u64.to_le_bytes());
    code.extend_from_slice(&[0x48, 0x83, 0xC0, 5, 0x48, 0xBB]); // add rax,5; mov rbx,DATA
    code.extend_from_slice(&DATA.to_le_bytes());
    code.extend_from_slice(&[0x48, 0x89, 0x03, 0x48, 0x8B, 0x33, 0xF4]); // store; load; hlt
    let mut m = Minimal::new(&code);
    // One identity-mapped 2 MiB page; unused table entries stay not present.
    // The table pages do not overlap CODE or DATA.
    m.write(0x1000, &0x4003u64.to_le_bytes());
    m.write(0x4000, &0x5003u64.to_le_bytes());
    m.write(0x5000, &0x83u64.to_le_bytes());
    m.cpu
        .set_seg_for_api(X86Reg::Cs, 8, 0, u32::MAX, SegmentSize::Long64);
    m.cpu.enter_long_mode_for_api(0x1000);
    assert_eq!(m.run(6).unwrap(), 6);
    assert_eq!(m.cpu.rax(), 12);
    assert_eq!(m.cpu.rsi(), 12);
    assert_eq!(m.read::<8>(DATA), 12u64.to_le_bytes());
    assert_eq!(m.cpu.rip(), CODE + code.len() as u64);
}

#[test]
fn cpu_only_io_privilege_fault_precedes_the_missing_provider_exit() {
    use crate::config::BxLinAddress;
    let mut m = Minimal::new(&[0xEE]); // out dx,al
    m.cpu.set_edx(0x21);
    // Reset supplies a usable cached TR; explicitly model a profile with no
    // task-state segment rather than accidentally granting its zero bitmap.
    m.cpu.tr.cache.valid = 0;
    // A same-ring CPL3 #GP handler needs no TSS stack transition. Install
    // actual GDT/IDT bytes so assertions observe delivery, not a diagnostic
    // counter or the failure of a missing handler.
    m.write(0x1018, &0x00CF_FB00_0000_FFFFu64.to_le_bytes()); // ring-3 code
    m.write(0x1020, &0x00CF_F300_0000_FFFFu64.to_le_bytes()); // ring-3 data
    m.cpu.gdtr.base = BxLinAddress::new(0x1000);
    m.cpu.gdtr.limit = 0x27;
    m.cpu.idtr.base = BxLinAddress::new(0x4000);
    m.cpu.idtr.limit = 13 * 8 + 7;
    m.write(0x4000 + 13 * 8, &[0, 0x30, 0x1B, 0, 0, 0xEE, 0, 0]);
    m.write(0x3000, &[0x90]); // handler's first instruction remains unexecuted
    m.cpu
        .set_seg_for_api(X86Reg::Cs, 0x1B, 0, u32::MAX, SegmentSize::Bits32);
    m.cpu
        .set_seg_for_api(X86Reg::Ss, 0x23, 0, u32::MAX, SegmentSize::Bits32);
    // CPL=3, IOPL=0, no valid TSS permission bitmap: #GP must precede
    // any attempt to obtain a port provider. The existing Bochs loop charges
    // one count for exception delivery; the OUT itself never retires.
    let exit = m.run(1);
    assert!(exit.is_ok(), "{exit:?}");
    assert_eq!(m.cpu.rip(), 0x3000, "the guest entered its #GP handler");
    assert_eq!(m.read::<4>(0x8FF0), 0u32.to_le_bytes(), "#GP(0)");
    assert_eq!(
        m.read::<4>(0x8FF4),
        (CODE as u32).to_le_bytes(),
        "fault saves the OUT's IP"
    );
    assert_eq!(m.cpu.icount, 1);
}

#[test]
fn cpu_only_ipi_reports_post_retirement_machine_work_without_losing_it() {
    // mov eax,ICR(all-but-self, vector 0x40); mov [LAPIC.ICR_LOW],eax; hlt.
    // Bochs apic.cc write_aligned / send_ipi: the local write retires, but
    // delivery belongs to the machine, which this profile does not have.
    let mut m = Minimal::new(&[0xB8, 0x40, 0, 0x0C, 0, 0xA3, 0, 3, 0xE0, 0xFE, 0xF4]);
    for _ in 0..2 {
        let exit = m.run(3);
        assert!(
            matches!(&exit, Err(CpuError::MachineBoundaryUnavailable)),
            "{exit:?}; RIP={:#x}; count={}; LAPIC work={}",
            m.cpu.rip(),
            m.cpu.icount,
            m.cpu.lapic.has_scheduler_work()
        );
        assert_eq!(
            m.cpu.icount, 2,
            "retry must not retire the LAPIC write twice"
        );
        assert_eq!(
            m.cpu.rip(),
            CODE + 10,
            "the committed write must NOT rewind"
        );
        assert!(m.cpu.wants_a_machine_boundary());
        assert!(
            m.cpu.lapic.has_scheduler_work(),
            "pending IPI remains for an owner to service"
        );
        assert_eq!(
            m.cpu.activity_state,
            CpuActivityState::Active,
            "following HLT did not execute"
        );
    }
}
