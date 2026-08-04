//! Every VEX-encoded instruction must check *AVX* availability, not SSE.
//!
//! Bochs tags the whole VEX surface `BX_PREPARE_AVX` in its fetch-decode
//! tables; when `BX_FETCH_MODE_AVX_OK` is clear the handler is replaced with
//! `BxNoAVX` (cpu/proc_ctrl.cc), which raises #UD unless the CPU is in
//! protected mode with CR4.OSXSAVE set and XCR0.SSE|XCR0.YMM both enabled, and
//! #NM when CR0.TS is set.
//!
//! rusty_box ports that gate in `cpu.rs` (`OpFlags::PREPARE_AVX` ->
//! `bx_no_avx_wrapper`), but it only fires for opcodes present in
//! `opcodes_table`. The AVX2/F16C opcodes wired up in Phase A are not in that
//! table, so `get_opcode_entry` returns `None`, the gate is skipped, and the
//! `prepare_*` call inside the handler is the only thing standing between the
//! guest and the instruction.
//!
//! Several of those handlers called `prepare_sse()`, which tests CR0.EM and
//! CR4.OSFXSR — and never looks at CR4.OSXSAVE or XCR0. An OS that had not
//! enabled AVX state could therefore execute AVX2 instructions, whose YMM
//! results it would then never save or restore across a context switch.
//!
//! The fixture below is the configuration the comment on
//! `enable_guest_avx_state` in fp_vex_scalar_ops.rs already describes: CR4
//! .OSXSAVE set but XCR0 left at its reset value, "a configuration in which
//! every VEX encoding would #UD".

#![cfg(feature = "std")]

use rusty_box::cpu::{core_i7_skylake::Corei7SkylakeX, CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
const CODE: u64 = 0x0020_0000;
const UD_VECTOR: usize = 6;

/// A CPU with SSE fully enabled and AVX state deliberately *not* enabled:
/// CR4.OSFXSR and CR4.OSXSAVE are set, but no XSETBV runs, so XCR0 keeps its
/// reset value of 1 (x87 only). `prepare_sse` is satisfied here; `prepare_avx`
/// — and Bochs's `BxNoAVX` — are not.
fn avx_state_disabled_emulator() -> Box<Emulator<'static, Corei7SkylakeX>> {
    let cfg = EmulatorConfig::default();
    let mut emu =
        Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64).expect("emulator");
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );
    emu
}

/// Run one encoding and return how many #UDs it raised.
fn ud_raised_by(emu: &mut Emulator<'static, Corei7SkylakeX>, code: &[u8]) -> u64 {
    let before = emu.cpu().get_exception_diag()[UD_VECTOR];
    let mut image = code.to_vec();
    image.extend_from_slice(&[0xEB, 0xFE]); // jmp $
    emu.mem_write(CODE, &image).expect("write code");
    emu.emu_start(CODE, Some(CODE + code.len() as u64), None, Some(4))
        .expect("emu_start");
    emu.cpu().get_exception_diag()[UD_VECTOR] - before
}

/// Every VEX encoding wired up in Phase A must #UD when XCR0 has not enabled
/// AVX state, exactly as Bochs's `BxNoAVX` does.
#[test]
fn vex_encodings_ud_when_guest_has_not_enabled_avx_state() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            // (name, encoding). Every one of these is VEX-only, so Bochs
            // reaches it exclusively through BX_PREPARE_AVX.
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
                // The shift-by-immediate groups, whose handlers are the ones
                // the Windows 7 stall was traced to. 2-byte VEX, vvvv selects
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
                // Handlers that still gate on SSE internally. They are covered
                // by the icache-fill state gate rather than by their own
                // prepare_* call, so they exercise the central path.
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
                    ud_raised_by(&mut emu, code),
                    1,
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
            const NM_VECTOR: usize = 7;
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

            let ud_before = emu.cpu().get_exception_diag()[UD_VECTOR];
            let nm_before = emu.cpu().get_exception_diag()[NM_VECTOR];

            // vpaddd xmm0, xmm1, xmm2
            let code = [0xC5, 0xF1, 0xFE, 0xC2];
            let mut image = code.to_vec();
            image.extend_from_slice(&[0xEB, 0xFE]);
            emu.mem_write(CODE, &image).expect("write code");
            emu.emu_start(CODE, Some(CODE + code.len() as u64), None, Some(4))
                .expect("emu_start");

            assert_eq!(
                emu.cpu().get_exception_diag()[NM_VECTOR] - nm_before,
                1,
                "CR0.TS must raise #NM so the guest's lazy-restore handler runs"
            );
            assert_eq!(
                emu.cpu().get_exception_diag()[UD_VECTOR] - ud_before,
                0,
                "CR0.TS must not be reported as #UD — the instruction is legal, \
                 the register file is merely not loaded yet"
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
                ud_raised_by(&mut emu, &code),
                0,
                "sanity: the instruction must execute while AVX state is on"
            );

            // Drop CR4.OSXSAVE. XCR0 still reads 7, but without OSXSAVE the
            // CPU is no longer in a state where AVX may execute.
            emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) & !(1 << 18));

            assert_eq!(
                ud_raised_by(&mut emu, &code),
                1,
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
/// the other half of the gate: the fix must not turn a working instruction
/// into a fault.
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
                let mut emu = avx_state_disabled_emulator();
                // XSETBV ECX=0, EDX:EAX=7 -> XCR0 = FPU|SSE|YMM.
                emu.reg_write(X86Reg::Rax, 0x7);
                emu.reg_write(X86Reg::Rcx, 0);
                emu.reg_write(X86Reg::Rdx, 0);
                emu.mem_write(CODE, &[0x0F, 0x01, 0xD1]).expect("xsetbv");
                emu.emu_start(CODE, Some(CODE + 3), None, Some(1))
                    .expect("enable AVX state");

                assert_eq!(
                    ud_raised_by(&mut emu, code),
                    0,
                    "{name}: must execute once XCR0 enables AVX state"
                );
            }
        })
        .expect("spawn")
        .join()
        .expect("join");
}
