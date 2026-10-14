//! VEX-encoded AVX instructions check *AVX* state, not SSE state.
//!
//! Bochs marks its AVX instructions `BX_PREPARE_AVX` in `ia_opcodes.def`. Not
//! every VEX encoding is one: the VEX-encoded BMI forms (ANDN, RORX, ...) carry
//! no `BX_PREPARE_*`, and the opmask instructions are `BX_PREPARE_OPMASK`.
//! While `BX_FETCH_MODE_AVX_OK` is clear, `assignHandler`
//! (cpu/decoder/fetchdecode32.cc) gives a `BX_PREPARE_AVX` instruction the
//! `BxNoAVX` handler (cpu/proc_ctrl.cc), which raises #UD unless the CPU is in
//! protected mode with CR4.OSXSAVE set and XCR0.SSE|XCR0.YMM both enabled, and
//! #NM when CR0.TS is set.
//!
//! rusty_box applies the same gate at icache fill: `state_resolve_opcode`
//! (cpu/decoder/mod.rs) reads the generated `opcode_isa::opcode_state` and
//! turns a `CpuState::Avx` opcode into `NoAvxState` while AVX state is
//! unavailable. The legacy SSE state CR4.OSFXSR names is a different state:
//! it says nothing about CR4.OSXSAVE or XCR0, so it is the AVX gate that keeps
//! an OS which has not enabled AVX state from running AVX instructions whose
//! YMM results it would never save or restore.
//!
//! The fixture below sets CR4.OSXSAVE but leaves XCR0 at its reset value, a
//! configuration in which every `BX_PREPARE_AVX` instruction #UDs.
//!
//! With AVX state enabled, the file also holds divergence D19
//! (`docs/bochs-parity-divergences.md`): a VEX shift by immediate with a
//! memory operand is #UD, as on hardware, where Bochs executes it.

#![cfg(feature = "std")]

mod state_gate;

use rusty_box::cpu::X86Reg;
use rusty_box::emulator::{Emulator, EmulatorConfig};
use state_gate::{long64_emulator, run_one, Outcome, CODE, TEST_STACK_SIZE};

/// A CPU with SSE fully enabled and AVX state deliberately *not* enabled:
/// CR4.OSFXSR and CR4.OSXSAVE are set, but no XSETBV runs, so XCR0 keeps its
/// reset value of 1 (x87 only). The SSE state gate is satisfied here; the AVX
/// one — Bochs's `BxNoAVX` — is not.
fn avx_state_disabled_emulator() -> Box<Emulator> {
    let mut emu = long64_emulator(EmulatorConfig::default());
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );
    emu
}

/// The same CPU after the guest's own XSETBV (ECX = 0, EDX:EAX = 7) has set
/// XCR0 to x87|SSE|AVX, the state an OS enables before it runs AVX code.
fn avx_state_enabled_emulator() -> Box<Emulator> {
    let mut emu = avx_state_disabled_emulator();
    emu.reg_write(X86Reg::Rax, 0x7);
    emu.reg_write(X86Reg::Rcx, 0);
    emu.reg_write(X86Reg::Rdx, 0);
    emu.mem_write(CODE, &[0x0F, 0x01, 0xD1]).expect("xsetbv");
    emu.emu_start(CODE, Some(CODE + 3), None, Some(1))
        .expect("enable AVX state");
    emu
}

/// Each of these instructions #UDs while XCR0 has not enabled AVX state: the
/// `BX_PREPARE_AVX` ones through Bochs proc_ctrl.cc `BxNoAVX`, and the EVEX one
/// (`BX_PREPARE_EVEX_NO_SAE`) through proc_ctrl.cc `BxNoEVEX`, which demands
/// the same XCR0.SSE|XCR0.YMM bits and the AVX-512 ones besides.
#[test]
fn vex_encodings_ud_when_guest_has_not_enabled_avx_state() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            // (name, encoding). Every VEX case is a BX_PREPARE_AVX
            // instruction and the EVEX case a BX_PREPARE_EVEX one, so in
            // Bochs each reaches the instruction only through BxNoAVX or
            // BxNoEVEX while their state is off.
            let cases: &[(&str, &[u8])] = &[
                // VTESTPS ymm1, ymm2          VEX.256.66.0F38.W0 0E /r
                ("vtestps", &[0xC4, 0xE2, 0x7D, 0x0E, 0xCA]),
                // VTESTPD xmm1, xmm2          VEX.128.66.0F38.W0 0F /r
                ("vtestpd", &[0xC4, 0xE2, 0x79, 0x0F, 0xCA]),
                // VPERMILPS xmm0, xmm1, xmm2  VEX.128.66.0F38.W0 0C /r
                ("vpermilps", &[0xC4, 0xE2, 0x71, 0x0C, 0xC2]),
                // VPERMILPD ymm0, ymm1, ymm2  VEX.256.66.0F38.W0 0D /r
                ("vpermilpd", &[0xC4, 0xE2, 0x75, 0x0D, 0xC2]),
                // VPERMILPS xmm0, xmm1, 0     VEX.128.66.0F3A.W0 04 /r ib
                ("vpermilps_imm", &[0xC4, 0xE3, 0x79, 0x04, 0xC1, 0x00]),
                // VPERMILPD ymm0, ymm1, 0     VEX.256.66.0F3A.W0 05 /r ib
                ("vpermilpd_imm", &[0xC4, 0xE3, 0x7D, 0x05, 0xC1, 0x00]),
                // VPERMPS ymm0, ymm1, ymm2    VEX.256.66.0F38.W0 16 /r
                ("vpermps", &[0xC4, 0xE2, 0x75, 0x16, 0xC2]),
                // VPERMPD ymm0, ymm1, 0       VEX.256.66.0F3A.W1 01 /r ib
                ("vpermpd", &[0xC4, 0xE3, 0xFD, 0x01, 0xC1, 0x00]),
                // VPSRLVD xmm0, xmm1, xmm2    VEX.128.66.0F38.W0 45 /r
                ("vpsrlvd", &[0xC4, 0xE2, 0x71, 0x45, 0xC2]),
                // VPSLLVQ xmm0, xmm1, xmm2    VEX.128.66.0F38.W1 47 /r
                ("vpsllvq", &[0xC4, 0xE2, 0xF1, 0x47, 0xC2]),
                // The shift-by-immediate groups. 2-byte VEX, vvvv selects
                // the destination and ModRM.reg selects the group member.
                // VPSRLD xmm0, xmm1, 4        C5 F9 72 /2
                ("vpsrld_imm", &[0xC5, 0xF9, 0x72, 0xD1, 0x04]),
                // VPSLLD xmm0, xmm1, 4        C5 F9 72 /6
                ("vpslld_imm", &[0xC5, 0xF9, 0x72, 0xF1, 0x04]),
                // VPSRLQ xmm0, xmm1, 4        C5 F9 73 /2
                ("vpsrlq_imm", &[0xC5, 0xF9, 0x73, 0xD1, 0x04]),
                // VPSLLDQ xmm0, xmm1, 4       C5 F9 73 /7
                ("vpslldq_imm", &[0xC5, 0xF9, 0x73, 0xF9, 0x04]),
                // VPSRAW xmm0, xmm1, 4        C5 F9 71 /4
                ("vpsraw_imm", &[0xC5, 0xF9, 0x71, 0xE1, 0x04]),
                // No handler checks CPU state itself, so only the icache-fill
                // state gate stands between these and a guest without AVX
                // state.
                // VPADDB xmm0, xmm1, xmm2     VEX.128.66.0F.W0 FC /r
                ("vpaddb", &[0xC5, 0xF1, 0xFC, 0xC2]),
                // VMPSADBW ymm0, ymm1, ymm2, 0  VEX.256.66.0F3A.W0 42 /r ib
                ("vmpsadbw", &[0xC4, 0xE3, 0x75, 0x42, 0xC2, 0x00]),
                // VMOVDQA [rax], ymm0         VEX.256.66.0F.W0 7F /r
                ("vmovdqa_store", &[0xC5, 0xFD, 0x7F, 0x00]),
                // EVEX VPADDD zmm0, zmm1, zmm2  EVEX.512.66.0F.W0 FE /r
                ("evex_vpaddd", &[0x62, 0xF1, 0x75, 0x48, 0xFE, 0xC2]),
                // Controls — these already gate on AVX, so they anchor the
                // fixture: if these ever stop faulting the harness is wrong,
                // not the handler.
                // VPCLMULQDQ xmm0, xmm1, xmm2, 0  VEX.128.66.0F3A.W0 44 /r ib
                ("vpclmulqdq", &[0xC4, 0xE3, 0x71, 0x44, 0xC2, 0x00]),
                // VCVTPH2PS xmm1, xmm2        VEX.128.66.0F38.W0 13 /r
                ("vcvtph2ps", &[0xC4, 0xE2, 0x79, 0x13, 0xCA]),
            ];

            for (name, code) in cases {
                let mut emu = avx_state_disabled_emulator();
                assert_eq!(
                    run_one(&mut emu, code),
                    Outcome::InvalidOpcode,
                    "{name}: a VEX encoding must raise #UD while XCR0 has not \
                     enabled AVX state — CR4.OSXSAVE and XCR0.SSE|YMM are what \
                     Bochs BxNoAVX tests, and CR4.OSFXSR is not a substitute"
                );
            }
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// CR0.TS owes the guest #NM, not #UD.
///
/// This is why the gate substitutes a sentinel opcode that dispatches to
/// `bx_no_avx` rather than plain `Opcode::IaError`: `BxNoAVX` raises #UD only
/// when the state is genuinely unavailable, and #NM when the state is fine but
/// CR0.TS is set. Collapsing both into #UD would break lazy FPU/AVX context
/// switching, where the #NM handler is what restores the register file.
#[test]
fn cr0_ts_raises_nm_rather_than_ud() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut emu = avx_state_disabled_emulator();

            // Enable AVX state properly, so the only thing left is CR0.TS.
            emu.reg_write(X86Reg::Rax, 0x7);
            emu.reg_write(X86Reg::Rcx, 0);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CODE, &[0x0F, 0x01, 0xD1]).expect("xsetbv");
            emu.emu_start(CODE, Some(CODE + 3), None, Some(1))
                .expect("enable AVX state");

            // CR0.TS — the guest deferred saving the vector register file.
            emu.reg_write(X86Reg::Cr0, emu.reg_read(X86Reg::Cr0) | (1 << 3));

            // vpaddd xmm0, xmm1, xmm2
            let code = [0xC5, 0xF1, 0xFE, 0xC2];
            assert_eq!(
                run_one(&mut emu, &code),
                Outcome::DeviceNotAvailable,
                "CR0.TS must raise #NM so the guest's lazy-restore handler runs, \
                 and not #UD — the instruction is legal, the register file is \
                 merely not loaded yet"
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// Clearing CR4.OSXSAVE must take effect immediately.
///
/// The gate reads `fetch_mode_mask`, which is recomputed by
/// `handle_avx_mode_change` rather than derived on each lookup, so every path
/// that changes CR0/CR4/XCR0 has to refresh it. A stale bit here would be
/// invisible until a guest disabled AVX and kept executing it.
#[test]
fn clearing_cr4_osxsave_disables_avx_immediately() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut emu = avx_state_disabled_emulator();
            emu.reg_write(X86Reg::Rax, 0x7);
            emu.reg_write(X86Reg::Rcx, 0);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CODE, &[0x0F, 0x01, 0xD1]).expect("xsetbv");
            emu.emu_start(CODE, Some(CODE + 3), None, Some(1))
                .expect("enable AVX state");

            // vpaddd xmm0, xmm1, xmm2 — runs while AVX state is enabled.
            let code = [0xC5u8, 0xF1, 0xFE, 0xC2];
            assert_eq!(
                run_one(&mut emu, &code),
                Outcome::Retired,
                "sanity: the instruction must execute while AVX state is on"
            );

            // Drop CR4.OSXSAVE. XCR0 still reads 7, but without OSXSAVE the
            // CPU is no longer in a state where AVX may execute.
            emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) & !(1 << 18));

            assert_eq!(
                run_one(&mut emu, &code),
                Outcome::InvalidOpcode,
                "clearing CR4.OSXSAVE must #UD the next AVX instruction — if it \
                 does not, fetch_mode_mask went stale and the icache state gate \
                 is running on out-of-date CPU state"
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// With AVX state properly enabled the very same encodings execute. This is
/// the other half of the gate: it must not turn a working instruction into a
/// fault.
#[test]
fn the_same_vex_encodings_execute_once_avx_state_is_enabled() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cases: &[(&str, &[u8])] = &[
                ("vtestps", &[0xC4, 0xE2, 0x7D, 0x0E, 0xCA]),
                ("vpermilps", &[0xC4, 0xE2, 0x71, 0x0C, 0xC2]),
                ("vpermps", &[0xC4, 0xE2, 0x75, 0x16, 0xC2]),
                ("vpsrlvd", &[0xC4, 0xE2, 0x71, 0x45, 0xC2]),
            ];

            for (name, code) in cases {
                let mut emu = avx_state_enabled_emulator();
                assert_eq!(
                    run_one(&mut emu, code),
                    Outcome::Retired,
                    "{name}: must execute once XCR0 enables AVX state"
                );
            }
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// One VEX shift-by-immediate group, `VEX.66.0F <opcode> /<member> ib`: the
/// opcode byte, and the ModRM.reg values that name an instruction in it.
struct ShiftByImmediateGroup {
    opcode: u8,
    members: &'static [u8],
}

/// Bochs fetchdecode_opmap_avx.cc `BxOpcodeGroup_VEX_0F71`, `_0F72`, `_0F73`.
const SHIFT_BY_IMMEDIATE_GROUPS: [ShiftByImmediateGroup; 3] = [
    // VPSRLW, VPSRAW, VPSLLW
    ShiftByImmediateGroup {
        opcode: 0x71,
        members: &[2, 4, 6],
    },
    // VPSRLD, VPSRAD, VPSLLD
    ShiftByImmediateGroup {
        opcode: 0x72,
        members: &[2, 4, 6],
    },
    // VPSRLQ, VPSRLDQ, VPSLLQ, VPSLLDQ
    ShiftByImmediateGroup {
        opcode: 0x73,
        members: &[2, 3, 6, 7],
    },
];

/// Divergence D19: every VEX shift by immediate, VEX.128 and VEX.256, retires
/// in its register form and raises #UD in its memory form. The SDM lists the
/// VEX forms register-only, and an i5-12450H raised #UD for the five forms
/// docs/bochs-vex-shift-imm-memory-probe.S asks it about (one per group and
/// vector length). Bochs executes the memory form because its VEX groups lack
/// ATTR_MODC0.
#[test]
fn vex_shift_by_immediate_refuses_a_memory_operand() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            for group in &SHIFT_BY_IMMEDIATE_GROUPS {
                for &member in group.members {
                    // 2-byte VEX, byte 1 = R̄ v̄vvv L pp: vvvv = xmm1 (the
                    // destination), pp = 66, L = 0 (0xF1) or 1 (0xF5).
                    for vex in [0xF1u8, 0xF5] {
                        let register = [0xC5, vex, group.opcode, 0xC0 | (member << 3) | 2, 0x04];
                        // ModRM mod = 00, rm = 101: [rip + 0x100].
                        let memory = [
                            0xC5,
                            vex,
                            group.opcode,
                            (member << 3) | 5,
                            0x00,
                            0x01,
                            0x00,
                            0x00,
                            0x04,
                        ];

                        let mut emu = avx_state_enabled_emulator();
                        assert_eq!(
                            run_one(&mut emu, &register),
                            Outcome::Retired,
                            "{register:02X?}: the register form executes"
                        );
                        let mut emu = avx_state_enabled_emulator();
                        assert_eq!(
                            run_one(&mut emu, &memory),
                            Outcome::InvalidOpcode,
                            "{memory:02X?}: the memory form is #UD"
                        );
                    }
                }
            }
        })
        .expect("spawn")
        .join()
        .expect("join");
}
