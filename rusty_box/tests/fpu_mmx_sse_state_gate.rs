//! x87, MMX and SSE instructions check their CPU state where Bochs checks it:
//! once, before the instruction runs.
//!
//! Bochs marks every instruction with the state it needs — field 10 of
//! `bx_define_opcode` in cpu/decoder/ia_opcodes.def, `BX_PREPARE_FPU`,
//! `BX_PREPARE_MMX` or `BX_PREPARE_SSE` — and while the matching
//! `BX_FETCH_MODE_*_OK` bit is clear, `assignHandler`
//! (cpu/decoder/fetchdecode32.cc) gives the instruction a handler from
//! cpu/proc_ctrl.cc that raises the fault instead:
//!
//! - `BxNoFPU`: #NM when CR0.EM or CR0.TS is set;
//! - `BxNoMMX`: #UD when CR0.EM is set, else #NM when CR0.TS is set;
//! - `BxNoSSE`: #UD when CR0.EM is set or CR4.OSFXSR is clear, else #NM when
//!   CR0.TS is set.
//!
//! rusty_box applies the same gate at icache fill (`state_resolve_opcode`,
//! cpu/decoder/mod.rs), and no handler checks the state itself. The gate is
//! what an OS's lazy FPU switch relies on: it leaves CR0.TS set and the
//! register file holding another thread's values, and the first instruction
//! that touches the file must raise #NM so the OS can swap it in.
//!
//! The gate reads `fetch_mode_mask`, which every write of CR0.EM, CR0.TS,
//! CR4.OSFXSR, CR4.OSXSAVE or XCR0 has to recompute — the guest's own `CLTS`
//! and task switch among them (Bochs crregs.cc `CLTS`, tasking.cc
//! `task_switch`), and a host's import of a processor's state.

#![cfg(feature = "std")]

mod state_gate;

use rusty_box::cpu::arch_state::{ArchGroups, VcpuArchState};
use rusty_box::cpu::decoder::{Opcode, X86Feature};
use rusty_box::cpu::{
    CpuSetupMode, HookMask, Instruction, Instrumentation, OpcodeEvent, X86Reg,
};
use rusty_box::emulator::{Emulator, EmulatorConfig};
use rusty_box::params::BxParams;
use state_gate::{
    install_fault_handlers, long64_emulator, run_installed_at, run_one, run_one_at, Gate, Outcome,
    CODE, MF_GATE, NM_GATE, TEST_STACK_SIZE, UD_GATE,
};

const CR0_MP: u64 = 1 << 1;
const CR0_EM: u64 = 1 << 2;
const CR0_TS: u64 = 1 << 3;
const CR0_NE: u64 = 1 << 5;
const CR4_OSFXSR: u64 = 1 << 9;
const CR4_OSXSAVE: u64 = 1 << 18;
/// XCR0 = x87 | SSE | AVX.
const XCR0_AVX: u64 = 0x07;
/// XCR0 = x87 | SSE | AVX | OPMASK | ZMM_HI256 | HI_ZMM.
const XCR0_AVX512: u64 = 0xE7;

/// One instruction, by encoding.
struct Case {
    name: &'static str,
    code: &'static [u8],
}

/// The control-register configurations a state class is tried under.
#[derive(Clone, Copy, Debug)]
enum Config {
    /// CR0.EM and CR0.TS clear, CR4.OSFXSR set.
    Enabled,
    /// As `Enabled`, with CR0.TS set.
    TaskSwitched,
    /// As `Enabled`, with CR0.EM set.
    Emulated,
    /// As `Enabled`, with CR4.OSFXSR clear.
    NoOsfxsr,
}

const CONFIGS: [Config; 4] = [
    Config::Enabled,
    Config::TaskSwitched,
    Config::Emulated,
    Config::NoOsfxsr,
];

/// What each configuration owes an instruction of one state class.
struct Owed {
    enabled: Outcome,
    task_switched: Outcome,
    emulated: Outcome,
    no_osfxsr: Outcome,
}

impl Owed {
    fn under(&self, config: Config) -> Outcome {
        match config {
            Config::Enabled => self.enabled,
            Config::TaskSwitched => self.task_switched,
            Config::Emulated => self.emulated,
            Config::NoOsfxsr => self.no_osfxsr,
        }
    }
}

/// One `BX_PREPARE_*` class: the Bochs handler `assignHandler` gives it, what
/// that handler owes the guest, and instructions that carry the class.
struct StateClass {
    handler: &'static str,
    owed: Owed,
    cases: &'static [Case],
}

/// proc_ctrl.cc `BxNoFPU`: CR0.EM and CR0.TS both raise #NM.
const FPU: StateClass = StateClass {
    handler: "BxNoFPU",
    owed: Owed {
        enabled: Outcome::Retired,
        task_switched: Outcome::DeviceNotAvailable,
        emulated: Outcome::DeviceNotAvailable,
        no_osfxsr: Outcome::Retired,
    },
    cases: &[
        // FNOP          D9 D0
        Case {
            name: "fnop",
            code: &[0xD9, 0xD0],
        },
        // FNINIT        DB E3
        Case {
            name: "fninit",
            code: &[0xDB, 0xE3],
        },
        // FNSTSW AX     DF E0
        Case {
            name: "fnstsw_ax",
            code: &[0xDF, 0xE0],
        },
        // FLD1          D9 E8
        Case {
            name: "fld1",
            code: &[0xD9, 0xE8],
        },
    ],
};

/// proc_ctrl.cc `BxNoMMX`: CR0.EM raises #UD, CR0.TS #NM; CR4.OSFXSR is not
/// MMX state.
const MMX: StateClass = StateClass {
    handler: "BxNoMMX",
    owed: Owed {
        enabled: Outcome::Retired,
        task_switched: Outcome::DeviceNotAvailable,
        emulated: Outcome::InvalidOpcode,
        no_osfxsr: Outcome::Retired,
    },
    cases: &[
        // PADDB mm0, mm1      NP 0F FC /r
        Case {
            name: "paddb_mm",
            code: &[0x0F, 0xFC, 0xC1],
        },
        // EMMS                NP 0F 77
        Case {
            name: "emms",
            code: &[0x0F, 0x77],
        },
        // PMOVMSKB eax, mm0   NP 0F D7 /r — an SSE-era form, still MMX state
        Case {
            name: "pmovmskb_mm",
            code: &[0x0F, 0xD7, 0xC0],
        },
    ],
};

/// proc_ctrl.cc `BxNoSSE`: CR0.EM or a clear CR4.OSFXSR raises #UD, CR0.TS
/// #NM.
const SSE: StateClass = StateClass {
    handler: "BxNoSSE",
    owed: Owed {
        enabled: Outcome::Retired,
        task_switched: Outcome::DeviceNotAvailable,
        emulated: Outcome::InvalidOpcode,
        no_osfxsr: Outcome::InvalidOpcode,
    },
    cases: &[
        // AESENC xmm0, xmm1               66 0F 38 DC /r
        Case {
            name: "aesenc",
            code: &[0x66, 0x0F, 0x38, 0xDC, 0xC1],
        },
        // AESKEYGENASSIST xmm0, xmm1, 0   66 0F 3A DF /r ib
        Case {
            name: "aeskeygenassist",
            code: &[0x66, 0x0F, 0x3A, 0xDF, 0xC1, 0x00],
        },
        // PCLMULQDQ xmm0, xmm1, 0         66 0F 3A 44 /r ib
        Case {
            name: "pclmulqdq",
            code: &[0x66, 0x0F, 0x3A, 0x44, 0xC1, 0x00],
        },
        // GF2P8MULB xmm0, xmm1            66 0F 38 CF /r
        Case {
            name: "gf2p8mulb",
            code: &[0x66, 0x0F, 0x38, 0xCF, 0xC1],
        },
        // SHA256RNDS2 xmm1, xmm2, <xmm0>  NP 0F 38 CB /r
        Case {
            name: "sha256rnds2",
            code: &[0x0F, 0x38, 0xCB, 0xCA],
        },
        // SHA1RNDS4 xmm0, xmm1, 0         NP 0F 3A CC /r ib
        Case {
            name: "sha1rnds4",
            code: &[0x0F, 0x3A, 0xCC, 0xC1, 0x00],
        },
        // PADDB xmm0, xmm1                66 0F FC /r
        Case {
            name: "paddb_xmm",
            code: &[0x66, 0x0F, 0xFC, 0xC1],
        },
        // PEXTRD eax, xmm1, 1             66 0F 3A 16 /r ib — its handler also
        // serves VEX and EVEX VPEXTRD
        Case {
            name: "pextrd",
            code: &[0x66, 0x0F, 0x3A, 0x16, 0xC8, 0x01],
        },
    ],
};

/// Flat long mode on the default model, offered SHA and GFNI besides, so
/// every instruction above passes the ISA gate and only its CPU state
/// decides what it does.
fn feature_complete_emulator() -> Box<Emulator> {
    long64_emulator(EmulatorConfig {
        cpu_params: BxParams::default()
            .including(X86Feature::IsaSha)
            .including(X86Feature::IsaGfni),
        ..EmulatorConfig::default()
    })
}

/// [`feature_complete_emulator`] with its control registers in `config`.
fn emulator_in(config: Config) -> Box<Emulator> {
    let mut emu = feature_complete_emulator();
    let cr0 = emu.reg_read(X86Reg::Cr0);
    let cr4 = emu.reg_read(X86Reg::Cr4);
    match config {
        Config::Enabled => emu.reg_write(X86Reg::Cr4, cr4 | CR4_OSFXSR),
        Config::TaskSwitched => {
            emu.reg_write(X86Reg::Cr4, cr4 | CR4_OSFXSR);
            emu.reg_write(X86Reg::Cr0, cr0 | CR0_TS);
        }
        Config::Emulated => {
            emu.reg_write(X86Reg::Cr4, cr4 | CR4_OSFXSR);
            emu.reg_write(X86Reg::Cr0, cr0 | CR0_EM);
        }
        Config::NoOsfxsr => emu.reg_write(X86Reg::Cr4, cr4 & !CR4_OSFXSR),
    }
    emu
}

/// The guest's own XSETBV (ECX = 0, EDX:EAX = `xcr0`). CR4.OSXSAVE must
/// already be set.
fn guest_xsetbv(emu: &mut Emulator, xcr0: u64) {
    emu.reg_write(X86Reg::Rax, xcr0);
    emu.reg_write(X86Reg::Rcx, 0);
    emu.reg_write(X86Reg::Rdx, 0);
    emu.mem_write(CODE, &[0x0F, 0x01, 0xD1]).expect("xsetbv");
    emu.emu_start(CODE, Some(CODE + 3), None, Some(1))
        .expect("xsetbv runs");
}

/// Every instruction of every class, under every configuration, raises
/// exactly what its Bochs `BxNo*` handler raises — and retires when its
/// state is enabled. A mismatch is reported for all of them at once.
#[test]
fn each_state_class_raises_what_its_bochs_handler_raises() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut mismatches = Vec::new();
            for class in [&FPU, &MMX, &SSE] {
                for config in CONFIGS {
                    for case in class.cases {
                        let mut emu = emulator_in(config);
                        let saw = run_one(&mut emu, case.code);
                        let owed = class.owed.under(config);
                        if saw != owed {
                            mismatches.push(format!(
                                "{} under {config:?}: saw {saw:?}, Bochs {} owes {owed:?}",
                                case.name, class.handler
                            ));
                        }
                    }
                }
            }
            assert!(
                mismatches.is_empty(),
                "{} instruction/configuration pairs differ from Bochs:\n{}",
                mismatches.len(),
                mismatches.join("\n")
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// An instruction the lazy-switch test runs, the register state it starts
/// from, and the value it must leave behind.
struct LazyCase {
    name: &'static str,
    code: &'static [u8],
    /// Puts the instruction's inputs in place.
    arrange: fn(&mut Emulator),
    /// Reads back the register the instruction writes.
    result: fn(&Emulator) -> u128,
    expected: u128,
}

/// What a run of a [`LazyCase`] left behind.
#[derive(Debug, PartialEq, Eq)]
struct LazyRun {
    outcome: Outcome,
    /// How many times the #NM handler ran (it counts itself in R15).
    nm_taken: u64,
    /// CR0.TS after the run.
    cr0_ts: u64,
    /// The register the instruction writes.
    result: u128,
}

fn rax_all_ones(emu: &mut Emulator) {
    emu.reg_write(X86Reg::Rax, u64::MAX);
}

fn rax(emu: &Emulator) -> u128 {
    u128::from(emu.reg_read(X86Reg::Rax))
}

fn xmm0(emu: &Emulator) -> u128 {
    u128::from_le_bytes(emu.reg_read_xmm(X86Reg::Xmm0))
}

fn xmm0_xmm1_zero(emu: &mut Emulator) {
    emu.reg_write_xmm(X86Reg::Xmm0, [0; 16]);
    emu.reg_write_xmm(X86Reg::Xmm1, [0; 16]);
}

fn bytes_one_and_two(emu: &mut Emulator) {
    emu.reg_write_xmm(X86Reg::Xmm0, [0x01; 16]);
    emu.reg_write_xmm(X86Reg::Xmm1, [0x02; 16]);
}

/// Four dwords of 1 in XMM1 and of 2 in XMM2, for `VPADDD xmm0, xmm1, xmm2`.
fn dwords_one_and_two(emu: &mut Emulator) {
    emu.reg_write_xmm(X86Reg::Xmm0, [0; 16]);
    emu.reg_write_xmm(X86Reg::Xmm1, DWORDS_OF_1);
    emu.reg_write_xmm(X86Reg::Xmm2, DWORDS_OF_2);
}

const DWORDS_OF_1: [u8; 16] = [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0];
const DWORDS_OF_2: [u8; 16] = [2, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0];
const DWORDS_OF_3: [u8; 16] = [3, 0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0];

/// One of each class, each with a result that shows it executed.
const LAZY_CASES: [LazyCase; 5] = [
    // FNSTSW AX: the reset status word is 0.
    LazyCase {
        name: "fnstsw_ax",
        code: &[0xDF, 0xE0],
        arrange: rax_all_ones,
        result: rax,
        expected: 0xFFFF_FFFF_FFFF_0000,
    },
    // PMOVMSKB eax, mm0: MM0 is 0 at reset; a 32-bit write clears the upper
    // half of RAX.
    LazyCase {
        name: "pmovmskb_mm",
        code: &[0x0F, 0xD7, 0xC0],
        arrange: rax_all_ones,
        result: rax,
        expected: 0,
    },
    // AESENC xmm0, xmm1 on zero state and key: SubBytes maps every byte to
    // 0x63, which MixColumns leaves alone.
    LazyCase {
        name: "aesenc",
        code: &[0x66, 0x0F, 0x38, 0xDC, 0xC1],
        arrange: xmm0_xmm1_zero,
        result: xmm0,
        expected: u128::from_le_bytes([0x63; 16]),
    },
    // PADDB xmm0, xmm1
    LazyCase {
        name: "paddb_xmm",
        code: &[0x66, 0x0F, 0xFC, 0xC1],
        arrange: bytes_one_and_two,
        result: xmm0,
        expected: u128::from_le_bytes([0x03; 16]),
    },
    // VPADDD xmm0, xmm1, xmm2   VEX.128.66.0F.W0 FE /r
    LazyCase {
        name: "vpaddd",
        code: &[0xC5, 0xF1, 0xFE, 0xC2],
        arrange: dwords_one_and_two,
        result: xmm0,
        expected: u128::from_le_bytes(DWORDS_OF_3),
    },
];

/// The mode a lazy switch runs in. They differ in what returns from the #NM
/// handler: IRETQ reloads CS in long mode, and a long-mode CS load re-derives
/// the AVX part of the gate by itself (Bochs segment_ctrl_pro.cc
/// `load_seg_reg` → `handleCpuModeChange` → `handleAvxModeChange`); IRETD in
/// protected mode re-derives nothing, which leaves CLTS as the only thing
/// that can tell the gate CR0.TS is clear.
#[derive(Clone, Copy, Debug)]
enum LazyMode {
    Long64,
    Protected32,
}

/// `inc r15d; clts; iretq` — the 64-bit #NM handler, counting itself in R15.
const LAZY_SWITCH_HANDLER_64: [u8; 7] = [0x41, 0xFF, 0xC7, 0x0F, 0x06, 0x48, 0xCF];
/// `inc edi; clts; iretd` — the 32-bit #NM handler, counting itself in EDI.
const LAZY_SWITCH_HANDLER_32: [u8; 4] = [0x47, 0x0F, 0x06, 0xCF];

/// Run every [`LAZY_CASES`] instruction through a lazy switch in `mode`, and
/// describe each run that differs from what Bochs owes.
fn lazy_switch_mismatches(mode: LazyMode) -> Vec<String> {
    let counter = match mode {
        LazyMode::Long64 => X86Reg::R15,
        LazyMode::Protected32 => X86Reg::Rdi,
    };
    let mut mismatches = Vec::new();
    for case in &LAZY_CASES {
        let mut emu = match mode {
            LazyMode::Long64 => feature_complete_emulator(),
            LazyMode::Protected32 => protected32_emulator(),
        };
        emu.reg_write(
            X86Reg::Cr4,
            emu.reg_read(X86Reg::Cr4) | CR4_OSFXSR | CR4_OSXSAVE,
        );
        guest_xsetbv(&mut emu, XCR0_AVX);
        (case.arrange)(&mut emu);
        emu.reg_write(counter, 0);
        emu.reg_write(X86Reg::Cr0, emu.reg_read(X86Reg::Cr0) | CR0_TS);

        let outcome = match mode {
            LazyMode::Long64 => {
                install_fault_handlers(&mut emu);
                emu.mem_write(NM_GATE.handler, &LAZY_SWITCH_HANDLER_64)
                    .expect("write the lazy-switch handler");
                run_installed_at(&mut emu, CODE, case.code)
            }
            LazyMode::Protected32 => run_protected32_lazy(&mut emu, case.code),
        };
        let saw = LazyRun {
            outcome,
            nm_taken: emu.reg_read(counter),
            cr0_ts: emu.reg_read(X86Reg::Cr0) & CR0_TS,
            result: (case.result)(&emu),
        };
        // It faulted once, the handler's CLTS cleared CR0.TS, and the
        // instruction then ran to completion and computed its result.
        let owed = LazyRun {
            outcome: Outcome::Retired,
            nm_taken: 1,
            cr0_ts: 0,
            result: case.expected,
        };
        if saw != owed {
            mismatches.push(format!("{} ({mode:?}): saw {saw:x?}, owed {owed:x?}", case.name));
        }
    }
    mismatches
}

/// Run one encoding in flat protected mode with a lazy-switch #NM handler and
/// `HLT` handlers for #UD and #MF, at the long-mode harness's addresses.
fn run_protected32_lazy(emu: &mut Emulator, code: &[u8]) -> Outcome {
    const IDT: u64 = 0x0028_0000;
    const STACK: u64 = 0x0030_0000;
    /// A gate and the code its handler runs.
    struct Handler {
        gate: &'static Gate,
        body: &'static [u8],
    }
    for handler in [
        Handler {
            gate: &UD_GATE,
            body: &[0xF4],
        },
        Handler {
            gate: &NM_GATE,
            body: &LAZY_SWITCH_HANDLER_32,
        },
        Handler {
            gate: &MF_GATE,
            body: &[0xF4],
        },
    ] {
        let gate = handler.gate;
        emu.mem_write(IDT + gate.vector * 8, &gate32(gate.handler).to_le_bytes())
            .expect("write the gate");
        emu.mem_write(gate.handler, handler.body)
            .expect("write the handler");
    }
    emu.reg_write(X86Reg::IdtrBase, IDT);
    emu.reg_write(X86Reg::IdtrLimit, 32 * 8 - 1);

    let mut image = code.to_vec();
    image.extend_from_slice(&[0xEB, 0xFE]); // jmp $
    emu.mem_write(CODE, &image).expect("write code");
    emu.reg_write(X86Reg::Rsp, STACK);
    let stop = emu.emu_start(CODE, None, None, Some(8)).expect("emu_start");
    let eip = emu.cpu().rip();
    if eip == CODE + code.len() as u64 {
        assert_eq!(
            emu.reg_read(X86Reg::Rsp),
            STACK,
            "a handler that returned has popped its frame"
        );
        Outcome::Retired
    } else if eip == UD_GATE.handler + 1 {
        Outcome::InvalidOpcode
    } else if eip == MF_GATE.handler + 1 {
        Outcome::FloatingPointError
    } else {
        panic!("the instruction neither retired nor took #UD or #MF: eip={eip:#x}, stop={stop:?}");
    }
}

/// The first x87, MMX, SSE or AVX instruction after a context switch takes
/// #NM; the OS's handler clears CR0.TS with CLTS and returns; the instruction
/// runs again — and must now execute, and compute its result.
///
/// The handler counts itself, so a run in which the instruction never faulted
/// is told apart from one in which it faulted and was then dropped. After
/// CLTS the gate must see CR0.TS clear: Bochs crregs.cc `CLTS` calls
/// `handleFpuMmxModeChange`, `handleSseModeChange` and `handleAvxModeChange`.
#[test]
fn a_lazy_fpu_switch_faults_once_and_then_executes() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut mismatches = lazy_switch_mismatches(LazyMode::Long64);
            mismatches.extend(lazy_switch_mismatches(LazyMode::Protected32));
            assert!(
                mismatches.is_empty(),
                "{} instructions did not survive the lazy switch:\n{}",
                mismatches.len(),
                mismatches.join("\n")
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// Flat 32-bit protected mode on the default model, offered SHA and GFNI.
fn protected32_emulator() -> Box<Emulator> {
    Emulator::new_with_mode(
        EmulatorConfig {
            cpu_params: BxParams::default()
                .including(X86Feature::IsaSha)
                .including(X86Feature::IsaGfni),
            ..EmulatorConfig::default()
        },
        CpuSetupMode::FlatProtected32,
    )
    .expect("emulator")
}

/// A present, DPL 0, 32-bit interrupt gate to `handler` through CS 0x08.
fn gate32(handler: u64) -> u64 {
    (handler & 0xFFFF) | (0x08 << 16) | (0x8E << 40) | ((handler >> 16) << 48)
}

/// Flat 32-bit code (0x08) and data (0x10) descriptors, as entries 1 and 2
/// of a protected-mode test's GDT.
const FLAT_CODE32: u64 = 0x00CF_9A00_0000_FFFF;
const FLAT_DATA32: u64 = 0x00CF_9200_0000_FFFF;

/// A present, DPL 0, available 32-bit TSS descriptor (type 9) of byte limit
/// 0x67.
fn tss_descriptor(base: u64) -> u64 {
    0x67 | ((base & 0x00FF_FFFF) << 16) | (0x89 << 40) | ((base >> 24) << 56)
}

/// Write `descriptors` as the GDT at `base` and load GDTR with it.
fn install_gdt(emu: &mut Emulator, base: u64, descriptors: &[u64]) {
    for (index, descriptor) in descriptors.iter().enumerate() {
        emu.mem_write(base + index as u64 * 8, &descriptor.to_le_bytes())
            .expect("write the GDT");
    }
    emu.reg_write(X86Reg::GdtrBase, base);
    emu.reg_write(X86Reg::GdtrLimit, descriptors.len() as u64 * 8 - 1);
}

/// Where the protected-mode tests put their IDT, and the `HLT` each vector's
/// gate leads to: 16 bytes apart, so where the processor stops names the
/// vector it took.
const PM_IDT: u64 = 0x0022_0000;
const PM_HANDLERS: u64 = 0x0023_0000;
const PM_VECTORS: u64 = 32;

/// A 32-bit IDT whose every vector below [`PM_VECTORS`] halts in its own
/// handler.
fn install_protected32_halt_gates(emu: &mut Emulator) {
    for vector in 0..PM_VECTORS {
        let handler = PM_HANDLERS + vector * 16;
        emu.mem_write(PM_IDT + vector * 8, &gate32(handler).to_le_bytes())
            .expect("write the IDT");
        emu.mem_write(handler, &[0xF4]).expect("write the handler");
    }
    emu.reg_write(X86Reg::IdtrBase, PM_IDT);
    emu.reg_write(X86Reg::IdtrLimit, PM_VECTORS * 8 - 1);
}

/// A hardware task switch sets CR0.TS (Bochs tasking.cc `task_switch`, step 9)
/// and recomputes the state gate, so the new task's first x87 instruction
/// raises #NM — the trap a 32-bit OS that switches FPU state lazily on task
/// gates depends on. After the new task's own CLTS the same instruction runs.
#[test]
fn a_task_switch_sets_cr0_ts_and_the_next_x87_instruction_faults() {
    /// Flat 32-bit code and data descriptors, then two available 32-bit TSS
    /// descriptors: the task that switches away, and the task it switches to.
    const GDT: u64 = 0x0021_0000;
    const OLD_TSS: u64 = 0x0021_1000;
    const NEW_TSS: u64 = 0x0021_2000;
    const OLD_TSS_SELECTOR: u16 = 0x18;
    const NEW_TSS_SELECTOR: u16 = 0x20;
    const NEW_TASK_CODE: u64 = 0x0020_1000;
    const NEW_STACK: u64 = 0x0030_0000;
    const NM_VECTOR: u64 = 7;

    struct NewTask {
        name: &'static str,
        code: &'static [u8],
        /// Where the run must end: the handler of `vector` (`Some`), or the
        /// task's closing `jmp $` (`None`).
        vector: Option<u64>,
    }

    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let tasks = [
                // FLD1; jmp $
                NewTask {
                    name: "fld1",
                    code: &[0xD9, 0xE8, 0xEB, 0xFE],
                    vector: Some(NM_VECTOR),
                },
                // CLTS; FNINIT; FLD1; FNSTSW AX; jmp $
                NewTask {
                    name: "clts_then_fld1",
                    code: &[0x0F, 0x06, 0xDB, 0xE3, 0xD9, 0xE8, 0xDF, 0xE0, 0xEB, 0xFE],
                    vector: None,
                },
            ];

            for task in &tasks {
                let mut emu = protected32_emulator();
                emu.reg_write(X86Reg::Rflags, 0x2);

                install_gdt(
                    &mut emu,
                    GDT,
                    &[
                        0,
                        FLAT_CODE32,
                        FLAT_DATA32,
                        tss_descriptor(OLD_TSS),
                        tss_descriptor(NEW_TSS),
                    ],
                );
                install_protected32_halt_gates(&mut emu);

                emu.mem_write(OLD_TSS, &[0; 0x68]).expect("old TSS");
                let mut tss = [0u8; 0x68];
                let mut put = |offset: usize, value: u32| {
                    tss[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
                };
                put(0x20, NEW_TASK_CODE as u32); // EIP
                put(0x24, 0x2); // EFLAGS
                put(0x28, 0); // EAX: a TOP of 0 until FNSTSW AX reports another
                put(0x38, NEW_STACK as u32); // ESP
                put(0x48, 0x10); // ES
                put(0x4C, 0x08); // CS
                put(0x50, 0x10); // SS
                put(0x54, 0x10); // DS
                put(0x58, 0x10); // FS
                put(0x5C, 0x10); // GS
                put(0x64, 0x68 << 16); // I/O map base past the limit
                emu.mem_write(NEW_TSS, &tss).expect("new TSS");
                emu.mem_write(NEW_TASK_CODE, task.code).expect("new task code");

                // mov ax, OLD_TSS_SELECTOR; ltr ax; jmp far NEW_TSS_SELECTOR:0
                let mut old_task = vec![0x66, 0xB8];
                old_task.extend_from_slice(&OLD_TSS_SELECTOR.to_le_bytes());
                old_task.extend_from_slice(&[0x0F, 0x00, 0xD8, 0xEA, 0, 0, 0, 0]);
                old_task.extend_from_slice(&NEW_TSS_SELECTOR.to_le_bytes());
                emu.mem_write(CODE, &old_task).expect("old task code");

                let stop = emu.emu_start(CODE, None, None, Some(16)).expect("emu_start");
                let eip = emu.cpu().rip();
                let name = task.name;
                assert_eq!(
                    emu.reg_read(X86Reg::TrSelector),
                    u64::from(NEW_TSS_SELECTOR),
                    "{name}: the far JMP must have switched to the new task (stop={stop:?})"
                );
                match task.vector {
                    Some(vector) => {
                        assert_eq!(
                            emu.reg_read(X86Reg::Cr0) & CR0_TS,
                            CR0_TS,
                            "{name}: the task switch sets CR0.TS"
                        );
                        assert_eq!(
                            eip,
                            PM_HANDLERS + vector * 16 + 1,
                            "{name}: the new task's first x87 instruction must raise #NM \
                             (eip={eip:#x}, stop={stop:?})"
                        );
                        let mut pushed_eip = [0u8; 4];
                        emu.mem_read(NEW_STACK - 12, &mut pushed_eip)
                            .expect("read the frame");
                        assert_eq!(
                            u64::from(u32::from_le_bytes(pushed_eip)),
                            NEW_TASK_CODE,
                            "{name}: the frame's EIP must be the faulting instruction"
                        );
                    }
                    None => {
                        assert_eq!(
                            emu.reg_read(X86Reg::Cr0) & CR0_TS,
                            0,
                            "{name}: the new task's CLTS cleared CR0.TS"
                        );
                        assert_eq!(
                            eip,
                            NEW_TASK_CODE + task.code.len() as u64 - 2,
                            "{name}: after CLTS the x87 instructions retire \
                             (eip={eip:#x}, stop={stop:?})"
                        );
                        // Retiring is not executing: a gate left on the old
                        // CR0.TS would retire them without running them. FNINIT
                        // empties the stack, FLD1 pushes 1.0, so TOP moves from
                        // 0 to 7 and ST0 holds 1.0 — and FNSTSW AX reports it.
                        let status_word = emu.reg_read(X86Reg::Rax) & 0xFFFF;
                        assert_eq!(
                            (status_word >> 11) & 7,
                            7,
                            "{name}: FLD1 must have pushed (FNSTSW AX = {status_word:#06x})"
                        );
                        assert_eq!(
                            emu.reg_read_fp80(X86Reg::Fpr0),
                            [0, 0, 0, 0, 0, 0, 0, 0x80, 0xFF, 0x3F],
                            "{name}: ST0 must hold the 1.0 FLD1 loaded"
                        );
                    }
                }
            }
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// A host that imports a processor's control registers or XCR0 gets the
/// processor that state describes: the gate is rebuilt from what was
/// imported, not left reading the values the import replaced.
#[test]
fn an_imported_state_rebuilds_the_state_gate() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut mismatches = Vec::new();

            // CR0.TS arrives with the control registers.
            for case in [
                // FNOP              D9 D0
                Case {
                    name: "fnop",
                    code: &[0xD9, 0xD0],
                },
                // PADDB xmm0, xmm1  66 0F FC /r
                Case {
                    name: "paddb_xmm",
                    code: &[0x66, 0x0F, 0xFC, 0xC1],
                },
            ] {
                let mut emu = emulator_in(Config::Enabled);
                let mut state = VcpuArchState::default();
                emu.cpu().export_arch_state(&mut state);
                state.cr0 |= CR0_TS;
                emu.processor(0)
                    .cpu_mut()
                    .import_arch_groups(&state, ArchGroups::CONTROL_REGS)
                    .expect("import the control registers");
                let saw = run_one(&mut emu, case.code);
                if saw != Outcome::DeviceNotAvailable {
                    mismatches.push(format!(
                        "{} after an imported CR0.TS: saw {saw:?}, owed DeviceNotAvailable",
                        case.name
                    ));
                }
            }

            // XCR0 arrives with the model-specific registers. A gate left on
            // the old XCR0 would not fault here — the AVX state check finds
            // the state enabled — but it would drop the instruction, so the
            // sum is what tells the two apart.
            let mut emu = emulator_in(Config::Enabled);
            emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | CR4_OSXSAVE);
            dwords_one_and_two(&mut emu);
            let mut state = VcpuArchState::default();
            emu.cpu().export_arch_state(&mut state);
            state.xcr0 = XCR0_AVX as u32;
            emu.processor(0)
                .cpu_mut()
                .import_arch_groups(&state, ArchGroups::MSRS)
                .expect("import the MSRs");
            // VPADDD xmm0, xmm1, xmm2
            let saw = run_one(&mut emu, &[0xC5, 0xF1, 0xFE, 0xC2]);
            let sum = emu.reg_read_xmm(X86Reg::Xmm0);
            if saw != Outcome::Retired || sum != DWORDS_OF_3 {
                mismatches.push(format!(
                    "vpaddd after an imported XCR0 = {XCR0_AVX:#x}: saw {saw:?} with \
                     xmm0 = {sum:02x?}, owed Retired with xmm0 = {DWORDS_OF_3:02x?}"
                ));
            }

            assert!(
                mismatches.is_empty(),
                "{} instructions ran against the state an import replaced:\n{}",
                mismatches.len(),
                mismatches.join("\n")
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// Bochs fpu_emu.cc `FWAIT`: #NM when CR0.TS and CR0.MP are both set, then
/// `FPU_check_pending_exceptions` — #MF for an unmasked exception the x87
/// unit is holding, when CR0.NE selects native error reporting. FWAIT carries
/// no `BX_PREPARE_*`, so this is its own check rather than the gate's.
#[test]
fn fwait_checks_cr0_ts_with_cr0_mp_and_pending_x87_exceptions() {
    struct FwaitCase {
        name: &'static str,
        cr0_set: u64,
        /// The x87 status word to start from: 0x0081 is ES (bit 7) with IE.
        status_word: u64,
        owed: Outcome,
    }

    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cases = [
                FwaitCase {
                    name: "ts_and_mp",
                    cr0_set: CR0_TS | CR0_MP,
                    status_word: 0,
                    owed: Outcome::DeviceNotAvailable,
                },
                FwaitCase {
                    name: "ts_without_mp",
                    cr0_set: CR0_TS,
                    status_word: 0,
                    owed: Outcome::Retired,
                },
                FwaitCase {
                    name: "pending_exception_native",
                    cr0_set: CR0_NE,
                    status_word: 0x0081,
                    owed: Outcome::FloatingPointError,
                },
                FwaitCase {
                    name: "ts_and_mp_before_the_pending_exception",
                    cr0_set: CR0_TS | CR0_MP | CR0_NE,
                    status_word: 0x0081,
                    owed: Outcome::DeviceNotAvailable,
                },
                FwaitCase {
                    name: "nothing_pending",
                    cr0_set: CR0_NE,
                    status_word: 0,
                    owed: Outcome::Retired,
                },
            ];
            let mut mismatches = Vec::new();
            for case in &cases {
                let mut emu = feature_complete_emulator();
                emu.reg_write(X86Reg::Cr0, emu.reg_read(X86Reg::Cr0) | case.cr0_set);
                emu.reg_write(X86Reg::FpSw, case.status_word);
                // FWAIT  9B
                let saw = run_one(&mut emu, &[0x9B]);
                if saw != case.owed {
                    mismatches.push(format!(
                        "{}: saw {saw:?}, Bochs FWAIT owes {:?}",
                        case.name, case.owed
                    ));
                }
            }
            assert!(
                mismatches.is_empty(),
                "{} FWAIT cases differ from Bochs:\n{}",
                mismatches.len(),
                mismatches.join("\n")
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// Virtual-8086 mode is not protected mode as Bochs's `protected_mode()`
/// counts it, though CR0.PE stays set. So `handleAvxModeChange` turns AVX and
/// AVX-512 state off there, and `BxNoAVX` / `BxNoEVEX` (proc_ctrl.cc) raise
/// #UD — even under an OS that enabled the state for its protected-mode code.
/// A check of CR0.PE alone would let the instruction through.
#[test]
fn vex_and_evex_raise_ud_in_virtual_8086_mode() {
    const GDT: u64 = 0x0021_0000;
    const TSS: u64 = 0x0021_1000;
    const TSS_SELECTOR: u16 = 0x18;
    /// The stack the processor switches to when the virtual-8086 task faults.
    const RING0_STACK: u64 = 0x0030_0000;
    /// The virtual-8086 task's code segment: linear 0x2_0000.
    const V86_CS: u16 = 0x2000;
    const V86_CODE: u64 = 0x0002_0000;
    const UD_VECTOR: u64 = 6;

    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cases = [
                // VPADDD xmm0, xmm1, xmm2   VEX.128.66.0F.W0 FE /r
                Case {
                    name: "vex_vpaddd",
                    code: &[0xC5, 0xF1, 0xFE, 0xC2],
                },
                // VPADDD xmm0, xmm1, xmm2   EVEX.128.66.0F.W0 FE /r
                Case {
                    name: "evex_vpaddd",
                    code: &[0x62, 0xF1, 0x75, 0x08, 0xFE, 0xC2],
                },
            ];

            let mut mismatches = Vec::new();
            for case in &cases {
                let mut emu = protected32_emulator();
                emu.reg_write(X86Reg::Rflags, 0x2);
                emu.reg_write(
                    X86Reg::Cr4,
                    emu.reg_read(X86Reg::Cr4) | CR4_OSFXSR | CR4_OSXSAVE,
                );
                guest_xsetbv(&mut emu, XCR0_AVX512);

                install_gdt(&mut emu, GDT, &[0, FLAT_CODE32, FLAT_DATA32, tss_descriptor(TSS)]);
                let mut tss = [0u8; 0x68];
                tss[0x04..0x08].copy_from_slice(&(RING0_STACK as u32).to_le_bytes()); // ESP0
                tss[0x08..0x0C].copy_from_slice(&0x10u32.to_le_bytes()); // SS0
                tss[0x64..0x68].copy_from_slice(&(0x68u32 << 16).to_le_bytes()); // no I/O map
                emu.mem_write(TSS, &tss).expect("TSS");
                install_protected32_halt_gates(&mut emu);

                let mut v86 = case.code.to_vec();
                v86.extend_from_slice(&[0xEB, 0xFE]); // jmp $
                emu.mem_write(V86_CODE, &v86).expect("virtual-8086 code");

                // mov ax, TSS_SELECTOR; ltr ax; then an IRETD frame for a
                // virtual-8086 task — GS, FS, DS, ES, SS, ESP, EFLAGS (VM set),
                // CS, EIP, pushed in that order — and IRETD.
                let mut ring0 = vec![0x66, 0xB8];
                ring0.extend_from_slice(&TSS_SELECTOR.to_le_bytes());
                ring0.extend_from_slice(&[0x0F, 0x00, 0xD8]);
                ring0.extend_from_slice(&[0x6A, 0x00, 0x6A, 0x00, 0x6A, 0x00, 0x6A, 0x00]);
                ring0.push(0x68);
                ring0.extend_from_slice(&u32::from(V86_CS).to_le_bytes()); // SS
                ring0.push(0x68);
                ring0.extend_from_slice(&0xFFF0u32.to_le_bytes()); // ESP
                ring0.push(0x68);
                ring0.extend_from_slice(&0x0002_0002u32.to_le_bytes()); // EFLAGS.VM
                ring0.push(0x68);
                ring0.extend_from_slice(&u32::from(V86_CS).to_le_bytes()); // CS
                ring0.extend_from_slice(&[0x6A, 0x00]); // EIP
                ring0.push(0xCF); // IRETD
                emu.mem_write(CODE, &ring0).expect("ring-0 code");
                // The IRETD frame is built on the ring-0 stack itself.
                emu.reg_write(X86Reg::Rsp, RING0_STACK);

                let stop = emu.emu_start(CODE, None, None, Some(32)).expect("emu_start");
                let eip = emu.cpu().rip();
                if eip != PM_HANDLERS + UD_VECTOR * 16 + 1 {
                    let mut frame = [0u8; 36];
                    emu.mem_read(RING0_STACK - 36, &mut frame).expect("read the frame");
                    mismatches.push(format!(
                        "{}: ended at eip={eip:#x} (stop={stop:?}, dr6={:#x}, top of the \
                         ring-0 stack {frame:02x?}), Bochs owes #UD",
                        case.name,
                        emu.reg_read(X86Reg::Dr6)
                    ));
                    continue;
                }
                // A fault from virtual-8086 mode switches to the ring-0 stack
                // and pushes GS, FS, DS, ES, SS, ESP, EFLAGS, CS and EIP.
                let mut frame = [0u8; 12];
                emu.mem_read(RING0_STACK - 36, &mut frame).expect("read the frame");
                let pushed_eip = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]);
                let pushed_cs = u32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
                let pushed_eflags = u32::from_le_bytes([frame[8], frame[9], frame[10], frame[11]]);
                if pushed_eip != 0 || pushed_cs != u32::from(V86_CS) || pushed_eflags & (1 << 17) == 0 {
                    mismatches.push(format!(
                        "{}: #UD frame names {pushed_cs:#x}:{pushed_eip:#x} with EFLAGS \
                         {pushed_eflags:#x}, not the virtual-8086 instruction at \
                         {V86_CS:#x}:0",
                        case.name
                    ));
                }
            }
            assert!(
                mismatches.is_empty(),
                "{} encodings differ from Bochs in virtual-8086 mode:\n{}",
                mismatches.len(),
                mismatches.join("\n")
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// What a tracer saw: the opcode of each instruction a trace fill decoded
/// (`opcode`), of each one about to execute (`before_execution`), and each
/// exception vector.
#[derive(Default)]
struct OpcodeWitness {
    filled: Vec<Opcode>,
    executed: Vec<Opcode>,
    exceptions: Vec<u8>,
}

impl Instrumentation for OpcodeWitness {
    fn active_hooks(&self) -> HookMask {
        HookMask::EXEC | HookMask::EXCEPTION
    }

    fn opcode(&mut self, ev: &OpcodeEvent) {
        self.filled.push(ev.instr.get_ia_opcode());
    }

    fn before_execution(&mut self, _rip: u64, instr: &Instruction) {
        self.executed.push(instr.get_ia_opcode());
    }

    fn exception(&mut self, vector: u8, _error_code: u32) {
        self.exceptions.push(vector);
    }
}

/// Bochs `assignHandler` replaces only an instruction's handler, so its
/// instrumentation still sees the instruction the guest wrote when that
/// instruction raises #UD or #NM instead of running: `BX_INSTR_OPCODE` and
/// `BX_INSTR_BEFORE_EXECUTION` name AESENC, never a stand-in for the fault.
/// The same holds for the ISA gate and for a page-straddling instruction.
#[test]
fn a_tracer_sees_the_instruction_that_faults_not_its_stand_in() {
    struct WitnessCase {
        name: &'static str,
        at: u64,
        code: &'static [u8],
        /// What the guest wrote, which the tracer must be shown.
        opcode: Opcode,
        cpu_params: fn() -> BxParams,
        cr0_set: u64,
        owed: Outcome,
        vector: u8,
    }

    fn with_gfni() -> BxParams {
        BxParams::default().including(X86Feature::IsaGfni)
    }

    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cases = [
                // AESENC xmm0, xmm1 under CR0.TS: the state gate.
                WitnessCase {
                    name: "aesenc",
                    at: CODE,
                    code: &[0x66, 0x0F, 0x38, 0xDC, 0xC1],
                    opcode: Opcode::AesencVdqWdq,
                    cpu_params: with_gfni,
                    cr0_set: CR0_TS,
                    owed: Outcome::DeviceNotAvailable,
                    vector: 7,
                },
                // PADDB xmm0, xmm1 across a page end under CR0.TS.
                WitnessCase {
                    name: "paddb_xmm_straddling",
                    at: 0x0020_0FFE,
                    code: &[0x66, 0x0F, 0xFC, 0xC1],
                    opcode: Opcode::PaddbVdqWdq,
                    cpu_params: with_gfni,
                    cr0_set: CR0_TS,
                    owed: Outcome::DeviceNotAvailable,
                    vector: 7,
                },
                // GF2P8MULB xmm0, xmm1 on a model without GFNI: the ISA gate.
                WitnessCase {
                    name: "gf2p8mulb_without_gfni",
                    at: CODE,
                    code: &[0x66, 0x0F, 0x38, 0xCF, 0xC1],
                    opcode: Opcode::Gf2p8mulbVdqWdq,
                    cpu_params: BxParams::default,
                    cr0_set: 0,
                    owed: Outcome::InvalidOpcode,
                    vector: 6,
                },
            ];

            let mut mismatches = Vec::new();
            for case in &cases {
                let mut emu = Emulator::<OpcodeWitness>::new_with_mode_and_instrumentation(
                    EmulatorConfig {
                        cpu_params: (case.cpu_params)(),
                        ..EmulatorConfig::default()
                    },
                    CpuSetupMode::FlatLong64,
                    OpcodeWitness::default(),
                )
                .expect("emulator");
                emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | CR4_OSFXSR);
                emu.reg_write(X86Reg::Cr0, emu.reg_read(X86Reg::Cr0) | case.cr0_set);

                let saw = run_one_at(&mut emu, case.at, case.code);
                let witness = emu.instrumentation();
                let name = case.name;
                if saw != case.owed {
                    mismatches.push(format!("{name}: saw {saw:?}, owed {:?}", case.owed));
                }
                if witness.filled.first() != Some(&case.opcode) {
                    mismatches.push(format!(
                        "{name}: the fill reported {:?} first, not {:?}",
                        witness.filled, case.opcode
                    ));
                }
                if witness.executed.first() != Some(&case.opcode) {
                    mismatches.push(format!(
                        "{name}: before_execution reported {:?} first, not {:?}",
                        witness.executed, case.opcode
                    ));
                }
                if witness.exceptions != [case.vector] {
                    mismatches.push(format!(
                        "{name}: exceptions {:?}, owed [{}]",
                        witness.exceptions, case.vector
                    ));
                }
            }
            assert!(
                mismatches.is_empty(),
                "{} differences from what Bochs's instrumentation sees:\n{}",
                mismatches.len(),
                mismatches.join("\n")
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// The machine a page-straddle case runs on.
#[derive(Clone, Copy, Debug)]
enum StraddleMachine {
    /// x87 and SSE state on: CR4.OSFXSR set, CR0.EM and CR0.TS clear.
    SseOn,
    /// As `SseOn`, with CR0.TS set.
    TaskSwitched,
    /// As `SseOn`, with CR4.OSFXSR clear.
    NoOsfxsr,
    /// CR4.OSXSAVE set, XCR0 = 1: AVX state off.
    AvxOff,
    /// CR4.OSXSAVE set, XCR0 = x87 | SSE | AVX.
    AvxOn,
    /// The default model, which does not offer GFNI, with SSE state on.
    WithoutGfni,
    /// The model offered GFNI, with SSE state on.
    WithGfni,
}

impl StraddleMachine {
    fn build(self) -> Box<Emulator> {
        let mut emu = match self {
            StraddleMachine::WithoutGfni => long64_emulator(EmulatorConfig::default()),
            _ => feature_complete_emulator(),
        };
        let cr4 = emu.reg_read(X86Reg::Cr4) | CR4_OSFXSR;
        emu.reg_write(X86Reg::Cr4, cr4);
        match self {
            StraddleMachine::SseOn | StraddleMachine::WithoutGfni | StraddleMachine::WithGfni => {}
            StraddleMachine::TaskSwitched => {
                emu.reg_write(X86Reg::Cr0, emu.reg_read(X86Reg::Cr0) | CR0_TS);
            }
            StraddleMachine::NoOsfxsr => emu.reg_write(X86Reg::Cr4, cr4 & !CR4_OSFXSR),
            StraddleMachine::AvxOff => emu.reg_write(X86Reg::Cr4, cr4 | CR4_OSXSAVE),
            StraddleMachine::AvxOn => {
                emu.reg_write(X86Reg::Cr4, cr4 | CR4_OSXSAVE);
                guest_xsetbv(&mut emu, XCR0_AVX);
            }
        }
        emu
    }
}

/// An instruction that starts in the last bytes of a 4 KiB page and ends in
/// the next, the machine it runs on, and what Bochs owes it there.
struct StraddleCase {
    name: &'static str,
    /// Where the instruction starts; its first bytes end the page at 0x20_1000.
    at: u64,
    code: &'static [u8],
    machine: StraddleMachine,
    owed: Outcome,
}

/// An instruction split across a page boundary is decoded by a second fill
/// path (Bochs icache.cc `boundaryFetch`), and Bochs gates it there too:
/// `boundaryFetch` calls `assignHandler` like `serveICacheMiss` does. So a
/// split instruction faults exactly as an unsplit one — the ISA gate and the
/// CPU-state gate alike — and the frame names its first byte.
#[test]
fn page_straddling_instructions_are_gated_like_any_other() {
    const PAGE_END: u64 = 0x0020_1000;

    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            // PADDB xmm0, xmm1   66 0F | FC C1
            const PADDB_XMM: &[u8] = &[0x66, 0x0F, 0xFC, 0xC1];
            // FLD1               D9 | E8
            const FLD1: &[u8] = &[0xD9, 0xE8];
            // VPADDD xmm0, xmm1, xmm2   C5 F1 | FE C2
            const VPADDD: &[u8] = &[0xC5, 0xF1, 0xFE, 0xC2];
            // GF2P8MULB xmm0, xmm1      66 0F 38 | CF C1
            const GF2P8MULB: &[u8] = &[0x66, 0x0F, 0x38, 0xCF, 0xC1];

            let cases = [
                StraddleCase {
                    name: "paddb_xmm",
                    at: PAGE_END - 2,
                    code: PADDB_XMM,
                    machine: StraddleMachine::SseOn,
                    owed: Outcome::Retired,
                },
                StraddleCase {
                    name: "paddb_xmm",
                    at: PAGE_END - 2,
                    code: PADDB_XMM,
                    machine: StraddleMachine::TaskSwitched,
                    owed: Outcome::DeviceNotAvailable,
                },
                StraddleCase {
                    name: "paddb_xmm",
                    at: PAGE_END - 2,
                    code: PADDB_XMM,
                    machine: StraddleMachine::NoOsfxsr,
                    owed: Outcome::InvalidOpcode,
                },
                StraddleCase {
                    name: "fld1",
                    at: PAGE_END - 1,
                    code: FLD1,
                    machine: StraddleMachine::SseOn,
                    owed: Outcome::Retired,
                },
                StraddleCase {
                    name: "fld1",
                    at: PAGE_END - 1,
                    code: FLD1,
                    machine: StraddleMachine::TaskSwitched,
                    owed: Outcome::DeviceNotAvailable,
                },
                StraddleCase {
                    name: "vpaddd",
                    at: PAGE_END - 2,
                    code: VPADDD,
                    machine: StraddleMachine::AvxOn,
                    owed: Outcome::Retired,
                },
                StraddleCase {
                    name: "vpaddd",
                    at: PAGE_END - 2,
                    code: VPADDD,
                    machine: StraddleMachine::AvxOff,
                    owed: Outcome::InvalidOpcode,
                },
                StraddleCase {
                    name: "gf2p8mulb",
                    at: PAGE_END - 3,
                    code: GF2P8MULB,
                    machine: StraddleMachine::WithGfni,
                    owed: Outcome::Retired,
                },
                StraddleCase {
                    name: "gf2p8mulb",
                    at: PAGE_END - 3,
                    code: GF2P8MULB,
                    machine: StraddleMachine::WithoutGfni,
                    owed: Outcome::InvalidOpcode,
                },
            ];

            let mut mismatches = Vec::new();
            for case in &cases {
                let mut emu = case.machine.build();
                let saw = run_one_at(&mut emu, case.at, case.code);
                if saw != case.owed {
                    mismatches.push(format!(
                        "{} at {:#x} on {:?}: saw {saw:?}, Bochs owes {:?}",
                        case.name, case.at, case.machine, case.owed
                    ));
                }
            }
            assert!(
                mismatches.is_empty(),
                "{} page-straddling cases differ from Bochs:\n{}",
                mismatches.len(),
                mismatches.join("\n")
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// A handler that legacy SSE shares with VEX and EVEX encodings checks no
/// state of its own, so each encoding gets only its own class's check. With
/// AVX-512 state enabled and CR4.OSFXSR clear, Bochs runs the VEX and EVEX
/// forms (`BxNoAVX` and `BxNoEVEX` test CR4.OSXSAVE and XCR0, never
/// CR4.OSFXSR) and raises #UD for the legacy one (`BxNoSSE`).
#[test]
fn shared_handlers_gate_each_encoding_on_its_own_state() {
    /// What a run left behind. RAX starts at 0; only the extracts write it,
    /// with dword 1 of XMM1.
    #[derive(Debug, PartialEq, Eq)]
    struct SharedRun {
        outcome: Outcome,
        rax: u64,
    }

    struct SharedCase {
        name: &'static str,
        code: &'static [u8],
        owed: SharedRun,
    }

    const EXECUTED: SharedRun = SharedRun {
        outcome: Outcome::Retired,
        rax: 0,
    };
    const EXTRACTED: SharedRun = SharedRun {
        outcome: Outcome::Retired,
        rax: 0x2222_2222,
    };
    const REFUSED: SharedRun = SharedRun {
        outcome: Outcome::InvalidOpcode,
        rax: 0,
    };

    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cases = [
                // VPEXTRD eax, xmm1, 1        VEX.128.66.0F3A.W0 16 /r ib
                SharedCase {
                    name: "vex_vpextrd",
                    code: &[0xC4, 0xE3, 0x79, 0x16, 0xC8, 0x01],
                    owed: EXTRACTED,
                },
                // VPEXTRD eax, xmm1, 1        EVEX.128.66.0F3A.W0 16 /r ib
                SharedCase {
                    name: "evex_vpextrd",
                    code: &[0x62, 0xF3, 0x7D, 0x08, 0x16, 0xC8, 0x01],
                    owed: EXTRACTED,
                },
                // PEXTRD eax, xmm1, 1         66 0F 3A 16 /r ib
                SharedCase {
                    name: "legacy_pextrd",
                    code: &[0x66, 0x0F, 0x3A, 0x16, 0xC8, 0x01],
                    owed: REFUSED,
                },
                // VADDPS xmm0, xmm1, xmm2     VEX.128.0F.WIG 58 /r
                SharedCase {
                    name: "vex_vaddps",
                    code: &[0xC5, 0xF0, 0x58, 0xC2],
                    owed: EXECUTED,
                },
                // VANDPS xmm0, xmm1, xmm2     VEX.128.0F.WIG 54 /r
                SharedCase {
                    name: "vex_vandps",
                    code: &[0xC5, 0xF0, 0x54, 0xC2],
                    owed: EXECUTED,
                },
                // VPMAXSB xmm0, xmm1, xmm2    VEX.128.66.0F38.WIG 3C /r
                SharedCase {
                    name: "vex_vpmaxsb",
                    code: &[0xC4, 0xE2, 0x71, 0x3C, 0xC2],
                    owed: EXECUTED,
                },
                // VPCMPISTRI xmm1, xmm2, 0    VEX.128.66.0F3A.WIG 63 /r ib
                SharedCase {
                    name: "vex_vpcmpistri",
                    code: &[0xC4, 0xE3, 0x79, 0x63, 0xCA, 0x00],
                    owed: EXECUTED,
                },
                // VMOVD xmm0, eax             EVEX.128.66.0F.W0 6E /r
                SharedCase {
                    name: "evex_vmovd",
                    code: &[0x62, 0xF1, 0x7D, 0x08, 0x6E, 0xC0],
                    owed: EXECUTED,
                },
                // VCOMISS xmm0, xmm1          EVEX.LLIG.0F.W0 2F /r
                SharedCase {
                    name: "evex_vcomiss",
                    code: &[0x62, 0xF1, 0x7C, 0x08, 0x2F, 0xC1],
                    owed: EXECUTED,
                },
            ];
            let mut mismatches = Vec::new();
            for case in &cases {
                let mut emu = feature_complete_emulator();
                emu.reg_write(
                    X86Reg::Cr4,
                    (emu.reg_read(X86Reg::Cr4) & !CR4_OSFXSR) | CR4_OSXSAVE,
                );
                guest_xsetbv(&mut emu, XCR0_AVX512);
                emu.reg_write_xmm(
                    X86Reg::Xmm1,
                    0x4444_4444_3333_3333_2222_2222_1111_1111u128.to_le_bytes(),
                );
                emu.reg_write(X86Reg::Rax, 0);
                let saw = SharedRun {
                    outcome: run_one(&mut emu, case.code),
                    rax: emu.reg_read(X86Reg::Rax),
                };
                if saw != case.owed {
                    mismatches.push(format!(
                        "{}: saw {saw:x?}, owed {:x?}",
                        case.name, case.owed
                    ));
                }
            }
            assert!(
                mismatches.is_empty(),
                "{} encodings differ from Bochs with AVX-512 state on and CR4.OSFXSR clear:\n{}",
                mismatches.len(),
                mismatches.join("\n")
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}
