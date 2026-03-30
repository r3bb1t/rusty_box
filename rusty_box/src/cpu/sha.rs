//! SHA-NI instruction handlers for hardware-accelerated SHA-1 and SHA-256.
//!
//! Based on Bochs cpu/sha.cc
//! Copyright (C) 2013-2023 Stanislav Shwartsman
//!
//! Implements:
//! - SHA1RNDS4  Vdq, Wdq, Ib  (0F 3A CC)
//! - SHA1NEXTE  Vdq, Wdq       (0F 38 C8)
//! - SHA1MSG1   Vdq, Wdq       (0F 38 C9)
//! - SHA1MSG2   Vdq, Wdq       (0F 38 CA)
//! - SHA256RNDS2 Vdq, Wdq      (0F 38 CB) — implicit XMM0
//! - SHA256MSG1  Vdq, Wdq      (0F 38 CC)
//! - SHA256MSG2  Vdq, Wdq      (0F 38 CD)

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::Instruction,
    xmm::BxPackedXmmRegister,
};

// ============================================================================
// SHA helper functions (matching Bochs sha.cc exactly)
// ============================================================================

/// Rotate left 32-bit (matching Bochs scalar_arith.h rol32)
#[inline]
fn rol32(v32: u32, count: u32) -> u32 {
    (v32 << count) | (v32 >> (32 - count))
}

/// Rotate right 32-bit (matching Bochs scalar_arith.h ror32)
#[inline]
fn ror32(v32: u32, count: u32) -> u32 {
    (v32 >> count) | (v32 << (32 - count))
}

/// sha_f0(B,C,D) := (B AND C) XOR ((NOT B) AND D)
///
/// Used in SHA1 rounds 1-20 (Bochs sha_f0).
#[inline]
fn sha_f0(b: u32, c: u32, d: u32) -> u32 {
    (b & c) ^ (!b & d)
}

/// sha_f1(B,C,D) := B XOR C XOR D
///
/// Used in SHA1 rounds 21-40 (Bochs sha_f1).
/// Also used for rounds 61-80 (sha_f3 is identical to sha_f1).
#[inline]
fn sha_f1(b: u32, c: u32, d: u32) -> u32 {
    b ^ c ^ d
}

/// sha_f2(B,C,D) := (B AND C) XOR (B AND D) XOR (C AND D)
///
/// Used in SHA1 rounds 41-60 (Bochs sha_f2).
/// Also used as SHA256 Maj(A,B,C).
#[inline]
fn sha_f2(b: u32, c: u32, d: u32) -> u32 {
    (b & c) ^ (b & d) ^ (c & d)
}

/// sha_f(B,C,D,index) — select the SHA1 round function by index
///
/// Bochs sha_f: index 0 → sha_f0, index 2 → sha_f2, else → sha_f1
#[inline]
fn sha_f(b: u32, c: u32, d: u32, index: u32) -> u32 {
    if index == 0 {
        sha_f0(b, c, d)
    } else if index == 2 {
        sha_f2(b, c, d)
    } else {
        // sha_f1 and sha_f3 are the same
        sha_f1(b, c, d)
    }
}

/// SHA256 Ch(E,F,G) := (E AND F) XOR ((NOT E) AND G)
///
/// Same as sha_f0 (Bochs #define sha_ch).
#[inline]
fn sha_ch(e: u32, f: u32, g: u32) -> u32 {
    sha_f0(e, f, g)
}

/// SHA256 Maj(A,B,C) := (A AND B) XOR (A AND C) XOR (B AND C)
///
/// Same as sha_f2 (Bochs #define sha_maj).
#[inline]
fn sha_maj(a: u32, b: u32, c: u32) -> u32 {
    sha_f2(a, b, c)
}

/// SHA256 transformation with three rotations (Bochs sha256_transformation_rrr).
///
/// Used for Sigma0 and Sigma1.
#[inline]
fn sha256_transformation_rrr(val_32: u32, rotate1: u32, rotate2: u32, rotate3: u32) -> u32 {
    ror32(val_32, rotate1) ^ ror32(val_32, rotate2) ^ ror32(val_32, rotate3)
}

/// SHA256 transformation with two rotations and one shift (Bochs sha256_transformation_rrs).
///
/// Used for sigma0 and sigma1 (lowercase — message schedule functions).
#[inline]
fn sha256_transformation_rrs(val_32: u32, rotate1: u32, rotate2: u32, shr: u32) -> u32 {
    ror32(val_32, rotate1) ^ ror32(val_32, rotate2) ^ (val_32 >> shr)
}

// ============================================================================
// Instruction handlers
// ============================================================================

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// SHA1NEXTE Vdq, Wdq — 0F 38 C8
    ///
    /// Calculates SHA1 state variable E after four rounds:
    /// dst[3] = src[3] + ROL(dst[3], 30)
    /// Other dwords from src are passed through unchanged.
    pub fn sha1nexte_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.read_xmm_reg(instr.dst());
        let mut op2 = self.sse_read_op2_xmm(instr)?;

            op2.set_xmm32u(3, op2.xmm32u(3).wrapping_add(rol32(op1.xmm32u(3), 30)));

        self.write_xmm_reg_lo128(instr.dst(), op2);
        Ok(())
    }

    /// SHA1MSG1 Vdq, Wdq — 0F 38 C9
    ///
    /// Performs an intermediate calculation for the next four SHA1 message dwords.
    pub fn sha1msg1_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

            op1.set_xmm32u(3, op1.xmm32u(3) ^ op1.xmm32u(1));
            op1.set_xmm32u(2, op1.xmm32u(2) ^ op1.xmm32u(0));
            op1.set_xmm32u(1, op1.xmm32u(1) ^ op2.xmm32u(3));
            op1.set_xmm32u(0, op1.xmm32u(0) ^ op2.xmm32u(2));

        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// SHA1MSG2 Vdq, Wdq — 0F 38 CA
    ///
    /// Performs the final calculation for the next four SHA1 message dwords.
    pub fn sha1msg2_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

            op1.set_xmm32u(3, rol32(op1.xmm32u(3) ^ op2.xmm32u(2), 1));
            op1.set_xmm32u(2, rol32(op1.xmm32u(2) ^ op2.xmm32u(1), 1));
            op1.set_xmm32u(1, rol32(op1.xmm32u(1) ^ op2.xmm32u(0), 1));
            // Note: uses already-updated op1.xmm32u(3) (Bochs matches this)
            op1.set_xmm32u(0, rol32(op1.xmm32u(0) ^ op1.xmm32u(3), 1));

        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// SHA256RNDS2 Vdq, Wdq — 0F 38 CB
    ///
    /// Performs two rounds of SHA256 operation using:
    /// - dst (Vdq) = current state C,D,G,H
    /// - src (Wdq) = current state A,B,E,F
    /// - implicit XMM0 = WK (message + round constant)
    ///
    /// Only the lower two dwords of XMM0 (wk[0] and wk[1]) are used.
    pub fn sha256rnds2_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        let wk = self.read_xmm_reg(0); // implicit XMM0

            let mut a = [0u32; 3];
            let mut b = [0u32; 3];
            let mut c = [0u32; 3];
            let mut d = [0u32; 3];
            let mut e = [0u32; 3];
            let mut f = [0u32; 3];
            let mut g = [0u32; 3];
            let mut h = [0u32; 3];

            a[0] = op2.xmm32u(3);
            b[0] = op2.xmm32u(2);
            e[0] = op2.xmm32u(1);
            f[0] = op2.xmm32u(0);

            c[0] = op1.xmm32u(3);
            d[0] = op1.xmm32u(2);
            g[0] = op1.xmm32u(1);
            h[0] = op1.xmm32u(0);

            for n in 0..2usize {
                let tmp = sha_ch(e[n], f[n], g[n])
                    .wrapping_add(sha256_transformation_rrr(e[n], 6, 11, 25))
                    .wrapping_add(wk.xmm32u(n))
                    .wrapping_add(h[n]);
                a[n + 1] = tmp
                    .wrapping_add(sha_maj(a[n], b[n], c[n]))
                    .wrapping_add(sha256_transformation_rrr(a[n], 2, 13, 22));
                b[n + 1] = a[n];
                c[n + 1] = b[n];
                d[n + 1] = c[n];
                e[n + 1] = tmp.wrapping_add(d[n]);
                f[n + 1] = e[n];
                g[n + 1] = f[n];
                h[n + 1] = g[n];
            }

            let mut result = BxPackedXmmRegister::default();
            result.set_xmm32u(0, f[2]);
            result.set_xmm32u(1, e[2]);
            result.set_xmm32u(2, b[2]);
            result.set_xmm32u(3, a[2]);

            self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SHA256MSG1 Vdq, Wdq — 0F 38 CC
    ///
    /// Performs an intermediate calculation for the next four SHA256 message dwords.
    /// Uses sigma0 transformation: ror(x,7) ^ ror(x,18) ^ (x >> 3)
    pub fn sha256msg1_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2_dword0 = self.sse_read_op2_xmm(instr)?.xmm32u(0);

            op1.set_xmm32u(0, op1.xmm32u(0)
                .wrapping_add(sha256_transformation_rrs(op1.xmm32u(1), 7, 18, 3)));
            op1.set_xmm32u(1, op1.xmm32u(1)
                .wrapping_add(sha256_transformation_rrs(op1.xmm32u(2), 7, 18, 3)));
            op1.set_xmm32u(2, op1.xmm32u(2)
                .wrapping_add(sha256_transformation_rrs(op1.xmm32u(3), 7, 18, 3)));
            op1.set_xmm32u(3, op1.xmm32u(3).wrapping_add(sha256_transformation_rrs(op2_dword0, 7, 18, 3)));

        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// SHA256MSG2 Vdq, Wdq — 0F 38 CD
    ///
    /// Performs the final calculation for the next four SHA256 message dwords.
    /// Uses sigma1 transformation: ror(x,17) ^ ror(x,19) ^ (x >> 10)
    pub fn sha256msg2_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

            op1.set_xmm32u(0, op1.xmm32u(0)
                .wrapping_add(sha256_transformation_rrs(op2.xmm32u(2), 17, 19, 10)));
            op1.set_xmm32u(1, op1.xmm32u(1)
                .wrapping_add(sha256_transformation_rrs(op2.xmm32u(3), 17, 19, 10)));
            // Note: uses already-updated op1.xmm32u(0) and [1] (Bochs matches this)
            op1.set_xmm32u(2, op1.xmm32u(2)
                .wrapping_add(sha256_transformation_rrs(op1.xmm32u(0), 17, 19, 10)));
            op1.set_xmm32u(3, op1.xmm32u(3)
                .wrapping_add(sha256_transformation_rrs(op1.xmm32u(1), 17, 19, 10)));

        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// SHA1RNDS4 Vdq, Wdq, Ib — 0F 3A CC
    ///
    /// Performs four rounds of SHA1 operation.
    /// The immediate byte (bits 1:0) selects the round function and constant:
    ///   0 → K=0x5A827999, f0(B,C,D)
    ///   1 → K=0x6ED9EBA1, f1(B,C,D)
    ///   2 → K=0x8F1BBCDC, f2(B,C,D)
    ///   3 → K=0xCA62C1D6, f3(B,C,D) [same as f1]
    pub fn sha1rnds4_vdq_wdq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        // SHA1 Constants dependent on immediate
        const SHA_KI: [u32; 4] = [0x5A827999, 0x6ED9EBA1, 0x8F1BBCDC, 0xCA62C1D6];

        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        let imm = (instr.ib() & 0x3) as u32;
        let k = SHA_KI[imm as usize];

            let mut a = op1.xmm32u(3);
            let mut b = op1.xmm32u(2);
            let mut c = op1.xmm32u(1);
            let mut d = op1.xmm32u(0);
            let mut e: u32 = 0;

            let w = [op2.xmm32u(3), op2.xmm32u(2), op2.xmm32u(1), op2.xmm32u(0)];

            for n in 0..4 {
                let a_next = sha_f(b, c, d, imm)
                    .wrapping_add(rol32(a, 5))
                    .wrapping_add(w[n])
                    .wrapping_add(e)
                    .wrapping_add(k);

                e = d;
                d = c;
                c = rol32(b, 30);
                b = a;
                a = a_next;
            }

            op1.set_xmm32u(3, a);
            op1.set_xmm32u(2, b);
            op1.set_xmm32u(1, c);
            op1.set_xmm32u(0, d);

        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha_f0() {
        // Ch(B,C,D) = (B AND C) XOR (NOT(B) AND D)
        assert_eq!(sha_f0(0xFFFFFFFF, 0xAAAAAAAA, 0x55555555), 0xAAAAAAAA);
        assert_eq!(sha_f0(0x00000000, 0xAAAAAAAA, 0x55555555), 0x55555555);
    }

    #[test]
    fn test_sha_f1() {
        // B XOR C XOR D
        assert_eq!(sha_f1(0xFF00FF00, 0x00FF00FF, 0x0F0F0F0F), 0xF0F0F0F0);
    }

    #[test]
    fn test_sha_f2() {
        // Maj(B,C,D) = (B AND C) XOR (B AND D) XOR (C AND D)
        assert_eq!(sha_f2(0xFFFFFFFF, 0xAAAAAAAA, 0x55555555), 0xFFFFFFFF);
        assert_eq!(sha_f2(0x00000000, 0x00000000, 0x00000000), 0x00000000);
    }

    #[test]
    fn test_sha_f_dispatch() {
        let b = 0xFFFFFFFF;
        let c = 0xAAAAAAAA;
        let d = 0x55555555;
        assert_eq!(sha_f(b, c, d, 0), sha_f0(b, c, d));
        assert_eq!(sha_f(b, c, d, 1), sha_f1(b, c, d));
        assert_eq!(sha_f(b, c, d, 2), sha_f2(b, c, d));
        assert_eq!(sha_f(b, c, d, 3), sha_f1(b, c, d)); // f3 == f1
    }

    #[test]
    fn test_sha256_transformation_rrr() {
        // Sigma0(x) = ror(x,2) ^ ror(x,13) ^ ror(x,22)
        let x = 0x6A09E667; // SHA256 initial H0
        let result = sha256_transformation_rrr(x, 2, 13, 22);
        // Just verify it's deterministic and nonzero
        assert_ne!(result, 0);
        assert_ne!(result, x);
    }

    #[test]
    fn test_sha256_transformation_rrs() {
        // sigma0(x) = ror(x,7) ^ ror(x,18) ^ (x >> 3)
        let x = 0x428A2F98; // SHA256 K[0]
        let result = sha256_transformation_rrs(x, 7, 18, 3);
        assert_ne!(result, 0);
    }

    #[test]
    fn test_rol32_ror32() {
        assert_eq!(rol32(1, 1), 2);
        assert_eq!(rol32(0x80000000, 1), 1);
        assert_eq!(ror32(1, 1), 0x80000000);
        assert_eq!(ror32(2, 1), 1);
        assert_eq!(rol32(ror32(0xDEADBEEF, 7), 7), 0xDEADBEEF);
    }
}
