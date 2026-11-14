//! The x87 register-stack sequences compiled C reaches for, run as a guest
//! runs them.
//!
//! The divide is the one busybox awk compiles for `L / R`: it loads zero,
//! exchanges it under the divisor, compares, drops the zero with
//! `FSTP ST(1)`, and divides. A guest whose x87 stack moves the wrong value
//! in any of those steps divides by zero and prints `inf`.

#![cfg(feature = "std")]

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::CpuSetupMode;
use rusty_box::emulator::{Emulator, EmulatorConfig};

/// Same sizing rationale as fp_vex_scalar_ops.rs.
const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;

const CODE: u64 = 0x0040_0000;
const DIVIDEND: u64 = 0x0050_0000;
const DIVISOR: u64 = 0x0050_0008;
const QUOTIENT: u64 = 0x0050_0010;

fn abs32(address: u64) -> [u8; 4] {
    u32::try_from(address).expect("below 4 GiB").to_le_bytes()
}

/// Run `body` after `fninit` on a flat 32-bit machine whose memory holds
/// `inputs` as doubles from [`DIVIDEND`] on, and read back `outputs` doubles
/// from [`QUOTIENT`] on. A process starts on an initialised x87, not the
/// power-on state whose tag word marks every register full.
fn run_x87(inputs: &[f64], body: &[u8], outputs: usize) -> Vec<f64> {
    let inputs = inputs.to_vec();
    let body = body.to_vec();
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(move || {
            let mut emu =
                Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32)
                    .expect("new emulator");
            for (slot, value) in (0u64..).zip(&inputs) {
                emu.mem_write(BxPhyAddress::new(DIVIDEND + slot * 8), &value.to_le_bytes())
                    .expect("write input");
            }
            let mut code: Vec<u8> = vec![0xDB, 0xE3];
            code.extend_from_slice(&body);
            let end = CODE + code.len() as u64;
            code.push(0xF4);
            emu.mem_write(BxPhyAddress::new(CODE), &code)
                .expect("write code");

            emu.emu_start(CODE, Some(end), None, None)
                .expect("run the sequence");

            (0u64..)
                .take(outputs)
                .map(|slot| {
                    let mut value = [0u8; 8];
                    emu.mem_read(BxPhyAddress::new(QUOTIENT + slot * 8), &mut value)
                        .expect("read output");
                    f64::from_le_bytes(value)
                })
                .collect()
        })
        .expect("spawn")
        .join()
        .expect("join")
}

/// `fld qword [address]`.
fn fld_m64(address: u64) -> Vec<u8> {
    let mut bytes = vec![0xDD, 0x05];
    bytes.extend_from_slice(&abs32(address));
    bytes
}

/// `fstp qword [address]`.
fn fstp_m64(address: u64) -> Vec<u8> {
    let mut bytes = vec![0xDD, 0x1D];
    bytes.extend_from_slice(&abs32(address));
    bytes
}

/// Each register form names ST(i) in ModRM.rm (Bochs `OP_STi`), whatever the
/// reg field holds: FLD ST(2), FXCH ST(2), FSUB ST(3),ST(0), FDIVP
/// ST(1),ST(0) and FSTP ST(1) each reach the register their rm names.
#[test]
fn register_forms_reach_the_register_rm_names() {
    let mut body = Vec::new();
    body.extend(fld_m64(DIVIDEND)); // 1.0
    body.extend(fld_m64(DIVIDEND + 8)); // 2.0
    body.extend(fld_m64(DIVIDEND + 16)); // 3.0: [3, 2, 1]
    body.extend_from_slice(&[
        0xD9, 0xC2, // fld st(2):            [1, 3, 2, 1]
        0xD9, 0xCA, // fxch st(2):           [2, 3, 1, 1]
        0xDC, 0xEB, // fsub st(3), st(0):    [2, 3, 1, -1]
        0xDE, 0xF9, // fdivp st(1), st(0):   [1.5, 1, -1]
        0xDD, 0xD9, // fstp st(1):           [1.5, -1]
    ]);
    body.extend(fstp_m64(QUOTIENT));
    body.extend(fstp_m64(QUOTIENT + 8));

    assert_eq!(run_x87(&[1.0, 2.0, 3.0], &body, 2), [1.5, -1.0]);
}

/// Busybox awk's `L / R` as Tiny Core 15's i386 busybox compiles it: ST(0)
/// holds R, and the dividend L sits in memory.
#[test]
fn the_awk_division_sequence_divides() {
    let mut body = fld_m64(DIVISOR);
    body.extend_from_slice(&[
        0xD9, 0xEE, // fldz
        0xD9, 0xC9, // fxch st(1)
        0xDD, 0xE1, // fucom st(1)
        0xDF, 0xE0, // fnstsw ax
        0xDD, 0xD9, // fstp st(1)
        0x9E, // sahf
        0xDC, 0x3D, // fdivr qword [DIVIDEND]
    ]);
    body.extend_from_slice(&abs32(DIVIDEND));
    body.extend(fstp_m64(QUOTIENT));

    assert_eq!(run_x87(&[10.0, 4.0], &body, 1), [2.5]);
}
