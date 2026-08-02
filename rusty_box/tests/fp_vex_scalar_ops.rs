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

/// Emulator construction needs a bigger stack than the default 2 MiB test
/// thread: `Emulator` is ~4 MiB and the debug build materialises a few
/// copies while boxing it. 64 MiB is ample; the previous 256 MiB made
/// enough concurrent reservations to intermittently exhaust the process
/// and fail unrelated tests with STATUS_STACK_OVERFLOW.
const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;

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
        .stack_size(TEST_STACK_SIZE)
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
    enable_guest_avx_state(&mut emu);
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
        .stack_size(TEST_STACK_SIZE)
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
    enable_guest_avx_state(&mut emu);

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
        .stack_size(TEST_STACK_SIZE)
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
    enable_guest_avx_state(&mut emu);

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


/// Enable the guest's XCR0 FPU+SSE+YMM state, the way a real OS does before
/// it runs any VEX instruction. Without it CR4.OSXSAVE is set but XCR0 is
/// still zero — a configuration in which every VEX encoding would #UD, and in
/// which `maxvl` leaves the upper half of the register file architecturally
/// invisible, so a VEX write does not clear it.
fn enable_guest_avx_state(emu: &mut Emulator<'static, Corei7SkylakeX>) {
    const XSETBV_SETUP_BASE: u64 = CASE_BASE + 0x800;
    emu.reg_write(X86Reg::Rax, 0x7);
    emu.reg_write(X86Reg::Rcx, 0);
    emu.reg_write(X86Reg::Rdx, 0);
    emu.mem_write(XSETBV_SETUP_BASE, &[0x0F, 0x01, 0xD1])
        .expect("write xsetbv");
    emu.emu_start(
        XSETBV_SETUP_BASE,
        Some(XSETBV_SETUP_BASE + 3),
        None,
        Some(1),
    )
    .expect("enable guest AVX state");
}

#[test]
fn vex_vpinsrw_sources_vvvv_and_clears_upper() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
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

// ════════════════════════════════════════════════════════════════════════
// VTESTPS/VTESTPD (VEX.66.0F38 0E/0F) are AVX instructions the CPUID model
// advertises. They set only ZF and CF from packed sign bits and write no
// destination; OF/SF/AF/PF are always cleared (Bochs avx_pfp.cc
// VTESTPS_VpsWpsR / VTESTPD_VpdWpdR: setEFlagsOSZAPC(ZF|CF seed)).
// ════════════════════════════════════════════════════════════════════════

#[test]
fn vex_vtestps_vtestpd_set_zf_cf_from_sign_bits() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");
            emu.reg_write(
                X86Reg::Cr4,
                emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
            );
            enable_guest_avx_state(&mut emu);
            emu.reg_write(X86Reg::Rax, 0x7);
            emu.reg_write(X86Reg::Rcx, 0);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1])
                .expect("write xsetbv");
            emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                .expect("enable guest AVX state");

            const ZF: u64 = 1 << 6;
            const CF: u64 = 1 << 0;
            const OF: u64 = 1 << 11;
            const SF: u64 = 1 << 7;
            const AF: u64 = 1 << 4;
            const PF: u64 = 1 << 2;

            // Sign bit of every dword / qword element.
            let ps_all: [u8; 32] = [[0x00, 0x00, 0x00, 0x80]; 8].concat().try_into().unwrap();
            let pd_all: [u8; 32] = [[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80]; 4]
                .concat()
                .try_into()
                .unwrap();
            // Sign bit of the LOW dword of each qword only: a VTESTPS sign bit
            // that is *not* a VTESTPD sign bit. This is what separates the two
            // masks — in `ps_all` the odd dwords' sign bits coincide with the
            // qword sign bits, so it cannot tell them apart.
            let ps_low_dwords: [u8; 32] =
                [[0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00]; 4]
                    .concat()
                    .try_into()
                    .unwrap();
            let zero = [0u8; 32];

            // (code, name, len, op1 (dst/nnn), op2 (rm), want_zf, want_cf)
            //
            // ZF = ((op2 & op1 & signmask) == 0); CF = ((op2 & ~op1 & signmask) == 0).
            let cases: &[(&[u8], &str, u64, [u8; 32], [u8; 32], bool, bool)] = &[
                // VTESTPS ymm1, ymm2 — C4 E2 7D 0E CA.
                (
                    &[0xC4, 0xE2, 0x7D, 0x0E, 0xCA],
                    "vtestps all/all",
                    5,
                    ps_all,
                    ps_all,
                    false,
                    true,
                ),
                (
                    &[0xC4, 0xE2, 0x7D, 0x0E, 0xCA],
                    "vtestps none/all",
                    5,
                    zero,
                    ps_all,
                    true,
                    false,
                ),
                (
                    &[0xC4, 0xE2, 0x7D, 0x0E, 0xCA],
                    "vtestps zero/zero",
                    5,
                    zero,
                    zero,
                    true,
                    true,
                ),
                // Low-dword sign bits only: VTESTPS sees them, VTESTPD must
                // not. A VTESTPD using the VTESTPS mask fails the ZF assert.
                (
                    &[0xC4, 0xE2, 0x7D, 0x0E, 0xCA],
                    "vtestps low-dword signs",
                    5,
                    ps_low_dwords,
                    ps_low_dwords,
                    false,
                    true,
                ),
                (
                    &[0xC4, 0xE2, 0x7D, 0x0F, 0xCA],
                    "vtestpd ignores dword-only signs",
                    5,
                    ps_low_dwords,
                    ps_low_dwords,
                    true,
                    true,
                ),
                (
                    &[0xC4, 0xE2, 0x7D, 0x0F, 0xCA],
                    "vtestpd all/all",
                    5,
                    pd_all,
                    pd_all,
                    false,
                    true,
                ),
                // VEX.128 must only look at the low lane: op1 has no sign bits
                // in the low 128 bits, so ZF stays set even though the upper
                // lane is fully signed.
                (
                    &[0xC4, 0xE2, 0x79, 0x0E, 0xCA],
                    "vtestps vl128 ignores upper lane",
                    5,
                    {
                        let mut v = ps_all;
                        v[..16].fill(0);
                        v
                    },
                    ps_all,
                    true,
                    false,
                ),
            ];

            for &(code, name, len, op1, op2, want_zf, want_cf) in cases {
                let mut prog = code.to_vec();
                prog.extend_from_slice(&[0xEB, 0xFE]); // park
                emu.mem_write(CASE_BASE, &prog).expect("write code");
                emu.reg_write_ymm(X86Reg::Ymm1, op1);
                emu.reg_write_ymm(X86Reg::Ymm2, op2);
                // Poison every flag VTEST must clear, so "cleared" is proven.
                emu.reg_write(
                    X86Reg::Rflags,
                    emu.reg_read(X86Reg::Rflags) | OF | SF | AF | PF | ZF | CF,
                );

                let stop = emu
                    .emu_start(CASE_BASE, Some(CASE_BASE + len), None, Some(9))
                    .expect("emu_start");
                assert_eq!(
                    emu.cpu().rip(),
                    CASE_BASE + len,
                    "{name}: did not park (stop={stop:?})"
                );

                let fl = emu.reg_read(X86Reg::Rflags);
                assert_eq!((fl & ZF) != 0, want_zf, "{name}: ZF");
                assert_eq!((fl & CF) != 0, want_cf, "{name}: CF");
                assert_eq!(fl & (OF | SF | AF | PF), 0, "{name}: OF/SF/AF/PF must clear");
                // No destination is written.
                assert_eq!(emu.reg_read_ymm(X86Reg::Ymm1), op1, "{name}: op1 clobbered");
                assert_eq!(emu.reg_read_ymm(X86Reg::Ymm2), op2, "{name}: op2 clobbered");
            }
        })
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}

// ════════════════════════════════════════════════════════════════════════
// AVX/AVX2 permutes and per-element variable shifts. The CPUID model
// advertises AVX and AVX2, so these must execute rather than #UD. The
// in-lane permutes (VPERMIL*) must not move data across the 128-bit lane
// boundary; the cross-lane ones (VPERMPS/VPERMPD) must.
// ════════════════════════════════════════════════════════════════════════

fn ymm_from_u32(v: [u32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (n, x) in v.iter().enumerate() {
        out[n * 4..n * 4 + 4].copy_from_slice(&x.to_le_bytes());
    }
    out
}

fn ymm_from_u64(v: [u64; 4]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (n, x) in v.iter().enumerate() {
        out[n * 8..n * 8 + 8].copy_from_slice(&x.to_le_bytes());
    }
    out
}

/// Boot an emulator with guest AVX state enabled, run `code` at CASE_BASE
/// after `setup`, and hand the parked emulator back for assertions.
fn with_avx_emu(f: impl FnOnce(&mut Emulator<'static, Corei7SkylakeX>) + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(move || {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");
            emu.reg_write(
                X86Reg::Cr4,
                emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
            );
            enable_guest_avx_state(&mut emu);
            emu.reg_write(X86Reg::Rax, 0x7);
            emu.reg_write(X86Reg::Rcx, 0);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1])
                .expect("write xsetbv");
            emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                .expect("enable guest AVX state");
            f(&mut emu);
        })
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}

fn run_one(emu: &mut Emulator<'static, Corei7SkylakeX>, name: &str, code: &[u8]) {
    let mut prog = code.to_vec();
    prog.extend_from_slice(&[0xEB, 0xFE]); // park
    emu.mem_write(CASE_BASE, &prog).expect("write code");
    let end = CASE_BASE + code.len() as u64;
    let stop = emu
        .emu_start(CASE_BASE, Some(end), None, Some(9))
        .expect("emu_start");
    assert_eq!(emu.cpu().rip(), end, "{name}: did not park (stop={stop:?})");
}

#[test]
fn vex_vpermilps_vpermilpd_stay_in_lane() {
    with_avx_emu(|emu| {
        // Element values equal their own index, so a lane-crossing bug in the
        // 256-bit form produces obviously wrong indices.
        let data_ps = ymm_from_u32([0, 1, 2, 3, 4, 5, 6, 7]);
        let data_pd = ymm_from_u64([10, 11, 12, 13]);

        // VPERMILPS ymm1, ymm2, ymm3 — C4 E2 6D 0C CB (vvvv=ymm2 data,
        // rm=ymm3 control). Control reverses each lane independently.
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, data_ps);
        emu.reg_write_ymm(X86Reg::Ymm3, ymm_from_u32([3, 2, 1, 0, 3, 2, 1, 0]));
        run_one(emu, "vpermilps var", &[0xC4, 0xE2, 0x6D, 0x0C, 0xCB]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([3, 2, 1, 0, 7, 6, 5, 4]),
            "VPERMILPS must permute within each 128-bit lane"
        );

        // VPERMILPD ymm1, ymm2, ymm3 — C4 E2 6D 0D CB. The selector for each
        // qword is bit 1 of dword 0 / dword 2 of that lane.
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, data_pd);
        emu.reg_write_ymm(X86Reg::Ymm3, ymm_from_u32([2, 0, 0, 0, 0, 0, 2, 0]));
        run_one(emu, "vpermilpd var", &[0xC4, 0xE2, 0x6D, 0x0D, 0xCB]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u64([11, 10, 12, 13]),
            "VPERMILPD selector is bit 1 of the even dword of each qword pair"
        );

        // VPERMILPS ymm1, ymm2, 0x1B — C4 E3 7D 04 CA 1B. Same imm8 per lane.
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, data_ps);
        run_one(emu, "vpermilps imm", &[0xC4, 0xE3, 0x7D, 0x04, 0xCA, 0x1B]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([3, 2, 1, 0, 7, 6, 5, 4]),
            "VPERMILPS imm8 applies the same control to both lanes"
        );

        // VPERMILPD ymm1, ymm2, 0x05 — C4 E3 7D 05 CA 05. Bochs shifts the
        // order right by 2 per lane, so lane 1 uses imm8 bits [3:2].
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, data_pd);
        run_one(emu, "vpermilpd imm", &[0xC4, 0xE3, 0x7D, 0x05, 0xCA, 0x05]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u64([11, 10, 13, 12]),
            "VPERMILPD imm8 consumes two bits per 128-bit lane"
        );

        // VEX.128 form must zero bits [255:128] — C4 E3 79 04 CA 1B.
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, data_ps);
        run_one(emu, "vpermilps imm vl128", &[0xC4, 0xE3, 0x79, 0x04, 0xCA, 0x1B]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([3, 2, 1, 0, 0, 0, 0, 0]),
            "VEX.128 VPERMILPS must clear ymm1 bits [255:128]"
        );
    });
}

#[test]
fn vex_vpermps_vpermpd_cross_lanes() {
    with_avx_emu(|emu| {
        // VPERMPS ymm1, ymm2, ymm3 — C4 E2 6D 16 CB. vvvv (ymm2) is the index
        // vector, rm (ymm3) the source; the permute is fully cross-lane.
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u32([7, 6, 5, 4, 3, 2, 1, 0]));
        emu.reg_write_ymm(
            X86Reg::Ymm3,
            ymm_from_u32([100, 101, 102, 103, 104, 105, 106, 107]),
        );
        run_one(emu, "vpermps", &[0xC4, 0xE2, 0x6D, 0x16, 0xCB]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([107, 106, 105, 104, 103, 102, 101, 100]),
            "VPERMPS must permute across both 128-bit lanes"
        );

        // VPERMPD ymm1, ymm2, 0x1B — C4 E3 FD 01 CA 1B (source is rm).
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u64([20, 21, 22, 23]));
        run_one(emu, "vpermpd", &[0xC4, 0xE3, 0xFD, 0x01, 0xCA, 0x1B]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u64([23, 22, 21, 20]),
            "VPERMPD imm8 selects any of the four qwords per element"
        );
    });
}

#[test]
fn vex_variable_shifts_saturate_out_of_range_counts() {
    with_avx_emu(|emu| {
        let counts_d = ymm_from_u32([0, 1, 2, 3, 31, 32, 33, 255]);
        let counts_q = ymm_from_u64([0, 1, 63, 64]);

        // VPSRLVD ymm1, ymm2, ymm3 — C4 E2 6D 45 CB. Counts > 31 give 0.
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u32([0x8000_0000; 8]));
        emu.reg_write_ymm(X86Reg::Ymm3, counts_d);
        run_one(emu, "vpsrlvd", &[0xC4, 0xE2, 0x6D, 0x45, 0xCB]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([
                0x8000_0000,
                0x4000_0000,
                0x2000_0000,
                0x1000_0000,
                1,
                0,
                0,
                0
            ]),
            "VPSRLVD: counts above 31 must produce zero"
        );

        // VPSRAVD — C4 E2 6D 46 CB. Counts > 31 replicate the sign bit.
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u32([0x8000_0000; 8]));
        emu.reg_write_ymm(X86Reg::Ymm3, counts_d);
        run_one(emu, "vpsravd", &[0xC4, 0xE2, 0x6D, 0x46, 0xCB]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([
                0x8000_0000,
                0xC000_0000,
                0xE000_0000,
                0xF000_0000,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF
            ]),
            "VPSRAVD: counts above 31 must replicate the sign bit"
        );

        // VPSLLVD — C4 E2 6D 47 CB.
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u32([1; 8]));
        emu.reg_write_ymm(X86Reg::Ymm3, counts_d);
        run_one(emu, "vpsllvd", &[0xC4, 0xE2, 0x6D, 0x47, 0xCB]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([1, 2, 4, 8, 0x8000_0000, 0, 0, 0]),
            "VPSLLVD: counts above 31 must produce zero"
        );

        // VPSRLVQ — C4 E2 ED 45 CB (VEX.W1).
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u64([0x8000_0000_0000_0000; 4]));
        emu.reg_write_ymm(X86Reg::Ymm3, counts_q);
        run_one(emu, "vpsrlvq", &[0xC4, 0xE2, 0xED, 0x45, 0xCB]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u64([0x8000_0000_0000_0000, 0x4000_0000_0000_0000, 1, 0]),
            "VPSRLVQ: counts above 63 must produce zero"
        );

        // VPSLLVQ — C4 E2 ED 47 CB (VEX.W1).
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u64([1; 4]));
        emu.reg_write_ymm(X86Reg::Ymm3, counts_q);
        run_one(emu, "vpsllvq", &[0xC4, 0xE2, 0xED, 0x47, 0xCB]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u64([1, 2, 0x8000_0000_0000_0000, 0]),
            "VPSLLVQ: counts above 63 must produce zero"
        );

        // VEX.128 VPSRLVD must zero bits [255:128] — C4 E2 69 45 CB.
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u32([0x8000_0000; 8]));
        emu.reg_write_ymm(X86Reg::Ymm3, counts_d);
        run_one(emu, "vpsrlvd vl128", &[0xC4, 0xE2, 0x69, 0x45, 0xCB]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([0x8000_0000, 0x4000_0000, 0x2000_0000, 0x1000_0000, 0, 0, 0, 0]),
            "VEX.128 VPSRLVD must clear ymm1 bits [255:128]"
        );
    });
}

#[test]
fn vex_f16c_round_trips_half_precision() {
    with_avx_emu(|emu| {
        // Halves: 1.0, 2.0, -2.0, +0.0, 0.5, -1.0, +inf, smallest subnormal.
        let halves: [u16; 8] = [
            0x3C00, 0x4000, 0xC000, 0x0000, 0x3800, 0xBC00, 0x7C00, 0x0001,
        ];
        let singles: [u32; 8] = [
            0x3F80_0000, // 1.0
            0x4000_0000, // 2.0
            0xC000_0000, // -2.0
            0x0000_0000, // +0.0
            0x3F00_0000, // 0.5
            0xBF80_0000, // -1.0
            0x7F80_0000, // +inf
            0x3380_0000, // 2^-24 (exp 127-24=103), the smallest f16 subnormal
        ];

        let mut src = [0u8; 32];
        for (n, h) in halves.iter().enumerate() {
            src[n * 2..n * 2 + 2].copy_from_slice(&h.to_le_bytes());
        }

        // VCVTPH2PS ymm1, xmm2 — C4 E2 7D 13 CA. Reads the low 16 bytes only.
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, src);
        run_one(emu, "vcvtph2ps", &[0xC4, 0xE2, 0x7D, 0x13, 0xCA]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32(singles),
            "VCVTPH2PS must widen eight halves, subnormals included"
        );

        // Round trip back: VCVTPS2PH xmm3, ymm1, 0 — C4 E3 7D 1D CB 00.
        // modrm CB → nnn=1 (source ymm1), rm=3 (destination xmm3).
        emu.reg_write_ymm(X86Reg::Ymm3, [0xAA; 32]);
        run_one(emu, "vcvtps2ph", &[0xC4, 0xE3, 0x7D, 0x1D, 0xCB, 0x00]);
        let got = emu.reg_read_ymm(X86Reg::Ymm3);
        assert_eq!(&got[..16], &src[..16], "VCVTPS2PH must narrow back exactly");
        assert_eq!(
            &got[16..],
            &[0u8; 16],
            "VCVTPS2PH register form must clear bits above the result"
        );

        // VL128 register form writes only the low qword and clears the rest —
        // C4 E3 79 1D CB 00.
        emu.reg_write_ymm(X86Reg::Ymm3, [0xAA; 32]);
        run_one(emu, "vcvtps2ph vl128", &[0xC4, 0xE3, 0x79, 0x1D, 0xCB, 0x00]);
        let got = emu.reg_read_ymm(X86Reg::Ymm3);
        assert_eq!(&got[..8], &src[..8], "VL128 VCVTPS2PH writes four halves");
        assert_eq!(&got[8..], &[0u8; 24], "VL128 VCVTPS2PH clears bits [255:64]");

        // Memory destination: VCVTPS2PH [rax], ymm1, 0 — C4 E3 7D 1D 08 00.
        let scratch = 0x0060_0000u64;
        emu.mem_write(scratch, &[0x5A; 32]).expect("poison scratch");
        emu.reg_write(X86Reg::Rax, scratch);
        run_one(emu, "vcvtps2ph mem", &[0xC4, 0xE3, 0x7D, 0x1D, 0x08, 0x00]);
        let mut back = [0u8; 32];
        emu.mem_read(scratch, &mut back).expect("read scratch");
        assert_eq!(&back[..16], &src[..16], "VCVTPS2PH must store 16 bytes");
        assert_eq!(
            &back[16..],
            &[0x5A; 16],
            "VCVTPS2PH must not write past the half-width destination"
        );

        // imm8 rounding override. f32 0x3F803000 = 1 + 1.5 f16-ULP, exactly
        // halfway between f16 0x3C01 and 0x3C02: round-to-nearest-even picks
        // the even 0x3C02, truncation toward zero keeps 0x3C01.
        emu.reg_write_ymm(X86Reg::Ymm1, ymm_from_u32([0x3F80_3000; 8]));
        emu.reg_write_ymm(X86Reg::Ymm3, [0xAA; 32]);
        run_one(emu, "vcvtps2ph rne", &[0xC4, 0xE3, 0x7D, 0x1D, 0xCB, 0x00]);
        let rne = u16::from_le_bytes([emu.reg_read_ymm(X86Reg::Ymm3)[0], emu.reg_read_ymm(X86Reg::Ymm3)[1]]);

        emu.reg_write_ymm(X86Reg::Ymm3, [0xAA; 32]);
        run_one(emu, "vcvtps2ph trunc", &[0xC4, 0xE3, 0x7D, 0x1D, 0xCB, 0x03]);
        let trunc = u16::from_le_bytes([emu.reg_read_ymm(X86Reg::Ymm3)[0], emu.reg_read_ymm(X86Reg::Ymm3)[1]]);

        assert_eq!(trunc, 0x3C01, "imm8=3 must truncate toward zero");
        assert_eq!(rne, 0x3C02, "imm8=0 must round to nearest-even");
    });
}

// ════════════════════════════════════════════════════════════════════════
// VMASKMOVPS/PD and VPMASKMOVD/Q. The parity-critical property is fault
// behaviour: masked-off elements are never accessed, so they cannot fault,
// and a store either faults with memory untouched or completes in full.
// ════════════════════════════════════════════════════════════════════════

/// PD entry index for `addr` in the FlatLong64 identity tables (PD0 = 0x3000,
/// 512 entries of 2 MiB each covering the first GiB).
fn flat_long64_pde_addr(addr: u64) -> u64 {
    assert!(addr < (1 << 30), "only the first GiB lives in PD0");
    0x3000 + (addr / 0x0020_0000) * 8
}

#[test]
fn vex_maskmov_suppresses_masked_off_elements() {
    with_avx_emu(|emu| {
        const DATA: u64 = 0x0060_0000;
        // 0x00C0_0000 is made not-present; nothing has touched it yet, so no
        // stale TLB entry can hide the change.
        const HOLE: u64 = 0x00C0_0000;
        const STRADDLE: u64 = HOLE - 16; // dwords 0..3 mapped, 4..7 in the hole
        const IDT: u64 = 0x0028_0000;
        const PF_HANDLER: u64 = 0x0029_0000;
        const STACK: u64 = 0x0030_0000;

        // #PF gate (vector 14) whose handler is a single HLT.
        let mut gate = [0u8; 16];
        gate[0..2].copy_from_slice(&(PF_HANDLER as u16).to_le_bytes());
        gate[2..4].copy_from_slice(&0x0008u16.to_le_bytes());
        gate[5] = 0x8e;
        gate[6..8].copy_from_slice(&((PF_HANDLER >> 16) as u16).to_le_bytes());
        gate[8..12].copy_from_slice(&((PF_HANDLER >> 32) as u32).to_le_bytes());
        emu.mem_write(IDT + 14 * 16, &gate).expect("write #PF gate");
        emu.mem_write(PF_HANDLER, &[0xF4]).expect("write handler");
        emu.mem_write(0x808, &0x00AF_9A00_0000_FFFFu64.to_le_bytes())
            .expect("long code descriptor");
        emu.reg_write(X86Reg::IdtrBase, IDT);
        emu.reg_write(X86Reg::IdtrLimit, 256 * 16 - 1);
        emu.reg_write(X86Reg::Rsp, STACK);
        emu.mem_write(flat_long64_pde_addr(HOLE), &0u64.to_le_bytes())
            .expect("unmap the hole page");

        let payload = ymm_from_u32([10, 11, 12, 13, 14, 15, 16, 17]);
        emu.mem_write(DATA, &payload).expect("write payload");

        let all_set = ymm_from_u32([0x8000_0000; 8]);
        let alternating = ymm_from_u32([
            0x8000_0000,
            0,
            0x8000_0000,
            0,
            0x8000_0000,
            0,
            0x8000_0000,
            0,
        ]);
        let none = [0u8; 32];

        // ---- loads: VMASKMOVPS ymm1, ymm2, [rax] — C4 E2 6D 2C 08 ----
        let load = &[0xC4u8, 0xE2, 0x6D, 0x2C, 0x08];

        emu.reg_write(X86Reg::Rax, DATA);
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, all_set);
        run_one(emu, "maskmov load full", load);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            payload,
            "full mask must load every element"
        );

        emu.reg_write(X86Reg::Rax, DATA);
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, alternating);
        run_one(emu, "maskmov load alternating", load);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([10, 0, 12, 0, 14, 0, 16, 0]),
            "masked-off elements must read as zero, not be preserved"
        );

        // Zero mask against an unmapped page: no element may be accessed.
        emu.reg_write(X86Reg::Rax, HOLE);
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, none);
        run_one(emu, "maskmov load zero mask", load);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            [0u8; 32],
            "zero mask must zero the destination without touching memory"
        );

        // ---- stores: VMASKMOVPS [rax], ymm2, ymm1 — C4 E2 6D 2E 08 ----
        let store = &[0xC4u8, 0xE2, 0x6D, 0x2E, 0x08];

        emu.mem_write(DATA, &[0x5A; 32]).expect("poison");
        emu.reg_write(X86Reg::Rax, DATA);
        emu.reg_write_ymm(X86Reg::Ymm1, payload);
        emu.reg_write_ymm(X86Reg::Ymm2, alternating);
        run_one(emu, "maskmov store alternating", store);
        let mut back = [0u8; 32];
        emu.mem_read(DATA, &mut back).expect("read back");
        let mut expect = [0x5Au8; 32];
        for n in [0usize, 2, 4, 6] {
            expect[n * 4..n * 4 + 4].copy_from_slice(&payload[n * 4..n * 4 + 4]);
        }
        assert_eq!(back, expect, "masked-off elements must not be written");

        // Zero mask store against an unmapped page: no access at all.
        emu.reg_write(X86Reg::Rax, HOLE);
        emu.reg_write_ymm(X86Reg::Ymm2, none);
        run_one(emu, "maskmov store zero mask", store);

        // ---- fault suppression across a page boundary ----
        // Elements 0..3 are mapped, 4..7 are not.
        let low_only = ymm_from_u32([0x8000_0000, 0x8000_0000, 0x8000_0000, 0x8000_0000, 0, 0, 0, 0]);
        let high_only = ymm_from_u32([0, 0, 0, 0, 0x8000_0000, 0, 0, 0]);

        emu.mem_write(STRADDLE, &[0x11, 0x22, 0x33, 0x44]).expect("seed");
        emu.reg_write(X86Reg::Rax, STRADDLE);
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, low_only);
        run_one(emu, "maskmov load mapped half", load);
        assert_eq!(
            &emu.reg_read_ymm(X86Reg::Ymm1)[..4],
            &[0x11, 0x22, 0x33, 0x44],
            "elements in the mapped page must load"
        );
        assert_eq!(
            emu.cpu().get_exception_diag()[14],
            0,
            "masked-off elements in an unmapped page must not fault"
        );

        // Same address, but now a masked-in element lands in the hole.
        emu.reg_write(X86Reg::Rax, STRADDLE);
        emu.reg_write_ymm(X86Reg::Ymm2, high_only);
        let mut prog = load.to_vec();
        prog.extend_from_slice(&[0xEB, 0xFE]);
        emu.mem_write(CASE_BASE, &prog).expect("write code");
        emu.emu_start(CASE_BASE, Some(PF_HANDLER + 1), None, Some(20))
            .expect("emu_start");
        assert_eq!(
            emu.cpu().get_exception_diag()[14],
            1,
            "a masked-in element in an unmapped page must raise #PF"
        );
    });
}

// ════════════════════════════════════════════════════════════════════════
// AVX2 VSIB gathers. Beyond loading the right elements, the architectural
// contract is that the mask register is cleared element by element as the
// loads retire, so a mid-instruction #PF leaves a restartable state.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn vex_vgather_loads_elements_and_clears_the_mask() {
    with_avx_emu(|emu| {
        const TABLE: u64 = 0x0060_0000;

        // table[n] = 100 + n, as dwords.
        let mut table = [0u8; 64];
        for n in 0..16u32 {
            table[n as usize * 4..n as usize * 4 + 4].copy_from_slice(&(100 + n).to_le_bytes());
        }
        emu.mem_write(TABLE, &table).expect("write table");

        let signed = 0x8000_0000u32;

        // VPGATHERDD ymm1, [rax+ymm2*4], ymm3 — C4 E2 65 90 0C 90.
        // 65: W=0 vvvv=~3 L=1 pp=66. sib 90: scale=*4 index=ymm2 base=rax.
        let gather = &[0xC4u8, 0xE2, 0x65, 0x90, 0x0C, 0x90];

        emu.reg_write(X86Reg::Rax, TABLE);
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u32([7, 6, 5, 4, 3, 2, 1, 0]));
        emu.reg_write_ymm(X86Reg::Ymm3, ymm_from_u32([signed; 8]));
        run_one(emu, "vpgatherdd full", gather);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([107, 106, 105, 104, 103, 102, 101, 100]),
            "a fully-armed gather loads every element through its own index"
        );
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm3),
            [0u8; 32],
            "the mask register must be fully cleared when the gather completes"
        );

        // Partial mask: unarmed elements keep their previous destination value.
        emu.reg_write(X86Reg::Rax, TABLE);
        emu.reg_write_ymm(X86Reg::Ymm1, ymm_from_u32([9; 8]));
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u32([7, 6, 5, 4, 3, 2, 1, 0]));
        emu.reg_write_ymm(
            X86Reg::Ymm3,
            ymm_from_u32([signed, 0, signed, 0, signed, 0, signed, 0]),
        );
        run_one(emu, "vpgatherdd partial", gather);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([107, 9, 105, 9, 103, 9, 101, 9]),
            "masked-off elements must keep their previous destination value"
        );
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm3),
            [0u8; 32],
            "the mask must end fully cleared even for masked-off elements"
        );

        // Zero mask: destination untouched, and no memory is read at all —
        // the base points into an unmapped page below.
        emu.reg_write(X86Reg::Rax, TABLE);
        emu.reg_write_ymm(X86Reg::Ymm1, ymm_from_u32([42; 8]));
        emu.reg_write_ymm(X86Reg::Ymm3, [0u8; 32]);
        run_one(emu, "vpgatherdd zero mask", gather);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([42; 8]),
            "a zero mask must leave the destination untouched"
        );

        // VL128 form clears bits [255:128] of both destination and mask —
        // C4 E2 61 90 0C 90.
        emu.reg_write(X86Reg::Rax, TABLE);
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u32([3, 2, 1, 0, 0, 0, 0, 0]));
        emu.reg_write_ymm(X86Reg::Ymm3, ymm_from_u32([signed; 8]));
        run_one(emu, "vpgatherdd vl128", &[0xC4, 0xE2, 0x61, 0x90, 0x0C, 0x90]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([103, 102, 101, 100, 0, 0, 0, 0]),
            "VEX.128 gather clears the destination above 128 bits"
        );

        // VGATHERQPS: qword indices, dword results, four elements max, and
        // everything above 128 bits cleared — C4 E2 65 93 0C 90.
        emu.reg_write(X86Reg::Rax, TABLE);
        emu.reg_write_ymm(X86Reg::Ymm1, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u64([3, 2, 1, 0]));
        emu.reg_write_ymm(X86Reg::Ymm3, ymm_from_u32([signed; 8]));
        run_one(emu, "vgatherqps", &[0xC4, 0xE2, 0x65, 0x93, 0x0C, 0x90]);
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm1),
            ymm_from_u32([103, 102, 101, 100, 0, 0, 0, 0]),
            "VGATHERQPS produces four dwords and clears above 128 bits"
        );

        // #UD when destination, mask and VSIB index are not all distinct.
        // VPGATHERDD ymm1, [rax+ymm1*4], ymm3 — index == destination.
        emu.reg_write_ymm(X86Reg::Ymm3, ymm_from_u32([signed; 8]));
        let bad = &[0xC4u8, 0xE2, 0x65, 0x90, 0x0C, 0x88]; // sib 88: index=ymm1
        let mut prog = bad.to_vec();
        prog.extend_from_slice(&[0xEB, 0xFE]);
        emu.mem_write(CASE_BASE, &prog).expect("write code");
        let before = emu.cpu().get_exception_diag()[6];
        emu.emu_start(CASE_BASE, Some(CASE_BASE + bad.len() as u64), None, Some(4))
            .expect("emu_start");
        assert_eq!(
            emu.cpu().get_exception_diag()[6],
            before + 1,
            "a gather whose index equals its destination must #UD"
        );
    });
}

#[test]
fn vex_vgather_is_restartable_after_a_page_fault() {
    with_avx_emu(|emu| {
        const TABLE: u64 = 0x0060_0000;
        const HOLE: u64 = 0x00C0_0000;
        const IDT: u64 = 0x0028_0000;
        const PF_HANDLER: u64 = 0x0029_0000;
        const STACK: u64 = 0x0030_0000;

        let mut gate = [0u8; 16];
        gate[0..2].copy_from_slice(&(PF_HANDLER as u16).to_le_bytes());
        gate[2..4].copy_from_slice(&0x0008u16.to_le_bytes());
        gate[5] = 0x8e;
        gate[6..8].copy_from_slice(&((PF_HANDLER >> 16) as u16).to_le_bytes());
        gate[8..12].copy_from_slice(&((PF_HANDLER >> 32) as u32).to_le_bytes());
        emu.mem_write(IDT + 14 * 16, &gate).expect("write #PF gate");
        emu.mem_write(PF_HANDLER, &[0xF4]).expect("write handler");
        emu.mem_write(0x808, &0x00AF_9A00_0000_FFFFu64.to_le_bytes())
            .expect("long code descriptor");
        emu.reg_write(X86Reg::IdtrBase, IDT);
        emu.reg_write(X86Reg::IdtrLimit, 256 * 16 - 1);
        emu.reg_write(X86Reg::Rsp, STACK);
        emu.mem_write(flat_long64_pde_addr(HOLE), &0u64.to_le_bytes())
            .expect("unmap the hole page");

        let mut table = [0u8; 32];
        for n in 0..8u32 {
            table[n as usize * 4..n as usize * 4 + 4].copy_from_slice(&(200 + n).to_le_bytes());
        }
        emu.mem_write(TABLE, &table).expect("write table");

        // Elements 0 and 1 read from the table; element 2 reads from the
        // unmapped page. Index units are dwords (scale *4).
        let hole_index = ((HOLE - TABLE) / 4) as u32;
        emu.reg_write(X86Reg::Rax, TABLE);
        emu.reg_write_ymm(X86Reg::Ymm1, ymm_from_u32([0xDEAD_BEEF; 8]));
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u32([0, 1, hole_index, 3, 4, 5, 6, 7]));
        emu.reg_write_ymm(X86Reg::Ymm3, ymm_from_u32([0x8000_0000; 8]));

        let gather = &[0xC4u8, 0xE2, 0x65, 0x90, 0x0C, 0x90];
        let mut prog = gather.to_vec();
        prog.extend_from_slice(&[0xEB, 0xFE]);
        emu.mem_write(CASE_BASE, &prog).expect("write code");
        emu.emu_start(CASE_BASE, Some(PF_HANDLER + 1), None, Some(20))
            .expect("emu_start");

        assert_eq!(
            emu.cpu().get_exception_diag()[14],
            1,
            "the third element must raise #PF"
        );

        let dst = emu.reg_read_ymm(X86Reg::Ymm1);
        assert_eq!(
            &dst[..8],
            &ymm_from_u32([200, 201, 0, 0, 0, 0, 0, 0])[..8],
            "elements retired before the fault must be written"
        );

        // The mask records exactly what is left to do: elements 0 and 1 are
        // cleared, the faulting element and everything above it stay armed.
        let mask = emu.reg_read_ymm(X86Reg::Ymm3);
        assert_eq!(
            mask,
            ymm_from_u32([
                0,
                0,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF
            ]),
            "the mask must reflect exactly the elements still outstanding"
        );
    });
}

// ════════════════════════════════════════════════════════════════════════
// VEX forms that reach a legacy SSE handler must still clear the bits above
// their vector length. Bochs writes these through BX_WRITE_XMM_REGZ, which
// preserves the upper lane for legacy SSE but clears it for VEX. A handler
// that always preserves leaks stale YMM data — the VPINSR bug class, and
// VMOVD/VMOVQ are far more common than VPINSR.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn vex_legacy_shared_handlers_clear_the_upper_lane() {
    with_avx_emu(|emu| {
        const SCRATCH: u64 = 0x0060_0000;

        // VMOVD xmm0, eax — C5 F9 6E C0 (VEX.128.66.0F.W0 6E).
        emu.reg_write_ymm(X86Reg::Ymm0, [0xAA; 32]);
        emu.reg_write(X86Reg::Rax, 0x1234_5678);
        run_one(emu, "vmovd", &[0xC5, 0xF9, 0x6E, 0xC0]);
        let got = emu.reg_read_ymm(X86Reg::Ymm0);
        assert_eq!(&got[..4], &0x1234_5678u32.to_le_bytes(), "VMOVD value");
        assert_eq!(&got[4..], &[0u8; 28], "VMOVD must clear ymm0 bits [255:32]");

        // VMOVQ xmm0, rax — C4 E1 F9 6E C0 (VEX.128.66.0F.W1 6E).
        emu.reg_write_ymm(X86Reg::Ymm0, [0xAA; 32]);
        emu.reg_write(X86Reg::Rax, 0x0011_2233_4455_6677);
        run_one(emu, "vmovq", &[0xC4, 0xE1, 0xF9, 0x6E, 0xC0]);
        let got = emu.reg_read_ymm(X86Reg::Ymm0);
        assert_eq!(
            &got[..8],
            &0x0011_2233_4455_6677u64.to_le_bytes(),
            "VMOVQ value"
        );
        assert_eq!(&got[8..], &[0u8; 24], "VMOVQ must clear ymm0 bits [255:64]");

        // VMOVNTDQA xmm0, [rax] — C4 E2 79 2A 00 (VEX.128.66.0F38 2A).
        let payload: [u8; 32] = core::array::from_fn(|n| (n as u8).wrapping_mul(3) ^ 0x5A);
        emu.mem_write(SCRATCH, &payload).expect("write payload");
        emu.reg_write(X86Reg::Rax, SCRATCH);
        emu.reg_write_ymm(X86Reg::Ymm0, [0xAA; 32]);
        run_one(emu, "vmovntdqa", &[0xC4, 0xE2, 0x79, 0x2A, 0x00]);
        let got = emu.reg_read_ymm(X86Reg::Ymm0);
        assert_eq!(&got[..16], &payload[..16], "VMOVNTDQA value");
        assert_eq!(
            &got[16..],
            &[0u8; 16],
            "VEX.128 VMOVNTDQA must clear ymm0 bits [255:128]"
        );

        // VPCMPISTRM xmm1, xmm2, 0 — C4 E3 79 62 CA 00. The mask result goes
        // to XMM0, and the upper lane of YMM0 must be cleared.
        emu.reg_write_ymm(X86Reg::Ymm0, [0xAA; 32]);
        emu.reg_write_xmm(X86Reg::Xmm1, *b"abcdefghijklmnop");
        emu.reg_write_xmm(X86Reg::Xmm2, *b"abcdefghijklmnop");
        run_one(emu, "vpcmpistrm", &[0xC4, 0xE3, 0x79, 0x62, 0xCA, 0x00]);
        assert_eq!(
            &emu.reg_read_ymm(X86Reg::Ymm0)[16..],
            &[0u8; 16],
            "VPCMPISTRM must clear ymm0 bits [255:128]"
        );
    });
}

#[test]
fn vex_vpclmulqdq_sources_vvvv_per_lane() {
    with_avx_emu(|emu| {
        // Carry-less products chosen to be easy to verify by hand:
        //   2 (0b10) x 3 (0b11) = 0b110  = 6
        //   5 (0b101) x 7 (0b111) = 0b11011 = 27
        emu.reg_write_ymm(X86Reg::Ymm0, [0xAA; 32]);
        emu.reg_write_ymm(X86Reg::Ymm1, ymm_from_u64([2, 0, 5, 0]));
        emu.reg_write_ymm(X86Reg::Ymm2, ymm_from_u64([3, 0, 7, 0]));

        // VPCLMULQDQ xmm0, xmm1, xmm2, 0 — C4 E3 71 44 C2 00.
        run_one(emu, "vpclmulqdq vl128", &[0xC4, 0xE3, 0x71, 0x44, 0xC2, 0x00]);
        let got = emu.reg_read_ymm(X86Reg::Ymm0);
        assert_eq!(
            &got[..16],
            &ymm_from_u64([6, 0, 0, 0])[..16],
            "VPCLMULQDQ must multiply vvvv (xmm1) by rm (xmm2), not the destination"
        );
        assert_eq!(
            &got[16..],
            &[0u8; 16],
            "VEX.128 VPCLMULQDQ must clear ymm0 bits [255:128]"
        );

        // The 256-bit form belongs to the separate VPCLMULQDQ extension
        // (Bochs BX_ISA_VAES_VPCLMULQDQ), which Corei7SkylakeX does not
        // advertise — so the ISA gate must turn it into #UD, exactly as
        // Bochs's init_FetchDecodeTables does. The handler behind it is
        // exercised by vex_vpclmulqdq_vl256_runs_when_the_feature_is_present.
        emu.reg_write_ymm(X86Reg::Ymm0, [0xAA; 32]);
        let before = emu.cpu().get_exception_diag()[6];
        let mut prog = vec![0xC4u8, 0xE3, 0x75, 0x44, 0xC2, 0x00];
        prog.extend_from_slice(&[0xEB, 0xFE]);
        emu.mem_write(CASE_BASE, &prog).expect("write code");
        emu.emu_start(CASE_BASE, Some(CASE_BASE + 6), None, Some(4))
            .expect("emu_start");
        assert_eq!(
            emu.cpu().get_exception_diag()[6],
            before + 1,
            "VEX.256 VPCLMULQDQ must #UD on a model without the VPCLMULQDQ feature"
        );
        assert_eq!(
            emu.reg_read_ymm(X86Reg::Ymm0),
            [0xAA; 32],
            "a #UD must leave the destination untouched"
        );
    });
}

#[test]
fn pextrw_to_rm16_extracts_the_selected_word() {
    with_avx_emu(|emu| {
        const SCRATCH: u64 = 0x0060_0000;
        // Words 0..7 = 0x1100, 0x3322, ... so the selector is unambiguous.
        let src: [u8; 16] = core::array::from_fn(|n| (n as u8) | ((n as u8) << 4));
        emu.reg_write_xmm(X86Reg::Xmm1, src);

        // PEXTRW eax, xmm1, 3 — 66 0F 3A 15 C8 03 (legacy, register form).
        emu.reg_write(X86Reg::Rax, 0xFFFF_FFFF_FFFF_FFFF);
        run_one(emu, "pextrw r", &[0x66, 0x0F, 0x3A, 0x15, 0xC8, 0x03]);
        let want = u32::from(u16::from_le_bytes([src[6], src[7]]));
        assert_eq!(
            emu.reg_read(X86Reg::Rax),
            u64::from(want),
            "PEXTRW must zero-extend the selected word into the full GPR"
        );

        // PEXTRW [rax], xmm1, 5 — 66 0F 3A 15 08 05 (memory form).
        emu.mem_write(SCRATCH, &[0x5A; 4]).expect("poison");
        emu.reg_write(X86Reg::Rax, SCRATCH);
        run_one(emu, "pextrw m", &[0x66, 0x0F, 0x3A, 0x15, 0x08, 0x05]);
        let mut back = [0u8; 4];
        emu.mem_read(SCRATCH, &mut back).expect("read back");
        assert_eq!(&back[..2], &src[10..12], "PEXTRW memory form writes 2 bytes");
        assert_eq!(&back[2..], &[0x5A; 2], "PEXTRW must not write past the word");
    });
}

#[test]
fn indexbyte_avx2_ingredients() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");
            emu.reg_write(
                X86Reg::Cr4,
                emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
            );
            enable_guest_avx_state(&mut emu);

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
        .stack_size(TEST_STACK_SIZE)
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
    enable_guest_avx_state(&mut emu);

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

    let hash_at = |emu: &mut Emulator<'static, Corei7SkylakeX>, addr: u64| -> [u8; 16] {
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
        .stack_size(TEST_STACK_SIZE)
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
    enable_guest_avx_state(&mut emu);

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
        .stack_size(TEST_STACK_SIZE)
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
    enable_guest_avx_state(&mut emu);

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
        .stack_size(TEST_STACK_SIZE)
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
    enable_guest_avx_state(&mut emu);

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
        .stack_size(TEST_STACK_SIZE)
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
    enable_guest_avx_state(&mut emu);

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
        .stack_size(TEST_STACK_SIZE)
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
    enable_guest_avx_state(&mut emu);
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

// ════════════════════════════════════════════════════════════════════════
// End-to-end proof that AVX-512 is reachable by a guest.
//
// Every link in the chain has to hold: CPUID must advertise AVX512F so the
// ISA gate resolves the opcode instead of rewriting it to IaError; XSETBV
// must accept the OPMASK|ZMM_HI256|HI_ZMM bits, which needs leaf 0xD to
// report them in xcr0_suppmask; handle_avx_mode_change must then open
// EVEX_OK so decode does not substitute BxNoEVEX; and the handler must run.
// Any one of those missing turns this into a #UD.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn evex_is_reachable_by_a_guest_end_to_end() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");
            emu.reg_write(
                X86Reg::Cr4,
                emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
            );

            // XSETBV with the full AVX-512 XCR0: FPU|SSE|YMM|OPMASK|ZMM_HI256|HI_ZMM.
            // This #GPs unless CPUID leaf 0xD advertises all six.
            emu.reg_write(X86Reg::Rax, 0xE7);
            emu.reg_write(X86Reg::Rcx, 0);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1])
                .expect("write xsetbv");
            emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                .expect("XSETBV with the AVX-512 XCR0 bits must succeed");

            // VPADDD zmm0, zmm1, zmm2 = EVEX.512.66.0F.W0 FE /r
            //   62 F1 75 48 FE C2
            //   P0=F1: mm=01 (0F map), R/X/B/R' inverted-high
            //   P1=75: W=0, vvvv=1110 -> zmm1, pp=01 (66)
            //   P2=48: L'L=10 (512-bit), z=0, b=0, V'=1(inv->0), aaa=000
            //   ModRM C2: reg=zmm0, rm=zmm2
            emu.mem_write(CASE_BASE, &[0x62, 0xF1, 0x75, 0x48, 0xFE, 0xC2, 0xEB, 0xFE])
                .expect("write vpaddd");

            let mut a = [0u8; 64];
            let mut b = [0u8; 64];
            for lane in 0..16 {
                a[lane * 4..lane * 4 + 4].copy_from_slice(&(lane as u32 + 1).to_le_bytes());
                b[lane * 4..lane * 4 + 4].copy_from_slice(&(100u32).to_le_bytes());
            }
            emu.reg_write_zmm(X86Reg::Zmm1, a);
            emu.reg_write_zmm(X86Reg::Zmm2, b);
            emu.reg_write_zmm(X86Reg::Zmm0, [0xAA; 64]);

            emu.emu_start(CASE_BASE, None, None, Some(1))
                .expect("EVEX VPADDD must execute, not #UD");

            let got = emu.reg_read_zmm(X86Reg::Zmm0);
            for lane in 0..16 {
                let v = u32::from_le_bytes(got[lane * 4..lane * 4 + 4].try_into().unwrap());
                assert_eq!(
                    v,
                    lane as u32 + 101,
                    "zmm0 dword {lane}: all 16 lanes must be added, including those above 256 bits"
                );
            }
        })
        .unwrap()
        .join()
        .expect("join test thread");
}

// ════════════════════════════════════════════════════════════════════════
// XSAVE/XRSTOR of the AVX-512 components.
//
// Advertising AVX-512 puts XCR0 bits 5/6/7 (OPMASK, ZMM_HI256, HI_ZMM) in
// reach for the first time, so the kernel starts saving and restoring 2688
// bytes of state on every context switch through code paths no guest could
// previously execute. A round-trip that loses or misplaces bytes corrupts
// vector state across a switch, which shows up as userspace computing
// garbage rather than as any fault.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn xsave_xrstor_round_trips_the_avx512_components() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            const BUF: u64 = CASE_BASE + 0x1_0000; // 64-byte aligned scratch

            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");
            emu.reg_write(
                X86Reg::Cr4,
                emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
            );

            // XSETBV: XCR0 = FPU|SSE|YMM|OPMASK|ZMM_HI256|HI_ZMM
            emu.reg_write(X86Reg::Rax, 0xE7);
            emu.reg_write(X86Reg::Rcx, 0);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1]).expect("write xsetbv");
            emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                .expect("enable AVX-512 guest state");

            // A pattern that differs in every 64-bit lane, so a swapped or
            // dropped lane cannot coincidentally compare equal.
            let pattern = |reg: u64| {
                let mut v = [0u8; 64];
                for lane in 0..8 {
                    let w = 0xC0DE_0000_0000_0000u64 | (reg << 32) | lane as u64;
                    v[lane * 8..lane * 8 + 8].copy_from_slice(&w.to_le_bytes());
                }
                v
            };
            for (i, reg) in [X86Reg::Zmm0, X86Reg::Zmm1, X86Reg::Zmm15].iter().enumerate() {
                emu.reg_write_zmm(*reg, pattern(i as u64));
            }

            // XSAVE [rdi] with RFBM = 0xE7, then clobber, then XRSTOR [rdi].
            emu.reg_write(X86Reg::Rdi, BUF);
            emu.reg_write(X86Reg::Rax, 0xE7);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CASE_BASE, &[0x0F, 0xAE, 0x27, 0xEB, 0xFE]).expect("write xsave");
            emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                .expect("XSAVE must not fault");

            for reg in [X86Reg::Zmm0, X86Reg::Zmm1, X86Reg::Zmm15] {
                emu.reg_write_zmm(reg, [0x5A; 64]);
            }

            emu.reg_write(X86Reg::Rdi, BUF);
            emu.reg_write(X86Reg::Rax, 0xE7);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CASE_BASE, &[0x0F, 0xAE, 0x2F, 0xEB, 0xFE]).expect("write xrstor");
            emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                .expect("XRSTOR must not fault");

            for (i, reg) in [X86Reg::Zmm0, X86Reg::Zmm1, X86Reg::Zmm15].iter().enumerate() {
                let got = emu.reg_read_zmm(*reg);
                let want = pattern(i as u64);
                assert_eq!(
                    got, want,
                    "{reg:?} did not survive the XSAVE/XRSTOR round trip \
                     (upper lanes live in ZMM_HI256 at offset 1152)"
                );
            }
        })
        .unwrap()
        .join()
        .expect("join test thread");
}

// ════════════════════════════════════════════════════════════════════════
// The EVEX opcodes an Ubuntu guest actually executes during boot.
//
// Captured with a first-seen probe against ubuntu-26.04-live-server: nine
// distinct opcodes run before init dies. VPTERNLOGD is the sharpest test of
// the three-source operand order, because imm8 can select a source
// outright: 0xF0 yields A (dest), 0xCC yields B (EVEX.vvvv), 0xAA yields C
// (the rm operand). A handler that swaps B and C returns the wrong vector
// with no fault, which is exactly the failure shape being chased.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn evex_vpternlogd_selects_the_right_source_for_each_imm8() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let a = {
                let mut v = [0u8; 64];
                for i in 0..16 { v[i * 4..i * 4 + 4].copy_from_slice(&0xAAAA_0000u32.to_le_bytes()); }
                v
            };
            let b = {
                let mut v = [0u8; 64];
                for i in 0..16 { v[i * 4..i * 4 + 4].copy_from_slice(&0xBBBB_1111u32.to_le_bytes()); }
                v
            };
            let c = {
                let mut v = [0u8; 64];
                for i in 0..16 { v[i * 4..i * 4 + 4].copy_from_slice(&0xCCCC_2222u32.to_le_bytes()); }
                v
            };

            // VPTERNLOGD zmm0, zmm1, zmm2, imm8 = EVEX.512.66.0F3A.W0 25 /r ib
            //   62 F3 75 48 25 C2 ib   (dest=zmm0=A, vvvv=zmm1=B, rm=zmm2=C)
            for (imm, want, which) in [
                (0xF0u8, a, "A (the destination operand)"),
                (0xCC, b, "B (EVEX.vvvv)"),
                (0xAA, c, "C (the rm operand)"),
            ] {
                let cfg = EmulatorConfig::default();
                let mut emu =
                    Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                        .expect("new emulator");
                emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18));
                emu.reg_write(X86Reg::Rax, 0xE7);
                emu.reg_write(X86Reg::Rcx, 0);
                emu.reg_write(X86Reg::Rdx, 0);
                emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1]).expect("write xsetbv");
                emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                    .expect("enable AVX-512 state");

                emu.reg_write_zmm(X86Reg::Zmm0, a);
                emu.reg_write_zmm(X86Reg::Zmm1, b);
                emu.reg_write_zmm(X86Reg::Zmm2, c);

                emu.mem_write(CASE_BASE, &[0x62, 0xF3, 0x75, 0x48, 0x25, 0xC2, imm, 0xEB, 0xFE])
                    .expect("write vpternlogd");
                emu.emu_start(CASE_BASE, None, None, Some(1))
                    .expect("VPTERNLOGD must execute");

                let got = emu.reg_read_zmm(X86Reg::Zmm0);
                let g = u32::from_le_bytes(got[0..4].try_into().unwrap());
                let w = u32::from_le_bytes(want[0..4].try_into().unwrap());
                assert_eq!(
                    g, w,
                    "VPTERNLOGD imm8={imm:#04X} must yield source {which}: got {g:#010X}, want {w:#010X}"
                );
            }
        })
        .unwrap()
        .join()
        .expect("join test thread");
}

#[test]
fn evex_vprord_rotates_each_dword_right() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");
            emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18));
            emu.reg_write(X86Reg::Rax, 0xE7);
            emu.reg_write(X86Reg::Rcx, 0);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1]).expect("write xsetbv");
            emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                .expect("enable AVX-512 state");

            let mut src = [0u8; 64];
            for i in 0..16 {
                src[i * 4..i * 4 + 4].copy_from_slice(&0x1234_5678u32.to_le_bytes());
            }
            emu.reg_write_zmm(X86Reg::Zmm2, src);
            emu.reg_write_zmm(X86Reg::Zmm1, [0x5A; 64]);

            // VPRORD zmm1, zmm2, 8 = EVEX.512.66.0F.W0 72 /0 ib (vvvv is the dest)
            emu.mem_write(CASE_BASE, &[0x62, 0xF1, 0x75, 0x48, 0x72, 0xC2, 0x08, 0xEB, 0xFE])
                .expect("write vprord");
            emu.emu_start(CASE_BASE, None, None, Some(1))
                .expect("VPRORD must execute");

            let got = emu.reg_read_zmm(X86Reg::Zmm1);
            for lane in 0..16 {
                let v = u32::from_le_bytes(got[lane * 4..lane * 4 + 4].try_into().unwrap());
                assert_eq!(
                    v, 0x7812_3456,
                    "dword {lane}: 0x12345678 rotated right by 8 is 0x78123456"
                );
            }
        })
        .unwrap()
        .join()
        .expect("join test thread");
}

#[test]
fn evex_vpermi2d_reads_the_first_table_from_vvvv() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");
            emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18));
            emu.reg_write(X86Reg::Rax, 0xE7);
            emu.reg_write(X86Reg::Rcx, 0);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1]).expect("write xsetbv");
            emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                .expect("enable AVX-512 state");

            // Indices 0..15 all have bit 4 clear, so every element must come
            // from SRC1 (EVEX.vvvv), per the SDM's
            //   DEST := TMP_DEST[i+log2(KL)] ? SRC2 : SRC1
            let mut idx = [0u8; 64];
            let mut t1 = [0u8; 64];
            let mut t2 = [0u8; 64];
            for i in 0..16 {
                idx[i * 4..i * 4 + 4].copy_from_slice(&(i as u32).to_le_bytes());
                t1[i * 4..i * 4 + 4].copy_from_slice(&(0xAAAA_0000u32 + i as u32).to_le_bytes());
                t2[i * 4..i * 4 + 4].copy_from_slice(&(0xBBBB_0000u32 + i as u32).to_le_bytes());
            }
            emu.reg_write_zmm(X86Reg::Zmm0, idx);
            emu.reg_write_zmm(X86Reg::Zmm1, t1);
            emu.reg_write_zmm(X86Reg::Zmm2, t2);

            // VPERMI2D zmm0, zmm1, zmm2 = EVEX.512.66.0F38.W0 76 /r
            emu.mem_write(CASE_BASE, &[0x62, 0xF2, 0x75, 0x48, 0x76, 0xC2, 0xEB, 0xFE])
                .expect("write vpermi2d");
            emu.emu_start(CASE_BASE, None, None, Some(1))
                .expect("VPERMI2D must execute");

            let got = emu.reg_read_zmm(X86Reg::Zmm0);
            for lane in 0..16 {
                let v = u32::from_le_bytes(got[lane * 4..lane * 4 + 4].try_into().unwrap());
                assert_eq!(
                    v,
                    0xAAAA_0000u32 + lane as u32,
                    "dword {lane}: index {lane} has bit 4 clear so it selects SRC1 (vvvv)"
                );
            }
        })
        .unwrap()
        .join()
        .expect("join test thread");
}

// VPCMPEQB writing an opmask is the core of glibc's strlen/memchr: a wrong
// mask bit means a wrong string length, with no fault anywhere. Read back
// through KMOVQ so the opmask write path is exercised end to end.
#[test]
fn evex_vpcmpeqb_sets_one_opmask_bit_per_equal_byte() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");
            emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18));
            emu.reg_write(X86Reg::Rax, 0xE7);
            emu.reg_write(X86Reg::Rcx, 0);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1]).expect("write xsetbv");
            emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                .expect("enable AVX-512 state");

            // 32 bytes of 0x41, with bytes 3 and 17 differing in the second
            // operand, so exactly those two mask bits must be clear.
            let a = [0x41u8; 64];
            let mut b = [0x41u8; 64];
            b[3] = 0x42;
            b[17] = 0x42;
            emu.reg_write_zmm(X86Reg::Zmm0, a);
            emu.reg_write_zmm(X86Reg::Zmm1, b);
            emu.reg_write(X86Reg::Rax, 0xDEAD_BEEF);

            // VPCMPEQB k1, ymm0, ymm1   62 F1 7D 28 74 C9
            // KMOVQ    rax, k1          C4 E1 FB 93 C1
            emu.mem_write(
                CASE_BASE,
                &[0x62, 0xF1, 0x7D, 0x28, 0x74, 0xC9, 0xC4, 0xE1, 0xFB, 0x93, 0xC1, 0xEB, 0xFE],
            )
            .expect("write vpcmpeqb + kmovq");
            let stop = emu
                .emu_start(CASE_BASE, None, None, Some(2))
                .expect("VPCMPEQB and KMOVQ must execute");
            let rip = emu.reg_read(X86Reg::Rip);

            let got = emu.reg_read(X86Reg::Rax);
            let want: u64 = (!((1u64 << 3) | (1u64 << 17))) & 0xFFFF_FFFF;
            assert_eq!(
                got, want,
                "k1 = {got:#018X}, want {want:#018X}: one bit per byte, 32 bits at VL256, \
                 clear only where the bytes differ. stop={stop:?} rip={rip:#X} \
                 (base={CASE_BASE:#X}, +6 after vpcmpeqb, +11 after kmovq)"
            );
        })
        .unwrap()
        .join()
        .expect("join test thread");
}

// Store-form EVEX opcodes (VEXTRACT*, VPMOV* truncating stores, VCOMPRESS*,
// VPEXTR*) write the rm operand and read the reg field — the opposite of the
// usual vector form. Upstream marks this by leading with OP_W*/OP_E*:
// EVEX_VEXTRACTF32x4_WpsVpsIb is Wps (rm) then Vps (reg).
#[test]
fn evex_vextractf32x4_writes_the_rm_operand() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");
            emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18));
            emu.reg_write(X86Reg::Rax, 0xE7);
            emu.reg_write(X86Reg::Rcx, 0);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1]).expect("write xsetbv");
            emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                .expect("enable AVX-512 state");

            // zmm1 = four distinguishable 128-bit lanes.
            let mut src = [0u8; 64];
            for lane in 0..4 {
                for d in 0..4 {
                    let v = 0x1000_0000u32 * (lane as u32 + 1) + d as u32;
                    let off = lane * 16 + d * 4;
                    src[off..off + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
            emu.reg_write_zmm(X86Reg::Zmm1, src);
            emu.reg_write_zmm(X86Reg::Zmm2, [0x5A; 64]);

            // VEXTRACTF32X4 xmm2, zmm1, 1 = EVEX.512.66.0F3A.W0 19 /r ib
            // reg=zmm1 (source), rm=zmm2 (destination), imm8=1 selects lane 1.
            emu.mem_write(
                CASE_BASE,
                &[0x62, 0xF3, 0x7D, 0x48, 0x19, 0xCA, 0x01, 0xEB, 0xFE],
            )
            .expect("write vextractf32x4");
            emu.emu_start(CASE_BASE, None, None, Some(1))
                .expect("VEXTRACTF32X4 must execute");

            let got = emu.reg_read_zmm(X86Reg::Zmm2);
            let v = u32::from_le_bytes(got[0..4].try_into().unwrap());
            assert_eq!(
                v, 0x2000_0000,
                "xmm2 dword 0 must hold zmm1's lane 1; the destination is the \
                 rm operand, not the reg field"
            );
        })
        .unwrap()
        .join()
        .expect("join test thread");
}

// The 0F 7A / 0F 7B EVEX conversions became reachable only when the legacy
// UD64 list stopped being applied to EVEX encodings. VCVTUDQ2PD is the
// sharpest of them: it is the *unsigned* conversion, so a signed handler
// turns the top half of the range negative.
#[test]
fn evex_vcvtudq2pd_converts_unsigned() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");
            emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18));
            emu.reg_write(X86Reg::Rax, 0xE7);
            emu.reg_write(X86Reg::Rcx, 0);
            emu.reg_write(X86Reg::Rdx, 0);
            emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1]).expect("write xsetbv");
            emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                .expect("enable AVX-512 state");

            let src_vals: [u32; 4] = [1, 2, 0x8000_0000, 0xFFFF_FFFF];
            let mut src = [0u8; 64];
            for (i, v) in src_vals.iter().enumerate() {
                src[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
            emu.reg_write_zmm(X86Reg::Zmm2, src);
            emu.reg_write_zmm(X86Reg::Zmm1, [0x5A; 64]);

            // VCVTUDQ2PD ymm1, xmm2 = EVEX.256.F3.0F.W0 7A /r
            emu.mem_write(CASE_BASE, &[0x62, 0xF1, 0x7E, 0x28, 0x7A, 0xCA, 0xEB, 0xFE])
                .expect("write vcvtudq2pd");
            emu.emu_start(CASE_BASE, None, None, Some(1))
                .expect("VCVTUDQ2PD must execute");

            let got = emu.reg_read_zmm(X86Reg::Zmm1);
            for (i, v) in src_vals.iter().enumerate() {
                let d = f64::from_le_bytes(got[i * 8..i * 8 + 8].try_into().unwrap());
                assert_eq!(
                    d, *v as f64,
                    "qword {i}: {v:#010X} must convert as unsigned to {}, got {d}",
                    *v as f64
                );
            }
        })
        .unwrap()
        .join()
        .expect("join test thread");
}

// The float-to-qword conversions at 0F 7A / 0F 7B. These produce integers
// that callers use as indices and offsets, so a wrong result turns into a
// bad pointer rather than a visibly wrong number. 7A truncates, 7B rounds
// to nearest even — testing both against the same inputs also catches the
// two being swapped.
#[test]
fn evex_float_to_qword_conversions() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            // (name, bytes, source bytes, expected two qwords)
            let ps = |a: f32, b: f32| {
                let mut v = [0u8; 64];
                v[0..4].copy_from_slice(&a.to_le_bytes());
                v[4..8].copy_from_slice(&b.to_le_bytes());
                v
            };
            let pd = |a: f64, b: f64| {
                let mut v = [0u8; 64];
                v[0..8].copy_from_slice(&a.to_le_bytes());
                v[8..16].copy_from_slice(&b.to_le_bytes());
                v
            };
            let cases: &[(&str, [u8; 6], [u8; 64], [i64; 2])] = &[
                ("VCVTTPS2QQ", [0x62, 0xF1, 0x7D, 0x08, 0x7A, 0xCA], ps(1.5, -2.7), [1, -2]),
                ("VCVTPS2QQ", [0x62, 0xF1, 0x7D, 0x08, 0x7B, 0xCA], ps(1.5, -2.7), [2, -3]),
                ("VCVTTPD2QQ", [0x62, 0xF1, 0xFD, 0x08, 0x7A, 0xCA], pd(1.5, -2.7), [1, -2]),
                ("VCVTPD2QQ", [0x62, 0xF1, 0xFD, 0x08, 0x7B, 0xCA], pd(1.5, -2.7), [2, -3]),
            ];

            for (name, enc, src, want) in cases {
                let cfg = EmulatorConfig::default();
                let mut emu =
                    Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                        .expect("new emulator");
                emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18));
                emu.reg_write(X86Reg::Rax, 0xE7);
                emu.reg_write(X86Reg::Rcx, 0);
                emu.reg_write(X86Reg::Rdx, 0);
                emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1]).expect("xsetbv");
                emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                    .expect("enable AVX-512 state");

                emu.reg_write_zmm(X86Reg::Zmm2, *src);
                emu.reg_write_zmm(X86Reg::Zmm1, [0x5A; 64]);

                let mut code = [0u8; 8];
                code[..6].copy_from_slice(enc);
                code[6] = 0xEB;
                code[7] = 0xFE;
                emu.mem_write(CASE_BASE, &code).expect("write conversion");
                emu.emu_start(CASE_BASE, None, None, Some(1))
                    .unwrap_or_else(|e| panic!("{name} must execute: {e:?}"));

                let got = emu.reg_read_zmm(X86Reg::Zmm1);
                for lane in 0..2 {
                    let v = i64::from_le_bytes(got[lane * 8..lane * 8 + 8].try_into().unwrap());
                    assert_eq!(v, want[lane], "{name} qword {lane}");
                }
            }
        })
        .unwrap()
        .join()
        .expect("join test thread");
}

// The remaining unsigned conversions at 0F 7A. Values above 2^31 (dword) and
// 2^63 (qword) are where a signed handler diverges, so each case includes one.
#[test]
fn evex_unsigned_int_to_float_conversions() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            fn mk(emu: &mut Emulator<'static, Corei7SkylakeX>) {
                emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18));
                emu.reg_write(X86Reg::Rax, 0xE7);
                emu.reg_write(X86Reg::Rcx, 0);
                emu.reg_write(X86Reg::Rdx, 0);
                emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1]).expect("xsetbv");
                emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                    .expect("enable AVX-512 state");
            }

            // VCVTUQQ2PD xmm1, xmm2 — EVEX.128.F3.0F.W1 7A
            {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(), CpuSetupMode::FlatLong64).expect("emu");
                mk(&mut emu);
                let vals: [u64; 2] = [5, 0x8000_0000_0000_0000];
                let mut src = [0u8; 64];
                for (i, v) in vals.iter().enumerate() {
                    src[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
                }
                emu.reg_write_zmm(X86Reg::Zmm2, src);
                emu.reg_write_zmm(X86Reg::Zmm1, [0x5A; 64]);
                emu.mem_write(CASE_BASE, &[0x62, 0xF1, 0xFE, 0x08, 0x7A, 0xCA, 0xEB, 0xFE])
                    .expect("write");
                emu.emu_start(CASE_BASE, None, None, Some(1)).expect("VCVTUQQ2PD");
                let got = emu.reg_read_zmm(X86Reg::Zmm1);
                for (i, v) in vals.iter().enumerate() {
                    let d = f64::from_le_bytes(got[i * 8..i * 8 + 8].try_into().unwrap());
                    assert_eq!(d, *v as f64, "VCVTUQQ2PD qword {i} ({v:#018X})");
                }
            }

            // VCVTUDQ2PS xmm1, xmm2 — EVEX.128.F2.0F.W0 7A
            {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(), CpuSetupMode::FlatLong64).expect("emu");
                mk(&mut emu);
                let vals: [u32; 4] = [1, 7, 0x8000_0000, 0xFFFF_FF00];
                let mut src = [0u8; 64];
                for (i, v) in vals.iter().enumerate() {
                    src[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
                emu.reg_write_zmm(X86Reg::Zmm2, src);
                emu.reg_write_zmm(X86Reg::Zmm1, [0x5A; 64]);
                emu.mem_write(CASE_BASE, &[0x62, 0xF1, 0x7F, 0x08, 0x7A, 0xCA, 0xEB, 0xFE])
                    .expect("write");
                emu.emu_start(CASE_BASE, None, None, Some(1)).expect("VCVTUDQ2PS");
                let got = emu.reg_read_zmm(X86Reg::Zmm1);
                for (i, v) in vals.iter().enumerate() {
                    let f = f32::from_le_bytes(got[i * 4..i * 4 + 4].try_into().unwrap());
                    assert_eq!(f, *v as f32, "VCVTUDQ2PS dword {i} ({v:#010X})");
                }
            }

            // VCVTUQQ2PS xmm1, xmm2 — EVEX.128.F2.0F.W1 7A (2 qwords -> 2 floats)
            {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(), CpuSetupMode::FlatLong64).expect("emu");
                mk(&mut emu);
                let vals: [u64; 2] = [9, 0xFFFF_FFFF_0000_0000];
                let mut src = [0u8; 64];
                for (i, v) in vals.iter().enumerate() {
                    src[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
                }
                emu.reg_write_zmm(X86Reg::Zmm2, src);
                emu.reg_write_zmm(X86Reg::Zmm1, [0x5A; 64]);
                emu.mem_write(CASE_BASE, &[0x62, 0xF1, 0xFF, 0x08, 0x7A, 0xCA, 0xEB, 0xFE])
                    .expect("write");
                emu.emu_start(CASE_BASE, None, None, Some(1)).expect("VCVTUQQ2PS");
                let got = emu.reg_read_zmm(X86Reg::Zmm1);
                for (i, v) in vals.iter().enumerate() {
                    let f = f32::from_le_bytes(got[i * 4..i * 4 + 4].try_into().unwrap());
                    assert_eq!(f, *v as f32, "VCVTUQQ2PS qword {i} ({v:#018X})");
                }
            }
        })
        .unwrap()
        .join()
        .expect("join test thread");
}

// VCVTUSI2SS / VCVTUSI2SD at 0F 7B take their source from a GPR and are the
// unsigned forms, so 0xFFFFFFFF and 0xFFFFFFFFFFFFFFFF must convert to large
// positives rather than -1.
#[test]
fn evex_vcvtusi2_scalar_conversions() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            // (name, encoding, rdx value, 32-bit source?, expected as f64)
            let cases: &[(&str, [u8; 6], u64, bool)] = &[
                // VCVTUSI2SD xmm1, xmm0, rdx  — EVEX.F2.0F.W1 7B
                ("VCVTUSI2SD r64", [0x62, 0xF1, 0xFF, 0x08, 0x7B, 0xCA], 0xFFFF_FFFF_FFFF_FFFF, false),
                // VCVTUSI2SD xmm1, xmm0, edx  — EVEX.F2.0F.W0 7B
                ("VCVTUSI2SD r32", [0x62, 0xF1, 0x7F, 0x08, 0x7B, 0xCA], 0xFFFF_FFFF, true),
                // VCVTUSI2SS xmm1, xmm0, rdx  — EVEX.F3.0F.W1 7B
                ("VCVTUSI2SS r64", [0x62, 0xF1, 0xFE, 0x08, 0x7B, 0xCA], 0xFFFF_FFFF_FFFF_FFFF, false),
                // VCVTUSI2SS xmm1, xmm0, edx  — EVEX.F3.0F.W0 7B
                ("VCVTUSI2SS r32", [0x62, 0xF1, 0x7E, 0x08, 0x7B, 0xCA], 0xFFFF_FFFF, true),
            ];

            for (name, enc, val, is32) in cases {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatLong64,
                )
                .expect("new emulator");
                emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18));
                emu.reg_write(X86Reg::Rax, 0xE7);
                emu.reg_write(X86Reg::Rcx, 0);
                emu.reg_write(X86Reg::Rdx, 0);
                emu.mem_write(CASE_BASE, &[0x0F, 0x01, 0xD1]).expect("xsetbv");
                emu.emu_start(CASE_BASE, Some(CASE_BASE + 3), None, Some(1))
                    .expect("enable AVX-512 state");

                emu.reg_write(X86Reg::Rdx, *val);
                emu.reg_write_zmm(X86Reg::Zmm0, [0u8; 64]);
                emu.reg_write_zmm(X86Reg::Zmm1, [0x5A; 64]);

                let mut code = [0u8; 8];
                code[..6].copy_from_slice(enc);
                code[6] = 0xEB;
                code[7] = 0xFE;
                emu.mem_write(CASE_BASE, &code).expect("write conversion");
                emu.emu_start(CASE_BASE, None, None, Some(1))
                    .unwrap_or_else(|e| panic!("{name} must execute: {e:?}"));

                let got = emu.reg_read_zmm(X86Reg::Zmm1);
                let want = if *is32 { (*val as u32) as f64 } else { *val as f64 };
                let actual = if name.contains("SS") {
                    f32::from_le_bytes(got[0..4].try_into().unwrap()) as f64
                } else {
                    f64::from_le_bytes(got[0..8].try_into().unwrap())
                };
                let want = if name.contains("SS") { want as f32 as f64 } else { want };
                assert_eq!(
                    actual, want,
                    "{name}: {val:#X} is unsigned, so it must convert to {want}, not a negative"
                );
            }
        })
        .unwrap()
        .join()
        .expect("join test thread");
}
