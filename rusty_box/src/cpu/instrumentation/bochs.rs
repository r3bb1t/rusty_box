//! Monomorphized instrumentation trait (primary API).
//!
//! Full-fidelity port of the C++ BOCHS instrumentation callbacks
//! (cpp_orig/bochs/instrument/stubs/instrument.h). All methods have
//! default no-op implementations — override only the hooks you need.
//!
//! ## Design
//!
//! The trait is generic (`T: Instrumentation`) rather than object-safe
//! (`Box<dyn Instrumentation>`). Composition is achieved through tuple
//! types — `(A, B)` implements `Instrumentation` when both `A` and `B`
//! do. No `Any` downcasting, no vtable dispatch on the hot path.
//!
//! `active_hooks()` returns a `HookMask` so the CPU hot path can skip
//! categories with no active hooks (predicted-not-taken branch, zero
//! cost when no instrumentation is attached).
//!
//! ## Callback design
//!
//! **0–2 arg hooks** take positional parameters (`exception(vector, error_code)`,
//! `clflush(laddr, paddr)`, `wrmsr(msr, value)`, …).
//!
//! **3+ arg hooks** take `&Event` structs with named fields:
//! [`OpcodeEvent`](super::types::OpcodeEvent),
//! [`MwaitEvent`](super::types::MwaitEvent),
//! [`HwInterruptEvent`](super::types::HwInterruptEvent),
//! [`LinAccess`](super::types::LinAccess),
//! [`PhyAccess`](super::types::PhyAccess),
//! [`IoHookEvent`](super::types::IoHookEvent),
//! [`PrefetchEvent`](super::types::PrefetchEvent),
//! [`MemUnmapped`](super::types::MemUnmapped),
//! [`MemPermViolation`](super::types::MemPermViolation),
//! [`BranchEvent`](super::types::BranchEvent). Adding a field is not a breaking
//! change, and call sites are self-documenting.
//!
//! **Consolidated branch hook.** BOCHS has four branch callbacks
//! (`cnear_taken`, `cnear_not_taken`, `ucnear`, `far`). They collapse into one
//! `fn branch(&mut self, ev: &BranchEvent)`; the [`BranchEvent`](super::types::BranchEvent)
//! variant carries the distinction.
//!
//! **Memory hooks carry `&[u8]`.** [`LinAccess`](super::types::LinAccess) and
//! [`PhyAccess`](super::types::PhyAccess) expose `data: &[u8]` — length is
//! implicit in the slice, and the actual bytes are available without a second
//! memory read.
//!
//! **Syscall hook is OS-agnostic.** [`pre_syscall`](Instrumentation::pre_syscall)
//! receives [`&mut HookCtx`](super::ctx::HookCtx) and returns
//! [`InstrAction`](super::types::InstrAction). The hook reads whichever
//! registers its target OS convention uses — the library itself assumes
//! nothing about syscall ABIs. `HookCtx` also provides memory r/w and stop.
//!
//! **Idiomatic enums instead of raw ints.** `TlbCntrl`, `CacheCntrl`,
//! `MwaitFlags`, `CodeSize`, `MemType`, `MemAccessRW` — every BOCHS `unsigned`
//! that carried a finite variant set is an enum or bitflags here.

use super::ctx::HookCtx;
use super::types::{
    BranchEvent, CacheCntrl, HookMask, HwInterruptEvent, InstrAction, IoHookEvent, LinAccess,
    MemPermViolation, MemUnmapped, MwaitEvent, OpcodeEvent, PhyAccess, PrefetchEvent, ResetType,
    TlbCntrl,
};
use crate::cpu::decoder::Instruction;

/// Instrumentation trait. Implement only the callbacks you need —
/// everything defaults to a no-op.
///
/// `active_hooks()` declares which hook categories this implementation
/// cares about, enabling the CPU to skip dispatch for inactive
/// categories. The default returns `HookMask::all()` (conservative).
#[allow(unused_variables)]
/// `'static` because a machine's CPUs may be lent to it for `'static` — the
/// no-alloc store borrows caller storage that is never freed — and a tracer
/// living inside those CPUs cannot then borrow from anything shorter. A
/// tracer that wants to hand its findings to the caller as it goes therefore
/// shares them through an owned handle — an `Arc<Mutex<_>>`, a channel — not
/// through a borrow of the caller's data.
///
/// `Default` because a hook runs with `&mut` access to the whole processor
/// and the tracer lives inside the processor, so dispatching one has to move
/// the tracer out for the duration and put it back after. Being able to leave
/// a default in the slot is what makes that window hold a real tracer rather
/// than an absence every reader would have to answer for — which is what
/// `Emulator::instrumentation` used to answer with a panic. If your tracer
/// owns something it cannot make a default of, hold it in an `Option` field.
pub trait Instrumentation: Default + 'static {
    /// Declare which hook categories this implementation uses.
    /// The CPU skips dispatch for categories not in the returned mask.
    fn active_hooks(&self) -> HookMask {
        HookMask::all()
    }

    // ── Lifecycle ──────────────────────────────────────────────────────────

    /// CPU reset.
    fn reset(&mut self, reset_type: ResetType) {}

    // ── Execution (hot path) ──────────────────────────────────────────────────

    /// Before each instruction executes.
    fn before_execution(&mut self, rip: u64, instr: &Instruction) {}

    /// After each instruction executes successfully.
    fn after_execution(&mut self, rip: u64, instr: &Instruction) {}

    /// Start of each REP / REPE / REPNE iteration.
    fn repeat_iteration(&mut self, rip: u64, instr: &Instruction) {}

    /// The decoder produced an instruction. `ev.bytes` is the raw opcode
    /// as it appeared in memory; `ev.instr` is the decoded form.
    fn opcode(&mut self, ev: &OpcodeEvent) {}

    // ── CPU state ─────────────────────────────────────────────────────────

    /// HLT instruction.
    fn hlt(&mut self) {}

    /// MWAIT / MWAITX.
    fn mwait(&mut self, ev: &MwaitEvent) {}

    // ── Branch (unified) ─────────────────────────────────────────────────

    /// Any branch (conditional near, unconditional near, or far). The
    /// variant of `BranchEvent` tells them apart — match on it if you only
    /// care about one kind.
    fn branch(&mut self, ev: &BranchEvent) {}

    // ── Syscall (can alter architectural effects) ───────────────────────────────────

    /// Fires on SYSCALL and on SYSENTER once the instruction is known to be a
    /// system call — EFER.SCE set for SYSCALL; for SYSENTER, protected mode and
    /// FRED or a usable `IA32_SYSENTER_CS` — and before the architectural
    /// CS/RIP transition. A SYSENTER that faults on those checks never reaches
    /// it; one whose long-mode `SYSENTER_EIP`/`ESP` is not canonical faults
    /// after it, because Bochs checks those after clearing VM, IF and RF.
    /// `ctx` exposes full CPU access (register r/w, memory r/w, stop).
    /// OS-agnostic — the hook reads whichever registers its target OS
    /// convention uses.
    ///
    /// Returns an [`InstrAction`] controlling what happens next:
    /// - [`InstrAction::Continue`]: architectural transition proceeds.
    /// - [`InstrAction::Skip`]: skip the transition; RIP advances past the
    ///   opcode only.
    /// - [`InstrAction::Stop`]: transition runs, then CPU stops.
    /// - [`InstrAction::SkipAndStop`]: both.
    ///
    /// This hook has no [`HookMask`] category and is not gated by one — it
    /// runs on every fast system call whatever `active_hooks()` returned,
    /// because its answer decides what the processor does next. A tracer that
    /// wants only this hook therefore declares nothing.
    #[allow(unused_variables)]
    fn pre_syscall(&mut self, ctx: &mut HookCtx) -> InstrAction {
        InstrAction::Continue
    }

    // ── Interrupts / Exceptions ────────────────────────────────────────────────

    /// Software interrupt (INT n).
    fn interrupt(&mut self, vector: u8) {}

    /// Exception delivery.
    fn exception(&mut self, vector: u8, error_code: u32) {}

    /// Hardware interrupt delivery.
    fn hwinterrupt(&mut self, ev: &HwInterruptEvent) {}

    // ── Memory ─────────────────────────────────────────────────────────────

    /// Linear memory access. `ev.data` is the actual bytes touched.
    fn lin_access(&mut self, ev: &LinAccess) {}

    /// Physical memory access the processor makes on its own account:
    /// paging-structure reads and accessed/dirty writes during a page walk,
    /// VMCS and VMCB fields, SVM and VMX permission bitmaps. `ev.data` is the
    /// actual bytes touched.
    ///
    /// Walks made on a host's behalf — `virt_to_phys`, a snapshot read — are
    /// not reported: the guest never made them.
    fn phy_access(&mut self, ev: &PhyAccess) {}

    // ── I/O ─────────────────────────────────────────────────────────────────

    /// I/O port read — fires BEFORE the read, value is unknown.
    fn inp(&mut self, port: u16, size: u8) {}

    /// I/O port read — fires AFTER the read, value is known.
    fn inp2(&mut self, ev: &IoHookEvent) {}

    /// I/O port write.
    fn outp(&mut self, ev: &IoHookEvent) {}

    // ── TLB / Cache ────────────────────────────────────────────────────────

    /// TLB control operation (MOV to CR0/CR3/CR4, task/context switch, INVLPG...).
    fn tlb_cntrl(&mut self, what: TlbCntrl) {}

    /// Cache control (INVD / WBINVD).
    fn cache_cntrl(&mut self, what: CacheCntrl) {}

    /// CLFLUSH instruction.
    fn clflush(&mut self, laddr: u64, paddr: u64) {}

    /// Prefetch hint.
    fn prefetch_hint(&mut self, ev: &PrefetchEvent) {}

    // ── Other ───────────────────────────────────────────────────────────────────

    /// CPUID instruction.
    fn cpuid(&mut self) {}

    /// WRMSR instruction.
    fn wrmsr(&mut self, msr: u32, value: u64) {}

    /// VMX exit, fired before the exit saves guest state or loads the host's.
    /// `reason` is the VMCS exit-reason word, with bit 31 set when the exit is
    /// a VM-entry failure. SVM's `#VMEXIT` does not come through here, matching
    /// Bochs, whose `BX_INSTR_VMEXIT` is raised only by `vmx.cc VMexit`.
    fn vmexit(&mut self, reason: u32, qualification: u64) {}

    // ── Unicorn-inspired hooks ────────────────────────────────────────────────

    /// Start of a basic block (trace).
    fn block_start(&mut self, rip: u64, block_size: u16) {}

    /// Before an undefined/unrecognized instruction raises #UD. Return
    /// `true` to suppress.
    fn invalid_instruction(&mut self, rip: u64) -> bool {
        false
    }

    /// Access to a not-present page. Return `true` to suppress the fault.
    fn mem_unmapped(&mut self, ev: &MemUnmapped) -> bool {
        false
    }

    /// Access denied by the `PagePermissions` bitmap. Return `true` to
    /// suppress the fault.
    fn mem_perm_violation(&mut self, ev: &MemPermViolation) -> bool {
        false
    }
}

// ── Unit impl (no-op sentinel) ───────────────────────────────────────────────

impl Instrumentation for () {
    /// Zero-cost no-op observer — tell the CPU to skip every dispatch.
    fn active_hooks(&self) -> HookMask {
        HookMask::empty()
    }
}

// ── Tuple composition ───────────────────────────────────────────────────

macro_rules! impl_instrumentation_tuple {
    ($($T:ident),+) => {
        #[allow(non_snake_case)]
        impl<$($T: Instrumentation),+> Instrumentation for ($($T,)+) {
            fn active_hooks(&self) -> HookMask {
                let ($($T,)+) = self;
                HookMask::empty() $(| $T.active_hooks())+
            }

            fn reset(&mut self, reset_type: ResetType) {
                let ($($T,)+) = self;
                $($T.reset(reset_type);)+
            }

            fn before_execution(&mut self, rip: u64, instr: &Instruction) {
                let ($($T,)+) = self;
                $($T.before_execution(rip, instr);)+
            }

            fn after_execution(&mut self, rip: u64, instr: &Instruction) {
                let ($($T,)+) = self;
                $($T.after_execution(rip, instr);)+
            }

            fn repeat_iteration(&mut self, rip: u64, instr: &Instruction) {
                let ($($T,)+) = self;
                $($T.repeat_iteration(rip, instr);)+
            }

            fn opcode(&mut self, ev: &OpcodeEvent) {
                let ($($T,)+) = self;
                $($T.opcode(ev);)+
            }

            fn hlt(&mut self) {
                let ($($T,)+) = self;
                $($T.hlt();)+
            }

            fn mwait(&mut self, ev: &MwaitEvent) {
                let ($($T,)+) = self;
                $($T.mwait(ev);)+
            }

            fn branch(&mut self, ev: &BranchEvent) {
                let ($($T,)+) = self;
                $($T.branch(ev);)+
            }

            fn pre_syscall(&mut self, ctx: &mut HookCtx) -> InstrAction {
                let ($($T,)+) = self;
                let mut action = InstrAction::Continue;
                $( action = action.combine($T.pre_syscall(ctx)); )+
                action
            }

            fn interrupt(&mut self, vector: u8) {
                let ($($T,)+) = self;
                $($T.interrupt(vector);)+
            }

            fn exception(&mut self, vector: u8, error_code: u32) {
                let ($($T,)+) = self;
                $($T.exception(vector, error_code);)+
            }

            fn hwinterrupt(&mut self, ev: &HwInterruptEvent) {
                let ($($T,)+) = self;
                $($T.hwinterrupt(ev);)+
            }

            fn lin_access(&mut self, ev: &LinAccess) {
                let ($($T,)+) = self;
                $($T.lin_access(ev);)+
            }

            fn phy_access(&mut self, ev: &PhyAccess) {
                let ($($T,)+) = self;
                $($T.phy_access(ev);)+
            }

            fn inp(&mut self, port: u16, size: u8) {
                let ($($T,)+) = self;
                $($T.inp(port, size);)+
            }

            fn inp2(&mut self, ev: &IoHookEvent) {
                let ($($T,)+) = self;
                $($T.inp2(ev);)+
            }

            fn outp(&mut self, ev: &IoHookEvent) {
                let ($($T,)+) = self;
                $($T.outp(ev);)+
            }

            fn tlb_cntrl(&mut self, what: TlbCntrl) {
                let ($($T,)+) = self;
                $($T.tlb_cntrl(what);)+
            }

            fn cache_cntrl(&mut self, what: CacheCntrl) {
                let ($($T,)+) = self;
                $($T.cache_cntrl(what);)+
            }

            fn clflush(&mut self, laddr: u64, paddr: u64) {
                let ($($T,)+) = self;
                $($T.clflush(laddr, paddr);)+
            }

            fn prefetch_hint(&mut self, ev: &PrefetchEvent) {
                let ($($T,)+) = self;
                $($T.prefetch_hint(ev);)+
            }

            fn cpuid(&mut self) {
                let ($($T,)+) = self;
                $($T.cpuid();)+
            }

            fn wrmsr(&mut self, msr: u32, value: u64) {
                let ($($T,)+) = self;
                $($T.wrmsr(msr, value);)+
            }

            fn vmexit(&mut self, reason: u32, qualification: u64) {
                let ($($T,)+) = self;
                $($T.vmexit(reason, qualification);)+
            }

            fn block_start(&mut self, rip: u64, block_size: u16) {
                let ($($T,)+) = self;
                $($T.block_start(rip, block_size);)+
            }

            fn invalid_instruction(&mut self, rip: u64) -> bool {
                let ($($T,)+) = self;
                false $(|| $T.invalid_instruction(rip))+
            }

            fn mem_unmapped(&mut self, ev: &MemUnmapped) -> bool {
                let ($($T,)+) = self;
                false $(|| $T.mem_unmapped(ev))+
            }

            fn mem_perm_violation(&mut self, ev: &MemPermViolation) -> bool {
                let ($($T,)+) = self;
                false $(|| $T.mem_perm_violation(ev))+
            }
        }
    }
}

impl_instrumentation_tuple!(A);
impl_instrumentation_tuple!(A, B);
impl_instrumentation_tuple!(A, B, C);
impl_instrumentation_tuple!(A, B, C, D);
impl_instrumentation_tuple!(A, B, C, D, E);
impl_instrumentation_tuple!(A, B, C, D, E, F);
impl_instrumentation_tuple!(A, B, C, D, E, F, G);
impl_instrumentation_tuple!(A, B, C, D, E, F, G, H);

/// A hook the processor never calls is a promise the API does not keep, and
/// nothing in the type system says otherwise: every callback here compiles
/// whether or not a single call site exists. The delivery of `phy_access`,
/// `vmexit` and `pre_syscall` is asserted here, against a machine driven the
/// way a caller drives one.
#[cfg(all(test, feature = "std"))]
mod hook_delivery {
    use super::*;
    use crate::cpu::instrumentation::{MemAccessRW, PhyAccess};
    use crate::cpu::msr::{
        BX_MSR_EFER, BX_MSR_LSTAR, BX_MSR_STAR, BX_MSR_SYSENTER_CS, BX_MSR_SYSENTER_EIP,
        BX_MSR_SYSENTER_ESP,
    };
    use crate::cpu::vmx::VmxVmexitReason;
    use crate::cpu::{CpuSetupMode, X86Reg};
    use crate::emulator::{Emulator, EmulatorConfig, MemorySize};

    /// An `Emulator` is several MiB; a test that builds one needs its own
    /// stack rather than libtest's.
    const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;

    fn on_a_large_stack(f: impl FnOnce() + Send + 'static) {
        match std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(f)
        {
            Ok(handle) => match handle.join() {
                Ok(()) => {}
                Err(panic) => std::panic::resume_unwind(panic),
            },
            Err(e) => panic!("test thread: {e}"),
        }
    }

    fn small_machine() -> EmulatorConfig {
        EmulatorConfig {
            memory: MemorySize::bytes(8 * 1024 * 1024),
            ..EmulatorConfig::default()
        }
    }

    /// Guest code sits above every structure `setup_cpu_mode` writes, so the
    /// setup cannot overwrite it.
    const CODE: u64 = 0x0010_0000;
    /// Where SYSENTER / SYSCALL are pointed. Far enough from `CODE` that the
    /// two are told apart by RIP alone.
    const HANDLER: u64 = 0x0011_0000;

    // ── phy_access ──────────────────────────────────────────────────────

    /// One physical access as the hook reported it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct PhyRecord {
        phy: u64,
        len: usize,
        memtype: crate::cpu::instrumentation::MemType,
    }

    /// Records every physical access the processor reports. A page-table
    /// walk is the one an automation caller expects to see, so the address
    /// and direction are kept, not just a count.
    #[derive(Default)]
    struct PhyWatch {
        reads: std::vec::Vec<PhyRecord>,
        writes: std::vec::Vec<PhyRecord>,
    }

    impl PhyWatch {
        /// Whether an access of `len` bytes at `phy` was reported as a read.
        fn read(&self, phy: u64, len: usize) -> bool {
            self.reads.iter().any(|access| access.phy == phy && access.len == len)
        }

        /// Whether an access of `len` bytes at `phy` was reported as a write.
        fn wrote(&self, phy: u64, len: usize) -> bool {
            self.writes.iter().any(|access| access.phy == phy && access.len == len)
        }

        /// The memory types reported for accesses at `phy`, reads and writes.
        fn types_at(&self, phy: u64) -> std::vec::Vec<crate::cpu::instrumentation::MemType> {
            self.reads
                .iter()
                .chain(&self.writes)
                .filter(|access| access.phy == phy)
                .map(|access| access.memtype)
                .collect()
        }
    }

    impl Instrumentation for PhyWatch {
        fn active_hooks(&self) -> HookMask {
            HookMask::MEM
        }

        fn phy_access(&mut self, ev: &PhyAccess) {
            let access = PhyRecord { phy: ev.phy, len: ev.data.len(), memtype: ev.memtype };
            match ev.rw {
                MemAccessRW::Write => self.writes.push(access),
                _ => self.reads.push(access),
            }
        }
    }

    /// The paging structures a long-mode machine walks to reach its own code.
    /// `setup_flat_long64` puts the PML4 at 0x1000 and the PDPT at 0x2000, so
    /// a fetch of any linear address reads both before it reaches a frame.
    const PML4: u64 = 0x1000;
    const PDPT: u64 = 0x2000;

    #[test]
    fn a_page_table_walk_reaches_the_phy_access_hook() {
        on_a_large_stack(|| {
            let mut machine = Emulator::<PhyWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatLong64,
                PhyWatch::default(),
            )
            .expect("machine");

            machine.mem_write(CODE, &[0x90, 0xF4]).expect("write code");
            machine
                .emu_start(CODE, None, None, Some(2))
                .expect("emu_start");

            let seen = machine.instrumentation();
            assert!(
                seen.read(PML4, 8),
                "the walk that resolved the first fetch read the PML4 entry at \
                 {PML4:#x}, and the hook must report it (Bochs paging.cc \
                 read_physical_qword): reads={:x?}",
                seen.reads
            );
            assert!(
                seen.read(PDPT, 8),
                "and the PDPT entry at {PDPT:#x}: reads={:x?}",
                seen.reads
            );
            assert!(
                seen.writes.iter().any(|access| access.phy >= PML4 && access.len == 8),
                "the accessed/dirty update the same walk performs is a physical \
                 WRITE, and must be reported as one: writes={:x?}",
                seen.writes
            );
            assert_eq!(
                seen.types_at(PML4)[0],
                crate::cpu::instrumentation::MemType::Invalid,
                "a long-mode walk reports BX_MEMTYPE_INVALID, as Bochs \
                 translate_linear_long_mode passes it"
            );
        });
    }

    /// A host asking where an address maps, or reading guest memory through
    /// the guest's page tables, is not the guest making an access, so no walk
    /// it causes reaches the hook — as Bochs's debugger translation
    /// `dbg_xlate_linear2phy` carries no instrumentation.
    #[test]
    fn a_host_translation_reports_nothing_to_the_phy_access_hook() {
        on_a_large_stack(|| {
            let mut machine = Emulator::<PhyWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatLong64,
                PhyWatch::default(),
            )
            .expect("machine");
            machine.mem_write(CODE, &[0x90, 0xF4]).expect("write code");

            machine.virt_to_phys(CODE).expect("the code page maps");
            let mut bytes = [0u8; 2];
            machine.virt_read(CODE, &mut bytes).expect("virt_read");

            let seen = machine.instrumentation();
            assert!(
                seen.reads.is_empty() && seen.writes.is_empty(),
                "host translation reached the hook: reads={:x?} writes={:x?}",
                seen.reads,
                seen.writes
            );
        });
    }

    /// An SMI saves the processor into SMRAM's state-save area, one dword at
    /// a time down from SMBASE + 0x10000, and Bochs smm.cc
    /// `enter_system_management_mode` reports every store. rombios32 takes an
    /// SMI in `smm_init`, so a tracer sees these on every boot.
    #[test]
    fn an_smi_reports_its_state_save_to_the_phy_access_hook() {
        on_a_large_stack(|| {
            let mut machine = Emulator::<PhyWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatProtected32,
                PhyWatch::default(),
            )
            .expect("machine");

            let smbase = {
                let mut ctx = machine.exec_ctx(0);
                let smbase = u64::from(ctx.smbase);
                ctx.enter_system_management_mode();
                smbase
            };

            let save_area = (smbase + 0x10000
                - 4 * u64::from(crate::cpu::smm::SMM_SAVE_STATE_MAP_SIZE))
                ..(smbase + 0x10000);
            let saved = machine
                .instrumentation()
                .writes
                .iter()
                .filter(|access| access.len == 4 && save_area.contains(&access.phy))
                .count();
            assert_eq!(
                saved,
                crate::cpu::smm::SMM_SAVE_STATE_MAP_SIZE as usize,
                "every dword of the state-save area is a reported physical write"
            );
            let top = smbase + 0x10000 - 4;
            assert_eq!(
                machine.instrumentation().types_at(top),
                [crate::cpu::instrumentation::MemType::Wb],
                "SMRAM is reported write-back, as smm.cc passes BX_MEMTYPE_WB"
            );
        });
    }

    /// A VMEXIT writes the guest's state into its VMCB, and Bochs svm.cc
    /// `vmcb_write*` reports every field store, whether through the cached
    /// host pointer or not.
    #[test]
    fn a_vmexit_reports_its_vmcb_stores_to_the_phy_access_hook() {
        on_a_large_stack(|| {
            const VMCB: u64 = 0x4_0000;

            let mut machine = Emulator::<PhyWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatProtected32,
                PhyWatch::default(),
            )
            .expect("machine");

            {
                let mut ctx = machine.exec_ctx(0);
                ctx.set_vmcbptr(VMCB);
                match ctx.svm_vmexit(crate::cpu::svm::SVM_VMEXIT_INVALID, 0, 0) {
                    Ok(()) | Err(crate::cpu::CpuError::CpuLoopRestart) => {}
                    Err(other) => panic!("the exit itself failed: {other:?}"),
                }
            }

            let seen = machine.instrumentation();
            assert!(
                seen.writes
                    .iter()
                    .any(|access| (VMCB..VMCB + 0x1000).contains(&access.phy)),
                "the guest state saved into the VMCB must be reported: writes={:x?}",
                seen.writes
            );
            assert!(
                seen.reads
                    .iter()
                    .chain(&seen.writes)
                    .filter(|access| (VMCB..VMCB + 0x1000).contains(&access.phy))
                    .all(|access| access.memtype == crate::cpu::instrumentation::MemType::Uc),
                "VMCB accesses report UC, what tlb.h MEMTYPE() yields in Bochs's \
                 default build"
            );
        });
    }

    /// Bochs vapic.cc `VMX_Posted_Interrupt_Processing` reads and clears the
    /// whole PIR as one 32-byte access each, and `VMX_Read_Virtual_APIC` /
    /// `VMX_Write_Virtual_APIC` move each virtual-APIC register as one 4-byte
    /// access, so a tracer sees those widths, not one event per byte.
    #[test]
    fn posted_interrupts_report_the_pir_and_virtual_apic_at_their_width() {
        on_a_large_stack(|| {
            use crate::cpu::vmx::{
                VMX_PIN_BASED_VMEXEC_CTRL_PROCESS_POSTED_INTERRUPTS,
                VMX_VM_EXEC_CTRL1_SECONDARY_CONTROLS, VMX_VM_EXEC_CTRL1_TPR_SHADOW,
                VMX_VM_EXEC_CTRL2_VIRTUAL_INT_DELIVERY,
            };
            const VAPIC_PAGE: u64 = 0x2_0000;
            const PID: u64 = 0x3_0000;
            const NOTIFICATION: u8 = 0xF2;

            let mut machine = Emulator::<PhyWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatProtected32,
                PhyWatch::default(),
            )
            .expect("machine");
            // Vector 0x30 posted, and the descriptor's ON bit set.
            machine.mem_write(PID + 0x30 / 8, &[1 << (0x30 % 8)]).expect("PIR");
            machine.mem_write(PID + 32, &[1]).expect("PID.ON");

            {
                let mut ctx = machine.exec_ctx(0);
                ctx.memory.set_a20_mask(u64::MAX);
                ctx.in_vmx = true;
                ctx.in_vmx_guest = true;
                ctx.vmcs.virtual_apic_page_addr = VAPIC_PAGE;
                ctx.vmcs.pi_desc_addr = PID;
                ctx.vmcs.proc_based_ctls =
                    VMX_VM_EXEC_CTRL1_TPR_SHADOW | VMX_VM_EXEC_CTRL1_SECONDARY_CONTROLS;
                ctx.vmcs.secondary_proc_based_ctls = VMX_VM_EXEC_CTRL2_VIRTUAL_INT_DELIVERY;
                ctx.vmcs.pin_based_ctls = VMX_PIN_BASED_VMEXEC_CTRL_PROCESS_POSTED_INTERRUPTS;
                ctx.vmcs.pi_notification_vector = u16::from(NOTIFICATION);
                assert!(ctx.vmx_posted_interrupt_processing(NOTIFICATION));
            }

            let seen = machine.instrumentation();
            assert!(seen.read(PID, 32), "the PIR read: reads={:x?}", seen.reads);
            assert!(seen.wrote(PID, 32), "the PIR clear: writes={:x?}", seen.writes);
            let pir_bytes = |accesses: &[PhyRecord]| {
                accesses
                    .iter()
                    .any(|access| (PID..PID + 32).contains(&access.phy) && access.len != 32)
            };
            assert!(
                !pir_bytes(&seen.reads) && !pir_bytes(&seen.writes),
                "no PIR access is reported in pieces"
            );
            let vapic: std::vec::Vec<usize> = seen
                .reads
                .iter()
                .chain(&seen.writes)
                .filter(|access| (VAPIC_PAGE..VAPIC_PAGE + 0x1000).contains(&access.phy))
                .map(|access| access.len)
                .collect();
            assert!(
                !vapic.is_empty() && vapic.iter().all(|&len| len == 4),
                "each virtual-APIC register moves as one 4-byte access: {vapic:?}"
            );
        });
    }

    /// An EPT walk reads one entry per level and sets their accessed bits,
    /// and Bochs paging.cc `translate_guest_physical` and
    /// `update_ept_access_dirty` report each read and each store.
    #[test]
    fn an_ept_walk_reports_its_entries_to_the_phy_access_hook() {
        on_a_large_stack(|| {
            use crate::cpu::vmx::{
                BxRwAccess, VMX_VM_EXEC_CTRL1_SECONDARY_CONTROLS,
                VMX_VM_EXEC_CTRL2_EPT_ENABLE,
            };
            const EPT_PML4: u64 = 0x20_0000;
            const EPT_PDPT: u64 = 0x20_1000;
            const EPT_PD: u64 = 0x20_2000;
            const EPT_PT: u64 = 0x20_3000;
            const EPT_TARGET: u64 = 0x20_4000;
            const EPT_RWX: u64 = 0x7;
            const EPT_WB_LEAF: u64 = 0x7 | (6 << 3);

            let mut machine = Emulator::<PhyWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatProtected32,
                PhyWatch::default(),
            )
            .expect("machine");

            {
                let mut ctx = machine.exec_ctx(0);
                ctx.memory.set_a20_mask(u64::MAX);
                ctx.in_vmx = true;
                ctx.in_vmx_guest = true;
                ctx.vmcs.proc_based_ctls = VMX_VM_EXEC_CTRL1_SECONDARY_CONTROLS;
                ctx.vmcs.secondary_proc_based_ctls = VMX_VM_EXEC_CTRL2_EPT_ENABLE;
                // EPTP bit 6 turns on the accessed/dirty updates the walk makes.
                ctx.vmcs.eptptr = EPT_PML4 | 0x40;
                ctx.mem_write_qword(EPT_PML4, EPT_PDPT | EPT_RWX);
                ctx.mem_write_qword(EPT_PDPT, EPT_PD | EPT_RWX);
                ctx.mem_write_qword(EPT_PD, EPT_PT | EPT_RWX);
                ctx.mem_write_qword(EPT_PT + 8, EPT_TARGET | EPT_WB_LEAF);

                ctx.ept_translate_for_data(0x1000, 0, false, true, false, BxRwAccess::Read)
                    .expect("a fully permitted mapping translates");
            }

            let seen = machine.instrumentation();
            for entry in [EPT_PML4, EPT_PDPT, EPT_PD, EPT_PT + 8] {
                assert!(
                    seen.read(entry, 8),
                    "the walk read the EPT entry at {entry:#x}: reads={:x?}",
                    seen.reads
                );
            }
            assert!(
                seen.wrote(EPT_PML4, 8),
                "setting the accessed bit is a reported store: writes={:x?}",
                seen.writes
            );
        });
    }

    // ── vmexit ──────────────────────────────────────────────────────────

    /// One VM exit as the hook reported it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct ReportedExit {
        reason: u32,
        qualification: u64,
    }

    /// Records every VM exit, and counts the context switches the processor
    /// reports — Bochs `BX_INSTR_TLB_CNTRL(BX_INSTR_CONTEXT_SWITCH)`, which
    /// ends every SMM, SVM and VMX change of context.
    #[derive(Default)]
    struct VmexitWatch {
        exits: std::vec::Vec<ReportedExit>,
        context_switches: usize,
    }

    impl Instrumentation for VmexitWatch {
        fn active_hooks(&self) -> HookMask {
            HookMask::VMEXIT | HookMask::TLB
        }

        fn vmexit(&mut self, reason: u32, qualification: u64) {
            self.exits.push(ReportedExit { reason, qualification });
        }

        fn tlb_cntrl(&mut self, what: TlbCntrl) {
            if what == TlbCntrl::ContextSwitch {
                self.context_switches += 1;
            }
        }
    }

    /// A guest in VMX non-root, with a 32-bit host to return to. Placed
    /// directly: the tests that use it assert what an exit does, not what
    /// VMENTRY validates.
    fn place_a_vmx_guest<T: Instrumentation>(ctx: &mut crate::cpu::exec_ctx::ExecCtx<'_, T>) {
        ctx.in_vmx = true;
        ctx.in_vmx_guest = true;
        ctx.vmcs.host_cr0 = u64::from(ctx.cr0.get32());
        ctx.vmcs.host_cr4 = 0;
        ctx.vmcs.host_cr3 = 0;
        ctx.vmcs.host_rsp = 0x0002_0000;
        ctx.vmcs.host_rip = CODE;
        ctx.vmcs.host_cs_selector = 0x08;
        ctx.vmcs.host_ss_selector = 0x10;
        ctx.vmcs.host_ds_selector = 0x10;
        ctx.vmcs.host_es_selector = 0x10;
        ctx.vmcs.host_fs_selector = 0x10;
        ctx.vmcs.host_gs_selector = 0x10;
        ctx.vmcs.host_tr_selector = 0x18;
        ctx.vmcs.host_gdtr_base = 0x0800;
    }

    /// Bochs exception.cc `BX_ET_DOUBLE_FAULT`: a #DF is being delivered, so
    /// the next fault is the third.
    const DOUBLE_FAULT_IN_DELIVERY: i32 = 10;

    /// A triple fault in a VMX guest is its host's (Bochs vmexit.cc
    /// `VMexit_TripleFault`): the processor exits with reason 2 and neither
    /// resets the machine nor shuts down, and the host's context comes back
    /// through the context-switch tail, which the hook reports.
    #[test]
    fn a_triple_fault_in_a_vmx_guest_exits_to_its_host() {
        on_a_large_stack(|| {
            let mut machine = Emulator::<VmexitWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatProtected32,
                VmexitWatch::default(),
            )
            .expect("machine");

            {
                let mut ctx = machine.exec_ctx(0);
                place_a_vmx_guest(&mut ctx);
                ctx.last_exception_type = DOUBLE_FAULT_IN_DELIVERY;
                let fault = ctx.exception(crate::cpu::cpu::Exception::Gp, 0);
                assert!(
                    matches!(fault, Err(crate::cpu::CpuError::CpuLoopRestart)),
                    "the exit ends the faulting instruction: {fault:?}"
                );
                assert!(!ctx.in_vmx_guest, "the processor is back in its host");
                assert!(!ctx.is_in_shutdown(), "the host keeps running");
            }

            let seen = machine.instrumentation();
            assert_eq!(
                seen.exits,
                std::vec![ReportedExit {
                    reason: VmxVmexitReason::TripleFault as u32,
                    qualification: 0,
                }]
            );
            assert_eq!(seen.context_switches, 1, "the host state load ends in a context switch");
        });
    }

    /// A fault the host intercepts ends the guest's instruction there: the
    /// exit loads the host, and a caller that raised the fault mid-instruction
    /// must not go on to finish it against host state (Bochs `VMexit` ends in
    /// a longjmp).
    #[test]
    fn an_intercepted_guest_fault_ends_the_instruction_that_raised_it() {
        on_a_large_stack(|| {
            let mut machine = Emulator::<VmexitWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatProtected32,
                VmexitWatch::default(),
            )
            .expect("machine");

            let mut ctx = machine.exec_ctx(0);
            place_a_vmx_guest(&mut ctx);
            ctx.vmcs.exception_bitmap = 1 << 13;
            let fault = ctx.exception(crate::cpu::cpu::Exception::Gp, 0);
            assert!(
                matches!(fault, Err(crate::cpu::CpuError::CpuLoopRestart)),
                "an intercepted #GP must unwind the instruction, not return to it: {fault:?}"
            );
            assert!(!ctx.in_vmx_guest);
        });
    }

    /// An SVM guest whose host intercepts `SHUTDOWN` takes that exit on a
    /// triple fault (Bochs exception.cc, the `SVM_INTERCEPT0_SHUTDOWN` test
    /// before the reset), and its VMCB says why.
    #[test]
    fn a_triple_fault_in_an_svm_guest_that_intercepts_shutdown_exits_to_its_host() {
        on_a_large_stack(|| {
            const VMCB: u64 = 0x4_0000;
            let mut machine = Emulator::<VmexitWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatProtected32,
                VmexitWatch::default(),
            )
            .expect("machine");

            {
                let mut ctx = machine.exec_ctx(0);
                ctx.set_vmcbptr(VMCB);
                ctx.in_svm_guest = true;
                ctx.vmcb.ctrls.intercept_vector[0] |= 1 << crate::cpu::svm::SVM_INTERCEPT0_SHUTDOWN;
                ctx.last_exception_type = DOUBLE_FAULT_IN_DELIVERY;
                let fault = ctx.exception(crate::cpu::cpu::Exception::Gp, 0);
                assert!(matches!(fault, Err(crate::cpu::CpuError::CpuLoopRestart)));
                assert!(!ctx.in_svm_guest, "the processor is back in its host");
                assert!(!ctx.is_in_shutdown());
            }

            let exit_code = machine
                .mem_read_vec(VMCB + u64::from(crate::cpu::svm::SVM_CONTROL64_EXITCODE), 8)
                .expect("VMCB");
            assert_eq!(
                u64::from_le_bytes(exit_code.try_into().expect("eight bytes")),
                crate::cpu::svm::SvmVmexit::Shutdown as u64
            );
            assert_eq!(machine.instrumentation().context_switches, 1);
        });
    }

    /// Bochs vmx.cc `VMabort` writes its code into the current VMCS's abort
    /// indicator — offset 4, vmcs.cc `VMCS_VMX_ABORT_FIELD_ADDR` — reports
    /// the write, and shuts the processor down.
    #[test]
    fn a_vmx_abort_records_its_code_in_the_vmcs_and_shuts_down() {
        on_a_large_stack(|| {
            const VMCS: u64 = 0x5_0000;
            let mut machine = Emulator::<PhyWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatProtected32,
                PhyWatch::default(),
            )
            .expect("machine");

            {
                let mut ctx = machine.exec_ctx(0);
                ctx.vmcsptr = VMCS;
                let abort = ctx.vmx_abort(crate::cpu::vmx::VmxAbortCode::LoadingHostMsrs);
                assert!(matches!(abort, Err(crate::cpu::CpuError::CpuLoopRestart)));
                assert!(ctx.is_in_shutdown());
            }

            let indicator = VMCS + crate::cpu::vmx::VMCS_VMX_ABORT_OFFSET;
            assert_eq!(
                machine.mem_read_u32_le(indicator).expect("VMCS"),
                crate::cpu::vmx::VmxAbortCode::LoadingHostMsrs as u32
            );
            assert!(machine.instrumentation().wrote(indicator, 4));
        });
    }

    /// An SMI enters system-management mode through the context-switch tail,
    /// and the hook reports it (Bochs smm.cc `enter_system_management_mode`).
    #[test]
    fn an_smi_reports_a_context_switch() {
        on_a_large_stack(|| {
            let mut machine = Emulator::<VmexitWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatProtected32,
                VmexitWatch::default(),
            )
            .expect("machine");

            machine.exec_ctx(0).enter_system_management_mode();

            assert_eq!(machine.instrumentation().context_switches, 1);
        });
    }

    /// A #VMEXIT to a PAE host whose PDPTEs set reserved bits shuts the
    /// processor down part-way through the host load (Bochs svm.cc
    /// `SvmExitLoadHostState`, `CheckPDPTR` then `shutdown()`).
    #[test]
    fn a_vmexit_to_a_pae_host_with_invalid_pdptes_shuts_down() {
        on_a_large_stack(|| {
            const VMCB: u64 = 0x4_0000;
            const HOST_PDPT: u64 = 0x6_0000;
            const HOST_RIP: u64 = 0x7_0000;
            let mut machine = Emulator::<VmexitWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatProtected32,
                VmexitWatch::default(),
            )
            .expect("machine");
            // Present, with bit 5 set — reserved in a PAE PDPTE.
            machine.mem_write(HOST_PDPT, &0x21u64.to_le_bytes()).expect("PDPTE");

            {
                let mut ctx = machine.exec_ctx(0);
                ctx.set_vmcbptr(VMCB);
                ctx.in_svm_guest = true;
                let pae_host_cr0 = ctx.cr0.get32() | 0x8000_0001;
                ctx.vmcb.host_state.cr0.set32(pae_host_cr0);
                ctx.vmcb.host_state.cr4.insert(crate::cpu::crregs::BxCr4::PAE);
                ctx.vmcb.host_state.cr3 = HOST_PDPT;
                ctx.vmcb.host_state.rip = HOST_RIP;
                let exit = ctx.svm_vmexit(crate::cpu::svm::SVM_VMEXIT_INVALID, 0, 0);
                assert!(matches!(exit, Err(crate::cpu::CpuError::CpuLoopRestart)));
                assert!(ctx.is_in_shutdown(), "a host with invalid PDPTEs cannot run");
                assert_ne!(ctx.rip(), HOST_RIP, "the load stopped before the host's RIP");
            }

            assert_eq!(
                machine.instrumentation().context_switches,
                0,
                "the load stopped before its context-switch tail"
            );
        });
    }

    #[test]
    fn a_vm_exit_reaches_the_vmexit_hook() {
        on_a_large_stack(|| {
            const QUALIFICATION: u64 = 0x5A5A_0000_1234;

            let mut machine = Emulator::<VmexitWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatProtected32,
                VmexitWatch::default(),
            )
            .expect("machine");

            {
                let mut ctx = machine.exec_ctx(0);
                place_a_vmx_guest(&mut ctx);

                match ctx.vmx_vmexit(VmxVmexitReason::Hlt, QUALIFICATION) {
                    Ok(()) => {}
                    Err(e) => panic!("the exit must complete into the host: {e:?}"),
                }
            }

            assert_eq!(
                machine.instrumentation().exits,
                std::vec![ReportedExit {
                    reason: VmxVmexitReason::Hlt as u32,
                    qualification: QUALIFICATION,
                }],
                "a VM exit must reach the hook with the reason and qualification \
                 the host reads out of the VMCS (Bochs vmx.cc VMexit)"
            );
        });
    }

    // ── pre_syscall ─────────────────────────────────────────────────────

    /// Records the RIP of every fast-syscall entry, and optionally intercepts
    /// it, so a test can prove the hook is in the control path and not merely
    /// alongside it.
    #[derive(Default)]
    struct SyscallWatch {
        entries: std::vec::Vec<u64>,
        skip: bool,
    }

    impl Instrumentation for SyscallWatch {
        /// Nothing: `pre_syscall` has no mask category, and a caller that
        /// wants only this hook must not have to invent one.
        fn active_hooks(&self) -> HookMask {
            HookMask::empty()
        }

        fn pre_syscall(&mut self, ctx: &mut HookCtx) -> InstrAction {
            self.entries.push(ctx.reg_read(X86Reg::Rip));
            if self.skip {
                InstrAction::Skip
            } else {
                InstrAction::Continue
            }
        }
    }

    /// A 32-bit flat machine whose SYSENTER MSRs point at `HANDLER`, with
    /// `SYSENTER; HLT` at `CODE` and `HLT` at `HANDLER`. The selectors are the
    /// ones `install_flat_segments` writes: code 0x08, data 0x10.
    fn sysenter_machine(skip: bool) -> alloc::boxed::Box<Emulator<SyscallWatch>> {
        let mut machine = Emulator::<SyscallWatch>::new_with_mode_and_instrumentation(
            small_machine(),
            CpuSetupMode::FlatProtected32,
            SyscallWatch {
                entries: std::vec::Vec::new(),
                skip,
            },
        )
        .expect("machine");

        machine
            .msr_write(BX_MSR_SYSENTER_CS, 0x08)
            .expect("SYSENTER_CS");
        machine
            .msr_write(BX_MSR_SYSENTER_ESP, 0x0002_0000)
            .expect("SYSENTER_ESP");
        machine
            .msr_write(BX_MSR_SYSENTER_EIP, HANDLER)
            .expect("SYSENTER_EIP");
        machine
            .mem_write(CODE, &[0x0F, 0x34, 0xF4])
            .expect("write SYSENTER");
        machine.mem_write(HANDLER, &[0xF4]).expect("write handler");
        machine
    }

    #[test]
    fn sysenter_reaches_the_pre_syscall_hook() {
        on_a_large_stack(|| {
            let mut machine = sysenter_machine(false);
            // One instruction: the SYSENTER itself, so RIP below is the one it
            // left behind and not the handler's.
            machine
                .emu_start(CODE, None, None, Some(1))
                .expect("emu_start");

            assert_eq!(
                machine.instrumentation().entries.len(),
                1,
                "SYSENTER is a fast system call, and the hook documents both \
                 forms — it must fire exactly once here"
            );
            assert_eq!(
                machine.reg_read(X86Reg::Rip),
                HANDLER,
                "and the transition it observed must still have happened"
            );
        });
    }

    /// A SYSENTER with a null `IA32_SYSENTER_CS` raises #GP (Bochs proc_ctrl.cc
    /// `SYSENTER`) and is no system call, so the hook must not see it — and a
    /// hook that asks to skip must not be able to swallow the fault.
    #[test]
    fn a_sysenter_that_faults_on_its_cs_never_reaches_the_hook() {
        on_a_large_stack(|| {
            let mut machine = sysenter_machine(true);
            machine.msr_write(BX_MSR_SYSENTER_CS, 0).expect("SYSENTER_CS");
            // What the #GP leads to in this bare machine does not matter here;
            // only whether the hook was asked about the instruction.
            match machine.emu_start(CODE, None, None, Some(1)) {
                Ok(_) | Err(_) => {}
            }

            assert!(
                machine.instrumentation().entries.is_empty(),
                "the hook saw a SYSENTER that faults: {:x?}",
                machine.instrumentation().entries
            );
        });
    }

    #[test]
    fn a_pre_syscall_hook_can_intercept_sysenter() {
        on_a_large_stack(|| {
            let mut machine = sysenter_machine(true);
            machine
                .emu_start(CODE, None, None, Some(1))
                .expect("emu_start");

            assert_eq!(machine.instrumentation().entries.len(), 1);
            assert_eq!(
                machine.reg_read(X86Reg::Rip),
                CODE + 2,
                "`InstrAction::Skip` must suppress the CS/RIP transition and \
                 leave RIP just past the opcode, which is only possible if the \
                 hook runs before the transition"
            );
        });
    }

    #[test]
    fn syscall_reaches_the_pre_syscall_hook() {
        on_a_large_stack(|| {
            // EFER.SCE enables SYSCALL; STAR bits 47:32 supply the CS selector.
            const EFER_SCE_LME_LMA: u64 = 0x501;

            let mut machine = Emulator::<SyscallWatch>::new_with_mode_and_instrumentation(
                small_machine(),
                CpuSetupMode::FlatLong64,
                SyscallWatch::default(),
            )
            .expect("machine");

            machine
                .msr_write(BX_MSR_EFER, EFER_SCE_LME_LMA)
                .expect("EFER");
            machine.msr_write(BX_MSR_STAR, 0x0008 << 32).expect("STAR");
            machine.msr_write(BX_MSR_LSTAR, HANDLER).expect("LSTAR");
            machine
                .mem_write(CODE, &[0x0F, 0x05, 0xF4])
                .expect("write SYSCALL");
            machine.mem_write(HANDLER, &[0xF4]).expect("write handler");

            machine
                .emu_start(CODE, None, None, Some(1))
                .expect("emu_start");

            assert_eq!(machine.instrumentation().entries.len(), 1);
            assert_eq!(machine.reg_read(X86Reg::Rip), HANDLER);
        });
    }
}
