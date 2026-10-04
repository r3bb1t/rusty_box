//! The six conversions between MMX integers and packed floats — CVTPI2PS,
//! CVTPI2PD, CVTTPS2PI, CVTTPD2PI, CVTPS2PI, CVTPD2PI (Bochs cpu/sse_pfp.cc) —
//! run as guest instructions.
//!
//! Besides the values, each pins the x87 side effect Bochs gives it: a form
//! that reads or writes an MMX register makes the FPU-to-MMX transition (TOS
//! to 0, every tag valid), and a memory source touches no MMX register and
//! leaves the x87 state alone. The 128-bit memory source of CVTPD2PI must be
//! 16-byte aligned.

#![cfg(feature = "std")]

use rusty_box::cpu::{CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
const CODE: u64 = 0x0020_0000;
/// Operands. 16-byte aligned, so `DATA + 8` is a misaligned 16-byte operand.
const DATA: u64 = 0x0021_0000;
/// Where results are stored for the host to read back.
const OUT: u64 = 0x0022_0000;
/// Where FNSTENV writes the x87 environment.
const ENV: u64 = 0x0023_0000;
const IDT: u64 = 0x0024_0000;
/// #GP handler: a HLT the run stops on.
const GP_HANDLER: u64 = 0x0025_0000;
const STACK_TOP: u64 = 0x0026_0000;

/// Flat long mode with CR4.OSFXSR set, so legacy SSE executes, and a #GP gate.
fn sse_emulator() -> Box<Emulator> {
    let mut emu = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
        .expect("emulator");
    emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | (1 << 9));
    emu.reg_write(X86Reg::IdtrBase, IDT);
    emu.reg_write(X86Reg::IdtrLimit, 256 * 16 - 1);
    emu.reg_write(X86Reg::Rsp, STACK_TOP);
    let mut gate = [0u8; 16];
    gate[0..2].copy_from_slice(&(GP_HANDLER as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&0x0008u16.to_le_bytes());
    gate[5] = 0x8E;
    gate[6..8].copy_from_slice(&((GP_HANDLER >> 16) as u16).to_le_bytes());
    gate[8..12].copy_from_slice(&((GP_HANDLER >> 32) as u32).to_le_bytes());
    emu.mem_write(IDT + 13 * 16, &gate).expect("idt");
    emu.mem_write(GP_HANDLER, &[0xF4]).expect("handler");
    emu
}

/// A 32-bit absolute address in a ModRM + SIB memory operand.
fn abs(address: u64) -> [u8; 4] {
    u32::try_from(address).expect("below 4 GiB").to_le_bytes()
}

/// `movq mm1, [address]`: 0F 6F /r, ModRM 0x0C (mm1, SIB), SIB 0x25.
fn movq_mm1_from(address: u64) -> Vec<u8> {
    [&[0x0F, 0x6F, 0x0C, 0x25][..], &abs(address)].concat()
}

/// `movq [address], mm0`: 0F 7F /r, ModRM 0x04 (mm0, SIB), SIB 0x25.
fn movq_mm0_to(address: u64) -> Vec<u8> {
    [&[0x0F, 0x7F, 0x04, 0x25][..], &abs(address)].concat()
}

/// `fnstenv [address]`: D9 /6, ModRM 0x34, SIB 0x25.
fn fnstenv(address: u64) -> Vec<u8> {
    [&[0xD9, 0x34, 0x25][..], &abs(address)].concat()
}

/// `fninit` then `fld1`: an empty stack with exceptions masked, then one
/// push, so TOS is 7 and only physical register 7 holds a value. The power-up
/// x87 state is no starting point: every register is tagged in use and every
/// exception unmasked, so the push would overflow instead.
const FNINIT_FLD1: [u8; 4] = [0xDB, 0xE3, 0xD9, 0xE8];

/// The x87 environment's stack top and tag word, as FNSTENV reports them.
/// FNSTENV derives each register's two-bit tag from its contents — valid,
/// zero or special — unless the register is empty (0b11).
#[derive(Debug, PartialEq, Eq)]
struct X87View {
    top: u16,
    tags: u16,
}

impl X87View {
    /// The state the FPU-to-MMX transition leaves: TOS 0 and no register
    /// empty, whatever each one holds.
    fn is_mmx_state(&self) -> bool {
        self.top == 0 && (0..8).all(|reg| (self.tags >> (reg * 2)) & 3 != 3)
    }
}

/// After FNINIT and FLD1: TOS 7, register 7 valid (1.0), the rest empty.
const AFTER_FLD1: X87View = X87View { top: 7, tags: 0x3FFF };

/// Run `code` and stop where it ends, failing if it faulted instead.
fn run(emu: &mut Emulator, code: &[u8]) {
    let mut image = code.to_vec();
    image.extend_from_slice(&[0xEB, 0xFE]);
    emu.mem_write(CODE, &image).expect("write code");
    let end = CODE + code.len() as u64;
    emu.emu_start(CODE, Some(end), None, Some(64))
        .expect("execute");
    assert_eq!(emu.cpu().rip(), end, "the sequence must run to its end, not fault");
}

fn x87_view(emu: &mut Emulator) -> X87View {
    let mut env = [0u8; 28];
    emu.mem_read(ENV, &mut env).expect("read env");
    let status = u16::from_le_bytes([env[4], env[5]]);
    X87View {
        top: (status >> 11) & 7,
        tags: u16::from_le_bytes([env[8], env[9]]),
    }
}

fn qwords(bytes: [u8; 16]) -> [u64; 2] {
    [
        u64::from_le_bytes(bytes[0..8].try_into().expect("8 bytes")),
        u64::from_le_bytes(bytes[8..16].try_into().expect("8 bytes")),
    ]
}

fn xmm(q: [u64; 2]) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&q[0].to_le_bytes());
    bytes[8..16].copy_from_slice(&q[1].to_le_bytes());
    bytes
}

fn dwords(lo: u32, hi: u32) -> u64 {
    u64::from(lo) | (u64::from(hi) << 32)
}

fn ints(lo: i32, hi: i32) -> u64 {
    dwords(lo as u32, hi as u32)
}

fn read_out(emu: &mut Emulator) -> u64 {
    let mut out = [0u8; 8];
    emu.mem_read(OUT, &mut out).expect("read out");
    u64::from_le_bytes(out)
}

fn on_big_stack(check: fn()) {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(check)
        .expect("spawn")
        .join()
        .expect("join");
}

/// What xmm0's high quadword holds before a conversion that must keep it.
const KEPT: u64 = 0xAAAA_BBBB_CCCC_DDDD;

/// CVTPI2PS xmm0, mm1 converts both integers into xmm0's low quadword, keeps
/// the high one, and puts the x87 unit in MMX state.
#[test]
fn cvtpi2ps_from_mmx_converts_and_enters_mmx_state() {
    on_big_stack(|| {
        let mut emu = sse_emulator();
        emu.mem_write(DATA, &ints(3, -2).to_le_bytes()).expect("data");
        emu.reg_write_xmm(X86Reg::Xmm0, xmm([0, KEPT]));
        let code = [
            movq_mm1_from(DATA),
            FNINIT_FLD1.to_vec(),
            vec![0x0F, 0x2A, 0xC1], // cvtpi2ps xmm0, mm1
            fnstenv(ENV),
        ]
        .concat();
        run(&mut emu, &code);
        assert_eq!(
            qwords(emu.reg_read_xmm(X86Reg::Xmm0)),
            [dwords(3.0f32.to_bits(), (-2.0f32).to_bits()), KEPT]
        );
        let view = x87_view(&mut emu);
        assert!(view.is_mmx_state(), "not in MMX state: {view:?}");
    });
}

/// From memory CVTPI2PS touches no MMX register, so the x87 stack FLD1 left
/// is still there afterwards.
#[test]
fn cvtpi2ps_from_memory_leaves_the_x87_state_alone() {
    on_big_stack(|| {
        let mut emu = sse_emulator();
        emu.mem_write(DATA, &ints(-7, 100).to_le_bytes()).expect("data");
        emu.reg_write_xmm(X86Reg::Xmm0, xmm([0, KEPT]));
        let code = [
            FNINIT_FLD1.to_vec(),
            [&[0x0F, 0x2A, 0x04, 0x25][..], &abs(DATA)].concat(), // cvtpi2ps xmm0, [DATA]
            fnstenv(ENV),
        ]
        .concat();
        run(&mut emu, &code);
        assert_eq!(
            qwords(emu.reg_read_xmm(X86Reg::Xmm0)),
            [dwords((-7.0f32).to_bits(), 100.0f32.to_bits()), KEPT]
        );
        assert_eq!(x87_view(&mut emu), AFTER_FLD1);
    });
}

/// CVTPI2PD writes both halves of xmm0 with exact doubles.
#[test]
fn cvtpi2pd_converts_both_integers_to_doubles() {
    on_big_stack(|| {
        let mut emu = sse_emulator();
        emu.mem_write(DATA, &ints(i32::MIN, 5).to_le_bytes()).expect("data");
        emu.reg_write_xmm(X86Reg::Xmm0, xmm([u64::MAX, u64::MAX]));
        let code = [
            movq_mm1_from(DATA),
            FNINIT_FLD1.to_vec(),
            vec![0x66, 0x0F, 0x2A, 0xC1], // cvtpi2pd xmm0, mm1
            fnstenv(ENV),
        ]
        .concat();
        run(&mut emu, &code);
        assert_eq!(
            qwords(emu.reg_read_xmm(X86Reg::Xmm0)),
            [f64::from(i32::MIN).to_bits(), 5.0f64.to_bits()]
        );
        let view = x87_view(&mut emu);
        assert!(view.is_mmx_state(), "not in MMX state: {view:?}");
    });
}

/// Convert xmm1 into mm0 with `opcode` (ModRM 0xC1: mm0, xmm1) and return
/// mm0, checking the transition to MMX state on the way.
fn to_mmx(opcode: &[u8], source: [u64; 2]) -> u64 {
    let mut emu = sse_emulator();
    emu.reg_write_xmm(X86Reg::Xmm1, xmm(source));
    let code = [
        FNINIT_FLD1.to_vec(),
        [opcode, &[0xC1]].concat(),
        movq_mm0_to(OUT),
        fnstenv(ENV),
    ]
    .concat();
    run(&mut emu, &code);
    let view = x87_view(&mut emu);
    assert!(view.is_mmx_state(), "not in MMX state: {view:?}");
    read_out(&mut emu)
}

/// The truncating forms drop the fraction; the others round to nearest-even
/// under the reset MXCSR.
#[test]
fn float_to_mmx_conversions_truncate_or_round() {
    on_big_stack(|| {
        let singles = dwords(2.7f32.to_bits(), (-2.5f32).to_bits());
        let doubles = [2.5f64.to_bits(), (-3.7f64).to_bits()];
        assert_eq!(to_mmx(&[0x0F, 0x2C], [singles, 0]), ints(2, -2), "cvttps2pi");
        assert_eq!(to_mmx(&[0x0F, 0x2D], [singles, 0]), ints(3, -2), "cvtps2pi");
        assert_eq!(to_mmx(&[0x66, 0x0F, 0x2C], doubles), ints(2, -3), "cvttpd2pi");
        assert_eq!(to_mmx(&[0x66, 0x0F, 0x2D], doubles), ints(2, -4), "cvtpd2pi");
    });
}

/// A NaN or out-of-range value converts to the integer indefinite, 0x80000000,
/// with the invalid exception masked at reset.
#[test]
fn an_unrepresentable_float_becomes_the_integer_indefinite() {
    on_big_stack(|| {
        let singles = dwords(f32::NAN.to_bits(), 1.0f32.to_bits());
        assert_eq!(to_mmx(&[0x0F, 0x2D], [singles, 0]), dwords(0x8000_0000, 1));
        let doubles = [1.0e10f64.to_bits(), (-1.0f64).to_bits()];
        assert_eq!(to_mmx(&[0x66, 0x0F, 0x2C], doubles), dwords(0x8000_0000, (-1i32) as u32));
    });
}

/// CVTPD2PI's 128-bit memory source must be 16-byte aligned: a misaligned one
/// raises #GP(0), an aligned one converts (Bochs sse_pfp.cc
/// `read_virtual_xmmword_aligned`).
#[test]
fn cvtpd2pi_from_memory_needs_16_byte_alignment() {
    on_big_stack(|| {
        let operand = [1.0f64.to_bits(), 2.0f64.to_bits(), 3.0f64.to_bits()];
        let mut bytes = Vec::new();
        for q in operand {
            bytes.extend_from_slice(&q.to_le_bytes());
        }

        let mut emu = sse_emulator();
        emu.mem_write(DATA, &bytes).expect("data");
        // cvtpd2pi mm0, [DATA]: 66 0F 2D /r, ModRM 0x04, SIB 0x25.
        let aligned = [&[0x66, 0x0F, 0x2D, 0x04, 0x25][..], &abs(DATA)].concat();
        run(&mut emu, &[aligned, movq_mm0_to(OUT)].concat());
        assert_eq!(read_out(&mut emu), ints(1, 2));

        let mut emu = sse_emulator();
        emu.mem_write(DATA, &bytes).expect("data");
        let misaligned = [&[0x66, 0x0F, 0x2D, 0x04, 0x25][..], &abs(DATA + 8)].concat();
        emu.mem_write(CODE, &misaligned).expect("write code");
        emu.emu_start(CODE, Some(GP_HANDLER), None, Some(4))
            .expect("execute");
        assert_eq!(emu.cpu().rip(), GP_HANDLER, "a misaligned operand takes #GP");
    });
}
