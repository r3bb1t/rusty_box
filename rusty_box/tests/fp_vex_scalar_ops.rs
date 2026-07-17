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
        emu.cpu().rip(),
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
                let end_rip = emu.cpu().rip();
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
// Go-runtime-critical integer ops: AESENC (map hashing) and 256-bit
// VPCMPEQB/VPMOVMSKB (bytealg.IndexByte). A deterministic bug here breaks
// Go map lookups / string searches — snapd's assertion-parser panic.
// ════════════════════════════════════════════════════════════════════════

fn xmm_from_u128(v: u128) -> [u8; 16] {
    v.to_le_bytes()
}

#[test]
fn go_runtime_integer_ops_aes_and_bytemask() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(run_go_runtime_cases)
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}

fn run_go_runtime_cases() {
    let cfg = EmulatorConfig::default();
    let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
        .expect("new emulator in flat long mode");
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );

    let programs: &[(&str, &[u8], u64)] = &[
        // AESENC xmm0, xmm1 (66 0F 38 DC /r)
        ("aesenc xmm0,xmm1", &[0x66, 0x0F, 0x38, 0xDC, 0xC1], 1),
        // AESENCLAST xmm0, xmm1 (66 0F 38 DD /r)
        ("aesenclast xmm0,xmm1", &[0x66, 0x0F, 0x38, 0xDD, 0xC1], 1),
        // VPMOVMSKB eax, ymm1 (VEX.256.66.0F D7 /r)
        ("vpmovmskb eax,ymm1", &[0xC5, 0xFD, 0xD7, 0xC1], 1),
        // VPCMPEQB ymm0,ymm1,ymm2 then VPMOVMSKB eax,ymm0
        (
            "vpcmpeqb ymm + vpmovmskb",
            &[0xC5, 0xF5, 0x74, 0xC2, 0xC5, 0xFD, 0xD7, 0xC0],
            2,
        ),
    ];
    for (i, (_, code, _)) in programs.iter().enumerate() {
        let addr = CASE_BASE + i as u64 * CASE_STRIDE;
        emu.mem_write(addr, code).expect("write case code");
        emu.mem_write(addr + code.len() as u64, &[0xEB, 0xFE])
            .expect("write park jump");
    }
    fn run(
        emu: &mut Emulator<'static, Corei7SkylakeX>,
        programs: &[(&str, &[u8], u64)],
        idx: usize,
    ) {
        let (name, code, insns) = programs[idx];
        let addr = CASE_BASE + idx as u64 * CASE_STRIDE;
        let park = addr + code.len() as u64;
        let stop = emu
            .emu_start(addr, Some(park), None, Some(insns + 8))
            .expect("emu_start");
        assert_eq!(emu.cpu().rip(), park, "{name}: did not park (stop={stop:?})");
    }

    // Intel AES-NI whitepaper reference vectors (also used by Bochs/QEMU):
    // state  = 0x7b5b54657374566563746f725d53475d
    // rndkey = 0x48692853686179295b477565726f6e5d
    let state = 0x7b5b54657374566563746f725d53475d_u128;
    let key = 0x48692853686179295b477565726f6e5d_u128;

    // 0: AESENC → 0xa8311c2f9fdba3c58b104b58ded7e595
    emu.reg_write_xmm(X86Reg::Xmm0, xmm_from_u128(state));
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_u128(key));
    run(&mut emu, programs, 0);
    let got = u128::from_le_bytes(emu.reg_read_xmm(X86Reg::Xmm0));
    assert_eq!(
        got, 0xa8311c2f9fdba3c58b104b58ded7e595_u128,
        "AESENC mismatch: got {got:#034x}"
    );

    // 1: AESENCLAST → 0xc7fb881e938c5964177ec42553fdc611
    emu.reg_write_xmm(X86Reg::Xmm0, xmm_from_u128(state));
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_u128(key));
    run(&mut emu, programs, 1);
    let got = u128::from_le_bytes(emu.reg_read_xmm(X86Reg::Xmm0));
    assert_eq!(
        got, 0xc7fb881e938c5964177ec42553fdc611_u128,
        "AESENCLAST mismatch: got {got:#034x}"
    );

    // 2: vpmovmskb on ymm with sign bits at byte lanes 0, 15, 16, 31 →
    // 0x8001_8001, upper 32 bits of rax cleared.
    let mut m = [0u8; 32];
    m[0] = 0x80;
    m[15] = 0xFF;
    m[16] = 0x80;
    m[31] = 0xC1;
    emu.reg_write(X86Reg::Rax, 0xDEAD_BEEF_DEAD_BEEF);
    emu.reg_write_ymm(X86Reg::Ymm1, m);
    run(&mut emu, programs, 2);
    assert_eq!(
        emu.reg_read(X86Reg::Rax),
        0x8001_8001,
        "vpmovmskb ymm mask (upper lanes 16..32 must be included)"
    );

    // 3: vpcmpeqb ymm: equal only at byte 17 → mask 1<<17.
    let mut a = [0u8; 32];
    let mut b = [0xFFu8; 32];
    a[17] = 0x3A; // ':' — the header separator IndexByte hunts for
    b[17] = 0x3A;
    emu.reg_write_ymm(X86Reg::Ymm1, a);
    emu.reg_write_ymm(X86Reg::Ymm2, b);
    emu.reg_write(X86Reg::Rax, 0);
    run(&mut emu, programs, 3);
    assert_eq!(
        emu.reg_read(X86Reg::Rax),
        1u64 << 17,
        "vpcmpeqb ymm lane 17 (crosses the 16-byte boundary)"
    );
}

// ════════════════════════════════════════════════════════════════════════
// Unaligned vector loads straddling page boundaries — Go's aeshash and
// memequal issue MOVDQU/VMOVDQU at arbitrary string addresses, so the
// same bytes are read at different alignments. A split-access bug makes
// hash(key@A) != hash(key@B) and breaks Go map lookups deterministically.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn unaligned_vector_loads_across_page_boundaries() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(run_split_load_cases)
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}

fn run_split_load_cases() {
    let cfg = EmulatorConfig::default();
    let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
        .expect("new emulator in flat long mode");
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );

    let programs: &[(&str, &[u8], u64)] = &[
        // movdqu xmm0, [rax] (F3 0F 6F /r)
        ("movdqu xmm0,[rax]", &[0xF3, 0x0F, 0x6F, 0x00], 1),
        // vmovdqu ymm0, [rax] (VEX.256.F3.0F 6F /r)
        ("vmovdqu ymm0,[rax]", &[0xC5, 0xFE, 0x6F, 0x00], 1),
        // movdqu xmm0, [rax+rcx*1-16] — Go aeshash17to32 tail load
        // (modrm 44 → SIB, SIB 08 → base=rax index=rcx, disp8 -16)
        (
            "movdqu xmm0,[rax+rcx-16]",
            &[0xF3, 0x0F, 0x6F, 0x44, 0x08, 0xF0],
            1,
        ),
        // aesenc xmm0, xmm1 (distinct registers holding equal values)
        ("aesenc xmm0,xmm1 equal", &[0x66, 0x0F, 0x38, 0xDC, 0xC1], 1),
        // aesenc xmm2, xmm2 (self-aliased, Go's AESENC X2, X2)
        ("aesenc xmm2,xmm2 self", &[0x66, 0x0F, 0x38, 0xDC, 0xD2], 1),
    ];
    for (i, (_, code, _)) in programs.iter().enumerate() {
        let addr = CASE_BASE + i as u64 * CASE_STRIDE;
        emu.mem_write(addr, code).expect("write case code");
        emu.mem_write(addr + code.len() as u64, &[0xEB, 0xFE])
            .expect("write park jump");
    }
    fn run(
        emu: &mut Emulator<'static, Corei7SkylakeX>,
        programs: &[(&str, &[u8], u64)],
        idx: usize,
    ) {
        let (name, code, insns) = programs[idx];
        let addr = CASE_BASE + idx as u64 * CASE_STRIDE;
        let park = addr + code.len() as u64;
        let stop = emu
            .emu_start(addr, Some(park), None, Some(insns + 8))
            .expect("emu_start");
        assert_eq!(emu.cpu().rip(), park, "{name}: did not park (stop={stop:?})");
    }

    // Recognizable 48-byte pattern.
    let pattern: Vec<u8> = (0u8..48).map(|i| i.wrapping_mul(7) ^ 0x5A).collect();

    // Boundaries to straddle: a 4 KiB page edge and the 2 MiB huge-page
    // edge the FlatLong64 tables use. Data addresses far from CASE_BASE.
    for &(name, base) in &[("4K", 0x0060_0FF8u64), ("2M", 0x009F_FFF8u64)] {
        emu.mem_write(base, &pattern).expect("write pattern");

        // movdqu: 16 bytes starting 8 bytes before the boundary.
        emu.reg_write(X86Reg::Rax, base);
        emu.reg_write_xmm(X86Reg::Xmm0, [0u8; 16]);
        run(&mut emu, programs, 0);
        let got = emu.reg_read_xmm(X86Reg::Xmm0);
        assert_eq!(
            &got[..],
            &pattern[..16],
            "movdqu split across {name} boundary at {base:#x}"
        );

        // vmovdqu ymm: 32 bytes straddling the same boundary.
        emu.reg_write(X86Reg::Rax, base);
        emu.reg_write_ymm(X86Reg::Ymm0, [0u8; 32]);
        run(&mut emu, programs, 1);
        let got = emu.reg_read_ymm(X86Reg::Ymm0);
        assert_eq!(
            &got[..],
            &pattern[..32],
            "vmovdqu ymm split across {name} boundary at {base:#x}"
        );
    }

    // Go aeshash17to32 tail load: movdqu xmm0,[rax+rcx-16] with rcx=17
    // reads bytes 1..17 of the buffer (the overlapping tail of a 17-byte
    // key like "sign-key-sha3-384").
    let base = 0x0070_0000u64;
    emu.mem_write(base, &pattern).expect("write pattern");
    emu.reg_write(X86Reg::Rax, base);
    emu.reg_write(X86Reg::Rcx, 17);
    emu.reg_write_xmm(X86Reg::Xmm0, [0u8; 16]);
    run(&mut emu, programs, 2);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    assert_eq!(
        &got[..],
        &pattern[1..17],
        "movdqu SIB + disp8(-16) addressing (aeshash tail load)"
    );

    // AESENC with dst==src must equal AESENC with two registers holding
    // the same value (Go emits AESENC X2, X2).
    let s = 0x0011_2233_4455_6677_8899_AABB_CCDD_EEFF_u128;
    emu.reg_write_xmm(X86Reg::Xmm0, s.to_le_bytes());
    emu.reg_write_xmm(X86Reg::Xmm1, s.to_le_bytes());
    run(&mut emu, programs, 3);
    let two_reg = emu.reg_read_xmm(X86Reg::Xmm0);
    emu.reg_write_xmm(X86Reg::Xmm2, s.to_le_bytes());
    run(&mut emu, programs, 4);
    let self_reg = emu.reg_read_xmm(X86Reg::Xmm2);
    assert_eq!(
        two_reg, self_reg,
        "AESENC self-aliased (dst==src) must match the two-register result"
    );
}

// ════════════════════════════════════════════════════════════════════════
// bytealg.IndexByte AVX2 ingredients not covered above: VPBROADCASTB and
// TZCNT. A wrong splat lane or a BSF-like TZCNT breaks newline splitting.
// ════════════════════════════════════════════════════════════════════════

// ════════════════════════════════════════════════════════════════════════
// VPINSRW is 3-operand under VEX: the base vector comes from vvvv (not the
// destination as legacy SSE PINSRW does), and the upper 128 bits of the YMM
// destination are cleared. Windows setup emits VEX integer-insert; decoding it
// as the 2-operand SSE form silently corrupts data.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn vex_vpinsrw_sources_vvvv_and_clears_upper() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");
            emu.reg_write(
                X86Reg::Cr4,
                emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
            );
            // XSETBV ECX=0, EDX:EAX=7 enables guest XCR0 FPU+SSE+YMM state.
            emu.reg_write(X86Reg::Rax, 0x7);
            emu.reg_write(X86Reg::Rcx, 0);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1])
                .expect("write xsetbv");
            emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                .expect("enable guest AVX state");

            // vpinsrw xmm0, xmm1, ecx, 3 (VEX.128.66.0F.W0 C4 /r ib, 2-byte VEX
            // C5 F1: vvvv=~1). modrm C1 → reg=xmm0, rm=ecx. imm=3. Then park.
            emu.mem_write(CASE_BASE, &[0xC5, 0xF1, 0xC4, 0xC1, 0x03, 0xEB, 0xFE])
                .expect("write code");

            // Destination xmm0 gets a poison value that must be overwritten, and
            // ymm0's upper lane is poisoned to prove it gets cleared.
            emu.reg_write_ymm(X86Reg::Ymm0, [0xAA; 32]);
            // vvvv source (xmm1) = bytes 00..0F → the real base vector.
            let base: [u8; 16] = [
                0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D,
                0x0E, 0x0F,
            ];
            emu.reg_write_xmm(X86Reg::Xmm1, base);
            emu.reg_write(X86Reg::Rcx, 0x1234);

            let stop = emu
                .emu_start(CASE_BASE, Some(CASE_BASE + 5), None, Some(9))
                .expect("emu_start");
            assert_eq!(
                emu.cpu().rip(),
                CASE_BASE + 5,
                "vpinsrw did not park (stop={stop:?})"
            );

            // Expected: copy of xmm1 with word[3] (bytes 6..8) replaced by 0x1234.
            let mut expect = base;
            expect[6] = 0x34;
            expect[7] = 0x12;
            let got = emu.reg_read_ymm(X86Reg::Ymm0);
            assert_eq!(
                &got[..16],
                &expect[..],
                "VPINSRW must build from vvvv (xmm1), not the destination"
            );
            assert_eq!(
                &got[16..],
                &[0u8; 16],
                "VEX-128 VPINSRW must clear ymm0 bits [255:128]"
            );
        })
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}

#[test]
fn indexbyte_avx2_ingredients() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");
            emu.reg_write(
                X86Reg::Cr4,
                emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
            );

            // vpbroadcastb ymm3, xmm1 (VEX.256.66.0F38.W0 78 /r); park jump.
            emu.mem_write(CASE_BASE, &[0xC4, 0xE2, 0x7D, 0x78, 0xD9, 0xEB, 0xFE])
                .expect("write code");
            let mut x1 = [0u8; 16];
            x1[0] = 0x0A; // '\n'
            x1[1] = 0x77; // garbage that must NOT leak into the splat
            emu.reg_write_xmm(X86Reg::Xmm1, x1);
            emu.reg_write_ymm(X86Reg::Ymm3, [0xEE; 32]);
            let stop = emu
                .emu_start(CASE_BASE, Some(CASE_BASE + 5), None, Some(9))
                .expect("emu_start");
            assert_eq!(
                emu.cpu().rip(),
                CASE_BASE + 5,
                "vpbroadcastb did not park (stop={stop:?})"
            );
            assert_eq!(
                emu.reg_read_ymm(X86Reg::Ymm3),
                [0x0Au8; 32],
                "vpbroadcastb must splat the low byte into all 32 lanes"
            );

            // tzcnt eax, ebx (F3 0F BC /r) — must be 32 for input 0 (not BSF
            // undefined) and count correctly otherwise.
            let addr2 = CASE_BASE + 64;
            emu.mem_write(addr2, &[0xF3, 0x0F, 0xBC, 0xC3, 0xEB, 0xFE])
                .expect("write code");
            for (input, want) in [(0x0002_0000u64, 17u64), (0u64, 32u64), (1u64, 0u64)] {
                emu.reg_write(X86Reg::Rbx, input);
                emu.reg_write(X86Reg::Rax, 0xFFFF_FFFF_FFFF_FFFF);
                let stop = emu
                    .emu_start(addr2, Some(addr2 + 4), None, Some(9))
                    .expect("emu_start");
                assert_eq!(emu.cpu().rip(), addr2 + 4, "tzcnt park (stop={stop:?})");
                assert_eq!(
                    emu.reg_read(X86Reg::Rax),
                    want,
                    "tzcnt eax, ebx with input {input:#x}"
                );
            }
        })
        .expect("spawn")
        .join()
        .expect("join");
}

// ════════════════════════════════════════════════════════════════════════
// Go runtime aeshash17to32 (runtime/asm_amd64.s aeshashbody), verbatim
// instruction sequence. Hashing the same 17-byte key at two different
// addresses must produce identical results — Go maps hash a key at its
// heap address on insert and at the constant's address on lookup. This is
// the exact path only ≥17-byte keys like "sign-key-sha3-384" take.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn go_aeshash17to32_is_address_independent() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(run_aeshash_cases)
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}

fn run_aeshash_cases() {
    let cfg = EmulatorConfig::default();
    let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
        .expect("new emulator in flat long mode");
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );

    // Go aeshash17to32 core (seeds preloaded in xmm0/xmm1; ptr=rax len=rcx):
    //   movdqu xmm2, [rax]
    //   movdqu xmm3, [rax+rcx*1-16]
    //   pxor   xmm2, xmm0
    //   pxor   xmm3, xmm1
    //   aesenc xmm2, xmm2   (x3, interleaved with xmm3)
    //   pxor   xmm2, xmm3
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xF3, 0x0F, 0x6F, 0x10,                   // movdqu xmm2, [rax]
        0xF3, 0x0F, 0x6F, 0x5C, 0x08, 0xF0,       // movdqu xmm3, [rax+rcx-16]
        0x66, 0x0F, 0xEF, 0xD0,                   // pxor xmm2, xmm0
        0x66, 0x0F, 0xEF, 0xD9,                   // pxor xmm3, xmm1
        0x66, 0x0F, 0x38, 0xDC, 0xD2,             // aesenc xmm2, xmm2
        0x66, 0x0F, 0x38, 0xDC, 0xDB,             // aesenc xmm3, xmm3
        0x66, 0x0F, 0x38, 0xDC, 0xD2,             // aesenc xmm2, xmm2
        0x66, 0x0F, 0x38, 0xDC, 0xDB,             // aesenc xmm3, xmm3
        0x66, 0x0F, 0x38, 0xDC, 0xD2,             // aesenc xmm2, xmm2
        0x66, 0x0F, 0x38, 0xDC, 0xDB,             // aesenc xmm3, xmm3
        0x66, 0x0F, 0xEF, 0xD3,                   // pxor xmm2, xmm3
    ];
    let insns = 11u64;
    emu.mem_write(CASE_BASE, code).expect("write code");
    emu.mem_write(CASE_BASE + code.len() as u64, &[0xEB, 0xFE])
        .expect("write park");
    let park = CASE_BASE + code.len() as u64;

    let key = b"sign-key-sha3-384"; // 17 bytes — the only >16-byte header
    let seed0 = 0x243F_6A88_85A3_08D3_1319_8A2E_0370_7344_u128; // arbitrary
    let seed1 = 0xA409_3822_299F_31D0_082E_FA98_EC4E_6C89_u128;

    let mut hash_at = |emu: &mut Emulator<'static, Corei7SkylakeX>, addr: u64| -> [u8; 16] {
        emu.mem_write(addr, key).expect("write key");
        emu.reg_write(X86Reg::Rax, addr);
        emu.reg_write(X86Reg::Rcx, key.len() as u64);
        emu.reg_write_xmm(X86Reg::Xmm0, seed0.to_le_bytes());
        emu.reg_write_xmm(X86Reg::Xmm1, seed1.to_le_bytes());
        emu.reg_write_xmm(X86Reg::Xmm2, [0u8; 16]);
        emu.reg_write_xmm(X86Reg::Xmm3, [0u8; 16]);
        let stop = emu
            .emu_start(CASE_BASE, Some(park), None, Some(insns + 8))
            .expect("emu_start");
        assert_eq!(emu.cpu().rip(), park, "hash run did not park (stop={stop:?})");
        emu.reg_read_xmm(X86Reg::Xmm2)
    };

    // The constant's address analogue: aligned, mid-page.
    let h_aligned = hash_at(&mut emu, 0x0060_0100);
    // Heap-copy analogues: odd offsets, page-crossing, hugepage-crossing.
    for &addr in &[
        0x0060_0207u64, // odd offset
        0x0060_0FF9,    // tail load crosses a 4 KiB boundary
        0x009F_FFF9,    // tail load crosses the 2 MiB hugepage boundary
        0x0061_0FFF,    // head load crosses, tail load crosses
    ] {
        let h = hash_at(&mut emu, addr);
        assert_eq!(
            h, h_aligned,
            "aeshash17to32(key @ {addr:#x}) differs from aligned hash — Go map lookups break"
        );
    }
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
        (
            "vcmpsd xmm0,xmm1,xmm2,0x0D",
            &[0xC5, 0xF3, 0xC2, 0xC2, 0x0D],
        ),
        (
            "vshufps xmm0,xmm1,xmm2,0x1B",
            &[0xC5, 0xF0, 0xC6, 0xC2, 0x1B],
        ),
        ("vunpcklpd xmm0,xmm1,xmm2", &[0xC5, 0xF1, 0x14, 0xC2]),
        (
            "minsd xmm0,xmm1 (SSE2 signed zero)",
            &[0xF2, 0x0F, 0x5D, 0xC1],
        ),
        ("vaddsubpd xmm0,xmm1,xmm2", &[0xC5, 0xF1, 0xD0, 0xC2]),
        (
            "vmovsd xmm0,xmm1,xmm2 (reg merge)",
            &[0xC5, 0xF3, 0x10, 0xC2],
        ),
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
    fn run(emu: &mut Emulator<'static, Corei7SkylakeX>, programs: &[(&str, &[u8])], idx: usize) {
        let (name, code) = programs[idx];
        let addr = CASE_BASE + idx as u64 * CASE_STRIDE;
        let park = addr + code.len() as u64;
        let stop = emu
            .emu_start(addr, Some(park), None, Some(9))
            .expect("emu_start");
        assert_eq!(emu.cpu().rip(), park, "{name}: did not park (stop={stop:?})");
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
    assert_eq!(
        &got[16..32],
        &[0u8; 16],
        "vmulps must zero ymm bits 255:128"
    );

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
    fn run(emu: &mut Emulator<'static, Corei7SkylakeX>, programs: &[(&str, &[u8])], idx: usize) {
        let (name, code) = programs[idx];
        let addr = CASE_BASE + idx as u64 * CASE_STRIDE;
        let park = addr + code.len() as u64;
        let stop = emu
            .emu_start(addr, Some(park), None, Some(9))
            .expect("emu_start");
        assert_eq!(emu.cpu().rip(), park, "{name}: did not park (stop={stop:?})");
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
        assert_eq!(xmm_lane32(&got, i), want[i].to_bits(), "haddps lane {i}");
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

// ════════════════════════════════════════════════════════════════════════
// VEX forms that must be VL-aware or 3-operand (previously fell through to
// the 128-bit legacy handlers): VPTEST, VMOVMSKPS, VPMOVSX/ZX, VPABSB,
// VINSERTPS, VMPSADBW, VPHMINPOSUW, VPBLENDVB, VMOVQ, VLDDQU, VAESENC.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn vex_vl_aware_and_three_operand_integer_ops() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(run_vex_integer_cases)
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}

fn run_vex_integer_cases() {
    let cfg = EmulatorConfig::default();
    let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
        .expect("new emulator in flat long mode");
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );

    let programs: &[(&str, &[u8])] = &[
        // vptest ymm0, ymm1; pushfq; pop rax
        (
            "vptest ymm0,ymm1 + flags",
            &[0xC4, 0xE2, 0x7D, 0x17, 0xC1, 0x9C, 0x58],
        ),
        ("vmovmskps eax,ymm1", &[0xC5, 0xFC, 0x50, 0xC1]),
        ("vpmovsxbw ymm0,xmm1", &[0xC4, 0xE2, 0x7D, 0x20, 0xC1]),
        ("vpmovzxbd ymm0,xmm1", &[0xC4, 0xE2, 0x7D, 0x31, 0xC1]),
        ("vpabsb ymm0,ymm1", &[0xC4, 0xE2, 0x7D, 0x1C, 0xC1]),
        (
            "vinsertps xmm0,xmm1,xmm2,0x61",
            &[0xC4, 0xE3, 0x71, 0x21, 0xC2, 0x61],
        ),
        ("vaesenc xmm0,xmm1,xmm2", &[0xC4, 0xE2, 0x71, 0xDC, 0xC2]),
        (
            "vpblendvb xmm0,xmm1,xmm2,xmm3",
            &[0xC4, 0xE3, 0x71, 0x4C, 0xC2, 0x30],
        ),
        (
            "vpblendvb ymm0,ymm1,ymm2,ymm3",
            &[0xC4, 0xE3, 0x75, 0x4C, 0xC2, 0x30],
        ),
        ("vphminposuw xmm0,xmm1", &[0xC4, 0xE2, 0x79, 0x41, 0xC1]),
        ("phminposuw xmm0,xmm1", &[0x66, 0x0F, 0x38, 0x41, 0xC1]),
        (
            "vmpsadbw ymm0,ymm1,ymm2,0x05",
            &[0xC4, 0xE3, 0x75, 0x42, 0xC2, 0x05],
        ),
        ("vmovq xmm0,xmm1", &[0xC5, 0xFA, 0x7E, 0xC1]),
        ("vlddqu ymm0,[rax]", &[0xC5, 0xFF, 0xF0, 0x00]),
        ("lddqu xmm0,[rax]", &[0xF2, 0x0F, 0xF0, 0x00]),
    ];
    for (i, (_, code)) in programs.iter().enumerate() {
        let addr = CASE_BASE + i as u64 * CASE_STRIDE;
        emu.mem_write(addr, code).expect("write case code");
        emu.mem_write(addr + code.len() as u64, &[0xEB, 0xFE])
            .expect("write park jump");
    }
    fn run(emu: &mut Emulator<'static, Corei7SkylakeX>, programs: &[(&str, &[u8])], idx: usize) {
        let (name, code) = programs[idx];
        let addr = CASE_BASE + idx as u64 * CASE_STRIDE;
        let park = addr + code.len() as u64;
        let stop = emu
            .emu_start(addr, Some(park), None, Some(12))
            .expect("emu_start");
        assert_eq!(emu.cpu().rip(), park, "{name}: did not park (stop={stop:?})");
    }

    // 0: vptest — the only overlapping bits sit in the UPPER 128-bit lane.
    //    ZF must be 0 (a VL-blind 128-bit PTEST would report ZF=1);
    //    CF must be 1 (rm AND NOT dst == 0 over all 256 bits).
    let mut d = [0u8; 32];
    d[24] = 0xFF; // qword 3 of ymm0
    let mut r = [0u8; 32];
    r[24] = 0xFF; // qword 3 of ymm1
    emu.reg_write_ymm(X86Reg::Ymm0, d);
    emu.reg_write_ymm(X86Reg::Ymm1, r);
    emu.reg_write(X86Reg::Rsp, STACK_TOP);
    run(&mut emu, programs, 0);
    let flags = emu.reg_read(X86Reg::Rax) & 0x41; // ZF|CF
    assert_eq!(
        flags, 0x01,
        "vptest ymm with upper-lane overlap: want CF=1 ZF=0, got flags {flags:#04x}"
    );

    // 1: vmovmskps ymm — sign bits in f32 lanes 1, 4, 7 → mask 0x92.
    //    Lanes 4-7 exist only in the 256-bit form.
    let mut m = [0u8; 32];
    m[4 + 3] = 0x80;
    m[4 * 4 + 3] = 0x80;
    m[4 * 7 + 3] = 0x80;
    emu.reg_write_ymm(X86Reg::Ymm1, m);
    emu.reg_write(X86Reg::Rax, 0xDEAD_BEEF_DEAD_BEEF);
    run(&mut emu, programs, 1);
    assert_eq!(
        emu.reg_read(X86Reg::Rax),
        0x92,
        "vmovmskps ymm must include lanes 4-7 and zero-extend"
    );

    // 2: vpmovsxbw ymm — all 16 source bytes sign-extend to 16 words.
    let src_bytes: [u8; 16] = [
        0x80, 0x7F, 0xFF, 0x01, 0xFE, 0x00, 0x05, 0xFB, 0x90, 0x10, 0xC0, 0x40, 0xAA, 0x55, 0x02,
        0xF0,
    ];
    emu.reg_write_xmm(X86Reg::Xmm1, src_bytes);
    emu.reg_write_ymm(X86Reg::Ymm0, [0x11; 32]);
    run(&mut emu, programs, 2);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    for (i, &b) in src_bytes.iter().enumerate() {
        let w = u16::from_le_bytes(got[i * 2..i * 2 + 2].try_into().unwrap());
        assert_eq!(
            w, b as i8 as i16 as u16,
            "vpmovsxbw ymm word {i} (byte {b:#04x})"
        );
    }

    // 3: vpmovzxbd ymm — 8 bytes zero-extend to 8 dwords.
    run(&mut emu, programs, 3);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    for i in 0..8 {
        assert_eq!(
            f32_lane(&got, i),
            src_bytes[i] as u32,
            "vpmovzxbd ymm dword {i}"
        );
    }

    // 4: vpabsb ymm — per-byte |x|, with |-128| staying 0x80, over 32 bytes.
    let mut abs_src = [0u8; 32];
    for (i, b) in abs_src.iter_mut().enumerate() {
        *b = [0x80u8, 0xFF, 0x7F, 0x00, 0xFE, 0x81, 0x01, 0xC0][i % 8];
    }
    emu.reg_write_ymm(X86Reg::Ymm1, abs_src);
    run(&mut emu, programs, 4);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    for i in 0..32 {
        assert_eq!(
            got[i],
            (abs_src[i] as i8).wrapping_abs() as u8,
            "vpabsb ymm byte {i}"
        );
    }

    // 5: vinsertps xmm0,xmm1,xmm2,0x61 — vvvv (xmm1) is the FIRST source:
    //    result = xmm1 with element 2 := xmm2[1], element 0 zeroed (imm[0]);
    //    upper ymm bits zeroed. A destructive 2-operand execution would
    //    build the result from the old xmm0 instead.
    let v1 = [10.0f32, 11.0, 12.0, 13.0];
    let v2 = [20.0f32, 21.0, 22.0, 23.0];
    emu.reg_write_ymm(X86Reg::Ymm0, [0xAB; 32]);
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f32x4(v1));
    emu.reg_write_xmm(X86Reg::Xmm2, xmm_from_f32x4(v2));
    run(&mut emu, programs, 5);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    let want = [0.0f32, v1[1], v2[1], v1[3]];
    for i in 0..4 {
        assert_eq!(f32_lane(&got, i), want[i].to_bits(), "vinsertps lane {i}");
    }
    assert_eq!(&got[16..32], &[0u8; 16], "vinsertps must zero ymm 255:128");

    // 6: vaesenc xmm0,xmm1,xmm2 — 3-operand: state comes from vvvv (xmm1),
    //    round key from rm (xmm2), xmm1 stays intact, upper ymm zeroed.
    //    Intel AES-NI whitepaper reference vector.
    let state = 0x7b5b54657374566563746f725d53475d_u128;
    let key = 0x48692853686179295b477565726f6e5d_u128;
    emu.reg_write_ymm(X86Reg::Ymm0, [0x77; 32]);
    emu.reg_write_xmm(X86Reg::Xmm1, state.to_le_bytes());
    emu.reg_write_xmm(X86Reg::Xmm2, key.to_le_bytes());
    run(&mut emu, programs, 6);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    assert_eq!(
        u128::from_le_bytes(got[..16].try_into().unwrap()),
        0xa8311c2f9fdba3c58b104b58ded7e595_u128,
        "vaesenc must encrypt vvvv state with rm round key"
    );
    assert_eq!(&got[16..32], &[0u8; 16], "vaesenc must zero ymm 255:128");
    assert_eq!(
        u128::from_le_bytes(emu.reg_read_xmm(X86Reg::Xmm1)),
        state,
        "vaesenc must not clobber the vvvv source"
    );

    // 7: vpblendvb xmm — mask register is is4 (xmm3), per-BYTE sign bits:
    //    even bytes (mask 0x80) from rm (xmm2), odd bytes from vvvv (xmm1).
    let mut b1 = [0u8; 16];
    let mut b2 = [0u8; 16];
    let mut bm = [0u8; 16];
    for i in 0..16 {
        b1[i] = 10 + i as u8;
        b2[i] = 110 + i as u8;
        bm[i] = if i % 2 == 0 { 0x80 } else { 0x7F };
    }
    emu.reg_write_ymm(X86Reg::Ymm0, [0xEE; 32]);
    emu.reg_write_xmm(X86Reg::Xmm1, b1);
    emu.reg_write_xmm(X86Reg::Xmm2, b2);
    emu.reg_write_xmm(X86Reg::Xmm3, bm);
    run(&mut emu, programs, 7);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    for i in 0..16 {
        let want = if i % 2 == 0 { b2[i] } else { b1[i] };
        assert_eq!(got[i], want, "vpblendvb xmm byte {i}");
    }
    assert_eq!(&got[16..32], &[0u8; 16], "vpblendvb must zero ymm 255:128");

    // 8: vpblendvb ymm — mask sign bits only in the UPPER lane: lane 0 from
    //    vvvv untouched, lane 1 fully from rm.
    let mut y1 = [0u8; 32];
    let mut y2 = [0u8; 32];
    let mut ym = [0u8; 32];
    for i in 0..32 {
        y1[i] = i as u8;
        y2[i] = 0xA0 + i as u8;
        ym[i] = if i >= 16 { 0x80 } else { 0x00 };
    }
    emu.reg_write_ymm(X86Reg::Ymm1, y1);
    emu.reg_write_ymm(X86Reg::Ymm2, y2);
    emu.reg_write_ymm(X86Reg::Ymm3, ym);
    run(&mut emu, programs, 8);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    assert_eq!(&got[..16], &y1[..16], "vpblendvb ymm lower lane from vvvv");
    assert_eq!(&got[16..], &y2[16..], "vpblendvb ymm upper lane from rm");

    // 9/10: vphminposuw (VEX zeroes ymm upper) vs legacy phminposuw
    //    (preserves ymm upper). Min word 55 first appears at index 2.
    let mut hw = [0u8; 16];
    let words: [u16; 8] = [700, 300, 55, 800, 55, 900, 1000, 65535];
    for (i, w) in words.iter().enumerate() {
        hw[i * 2..i * 2 + 2].copy_from_slice(&w.to_le_bytes());
    }
    for (idx, is_vex) in [(9usize, true), (10usize, false)] {
        emu.reg_write_ymm(X86Reg::Ymm0, [0xEE; 32]);
        emu.reg_write_xmm(X86Reg::Xmm1, hw);
        run(&mut emu, programs, idx);
        let got = emu.reg_read_ymm(X86Reg::Ymm0);
        assert_eq!(
            u16::from_le_bytes(got[0..2].try_into().unwrap()),
            55,
            "phminposuw min value (vex={is_vex})"
        );
        assert_eq!(
            u16::from_le_bytes(got[2..4].try_into().unwrap()),
            2,
            "phminposuw min index (vex={is_vex})"
        );
        assert_eq!(&got[4..16], &[0u8; 12], "phminposuw upper words zeroed");
        if is_vex {
            assert_eq!(&got[16..32], &[0u8; 16], "vphminposuw zeroes ymm upper");
        } else {
            assert_eq!(
                &got[16..32],
                &[0xEE; 16],
                "legacy phminposuw preserves ymm upper"
            );
        }
    }

    // 11: vmpsadbw ymm, imm 0x05 — per-lane control: lane 0 uses bits [2:0]
    //    (5 → src quad at op2 bytes 4..8, dst window base 4), lane 1 uses
    //    bits [5:3] (0 → src quad 0..4, base 0). With op1 (vvvv) all zero,
    //    every result word is the plain byte sum of the selected quadruple.
    let mut sadsrc = [0u8; 32];
    sadsrc[4..8].copy_from_slice(&[1, 2, 3, 4]); // lane 0 quad → sum 10
    sadsrc[16..20].copy_from_slice(&[5, 6, 7, 8]); // lane 1 quad → sum 26
    emu.reg_write_ymm(X86Reg::Ymm1, [0u8; 32]);
    emu.reg_write_ymm(X86Reg::Ymm2, sadsrc);
    run(&mut emu, programs, 11);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    for j in 0..8 {
        let lo = u16::from_le_bytes(got[j * 2..j * 2 + 2].try_into().unwrap());
        let hi = u16::from_le_bytes(got[16 + j * 2..16 + j * 2 + 2].try_into().unwrap());
        assert_eq!(lo, 10, "vmpsadbw lane 0 word {j}");
        assert_eq!(hi, 26, "vmpsadbw lane 1 word {j}");
    }

    // 12: vmovq xmm0,xmm1 — low qword copied, bits 255:64 zeroed (the
    //    legacy handler would leave ymm bits 255:128 intact).
    let mut q1 = [0u8; 16];
    q1[..8].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    q1[8..].copy_from_slice(&u64::MAX.to_le_bytes());
    emu.reg_write_ymm(X86Reg::Ymm0, [0xCD; 32]);
    emu.reg_write_xmm(X86Reg::Xmm1, q1);
    run(&mut emu, programs, 12);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    assert_eq!(f64_lane(&got, 0), 0x1122_3344_5566_7788, "vmovq low qword");
    assert_eq!(&got[8..32], &[0u8; 24], "vmovq must zero bits 255:64");

    // 13/14: vlddqu ymm (32-byte load) and legacy lddqu xmm (16-byte load).
    let pattern: Vec<u8> = (0u8..32).map(|i| i.wrapping_mul(13) ^ 0x27).collect();
    let base = 0x0060_0100u64;
    emu.mem_write(base, &pattern).expect("write pattern");
    emu.reg_write(X86Reg::Rax, base);
    emu.reg_write_ymm(X86Reg::Ymm0, [0u8; 32]);
    run(&mut emu, programs, 13);
    assert_eq!(
        &emu.reg_read_ymm(X86Reg::Ymm0)[..],
        &pattern[..],
        "vlddqu ymm must load all 32 bytes"
    );
    emu.reg_write_xmm(X86Reg::Xmm0, [0u8; 16]);
    run(&mut emu, programs, 14);
    assert_eq!(
        &emu.reg_read_xmm(X86Reg::Xmm0)[..],
        &pattern[..16],
        "legacy lddqu xmm must load 16 bytes"
    );
}

// ════════════════════════════════════════════════════════════════════════
// Float→int conversion boundaries: the rounded/truncated INTEGER decides
// validity, not a float comparison against i32::MAX (Bochs softfloat
// f32_to_i32 / f64_to_i32 semantics).
// ════════════════════════════════════════════════════════════════════════

#[test]
fn cvt_float_to_int_boundary_semantics() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(run_cvt_boundary_cases)
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}

fn run_cvt_boundary_cases() {
    let cfg = EmulatorConfig::default();
    let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
        .expect("new emulator in flat long mode");
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );

    let programs: &[(&str, &[u8])] = &[
        ("cvttsd2si eax,xmm1", &[0xF2, 0x0F, 0x2C, 0xC1]),
        ("cvtsd2si eax,xmm1", &[0xF2, 0x0F, 0x2D, 0xC1]),
        ("cvttss2si eax,xmm1", &[0xF3, 0x0F, 0x2C, 0xC1]),
        ("cvttsd2si rax,xmm1", &[0xF2, 0x48, 0x0F, 0x2C, 0xC1]),
        ("vcvttps2dq xmm0,xmm1", &[0xC5, 0xFA, 0x5B, 0xC1]),
        ("cvttps2dq xmm0,xmm1", &[0xF3, 0x0F, 0x5B, 0xC1]),
    ];
    for (i, (_, code)) in programs.iter().enumerate() {
        let addr = CASE_BASE + i as u64 * CASE_STRIDE;
        emu.mem_write(addr, code).expect("write case code");
        emu.mem_write(addr + code.len() as u64, &[0xEB, 0xFE])
            .expect("write park jump");
    }
    fn run(emu: &mut Emulator<'static, Corei7SkylakeX>, programs: &[(&str, &[u8])], idx: usize) {
        let (name, code) = programs[idx];
        let addr = CASE_BASE + idx as u64 * CASE_STRIDE;
        let park = addr + code.len() as u64;
        let stop = emu
            .emu_start(addr, Some(park), None, Some(9))
            .expect("emu_start");
        assert_eq!(emu.cpu().rip(), park, "{name}: did not park (stop={stop:?})");
    }

    // 0: truncation of f64 values in (i32::MAX, 2^31) is VALID and yields
    //    i32::MAX; from 2^31 upward it is invalid (integer indefinite).
    for (input, want) in [
        (2147483647.5f64, 0x7FFF_FFFFu64),
        (2147483647.0, 0x7FFF_FFFF),
        (2147483648.0, 0x8000_0000),
        (-2147483648.9, 0x8000_0000), // truncates to exactly i32::MIN — valid
        (-2147483649.0, 0x8000_0000), // invalid → indefinite (same bit pattern)
        (f64::NAN, 0x8000_0000),
    ] {
        emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f64(input));
        emu.reg_write(X86Reg::Rax, 0xDEAD_BEEF_DEAD_BEEF);
        run(&mut emu, programs, 0);
        assert_eq!(emu.reg_read(X86Reg::Rax), want, "cvttsd2si eax of {input}");
    }

    // 1: round-to-nearest-even — values below 2^31 - 0.5 round to i32::MAX
    //    (valid); the 2147483647.5 tie rounds to even 2^31 → indefinite.
    for (input, want) in [
        (2147483647.4f64, 0x7FFF_FFFFu64),
        (2147483647.5, 0x8000_0000),
        (2147483646.5, 0x7FFF_FFFE), // tie → even 2147483646
    ] {
        emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f64(input));
        emu.reg_write(X86Reg::Rax, 0xDEAD_BEEF_DEAD_BEEF);
        run(&mut emu, programs, 1);
        assert_eq!(emu.reg_read(X86Reg::Rax), want, "cvtsd2si eax of {input}");
    }

    // 2: f32 2^31 exactly must be indefinite (a float-domain check against
    //    `i32::MAX as f32` == 2^31 wrongly accepts and saturates it);
    //    the largest f32 below 2^31 converts exactly.
    for (input, want) in [
        (2147483648.0f32, 0x8000_0000u64), // 2^31, exact in f32
        (2147483520.0f32, 0x7FFF_FF80),    // 2^31 - 128, largest valid f32
    ] {
        let mut x = [0u8; 16];
        x[..4].copy_from_slice(&input.to_bits().to_le_bytes());
        emu.reg_write_xmm(X86Reg::Xmm1, x);
        emu.reg_write(X86Reg::Rax, 0xDEAD_BEEF_DEAD_BEEF);
        run(&mut emu, programs, 2);
        assert_eq!(emu.reg_read(X86Reg::Rax), want, "cvttss2si eax of {input}");
    }

    // 3: 64-bit destination — 2^63 f64 is invalid, the largest f64 below
    //    2^63 (2^63 - 1024) is valid.
    for (input, want) in [
        (9223372036854775808.0f64, 0x8000_0000_0000_0000u64),
        (9223372036854774784.0f64, 9223372036854774784),
    ] {
        emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f64(input));
        emu.reg_write(X86Reg::Rax, 0);
        run(&mut emu, programs, 3);
        assert_eq!(emu.reg_read(X86Reg::Rax), want, "cvttsd2si rax of {input}");
    }

    // 4/5: packed truncation — VEX and legacy must agree lane-for-lane.
    let lanes = [2147483648.0f32, -2147483520.0, 2147483520.0, -2.5];
    let want = [0x8000_0000u32, 0x8000_0080, 0x7FFF_FF80, (-2i32) as u32];
    for idx in [4usize, 5] {
        emu.reg_write_xmm(X86Reg::Xmm1, xmm_from_f32x4(lanes));
        emu.reg_write_xmm(X86Reg::Xmm0, [0u8; 16]);
        run(&mut emu, programs, idx);
        let got = emu.reg_read_xmm(X86Reg::Xmm0);
        for i in 0..4 {
            assert_eq!(
                xmm_lane32(&got, i),
                want[i],
                "cvttps2dq (program {idx}) lane {i} of {}",
                lanes[i]
            );
        }
    }
}

// ════════════════════════════════════════════════════════════════════════
// VEX remap-gap families (ptest / movmsk / pmovzx / movq / blendvb) and cvt
// overflow boundaries. These executed as legacy 128-bit SSE under VEX before
// the remap was completed: upper YMM lanes ignored, destinations not zeroed.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn vex_remap_gap_families() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(run_remap_gap_cases)
        .expect("spawn")
        .join()
        .expect("join");
}

fn run_remap_gap_cases() {
    let cfg = EmulatorConfig::default();
    let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
        .expect("new emulator");
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );
    emu.reg_write(X86Reg::Rsp, STACK_TOP); // pushfq/pop in the vptest case

    // idx, name, bytes, insns
    let programs: &[(&str, &[u8], u64)] = &[
        // vptest ymm0, ymm1 ; pushfq ; pop rax   (VEX.256.66.0F38 17)
        ("vptest ymm", &[0xC4, 0xE2, 0x7D, 0x17, 0xC1, 0x9C, 0x58], 3),
        // vmovmskps eax, ymm1                     (VEX.256.0F 50)
        ("vmovmskps ymm", &[0xC5, 0xFC, 0x50, 0xC1], 1),
        // vpmovzxbw ymm0, xmm1                    (VEX.256.66.0F38 30)
        ("vpmovzxbw", &[0xC4, 0xE2, 0x7D, 0x30, 0xC1], 1),
        // vmovq xmm0, xmm1                        (VEX.128.F3.0F 7E)
        ("vmovq upper-zero", &[0xC5, 0xFA, 0x7E, 0xC1], 1),
        // vpblendvb xmm0, xmm1, xmm2, xmm3        (VEX.128.66.0F3A.W0 4C, is4=3)
        ("vpblendvb is4", &[0xC4, 0xE3, 0x71, 0x4C, 0xC2, 0x30], 1),
        // vcvttpd2dq xmm0, xmm1                   (VEX.128.66.0F E6, truncate)
        ("vcvttpd2dq boundary", &[0xC5, 0xF9, 0xE6, 0xC1], 1),
        // vcvtpd2dq xmm0, xmm1                    (VEX.128.F2.0F E6, round)
        ("vcvtpd2dq boundary", &[0xC5, 0xFB, 0xE6, 0xC1], 1),
    ];
    for (i, (_, code, _)) in programs.iter().enumerate() {
        let addr = CASE_BASE + i as u64 * CASE_STRIDE;
        emu.mem_write(addr, code).expect("write code");
        emu.mem_write(addr + code.len() as u64, &[0xEB, 0xFE])
            .expect("write park");
    }
    fn run(
        emu: &mut Emulator<'static, Corei7SkylakeX>,
        programs: &[(&str, &[u8], u64)],
        idx: usize,
    ) {
        let (name, code, insns) = programs[idx];
        let addr = CASE_BASE + idx as u64 * CASE_STRIDE;
        let park = addr + code.len() as u64;
        let stop = emu
            .emu_start(addr, Some(park), None, Some(insns + 8))
            .expect("emu_start");
        assert_eq!(emu.cpu().rip(), park, "{name}: no park (stop={stop:?})");
    }

    // 0: vptest — overlap ONLY in the upper 128-bit lane. A 128-bit-only
    // handler sees all-zero (ZF=1); a correct 256-bit handler sees the
    // overlap (ZF=0).
    let mut y = [0u8; 32];
    y[16] = 0x01; // byte in the upper lane
    emu.reg_write_ymm(X86Reg::Ymm0, y);
    emu.reg_write_ymm(X86Reg::Ymm1, y);
    emu.reg_write(X86Reg::Rax, 0);
    run(&mut emu, programs, 0);
    assert_eq!(
        emu.reg_read(X86Reg::Rax) & 0x40,
        0,
        "vptest ZF must be 0 — upper-lane AND is nonzero (256-bit tested)"
    );

    // 1: vmovmskps ymm — sign bits in lanes 0,3,4,7 → 0b1001_1001 = 0x99.
    // A 128-bit handler would return only 0x09.
    let mut y = [0u8; 32];
    for lane in [0usize, 3, 4, 7] {
        y[lane * 4 + 3] = 0x80; // sign bit of each f32 lane
    }
    emu.reg_write_ymm(X86Reg::Ymm1, y);
    emu.reg_write(X86Reg::Rax, 0xDEAD_BEEF_0000_0000);
    run(&mut emu, programs, 1);
    assert_eq!(
        emu.reg_read(X86Reg::Rax),
        0x99,
        "vmovmskps ymm must include lanes 4..7 and zero-extend to rax"
    );

    // 2: vpmovzxbw ymm0, xmm1 — 16 bytes zero-extend to 16 words; the upper
    // words (from source bytes 8..15) must be present, not zero.
    let mut x = [0u8; 16];
    for i in 0..16 {
        x[i] = (i as u8) | 0x80; // 0x80..0x8F, high bit set (proves zero-ext, not sign-ext)
    }
    emu.reg_write_ymm(X86Reg::Ymm0, [0xAA; 32]);
    emu.reg_write_xmm(X86Reg::Xmm1, x);
    run(&mut emu, programs, 2);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    for i in 0..16 {
        let word = u16::from_le_bytes([got[i * 2], got[i * 2 + 1]]);
        assert_eq!(word, (x[i] as u16), "vpmovzxbw word {i} zero-extend");
    }

    // 3: vmovq — low qword from xmm1, bits 64..256 zeroed even though ymm0
    // was pre-dirtied.
    emu.reg_write_ymm(X86Reg::Ymm0, [0xAA; 32]);
    let mut x = [0u8; 16];
    x[..8].copy_from_slice(&0x0123_4567_89AB_CDEF_u64.to_le_bytes());
    x[8..].copy_from_slice(&0xFFFF_FFFF_FFFF_FFFF_u64.to_le_bytes());
    emu.reg_write_xmm(X86Reg::Xmm1, x);
    run(&mut emu, programs, 3);
    let got = emu.reg_read_ymm(X86Reg::Ymm0);
    assert_eq!(
        u64::from_le_bytes(got[..8].try_into().unwrap()),
        0x0123_4567_89AB_CDEF,
        "vmovq low qword"
    );
    assert_eq!(&got[8..32], &[0u8; 24], "vmovq must zero bits 255:64");

    // 4: vpblendvb — is4 mask xmm3; byte sign bit selects xmm2(rm) else
    // xmm1(vvvv). Only bit 7 of each mask byte matters (not 0x01).
    emu.reg_write_xmm(X86Reg::Xmm1, [0x11; 16]);
    emu.reg_write_xmm(X86Reg::Xmm2, [0x22; 16]);
    let mut mask = [0u8; 16];
    for i in 0..16 {
        mask[i] = if i % 2 == 0 { 0x80 } else { 0x7F }; // even: pick rm; odd: 0x7F has no sign → vvvv
    }
    emu.reg_write_xmm(X86Reg::Xmm3, mask);
    emu.reg_write_xmm(X86Reg::Xmm0, [0u8; 16]);
    run(&mut emu, programs, 4);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    for i in 0..16 {
        let want = if i % 2 == 0 { 0x22 } else { 0x11 };
        assert_eq!(got[i], want, "vpblendvb byte {i} (sign-bit-only mask)");
    }

    // 5: vcvttpd2dq — truncation of 2147483647.5 is in-range → 0x7FFFFFFF.
    let mut x = [0u8; 16];
    x[..8].copy_from_slice(&2147483647.5f64.to_bits().to_le_bytes());
    emu.reg_write_xmm(X86Reg::Xmm1, x);
    run(&mut emu, programs, 5);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    assert_eq!(
        i32::from_le_bytes(got[..4].try_into().unwrap()),
        0x7FFF_FFFF,
        "vcvttpd2dq(2147483647.5) truncates to i32::MAX, not indefinite"
    );

    // 6: vcvtpd2dq — round-ties-even of 2147483647.5 → 2^31 (out of range) →
    // integer-indefinite 0x80000000.
    run(&mut emu, programs, 6);
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    assert_eq!(
        got[..4],
        [0x00, 0x00, 0x00, 0x80],
        "vcvtpd2dq(2147483647.5) rounds up out of range → 0x80000000"
    );
}
