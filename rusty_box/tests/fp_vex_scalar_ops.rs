//! Differential tests for VEX-encoded scalar double-precision ops.
//!
//! Ubuntu's glibc ships an x86-64-v3 build of libm under glibc-hwcaps; on a
//! CPUID model advertising AVX2+FMA (Corei7SkylakeX) the guest dynamic linker
//! selects it, so transcendental functions (log, exp, pow) execute VEX scalar
//! ops — above all VFMADD — that the baseline-v1 userspace never touches.
//! A guest failure of `math.log(4.0)` (CPython: "ValueError: expected a
//! positive input, got 4.0") implicates exactly these instructions.
//!
//! Each case executes ONE real instruction through the full CPU loop
//! (decode → dispatch → softfloat) in flat long mode and compares the result
//! bit-for-bit against the host's IEEE-754 arithmetic, which is correctly
//! rounded for +,-,*,/,sqrt,fma — the same rounding the guest MXCSR
//! (reset state: round-to-nearest, no DAZ/FTZ) must produce.

#![cfg(feature = "std")]

use rusty_box::cpu::{core_i7_skylake::Corei7SkylakeX, CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

const CASE_BASE: u64 = 0x0020_0000;
const CASE_STRIDE: u64 = 64;
const STACK_TOP: u64 = 0x0050_0000;

/// One instruction under test. Inputs go to xmm0/xmm1/xmm2 (+rax); the
/// low qword of xmm0 is the result. `flags` cases append pushfq; pop rax.
struct Case {
    name: &'static str,
    code: &'static [u8],
    insns: u64,
    xmm0: f64,
    xmm1: f64,
    xmm2: f64,
    rax: u64,
    expected: Expected,
}

enum Expected {
    /// Low qword of xmm0, compared bit-for-bit.
    F64Bits(u64),
    /// rax & 0x45 (CF|PF|ZF) after pushfq; pop rax.
    ArithFlags(u64),
}

fn f(bits_of: f64) -> u64 {
    bits_of.to_bits()
}

fn cases() -> Vec<Case> {
    // Distinct exact values so each FMA form (132/213/231) yields a
    // different result — catches operand-order mis-wiring outright.
    let a = 1.5_f64; // xmm0 (dst)
    let b = 2.5_f64; // xmm1 (VEX.vvvv)
    let c = 4.25_f64; // xmm2 (modrm.rm)

    // glibc e_log.c core step: r = fma(z, invc, -1.0) — huge cancellation,
    // the most bit-sensitive operation in log().
    let z = 1.376953125_f64;
    let invc = 1.0_f64 / z;

    vec![
        Case {
            name: "nop canary (harness sanity)",
            code: &[0x90],
            insns: 1,
            xmm0: 1.5,
            xmm1: 0.0,
            xmm2: 0.0,
            rax: 0,
            expected: Expected::F64Bits(f(1.5)),
        },
        Case {
            name: "vfmadd213sd xmm0,xmm1,xmm2 (xmm1*xmm0+xmm2)",
            code: &[0xC4, 0xE2, 0xF1, 0xA9, 0xC2],
            insns: 1,
            xmm0: a,
            xmm1: b,
            xmm2: c,
            rax: 0,
            expected: Expected::F64Bits(f(b.mul_add(a, c))), // 8.0
        },
        Case {
            name: "vfmadd231sd xmm0,xmm1,xmm2 (xmm1*xmm2+xmm0)",
            code: &[0xC4, 0xE2, 0xF1, 0xB9, 0xC2],
            insns: 1,
            xmm0: a,
            xmm1: b,
            xmm2: c,
            rax: 0,
            expected: Expected::F64Bits(f(b.mul_add(c, a))), // 12.125
        },
        Case {
            name: "vfmadd132sd xmm0,xmm1,xmm2 (xmm0*xmm2+xmm1)",
            code: &[0xC4, 0xE2, 0xF1, 0x99, 0xC2],
            insns: 1,
            xmm0: a,
            xmm1: b,
            xmm2: c,
            rax: 0,
            expected: Expected::F64Bits(f(a.mul_add(c, b))), // 8.875
        },
        Case {
            name: "vfmadd213sd glibc-log cancellation fma(z,invc,-1)",
            code: &[0xC4, 0xE2, 0xF1, 0xA9, 0xC2],
            insns: 1,
            xmm0: z,
            xmm1: invc,
            xmm2: -1.0,
            rax: 0,
            expected: Expected::F64Bits(f(invc.mul_add(z, -1.0))),
        },
        Case {
            // (1+2^-52)*2 - 2 = 2^-51 exactly — independent of host libm.
            name: "vfmadd213sd exact tie fma(1+ulp,2,-2)",
            code: &[0xC4, 0xE2, 0xF1, 0xA9, 0xC2],
            insns: 1,
            xmm0: f64::from_bits(0x3FF0_0000_0000_0001),
            xmm1: 2.0,
            xmm2: -2.0,
            rax: 0,
            expected: Expected::F64Bits(0x3CC0_0000_0000_0000), // 2^-51
        },
        Case {
            name: "vmulsd xmm0,xmm1,xmm2 (0.1*0.2)",
            code: &[0xC5, 0xF3, 0x59, 0xC2],
            insns: 1,
            xmm0: 0.0,
            xmm1: 0.1,
            xmm2: 0.2,
            rax: 0,
            expected: Expected::F64Bits(f(0.1_f64 * 0.2_f64)),
        },
        Case {
            name: "vaddsd xmm0,xmm1,xmm2 (0.1+0.2)",
            code: &[0xC5, 0xF3, 0x58, 0xC2],
            insns: 1,
            xmm0: 0.0,
            xmm1: 0.1,
            xmm2: 0.2,
            rax: 0,
            expected: Expected::F64Bits(f(0.1_f64 + 0.2_f64)),
        },
        Case {
            name: "vsubsd xmm0,xmm1,xmm2 (1.0-1/3)",
            code: &[0xC5, 0xF3, 0x5C, 0xC2],
            insns: 1,
            xmm0: 0.0,
            xmm1: 1.0,
            xmm2: 1.0_f64 / 3.0_f64,
            rax: 0,
            expected: Expected::F64Bits(f(1.0_f64 - 1.0_f64 / 3.0_f64)),
        },
        Case {
            name: "vdivsd xmm0,xmm1,xmm2 (1/3)",
            code: &[0xC5, 0xF3, 0x5E, 0xC2],
            insns: 1,
            xmm0: 0.0,
            xmm1: 1.0,
            xmm2: 3.0,
            rax: 0,
            expected: Expected::F64Bits(f(1.0_f64 / 3.0_f64)),
        },
        Case {
            name: "vsqrtsd xmm0,xmm1,xmm2 (sqrt 2)",
            code: &[0xC5, 0xF3, 0x51, 0xC2],
            insns: 1,
            xmm0: 0.0,
            xmm1: 0.0,
            xmm2: 2.0,
            rax: 0,
            expected: Expected::F64Bits(f(2.0_f64.sqrt())),
        },
        Case {
            name: "vcvtsi2sd xmm0,xmm1,rax (rounding case 2^62+1)",
            code: &[0xC4, 0xE1, 0xF3, 0x2A, 0xC0],
            insns: 1,
            xmm0: 0.0,
            xmm1: 0.0,
            xmm2: 0.0,
            rax: (1u64 << 62) + 1,
            expected: Expected::F64Bits(f(((1i64 << 62) + 1) as f64)),
        },
        Case {
            name: "vcvtsi2sd xmm0,xmm1,rax (-7)",
            code: &[0xC4, 0xE1, 0xF3, 0x2A, 0xC0],
            insns: 1,
            xmm0: 0.0,
            xmm1: 0.0,
            xmm2: 0.0,
            rax: (-7i64) as u64,
            expected: Expected::F64Bits(f(-7.0)),
        },
        // CPython m_log guard: `if (x > 0.0)` — comisd/ucomisd flags.
        // Above: CF=0 ZF=0 PF=0.
        Case {
            name: "vucomisd 4.0 vs 0.0 (above => CF=ZF=PF=0)",
            code: &[0xC5, 0xF9, 0x2E, 0xC1, 0x9C, 0x58], // vucomisd; pushfq; pop rax
            insns: 3,
            xmm0: 4.0,
            xmm1: 0.0,
            xmm2: 0.0,
            rax: 0,
            expected: Expected::ArithFlags(0x00),
        },
        Case {
            name: "vucomisd 0.0 vs 0.0 (equal => ZF)",
            code: &[0xC5, 0xF9, 0x2E, 0xC1, 0x9C, 0x58],
            insns: 3,
            xmm0: 0.0,
            xmm1: 0.0,
            xmm2: 0.0,
            rax: 0,
            expected: Expected::ArithFlags(0x40),
        },
        Case {
            name: "vucomisd NaN vs 1.0 (unordered => CF|PF|ZF)",
            code: &[0xC5, 0xF9, 0x2E, 0xC1, 0x9C, 0x58],
            insns: 3,
            xmm0: f64::NAN,
            xmm1: 1.0,
            xmm2: 0.0,
            rax: 0,
            expected: Expected::ArithFlags(0x45),
        },
        Case {
            name: "ucomisd (SSE2) 4.0 vs 0.0 (above => CF=ZF=PF=0)",
            code: &[0x66, 0x0F, 0x2E, 0xC1, 0x9C, 0x58], // ucomisd; pushfq; pop rax
            insns: 3,
            xmm0: 4.0,
            xmm1: 0.0,
            xmm2: 0.0,
            rax: 0,
            expected: Expected::ArithFlags(0x00),
        },
    ]
}

fn xmm_from_f64(v: f64) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&v.to_bits().to_le_bytes());
    out
}

#[test]
fn vex_scalar_double_ops_match_host_ieee() {
    // Long-mode page tables + emulator need a large stack.
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(run_all_cases)
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}

fn run_all_cases() {
    let cfg = EmulatorConfig::default();
    let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
        .expect("new emulator in flat long mode");

    // Enable SSE/AVX the way the API tests do (CR4.OSFXSR | CR4.OSXSAVE).
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );
    eprintln!(
        "harness: cr0={:#x} cr4={:#x} rip={:#x}",
        emu.reg_read(X86Reg::Cr0),
        emu.reg_read(X86Reg::Cr4),
        emu.cpu.rip(),
    );

    // Each case is followed by `jmp $` (EB FE): a branch ends the icache
    // trace and parks RIP, so the batch executor cannot run over into the
    // zero bytes behind the case (executing garbage until a fault).
    let cases = cases();
    for (i, case) in cases.iter().enumerate() {
        let addr = CASE_BASE + i as u64 * CASE_STRIDE;
        emu.mem_write(addr, case.code).expect("write case code");
        emu.mem_write(addr + case.code.len() as u64, &[0xEB, 0xFE])
            .expect("write park jump");
    }

    let mut failures: Vec<String> = Vec::new();

    for (i, case) in cases.iter().enumerate() {
        emu.reg_write_xmm(X86Reg::Xmm0, xmm_from_f64(case.xmm0));
        emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f64(case.xmm1));
        emu.reg_write_xmm(X86Reg::Xmm2, xmm_from_f64(case.xmm2));
        emu.reg_write(X86Reg::Rax, case.rax);
        emu.reg_write(X86Reg::Rsp, STACK_TOP);

        let addr = CASE_BASE + i as u64 * CASE_STRIDE;
        let park = addr + case.code.len() as u64;
        // Budget: the case's instructions plus a few laps of the park loop;
        // `until` stops the run once RIP sits at the park jump.
        match emu.emu_start(addr, Some(park), None, Some(case.insns + 8)) {
            Ok(stop) => {
                let end_rip = emu.cpu.rip();
                if end_rip != park {
                    failures.push(format!(
                        "{}: did not park — stop={stop:?} rip={end_rip:#x} want {park:#x}",
                        case.name
                    ));
                    continue;
                }
            }
            Err(e) => {
                failures.push(format!("{}: execution error {e:?}", case.name));
                continue;
            }
        }

        match case.expected {
            Expected::F64Bits(want) => {
                let got_bytes = emu.reg_read_xmm(X86Reg::Xmm0);
                let got = u64::from_le_bytes(got_bytes[..8].try_into().unwrap());
                if got != want {
                    failures.push(format!(
                        "{}: got {:#018x} ({}), want {:#018x} ({})",
                        case.name,
                        got,
                        f64::from_bits(got),
                        want,
                        f64::from_bits(want),
                    ));
                }
            }
            Expected::ArithFlags(want) => {
                let got = emu.reg_read(X86Reg::Rax) & 0x45;
                if got != want {
                    failures.push(format!(
                        "{}: flags(CF|PF|ZF) got {got:#04x}, want {want:#04x}",
                        case.name
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} VEX scalar-double cases diverged from host IEEE:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

// ════════════════════════════════════════════════════════════════════════
// Packed VEX FP, VL=256, upper-lane semantics, and legacy signed-zero
// min/max behavior.
// ════════════════════════════════════════════════════════════════════════

fn ymm_from_f64x4(v: [f64; 4]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, x) in v.iter().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&x.to_bits().to_le_bytes());
    }
    out
}

fn ymm_from_f32x8(v: [f32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, x) in v.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_bits().to_le_bytes());
    }
    out
}

fn f64_lane(ymm: &[u8; 32], i: usize) -> u64 {
    u64::from_le_bytes(ymm[i * 8..i * 8 + 8].try_into().unwrap())
}

fn f32_lane(ymm: &[u8; 32], i: usize) -> u32 {
    u32::from_le_bytes(ymm[i * 4..i * 4 + 4].try_into().unwrap())
}

fn xmm_lane64(xmm: &[u8; 16], i: usize) -> u64 {
    u64::from_le_bytes(xmm[i * 8..i * 8 + 8].try_into().unwrap())
}

#[test]
fn vex_packed_fp_upper_zeroing_and_legacy_minmax() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(run_packed_cases)
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}

fn run_packed_cases() {
    let cfg = EmulatorConfig::default();
    let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
        .expect("new emulator in flat long mode");
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );

    // Each case: (name, code) at CASE_BASE + i*CASE_STRIDE with a park jump.
    let programs: &[(&str, &[u8])] = &[
        ("vaddpd ymm0,ymm1,ymm2", &[0xC5, 0xF5, 0x58, 0xC2]),
        ("vmulps xmm0,xmm1,xmm2", &[0xC5, 0xF0, 0x59, 0xC2]),
        ("vminsd xmm0,xmm1,xmm2", &[0xC5, 0xF3, 0x5D, 0xC2]),
        ("vmaxsd xmm0,xmm1,xmm2 (NaN)", &[0xC5, 0xF3, 0x5F, 0xC2]),
        ("vsqrtsd xmm0,xmm1,xmm2", &[0xC5, 0xF3, 0x51, 0xC2]),
        ("vcmpsd xmm0,xmm1,xmm2,0x0D", &[0xC5, 0xF3, 0xC2, 0xC2, 0x0D]),
        ("vshufps xmm0,xmm1,xmm2,0x1B", &[0xC5, 0xF0, 0xC6, 0xC2, 0x1B]),
        ("vunpcklpd xmm0,xmm1,xmm2", &[0xC5, 0xF1, 0x14, 0xC2]),
        ("minsd xmm0,xmm1 (SSE2 signed zero)", &[0xF2, 0x0F, 0x5D, 0xC1]),
        ("vaddsubpd xmm0,xmm1,xmm2", &[0xC5, 0xF1, 0xD0, 0xC2]),
        ("vmovsd xmm0,xmm1,xmm2 (reg merge)", &[0xC5, 0xF3, 0x10, 0xC2]),
        (
            "vroundsd xmm0,xmm1,xmm2,0x09 (floor)",
            &[0xC4, 0xE3, 0x71, 0x0B, 0xC2, 0x09],
        ),
        ("vcvtss2sd xmm0,xmm1,xmm2", &[0xC5, 0xF2, 0x5A, 0xC2]),
    ];
    for (i, (_, code)) in programs.iter().enumerate() {
        let addr = CASE_BASE + i as u64 * CASE_STRIDE;
        emu.mem_write(addr, code).expect("write case code");
        emu.mem_write(addr + code.len() as u64, &[0xEB, 0xFE])
            .expect("write park jump");
    }
    fn run(
        emu: &mut Emulator<'static, Corei7SkylakeX>,
        programs: &[(&str, &[u8])],
        idx: usize,
    ) {
        let (name, code) = programs[idx];
        let addr = CASE_BASE + idx as u64 * CASE_STRIDE;
        let park = addr + code.len() as u64;
        let stop = emu
            .emu_start(addr, Some(park), None, Some(9))
            .expect("emu_start");
        assert_eq!(emu.cpu.rip(), park, "{name}: did not park (stop={stop:?})");
    }

    // 0: vaddpd ymm — 4 lanes, host-IEEE exact match.
    let a = [1.5f64, 2.5, 3.5, 4.5];
    let b = [0.1f64, 0.2, 0.3, 0.4];
    emu.reg_write_ymm(X86Reg::Ymm1, ymm_from_f64x4(a));
    emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_f64x4(b));
    run(&mut emu, programs, 0);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    for i in 0..4 {
        assert_eq!(
            f64_lane(&got, i),
            (a[i] + b[i]).to_bits(),
            "vaddpd ymm lane {i}"
        );
    }

    // 1: vmulps xmm with ymm0 preloaded garbage — checks result lanes AND
    // that bits 255:128 of the destination were zeroed (VEX.128 semantics).
    let ga = [1.5f32, -2.5, 3.25, 0.1, 0.0, 0.0, 0.0, 0.0];
    let gb = [4.0f32, 8.0, -1.5, 0.2, 0.0, 0.0, 0.0, 0.0];
    emu.reg_write_ymm(X86Reg::Ymm0, [0xAA; 32]); // garbage incl. upper 128
    emu.reg_write_ymm(X86Reg::Ymm1, ymm_from_f32x8(ga));
    emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_f32x8(gb));
    run(&mut emu, programs, 1);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    for i in 0..4 {
        assert_eq!(
            f32_lane(&got, i),
            (ga[i] * gb[i]).to_bits(),
            "vmulps lane {i}"
        );
    }
    assert_eq!(&got[16..32], &[0u8; 16], "vmulps must zero ymm bits 255:128");

    // 2: vminsd(+0.0, -0.0) must return the SECOND source (-0.0), matching
    // Bochs f64_min / Intel MINSD.
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f64(0.0));
    emu.reg_write_xmm(X86Reg::Xmm2, xmm_from_f64(-0.0));
    run(&mut emu, programs, 2);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    assert_eq!(
        xmm_lane64(&got, 0),
        (-0.0f64).to_bits(),
        "vminsd(+0,-0) must return -0.0 (second source)"
    );

    // 3: vmaxsd(NaN, 5.0) must return the SECOND source (5.0).
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f64(f64::NAN));
    emu.reg_write_xmm(X86Reg::Xmm2, xmm_from_f64(5.0));
    run(&mut emu, programs, 3);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    assert_eq!(
        xmm_lane64(&got, 0),
        5.0f64.to_bits(),
        "vmaxsd(NaN, 5.0) must return 5.0"
    );

    // 4: vsqrtsd — low lane = sqrt(rm.low), HIGH lane passes through from
    // vvvv (xmm1), not from the old destination.
    let mut x1 = xmm_from_f64(99.0);
    x1[8..16].copy_from_slice(&77.0f64.to_bits().to_le_bytes());
    emu.reg_write_xmm(X86Reg::Xmm0, [0x55; 16]);
    emu.reg_write_xmm(X86Reg::Xmm1, x1);
    emu.reg_write_xmm(X86Reg::Xmm2, xmm_from_f64(2.0));
    run(&mut emu, programs, 4);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    assert_eq!(
        xmm_lane64(&got, 0),
        2.0f64.sqrt().to_bits(),
        "vsqrtsd low lane"
    );
    assert_eq!(
        xmm_lane64(&got, 1),
        77.0f64.to_bits(),
        "vsqrtsd high lane must come from vvvv"
    );

    // 5: vcmpsd predicate 0x0D (GE_OS): 4.0 >= 4.0 → all-ones mask. GE is
    // an AVX-only predicate (>7), unreachable via legacy CMPSD.
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f64(4.0));
    emu.reg_write_xmm(X86Reg::Xmm2, xmm_from_f64(4.0));
    run(&mut emu, programs, 5);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    assert_eq!(
        xmm_lane64(&got, 0),
        u64::MAX,
        "vcmpsd GE_OS(4,4) must set the mask"
    );

    // 6: vshufps imm=0x1B: r = [a[3], a[2], b[1], b[0]] with a=vvvv, b=rm.
    let sa = [10.0f32, 11.0, 12.0, 13.0, 0.0, 0.0, 0.0, 0.0];
    let sb = [20.0f32, 21.0, 22.0, 23.0, 0.0, 0.0, 0.0, 0.0];
    emu.reg_write_ymm(X86Reg::Ymm1, ymm_from_f32x8(sa));
    emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_f32x8(sb));
    run(&mut emu, programs, 6);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    let want = [13.0f32, 12.0, 21.0, 20.0];
    for i in 0..4 {
        assert_eq!(f32_lane(&got, i), want[i].to_bits(), "vshufps lane {i}");
    }

    // 7: vunpcklpd: r = [vvvv[0], rm[0]].
    let mut u1 = xmm_from_f64(1.25);
    u1[8..16].copy_from_slice(&2.25f64.to_bits().to_le_bytes());
    let mut u2 = xmm_from_f64(3.25);
    u2[8..16].copy_from_slice(&4.25f64.to_bits().to_le_bytes());
    emu.reg_write_xmm(X86Reg::Xmm1, u1);
    emu.reg_write_xmm(X86Reg::Xmm2, u2);
    run(&mut emu, programs, 7);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    assert_eq!(
        xmm_lane64(&got, 0),
        1.25f64.to_bits(),
        "vunpcklpd lane 0 from vvvv"
    );
    assert_eq!(
        xmm_lane64(&got, 1),
        3.25f64.to_bits(),
        "vunpcklpd lane 1 from rm"
    );

    // 8: legacy SSE2 minsd(+0.0, -0.0) — Bochs f64_min returns the SECOND
    // operand for equal operands, so the result is -0.0.
    emu.reg_write_xmm(X86Reg::Xmm0, xmm_from_f64(0.0));
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f64(-0.0));
    run(&mut emu, programs, 8);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    assert_eq!(
        xmm_lane64(&got, 0),
        (-0.0f64).to_bits(),
        "legacy minsd(+0,-0) must return -0.0 (Bochs f64_min)"
    );

    // 9: vaddsubpd: r = [a0-b0, a1+b1].
    let mut s1 = xmm_from_f64(10.0);
    s1[8..16].copy_from_slice(&20.0f64.to_bits().to_le_bytes());
    let mut s2 = xmm_from_f64(1.0);
    s2[8..16].copy_from_slice(&2.0f64.to_bits().to_le_bytes());
    emu.reg_write_xmm(X86Reg::Xmm1, s1);
    emu.reg_write_xmm(X86Reg::Xmm2, s2);
    run(&mut emu, programs, 9);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    assert_eq!(
        xmm_lane64(&got, 0),
        9.0f64.to_bits(),
        "vaddsubpd lane 0 (sub)"
    );
    assert_eq!(
        xmm_lane64(&got, 1),
        22.0f64.to_bits(),
        "vaddsubpd lane 1 (add)"
    );

    // 10: vmovsd reg form: low from rm (xmm2), HIGH from vvvv (xmm1),
    // upper 128 cleared.
    let mut m1 = xmm_from_f64(0.5);
    m1[8..16].copy_from_slice(&8.5f64.to_bits().to_le_bytes());
    emu.reg_write_ymm(X86Reg::Ymm0, [0xCC; 32]);
    emu.reg_write_xmm(X86Reg::Xmm1, m1);
    emu.reg_write_xmm(X86Reg::Xmm2, xmm_from_f64(6.25));
    run(&mut emu, programs, 10);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    assert_eq!(f64_lane(&got, 0), 6.25f64.to_bits(), "vmovsd low from rm");
    assert_eq!(f64_lane(&got, 1), 8.5f64.to_bits(), "vmovsd high from vvvv");
    assert_eq!(&got[16..32], &[0u8; 16], "vmovsd must zero ymm upper");

    // 11: vroundsd imm=0x09 (floor): floor(2.7) = 2.0, high from vvvv.
    let mut r1 = xmm_from_f64(0.0);
    r1[8..16].copy_from_slice(&3.5f64.to_bits().to_le_bytes());
    emu.reg_write_xmm(X86Reg::Xmm1, r1);
    emu.reg_write_xmm(X86Reg::Xmm2, xmm_from_f64(2.7));
    run(&mut emu, programs, 11);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    assert_eq!(xmm_lane64(&got, 0), 2.0f64.to_bits(), "vroundsd floor(2.7)");
    assert_eq!(
        xmm_lane64(&got, 1),
        3.5f64.to_bits(),
        "vroundsd high from vvvv"
    );

    // 12: vcvtss2sd: f32 1.5 → f64 1.5, high from vvvv.
    let mut c1 = xmm_from_f64(0.0);
    c1[8..16].copy_from_slice(&9.5f64.to_bits().to_le_bytes());
    let mut c2 = [0u8; 16];
    c2[..4].copy_from_slice(&1.5f32.to_bits().to_le_bytes());
    emu.reg_write_xmm(X86Reg::Xmm1, c1);
    emu.reg_write_xmm(X86Reg::Xmm2, c2);
    run(&mut emu, programs, 12);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    assert_eq!(xmm_lane64(&got, 0), 1.5f64.to_bits(), "vcvtss2sd low");
    assert_eq!(
        xmm_lane64(&got, 1),
        9.5f64.to_bits(),
        "vcvtss2sd high from vvvv"
    );
}

// ════════════════════════════════════════════════════════════════════════
// SSE3 horizontal add, SSE4.1 blend / variable blend / dot product, and
// their VEX forms (per-128-bit-lane, is4 mask register).
// ════════════════════════════════════════════════════════════════════════

fn xmm_from_f32x4(v: [f32; 4]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, x) in v.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_bits().to_le_bytes());
    }
    out
}

fn xmm_from_f64x2(v: [f64; 2]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, x) in v.iter().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&x.to_bits().to_le_bytes());
    }
    out
}

fn xmm_lane32(xmm: &[u8; 16], i: usize) -> u32 {
    u32::from_le_bytes(xmm[i * 4..i * 4 + 4].try_into().unwrap())
}

#[test]
fn sse3_sse41_hadd_blend_dpp_families() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(run_hadd_blend_dpp_cases)
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}

fn run_hadd_blend_dpp_cases() {
    let cfg = EmulatorConfig::default();
    let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
        .expect("new emulator in flat long mode");
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );

    let programs: &[(&str, &[u8])] = &[
        ("haddps xmm0,xmm1 (SSE3)", &[0xF2, 0x0F, 0x7C, 0xC1]),
        ("vhaddps ymm0,ymm1,ymm2", &[0xC5, 0xF7, 0x7C, 0xC2]),
        (
            "blendps xmm0,xmm1,0x0A (SSE4.1)",
            &[0x66, 0x0F, 0x3A, 0x0C, 0xC1, 0x0A],
        ),
        (
            "blendvps xmm2,xmm1 (implicit xmm0 mask)",
            &[0x66, 0x0F, 0x38, 0x14, 0xD1],
        ),
        (
            "vblendvps xmm0,xmm1,xmm2,xmm3 (is4)",
            &[0xC4, 0xE3, 0x71, 0x4A, 0xC2, 0x30],
        ),
        (
            "dpps xmm0,xmm1,0x71 (SSE4.1)",
            &[0x66, 0x0F, 0x3A, 0x40, 0xC1, 0x71],
        ),
        (
            "vdppd xmm0,xmm1,xmm2,0x31",
            &[0xC4, 0xE3, 0x71, 0x41, 0xC2, 0x31],
        ),
        (
            "vdpps ymm0,ymm1,ymm2,0xF1",
            &[0xC4, 0xE3, 0x75, 0x40, 0xC2, 0xF1],
        ),
        (
            "vblendps ymm0,ymm1,ymm2,0xA5",
            &[0xC4, 0xE3, 0x75, 0x0C, 0xC2, 0xA5],
        ),
    ];
    for (i, (_, code)) in programs.iter().enumerate() {
        let addr = CASE_BASE + i as u64 * CASE_STRIDE;
        emu.mem_write(addr, code).expect("write case code");
        emu.mem_write(addr + code.len() as u64, &[0xEB, 0xFE])
            .expect("write park jump");
    }
    fn run(
        emu: &mut Emulator<'static, Corei7SkylakeX>,
        programs: &[(&str, &[u8])],
        idx: usize,
    ) {
        let (name, code) = programs[idx];
        let addr = CASE_BASE + idx as u64 * CASE_STRIDE;
        let park = addr + code.len() as u64;
        let stop = emu
            .emu_start(addr, Some(park), None, Some(9))
            .expect("emu_start");
        assert_eq!(emu.cpu.rip(), park, "{name}: did not park (stop={stop:?})");
    }

    // 0: haddps xmm0,xmm1 (legacy):
    //    result = [a0+a1, a2+a3, b0+b1, b2+b3], upper preserved untouched.
    let a = [1.5f32, 2.25, -4.0, 10.0];
    let b = [100.0f32, -1.0, 0.5, 0.25];
    emu.reg_write_xmm(X86Reg::Xmm0, xmm_from_f32x4(a));
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f32x4(b));
    run(&mut emu, programs, 0);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    let want = [a[0] + a[1], a[2] + a[3], b[0] + b[1], b[2] + b[3]];
    for i in 0..4 {
        assert_eq!(
            xmm_lane32(&got, i),
            want[i].to_bits(),
            "haddps lane {i}"
        );
    }

    // 1: vhaddps ymm — horizontal add PER 128-BIT LANE:
    //    dst = [v0+v1, v2+v3, w0+w1, w2+w3 | v4+v5, v6+v7, w4+w5, w6+w7].
    let va = [1.0f32, 2.0, 3.0, 4.0, 10.0, 20.0, 30.0, 40.0];
    let wa = [0.5f32, 0.25, -1.0, 8.0, -2.0, 5.0, 7.0, 9.0];
    emu.reg_write_ymm(X86Reg::Ymm1, ymm_from_f32x8(va));
    emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_f32x8(wa));
    run(&mut emu, programs, 1);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    let want = [
        va[0] + va[1],
        va[2] + va[3],
        wa[0] + wa[1],
        wa[2] + wa[3],
        va[4] + va[5],
        va[6] + va[7],
        wa[4] + wa[5],
        wa[6] + wa[7],
    ];
    for i in 0..8 {
        assert_eq!(f32_lane(&got, i), want[i].to_bits(), "vhaddps lane {i}");
    }

    // 2: blendps xmm0,xmm1,0x0A — lanes 1 and 3 from xmm1, 0 and 2 kept.
    let d = [1.0f32, 2.0, 3.0, 4.0];
    let s = [-1.0f32, -2.0, -3.0, -4.0];
    emu.reg_write_xmm(X86Reg::Xmm0, xmm_from_f32x4(d));
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f32x4(s));
    run(&mut emu, programs, 2);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    let want = [d[0], s[1], d[2], s[3]];
    for i in 0..4 {
        assert_eq!(xmm_lane32(&got, i), want[i].to_bits(), "blendps lane {i}");
    }

    // 3: blendvps xmm2,xmm1 — implicit XMM0 sign-bit mask: lanes whose
    //    xmm0 element is negative (sign bit set) come from xmm1.
    let mask = [-1.0f32, 2.0, -0.0, 7.0]; // sign bits: 1,0,1,0
    let dst = [10.0f32, 11.0, 12.0, 13.0];
    let src = [20.0f32, 21.0, 22.0, 23.0];
    emu.reg_write_xmm(X86Reg::Xmm0, xmm_from_f32x4(mask));
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f32x4(src));
    emu.reg_write_xmm(X86Reg::Xmm2, xmm_from_f32x4(dst));
    run(&mut emu, programs, 3);
    let got = emu.reg_read_xmm(X86Reg::Xmm2);
    let want = [src[0], dst[1], src[2], dst[3]];
    for i in 0..4 {
        assert_eq!(xmm_lane32(&got, i), want[i].to_bits(), "blendvps lane {i}");
    }

    // 4: vblendvps xmm0,xmm1,xmm2,xmm3 — the mask is the is4 register
    //    (imm8[7:4] = 3), NOT xmm0. xmm0 is preloaded with garbage that
    //    must be fully replaced; upper ymm bits must be zeroed.
    let vmask = [1.0f32, -1.0, 5.0, -5.0]; // sign bits: 0,1,0,1
    let v1 = [10.0f32, 11.0, 12.0, 13.0]; // vvvv (first source)
    let v2 = [20.0f32, 21.0, 22.0, 23.0]; // rm (second source)
    emu.reg_write_ymm(X86Reg::Ymm0, [0xAA; 32]);
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f32x4(v1));
    emu.reg_write_xmm(X86Reg::Xmm2, xmm_from_f32x4(v2));
    emu.reg_write_xmm(X86Reg::Xmm3, xmm_from_f32x4(vmask));
    run(&mut emu, programs, 4);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    let want = [v1[0], v2[1], v1[2], v2[3]];
    for i in 0..4 {
        assert_eq!(f32_lane(&got, i), want[i].to_bits(), "vblendvps lane {i}");
    }
    assert_eq!(
        &got[16..32],
        &[0u8; 16],
        "vblendvps must zero ymm bits 255:128"
    );

    // 5: dpps xmm0,xmm1,0x71 — multiply lanes 0-2 (imm[6:4]=0x7), lane 3
    //    excluded, sum to lane 0 only (imm[3:0]=0x1). Lane-3 products
    //    (100*100) must not leak into the dot product.
    let p1 = [1.5f32, 2.0, 3.0, 100.0];
    let p2 = [2.0f32, 4.0, 0.5, 100.0];
    emu.reg_write_xmm(X86Reg::Xmm0, xmm_from_f32x4(p1));
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f32x4(p2));
    run(&mut emu, programs, 5);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    // 1.5*2 + 2*4 + 3*0.5 = 3 + 8 + 1.5 = 12.5 (all exact in f32)
    let want = [12.5f32, 0.0, 0.0, 0.0];
    for i in 0..4 {
        assert_eq!(xmm_lane32(&got, i), want[i].to_bits(), "dpps lane {i}");
    }

    // 6: vdppd xmm0,xmm1,xmm2,0x31 — both products (imm[5:4]=0x3), sum to
    //    lane 0 only (imm[1:0]=0x1); upper ymm bits zeroed (VEX.128).
    let q1 = [1.5f64, 2.5];
    let q2 = [4.0f64, 8.0];
    emu.reg_write_ymm(X86Reg::Ymm0, [0xCC; 32]);
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f64x2(q1));
    emu.reg_write_xmm(X86Reg::Xmm2, xmm_from_f64x2(q2));
    run(&mut emu, programs, 6);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    // 1.5*4 + 2.5*8 = 6 + 20 = 26 (exact)
    assert_eq!(f64_lane(&got, 0), 26.0f64.to_bits(), "vdppd dot product");
    assert_eq!(f64_lane(&got, 1), 0.0f64.to_bits(), "vdppd masked lane 1");
    assert_eq!(&got[16..32], &[0u8; 16], "vdppd must zero ymm bits 255:128");

    // 7: vdpps ymm0,ymm1,ymm2,0xF1 — full dot product PER 128-BIT LANE
    //    (same imm8 for both lanes): result element 0 = lane-0 dot,
    //    element 4 = lane-1 dot, all other elements 0.
    let da = [1.0f32, 2.0, 3.0, 4.0, 0.5, 0.25, 8.0, 16.0];
    let db = [2.0f32, 2.0, 2.0, 2.0, 4.0, 8.0, 0.5, 0.25];
    emu.reg_write_ymm(X86Reg::Ymm1, ymm_from_f32x8(da));
    emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_f32x8(db));
    run(&mut emu, programs, 7);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    // lane 0: 2+4+6+8 = 20; lane 1: 2+2+4+4 = 12 (all exact)
    let want = [20.0f32, 0.0, 0.0, 0.0, 12.0, 0.0, 0.0, 0.0];
    for i in 0..8 {
        assert_eq!(f32_lane(&got, i), want[i].to_bits(), "vdpps lane {i}");
    }

    // 8: vblendps ymm0,ymm1,ymm2,0xA5 — imm8 consumed 4 bits per 128-bit
    //    lane: low lane mask 0x5 (elements 0,2), high lane mask 0xA
    //    (elements 5,7).
    let ba = [10.0f32, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0];
    let bb = [20.0f32, 21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0];
    emu.reg_write_ymm(X86Reg::Ymm1, ymm_from_f32x8(ba));
    emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_f32x8(bb));
    run(&mut emu, programs, 8);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    let want = [bb[0], ba[1], bb[2], ba[3], ba[4], bb[5], ba[6], bb[7]];
    for i in 0..8 {
        assert_eq!(f32_lane(&got, i), want[i].to_bits(), "vblendps lane {i}");
    }
}
