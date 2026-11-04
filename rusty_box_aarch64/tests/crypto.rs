//! The cryptographic and CRC32 extensions as a guest uses them: A64 code on a
//! bare machine computes known answers from the standards that define the
//! algorithms — FIPS-197 Appendix C.1 for AES-128, FIPS 180-4 (the "abc"
//! examples) for SHA-1 and SHA-256, and the CRC-32 and CRC-32C check values
//! over "123456789" — and the assertions are what the guest's registers and
//! memory hold afterwards (R9).
//!
//! The programs are built by the encoders below, each the Arm ARM encoding
//! of one instruction form; `the_encoders_are_what_gnu_as_assembles` pins
//! every one to the word GNU as (`-march=armv8-a+crypto+crc`) produces.

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{
    LentStorage, MachineBuilder, MemoryLoan, NoEl2, RamSize, Reg, StopReason, SysReg, VReg,
    VirtConfig, VirtMachine,
};

// ----- encoders (Arm ARM C6.2 and C7.2, the instruction encodings) -----

/// `AESE Vd.16B, Vn.16B`.
const fn aese(d: u32, n: u32) -> u32 {
    0x4E28_4800 | n << 5 | d
}
/// `AESD Vd.16B, Vn.16B`.
const fn aesd(d: u32, n: u32) -> u32 {
    0x4E28_5800 | n << 5 | d
}
/// `AESMC Vd.16B, Vn.16B`.
const fn aesmc(d: u32, n: u32) -> u32 {
    0x4E28_6800 | n << 5 | d
}
/// `AESIMC Vd.16B, Vn.16B`.
const fn aesimc(d: u32, n: u32) -> u32 {
    0x4E28_7800 | n << 5 | d
}
/// `EOR Vd.16B, Vn.16B, Vm.16B`.
const fn eor_16b(d: u32, n: u32, m: u32) -> u32 {
    0x6E20_1C00 | m << 16 | n << 5 | d
}
/// `MOV Vd.16B, Vn.16B` (`ORR Vd.16B, Vn.16B, Vn.16B`).
const fn mov_16b(d: u32, n: u32) -> u32 {
    0x4EA0_1C00 | n << 16 | n << 5 | d
}
/// `B .`
const SPIN: u32 = 0x1400_0000;
/// `LD1 {Vt.4S, Vt+1.4S}, [Xn]`.
const fn ld1_2x4s(t: u32, n: u32) -> u32 {
    0x4C40_A800 | n << 5 | t
}
/// `LD1 {Vt.16B - Vt+3.16B}, [Xn]`.
const fn ld1_4x16b(t: u32, n: u32) -> u32 {
    0x4C40_2000 | n << 5 | t
}
/// `LD1 {Vt.4S}, [Xn], #16`.
const fn ld1_4s_post16(t: u32, n: u32) -> u32 {
    0x4CDF_7800 | n << 5 | t
}
/// `ST1 {Vt.4S, Vt+1.4S}, [Xn]`.
const fn st1_2x4s(t: u32, n: u32) -> u32 {
    0x4C00_A800 | n << 5 | t
}
/// `LD4R {Vt.4S - Vt+3.4S}, [Xn]`.
const fn ld4r_4s(t: u32, n: u32) -> u32 {
    0x4D60_E800 | n << 5 | t
}
/// `REV32 Vd.16B, Vn.16B`.
const fn rev32_16b(d: u32, n: u32) -> u32 {
    0x6E20_0800 | n << 5 | d
}
/// `ADD Vd.4S, Vn.4S, Vm.4S`.
const fn add_4s(d: u32, n: u32, m: u32) -> u32 {
    0x4EA0_8400 | m << 16 | n << 5 | d
}
/// `ADD Vd.2S, Vn.2S, Vm.2S`.
const fn add_2s(d: u32, n: u32, m: u32) -> u32 {
    0x0EA0_8400 | m << 16 | n << 5 | d
}
/// `SHA256H Qd, Qn, Vm.4S`.
const fn sha256h(d: u32, n: u32, m: u32) -> u32 {
    0x5E00_4000 | m << 16 | n << 5 | d
}
/// `SHA256H2 Qd, Qn, Vm.4S`.
const fn sha256h2(d: u32, n: u32, m: u32) -> u32 {
    0x5E00_5000 | m << 16 | n << 5 | d
}
/// `SHA256SU0 Vd.4S, Vn.4S`.
const fn sha256su0(d: u32, n: u32) -> u32 {
    0x5E28_2800 | n << 5 | d
}
/// `SHA256SU1 Vd.4S, Vn.4S, Vm.4S`.
const fn sha256su1(d: u32, n: u32, m: u32) -> u32 {
    0x5E00_6000 | m << 16 | n << 5 | d
}
/// `SHA1C`, `SHA1P`, `SHA1M Qd, Sn, Vm.4S`, by `op` 0, 1, 2.
const fn sha1_hash(op: u32, d: u32, n: u32, m: u32) -> u32 {
    0x5E00_0000 | m << 16 | op << 12 | n << 5 | d
}
/// `SHA1SU0 Vd.4S, Vn.4S, Vm.4S`.
const fn sha1su0(d: u32, n: u32, m: u32) -> u32 {
    0x5E00_3000 | m << 16 | n << 5 | d
}
/// `SHA1H Sd, Sn`.
const fn sha1h(d: u32, n: u32) -> u32 {
    0x5E28_0800 | n << 5 | d
}
/// `SHA1SU1 Vd.4S, Vn.4S`.
const fn sha1su1(d: u32, n: u32) -> u32 {
    0x5E28_1800 | n << 5 | d
}
/// `LDR Qt, [Xn]`.
const fn ldr_q(t: u32, n: u32) -> u32 {
    0x3DC0_0000 | n << 5 | t
}
/// `LDR St, [Xn, #16]`.
const fn ldr_s_16(t: u32, n: u32) -> u32 {
    0xBD40_1000 | n << 5 | t
}
/// `STR Qt, [Xn]`.
const fn str_q(t: u32, n: u32) -> u32 {
    0x3D80_0000 | n << 5 | t
}
/// `STR St, [Xn, #16]`.
const fn str_s_16(t: u32, n: u32) -> u32 {
    0xBD00_1000 | n << 5 | t
}
/// `CRC32B`, `CRC32H`, `CRC32W`, `CRC32X` (`sz` 0 to 3), and the `C`
/// forms when `castagnoli`: `Wd, Wn, Wm` (`Xm` for the X forms).
const fn crc32(castagnoli: bool, sz: u32, d: u32, n: u32, m: u32) -> u32 {
    let sf = if sz == 3 { 1 << 31 } else { 0 };
    sf | 0x1AC0_4000 | m << 16 | (castagnoli as u32) << 12 | sz << 10 | n << 5 | d
}
/// `PMULL Vd.1Q, Vn.1D, Vm.1D`, or `PMULL2 Vd.1Q, Vn.2D, Vm.2D` when `upper`.
const fn pmull_1q(upper: bool, d: u32, n: u32, m: u32) -> u32 {
    (upper as u32) << 30 | 0x0EE0_E000 | m << 16 | n << 5 | d
}

#[test]
fn the_encoders_are_what_gnu_as_assembles() {
    let pairs: [(u32, u32, &str); 39] = [
        (aese(3, 17), 0x4E28_4A23, "aese v3.16b, v17.16b"),
        (aesd(3, 17), 0x4E28_5A23, "aesd v3.16b, v17.16b"),
        (aesmc(3, 17), 0x4E28_6A23, "aesmc v3.16b, v17.16b"),
        (aesimc(3, 17), 0x4E28_7A23, "aesimc v3.16b, v17.16b"),
        (
            eor_16b(3, 5, 17),
            0x6E31_1CA3,
            "eor v3.16b, v5.16b, v17.16b",
        ),
        (mov_16b(3, 17), 0x4EB1_1E23, "mov v3.16b, v17.16b"),
        (SPIN, 0x1400_0000, "b ."),
        (ld1_2x4s(3, 5), 0x4C40_A8A3, "ld1 {v3.4s, v4.4s}, [x5]"),
        (ld1_4x16b(3, 5), 0x4C40_20A3, "ld1 {v3.16b-v6.16b}, [x5]"),
        (ld1_4s_post16(3, 5), 0x4CDF_78A3, "ld1 {v3.4s}, [x5], #16"),
        (st1_2x4s(3, 5), 0x4C00_A8A3, "st1 {v3.4s, v4.4s}, [x5]"),
        (rev32_16b(3, 17), 0x6E20_0A23, "rev32 v3.16b, v17.16b"),
        (add_4s(3, 5, 17), 0x4EB1_84A3, "add v3.4s, v5.4s, v17.4s"),
        (add_2s(3, 5, 17), 0x0EB1_84A3, "add v3.2s, v5.2s, v17.2s"),
        (sha256h(3, 5, 17), 0x5E11_40A3, "sha256h q3, q5, v17.4s"),
        (sha256h2(3, 5, 17), 0x5E11_50A3, "sha256h2 q3, q5, v17.4s"),
        (sha256su0(3, 17), 0x5E28_2A23, "sha256su0 v3.4s, v17.4s"),
        (
            sha256su1(3, 5, 17),
            0x5E11_60A3,
            "sha256su1 v3.4s, v5.4s, v17.4s",
        ),
        (sha1_hash(0, 3, 5, 17), 0x5E11_00A3, "sha1c q3, s5, v17.4s"),
        (sha1_hash(1, 3, 5, 17), 0x5E11_10A3, "sha1p q3, s5, v17.4s"),
        (sha1_hash(2, 3, 5, 17), 0x5E11_20A3, "sha1m q3, s5, v17.4s"),
        (
            sha1su0(3, 5, 17),
            0x5E11_30A3,
            "sha1su0 v3.4s, v5.4s, v17.4s",
        ),
        (sha1h(3, 17), 0x5E28_0A23, "sha1h s3, s17"),
        (sha1su1(3, 17), 0x5E28_1A23, "sha1su1 v3.4s, v17.4s"),
        (ldr_q(3, 5), 0x3DC0_00A3, "ldr q3, [x5]"),
        (ldr_s_16(3, 5), 0xBD40_10A3, "ldr s3, [x5, #16]"),
        (str_q(3, 5), 0x3D80_00A3, "str q3, [x5]"),
        (str_s_16(3, 5), 0xBD00_10A3, "str s3, [x5, #16]"),
        (ld4r_4s(3, 5), 0x4D60_E8A3, "ld4r {v3.4s-v6.4s}, [x5]"),
        (crc32(false, 0, 3, 5, 17), 0x1AD1_40A3, "crc32b w3, w5, w17"),
        (crc32(false, 1, 3, 5, 17), 0x1AD1_44A3, "crc32h w3, w5, w17"),
        (crc32(false, 2, 3, 5, 17), 0x1AD1_48A3, "crc32w w3, w5, w17"),
        (crc32(false, 3, 3, 5, 17), 0x9AD1_4CA3, "crc32x w3, w5, x17"),
        (crc32(true, 0, 3, 5, 17), 0x1AD1_50A3, "crc32cb w3, w5, w17"),
        (crc32(true, 1, 3, 5, 17), 0x1AD1_54A3, "crc32ch w3, w5, w17"),
        (crc32(true, 2, 3, 5, 17), 0x1AD1_58A3, "crc32cw w3, w5, w17"),
        (crc32(true, 3, 3, 5, 17), 0x9AD1_5CA3, "crc32cx w3, w5, x17"),
        (
            pmull_1q(false, 3, 5, 17),
            0x0EF1_E0A3,
            "pmull v3.1q, v5.1d, v17.1d",
        ),
        (
            pmull_1q(true, 3, 5, 17),
            0x4EF1_E0A3,
            "pmull2 v3.1q, v5.2d, v17.2d",
        ),
    ];
    for (built, assembled, text) in pairs {
        assert_eq!(built, assembled, "{text}");
    }
}

// ----- the machine -----

const RAM_LEN: usize = 1 << 20;
/// Where the data a program reads and writes lives.
const DATA: u64 = RAM_BASE + 0x8_0000;

fn machine(ram: &mut [u8]) -> VirtMachine<LentStorage<'_>, NoEl2> {
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds");
    m.sysreg_write(SysReg::CPACR_EL1, 3 << 20)
        .expect("CPACR_EL1 is writable");
    m
}

/// Run `program` from the base of RAM up to its final `B .`.
fn run(m: &mut VirtMachine<LentStorage<'_>, NoEl2>, program: &[u32]) {
    for (i, word) in program.iter().enumerate() {
        m.mem_write(RAM_BASE + 4 * i as u64, &word.to_le_bytes())
            .expect("code is in RAM");
    }
    let end = RAM_BASE + 4 * (program.len() as u64 - 1);
    let outcome = m.emu_start(RAM_BASE, Some(end), Some(10_000));
    assert_eq!(
        outcome.stop,
        StopReason::ExitReached,
        "the program runs to its end"
    );
}

fn v(n: u8) -> VReg {
    VReg::new(n).expect("a vector register")
}

/// A 128-bit value whose bytes in memory order are `hex` (the byte order
/// FIPS-197 writes states and keys in).
fn bytes128(hex: &str) -> u128 {
    let mut bytes = [0u8; 16];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex");
    }
    u128::from_le_bytes(bytes)
}

// ----- AES -----

/// FIPS-197 Appendix C.1: AES-128, key 000102…0f, the round keys of its
/// key expansion.
const AES128_ROUND_KEYS: [&str; 11] = [
    "000102030405060708090a0b0c0d0e0f",
    "d6aa74fdd2af72fadaa678f1d6ab76fe",
    "b692cf0b643dbdf1be9bc5006830b3fe",
    "b6ff744ed2c2c9bf6c590cbf0469bf41",
    "47f7f7bc95353e03f96c32bcfd058dfd",
    "3caaa3e8a99f9deb50f3af57adf622aa",
    "5e390f7df7a69296a7553dc10aa31f6b",
    "14f9701ae35fe28c440adf4d4ea9c026",
    "47438735a41c65b9e016baf4aebf7ad2",
    "549932d1f08557681093ed9cbe2c974e",
    "13111d7fe3944a17f307a78b4d2b30c5",
];

/// Encrypt v0 with the round keys in v16..v26 (AESE, AESMC), copy it to v1,
/// turn the middle round keys into decryption keys (AESIMC), and decrypt v1
/// with the equivalent inverse cipher (AESD, AESIMC).
fn aes128_program() -> Vec<u32> {
    let mut p = Vec::new();
    for r in 0..9 {
        p.extend([aese(0, 16 + r), aesmc(0, 0)]);
    }
    p.extend([aese(0, 25), eor_16b(0, 0, 26), mov_16b(1, 0)]);
    for r in 1..10 {
        p.push(aesimc(16 + r, 16 + r));
    }
    p.extend([aesd(1, 26), aesimc(1, 1)]);
    for r in (2..10).rev() {
        p.extend([aesd(1, 16 + r), aesimc(1, 1)]);
    }
    p.extend([aesd(1, 17), eor_16b(1, 1, 16), SPIN]);
    p
}

#[test]
fn aes128_encrypts_and_decrypts_the_fips_197_example() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(&mut ram);
    let plaintext = bytes128("00112233445566778899aabbccddeeff");
    m.vreg_write(v(0), plaintext);
    for (r, key) in AES128_ROUND_KEYS.iter().enumerate() {
        m.vreg_write(v(16 + r as u8), bytes128(key));
    }
    run(&mut m, &aes128_program());
    assert_eq!(
        m.vreg_read(v(0)),
        bytes128("69c4e0d86a7b0430d8cdb78070b4c55a"),
        "the FIPS-197 ciphertext"
    );
    assert_eq!(
        m.vreg_read(v(1)),
        plaintext,
        "decryption gives the plaintext back"
    );
}

// ----- SHA -----

/// "abc", padded to one 512-bit block (FIPS 180-4 5.1.1).
fn abc_block() -> [u8; 64] {
    let mut block = [0u8; 64];
    block[..3].copy_from_slice(b"abc");
    block[3] = 0x80;
    block[63] = 24;
    block
}

/// FIPS 180-4 4.2.2: the SHA-256 constants.
const SHA256_K: [u32; 64] = [
    0x428A_2F98,
    0x7137_4491,
    0xB5C0_FBCF,
    0xE9B5_DBA5,
    0x3956_C25B,
    0x59F1_11F1,
    0x923F_82A4,
    0xAB1C_5ED5,
    0xD807_AA98,
    0x1283_5B01,
    0x2431_85BE,
    0x550C_7DC3,
    0x72BE_5D74,
    0x80DE_B1FE,
    0x9BDC_06A7,
    0xC19B_F174,
    0xE49B_69C1,
    0xEFBE_4786,
    0x0FC1_9DC6,
    0x240C_A1CC,
    0x2DE9_2C6F,
    0x4A74_84AA,
    0x5CB0_A9DC,
    0x76F9_88DA,
    0x983E_5152,
    0xA831_C66D,
    0xB003_27C8,
    0xBF59_7FC7,
    0xC6E0_0BF3,
    0xD5A7_9147,
    0x06CA_6351,
    0x1429_2967,
    0x27B7_0A85,
    0x2E1B_2138,
    0x4D2C_6DFC,
    0x5338_0D13,
    0x650A_7354,
    0x766A_0ABB,
    0x81C2_C92E,
    0x9272_2C85,
    0xA2BF_E8A1,
    0xA81A_664B,
    0xC24B_8B70,
    0xC76C_51A3,
    0xD192_E819,
    0xD699_0624,
    0xF40E_3585,
    0x106A_A070,
    0x19A4_C116,
    0x1E37_6C08,
    0x2748_774C,
    0x34B0_BCB5,
    0x391C_0CB3,
    0x4ED8_AA4A,
    0x5B9C_CA4F,
    0x682E_6FF3,
    0x748F_82EE,
    0x78A5_636F,
    0x84C8_7814,
    0x8CC7_0208,
    0x90BE_FFFA,
    0xA450_6CEB,
    0xBEF9_A3F7,
    0xC671_78F2,
];

/// One SHA-256 block: x0 points at the block, x1 at the constants, x2 at
/// the hash state, which the program updates.
fn sha256_block_program() -> Vec<u32> {
    let mut p = vec![ld1_2x4s(0, 2), ld1_4x16b(16, 0)];
    p.extend((16..20).map(|r| rev32_16b(r, r)));
    p.extend([mov_16b(20, 0), mov_16b(21, 1)]);
    for i in 0..16 {
        let w = 16 + i % 4;
        p.extend([
            ld1_4s_post16(22, 1),
            add_4s(22, 22, w),
            mov_16b(2, 0),
            sha256h(0, 1, 22),
            sha256h2(1, 2, 22),
        ]);
        if i < 12 {
            p.extend([
                sha256su0(w, 16 + (i + 1) % 4),
                sha256su1(w, 16 + (i + 2) % 4, 16 + (i + 3) % 4),
            ]);
        }
    }
    p.extend([add_4s(0, 0, 20), add_4s(1, 1, 21), st1_2x4s(0, 2), SPIN]);
    p
}

#[test]
fn sha256_of_abc_is_the_fips_180_4_digest() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(&mut ram);
    let (block, k, state) = (DATA, DATA + 0x100, DATA + 0x200);
    m.mem_write(block, &abc_block()).expect("in RAM");
    let k_bytes: Vec<u8> = SHA256_K.iter().flat_map(|w| w.to_le_bytes()).collect();
    m.mem_write(k, &k_bytes).expect("in RAM");
    let initial: [u32; 8] = [
        0x6A09_E667,
        0xBB67_AE85,
        0x3C6E_F372,
        0xA54F_F53A,
        0x510E_527F,
        0x9B05_688C,
        0x1F83_D9AB,
        0x5BE0_CD19,
    ];
    let h_bytes: Vec<u8> = initial.iter().flat_map(|w| w.to_le_bytes()).collect();
    m.mem_write(state, &h_bytes).expect("in RAM");
    m.reg_write(Reg::X0, block);
    m.reg_write(Reg::X1, k);
    m.reg_write(Reg::X2, state);
    run(&mut m, &sha256_block_program());
    let mut out = [0u8; 32];
    m.mem_read(state, &mut out).expect("in RAM");
    let digest: Vec<u32> = out
        .chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    assert_eq!(
        digest,
        [
            0xBA78_16BF,
            0x8F01_CFEA,
            0x4141_40DE,
            0x5DAE_2223,
            0xB003_61A3,
            0x9617_7A9C,
            0xB410_FF61,
            0xF200_15AD
        ]
    );
}

/// One SHA-1 block: x0 points at the block, x1 at the four constants, x2 at
/// the hash state (A..E), which the program updates.
fn sha1_block_program() -> Vec<u32> {
    let mut p = vec![ldr_q(0, 2), ldr_s_16(16, 2), ld1_4x16b(8, 0)];
    p.extend((8..12).map(|r| rev32_16b(r, r)));
    p.extend([ld4r_4s(4, 1), mov_16b(20, 0), mov_16b(21, 16)]);
    // Rounds 0-19 choose, 20-39 parity, 40-59 majority, 60-79 parity.
    let round_fn = [0, 1, 2, 1];
    for i in 0..20 {
        let w = 8 + i % 4;
        p.extend([
            add_4s(12, w, 4 + i / 5),
            sha1h(13, 0),
            sha1_hash(round_fn[(i / 5) as usize], 0, 16, 12),
            mov_16b(16, 13),
        ]);
        if i < 16 {
            p.extend([
                sha1su0(w, 8 + (i + 1) % 4, 8 + (i + 2) % 4),
                sha1su1(w, 8 + (i + 3) % 4),
            ]);
        }
    }
    p.extend([
        add_4s(0, 0, 20),
        add_2s(16, 16, 21),
        str_q(0, 2),
        str_s_16(16, 2),
        SPIN,
    ]);
    p
}

#[test]
fn sha1_of_abc_is_the_fips_180_4_digest() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(&mut ram);
    let (block, k, state) = (DATA, DATA + 0x100, DATA + 0x200);
    m.mem_write(block, &abc_block()).expect("in RAM");
    let k_bytes: Vec<u8> = [0x5A82_7999u32, 0x6ED9_EBA1, 0x8F1B_BCDC, 0xCA62_C1D6]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect();
    m.mem_write(k, &k_bytes).expect("in RAM");
    let initial: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let h_bytes: Vec<u8> = initial.iter().flat_map(|w| w.to_le_bytes()).collect();
    m.mem_write(state, &h_bytes).expect("in RAM");
    m.reg_write(Reg::X0, block);
    m.reg_write(Reg::X1, k);
    m.reg_write(Reg::X2, state);
    run(&mut m, &sha1_block_program());
    let mut out = [0u8; 20];
    m.mem_read(state, &mut out).expect("in RAM");
    let digest: Vec<u32> = out
        .chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    assert_eq!(
        digest,
        [
            0xA999_3E36,
            0x4706_816A,
            0xBA3E_2571,
            0x7850_C26C,
            0x9CD0_D89D
        ]
    );
}

// ----- CRC32 -----

/// CRC-32 and CRC-32C of "123456789", each three ways: 8 + 1 bytes (X, B),
/// 4 + 2 + 2 + 1 (W, H, H, B), and with junk above every operand's size,
/// which the instructions ignore. The accumulators start at all ones and
/// the test inverts the results, as the standard CRC-32 does.
#[test]
fn crc32_and_crc32c_give_the_catalogue_check_values() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(&mut ram);
    let text = b"123456789";
    let eight = u64::from_le_bytes([
        text[0], text[1], text[2], text[3], text[4], text[5], text[6], text[7],
    ]);
    // x10: bytes 0-7; x11: byte 8 with junk above it; x12: bytes 0-3 with
    // junk above; x13, x14: bytes 4-5 and 6-7 with junk above.
    m.reg_write(Reg::X10, eight);
    m.reg_write(Reg::X11, 0xFFFF_FFFF_FFFF_FF00 | u64::from(text[8]));
    m.reg_write(Reg::X12, 0xDEAD_BEEF_0000_0000 | (eight & 0xFFFF_FFFF));
    m.reg_write(Reg::X13, 0xFFFF_0000 | ((eight >> 32) & 0xFFFF));
    m.reg_write(Reg::X14, 0xABCD_0000 | ((eight >> 48) & 0xFFFF));
    for acc in [Reg::X2, Reg::X3, Reg::X4, Reg::X5] {
        // The accumulator is W[n]: the upper half is ignored.
        m.reg_write(acc, 0x1234_5678_FFFF_FFFF);
    }
    let program = [
        crc32(false, 3, 2, 2, 10),
        crc32(false, 0, 2, 2, 11),
        crc32(true, 3, 3, 3, 10),
        crc32(true, 0, 3, 3, 11),
        crc32(false, 2, 4, 4, 12),
        crc32(false, 1, 4, 4, 13),
        crc32(false, 1, 4, 4, 14),
        crc32(false, 0, 4, 4, 11),
        crc32(true, 2, 5, 5, 12),
        crc32(true, 1, 5, 5, 13),
        crc32(true, 1, 5, 5, 14),
        crc32(true, 0, 5, 5, 11),
        SPIN,
    ];
    run(&mut m, &program);
    // The result is W[d], zero-extended.
    assert_eq!(
        m.reg_read(Reg::X2),
        u64::from(!0xCBF4_3926u32),
        "CRC-32 by X and B"
    );
    assert_eq!(
        m.reg_read(Reg::X4),
        u64::from(!0xCBF4_3926u32),
        "CRC-32 by W, H and B"
    );
    assert_eq!(
        m.reg_read(Reg::X3),
        u64::from(!0xE306_9283u32),
        "CRC-32C by X and B"
    );
    assert_eq!(
        m.reg_read(Reg::X5),
        u64::from(!0xE306_9283u32),
        "CRC-32C by W, H and B"
    );
}

// ----- PMULL -----

/// PMULL multiplies the low 64-bit elements as polynomials over GF(2), and
/// PMULL2 the high ones: (x^63 + 1)(x + 1) = x^64 + x^63 + x + 1, and
/// (x^63 + x^62)(x^63 + x^62) = x^126 + x^124 (the cross terms cancel).
#[test]
fn pmull_multiplies_64_bit_polynomials() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(&mut ram);
    let high = 0xC000_0000_0000_0000u128;
    m.vreg_write(v(1), (high << 64) | 0x8000_0000_0000_0001);
    m.vreg_write(v(2), (high << 64) | 0b11);
    run(
        &mut m,
        &[pmull_1q(false, 0, 1, 2), pmull_1q(true, 3, 1, 1), SPIN],
    );
    assert_eq!(m.vreg_read(v(0)), (1u128 << 64) | (1 << 63) | 0b11);
    assert_eq!(m.vreg_read(v(3)), (1u128 << 126) | (1 << 124));
}

// ----- traps -----

/// With SIMD&FP disabled, the cryptographic instructions take the SIMD&FP
/// access trap (EC 0x07), as every Advanced SIMD instruction does; CRC32 is
/// a general-purpose instruction and executes.
#[test]
fn cpacr_traps_the_crypto_instructions_but_not_crc32() {
    for (word, traps) in [
        (aese(0, 1), true),
        (aesimc(0, 1), true),
        (sha1h(0, 1), true),
        (sha256h(0, 1, 2), true),
        (sha256su0(0, 1), true),
        (pmull_1q(false, 0, 1, 2), true),
        (crc32(false, 0, 0, 1, 2), false),
        (crc32(true, 3, 0, 1, 2), false),
    ] {
        let mut ram = vec![0u8; RAM_LEN];
        let mut m = machine(&mut ram);
        m.sysreg_write(SysReg::CPACR_EL1, 0)
            .expect("CPACR_EL1 is writable");
        m.mem_write(RAM_BASE, &word.to_le_bytes())
            .expect("code is in RAM");
        let outcome = m.emu_start(RAM_BASE, None, Some(1));
        assert_eq!(
            outcome.stop,
            StopReason::BudgetExhausted,
            "{word:#010x} retires"
        );
        let esr = m.sysreg_read(SysReg::ESR_EL1).expect("ESR_EL1 reads");
        if traps {
            assert_eq!(esr >> 26, 0x07, "{word:#010x}: the SIMD&FP access trap");
            assert_ne!(
                m.reg_read(Reg::Pc),
                RAM_BASE + 4,
                "{word:#010x} went to the vector"
            );
        } else {
            assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 4, "{word:#010x} executed");
        }
    }
}
