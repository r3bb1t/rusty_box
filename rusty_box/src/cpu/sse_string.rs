//! SSE4.2 string comparison instructions (PCMPESTRM, PCMPESTRI, PCMPISTRM, PCMPISTRI)
//!
//! Based on Bochs cpu/sse_string.cc
//!
//! Implements the four SSE4.2 string/text processing instructions:
//! - PCMPESTRM (66 0F 3A 60): Explicit-length, result to XMM0
//! - PCMPESTRI (66 0F 3A 61): Explicit-length, index to ECX
//! - PCMPISTRM (66 0F 3A 62): Implicit-length (null-terminated), result to XMM0
//! - PCMPISTRI (66 0F 3A 63): Implicit-length, index to ECX

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::Instruction,
    xmm::BxPackedXmmRegister,
};

// ============================================================================
// Helper functions (free functions matching Bochs static helpers)
// ============================================================================

/// Compare all pairs of Ai, Bj according to imm8 control.
///
/// Based on imm bits 0-1 (source data type):
///   0: unsigned bytes (16 elements)
///   1: unsigned words (8 elements)
///   2: signed bytes (16 elements)
///   3: signed words (8 elements)
///
/// Based on imm bits 2-3 (aggregation, affects comparison type):
///   0,2,3: 'equal' comparison
///   1: 'ranges' comparison (even index = <=, odd index = >=)
///
/// For each pair (i,j), the result is stored in bool_res[j][i].
#[allow(clippy::needless_range_loop)]
fn compare_strings(
    bool_res: &mut [[u8; 16]; 16],
    op1: &BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    imm: u8,
) {
    let aggregation_operation = ((imm >> 2) & 3) as usize;

    match imm & 3 {
        0 => {
            // unsigned bytes compare
            for i in 0..16 {
                for j in 0..16 {
                    let a = op1.xmmubyte(i);
                    let b = op2.xmmubyte(j);
                    bool_res[j][i] = match aggregation_operation {
                        0 | 2 | 3 => {
                            // 'equal' comparison
                            if a == b { 1 } else { 0 }
                        }
                        1 => {
                            // 'ranges' comparison
                            if (i % 2) == 0 {
                                if a <= b { 1 } else { 0 }
                            } else {
                                if a >= b { 1 } else { 0 }
                            }
                        }
                        _ => unreachable!("aggregation_operation & 3 cannot exceed 3"),
                    };
                }
            }
        }
        1 => {
            // unsigned words compare
            for i in 0..8 {
                for j in 0..8 {
                    let a = op1.xmm16u(i);
                    let b = op2.xmm16u(j);
                    bool_res[j][i] = match aggregation_operation {
                        0 | 2 | 3 => {
                            if a == b { 1 } else { 0 }
                        }
                        1 => {
                            if (i % 2) == 0 {
                                if a <= b { 1 } else { 0 }
                            } else {
                                if a >= b { 1 } else { 0 }
                            }
                        }
                        _ => unreachable!("aggregation_operation & 3 cannot exceed 3"),
                    };
                }
            }
        }
        2 => {
            // signed bytes compare
            for i in 0..16 {
                for j in 0..16 {
                    let a = op1.xmm_sbyte(i);
                    let b = op2.xmm_sbyte(j);
                    bool_res[j][i] = match aggregation_operation {
                        0 | 2 | 3 => {
                            if a == b { 1 } else { 0 }
                        }
                        1 => {
                            if (i % 2) == 0 {
                                if a <= b { 1 } else { 0 }
                            } else {
                                if a >= b { 1 } else { 0 }
                            }
                        }
                        _ => unreachable!("aggregation_operation & 3 cannot exceed 3"),
                    };
                }
            }
        }
        3 => {
            // signed words compare
            for i in 0..8 {
                for j in 0..8 {
                    let a = op1.xmm16s(i);
                    let b = op2.xmm16s(j);
                    bool_res[j][i] = match aggregation_operation {
                        0 | 2 | 3 => {
                            if a == b { 1 } else { 0 }
                        }
                        1 => {
                            if (i % 2) == 0 {
                                if a <= b { 1 } else { 0 }
                            } else {
                                if a >= b { 1 } else { 0 }
                            }
                        }
                        _ => unreachable!("aggregation_operation & 3 cannot exceed 3"),
                    };
                }
            }
        }
        _ => unreachable!("imm8 & 3 cannot exceed 3"),
    }
}

/// Find first null terminator in implicit-length ops.
///
/// If imm & 1 (words): scan xmm16u for first 0, return index (max 8).
/// Else (bytes): scan xmmubyte for first 0, return index (max 16).
fn find_eos(op: &BxPackedXmmRegister, imm: u8) -> usize {
    if imm & 0x1 != 0 {
        // 8 elements (words)
        for i in 0..8 {
            if op.xmm16u(i) == 0 {
                return i;
            }
        }
        8
    } else {
        // 16 elements (bytes)
        for i in 0..16 {
            if op.xmmubyte(i) == 0 {
                return i;
            }
        }
        16
    }
}

/// Clamp explicit length for 32-bit mode.
///
/// If imm & 1: clamp abs(reg32) to 8.
/// Else: clamp abs(reg32) to 16.
fn find_eos32(reg32: i32, imm: u8) -> usize {
    if imm & 0x1 != 0 {
        // 8 elements
        if !(-8..=8).contains(&reg32) {
            8
        } else {
            reg32.unsigned_abs() as usize
        }
    } else {
        // 16 elements
        if !(-16..=16).contains(&reg32) {
            16
        } else {
            reg32.unsigned_abs() as usize
        }
    }
}

/// Clamp explicit length for 64-bit mode.
///
/// If imm & 1: clamp abs(reg64) to 8.
/// Else: clamp abs(reg64) to 16.
fn find_eos64(reg64: i64, imm: u8) -> usize {
    if imm & 0x1 != 0 {
        // 8 elements
        if !(-8..=8).contains(&reg64) {
            8
        } else {
            reg64.unsigned_abs() as usize
        }
    } else {
        // 16 elements
        if !(-16..=16).contains(&reg64) {
            16
        } else {
            reg64.unsigned_abs() as usize
        }
    }
}

/// Override comparison result when one or both elements are invalid
/// (past the end of the string).
///
/// Based on aggregation_operation = (imm >> 2) & 3:
///   0 (equal any): invalid -> false
///   1 (ranges): invalid -> false
///   2 (equal each): both invalid -> true, one invalid -> false
///   3 (equal ordered): i invalid -> true, only j invalid -> false
fn override_if_data_invalid(val: bool, i_valid: bool, j_valid: bool, imm: u8) -> bool {
    let aggregation_operation = ((imm >> 2) & 3) as usize;

    match aggregation_operation {
        0 | 1 => {
            // 'equal any' / 'ranges'
            if !i_valid || !j_valid {
                return false;
            }
        }
        2 => {
            // 'equal each'
            if !i_valid {
                if !j_valid {
                    return true; // both elements are invalid
                } else {
                    return false; // only i is invalid
                }
            } else if !j_valid {
                return false; // only j is invalid
            }
        }
        3 => {
            // 'equal ordered'
            if !i_valid {
                // element i is invalid
                return true;
            } else if !j_valid {
                // only j is invalid
                return false;
            }
        }
        _ => unreachable!("aggregation_operation & 3 cannot exceed 3"),
    }

    val
}

/// Aggregate boolean comparison results according to imm8 control.
///
/// Based on aggregation_operation = (imm >> 2) & 3:
///   0 (equal any): for each j, OR across all i comparisons
///   1 (ranges): for each j, check pairs (i, i+1) as range bounds
///   2 (equal each): diagonal comparison bool_res[j][j]
///   3 (equal ordered): substring match starting at each j
///
/// Then apply polarity (imm bits 4-5):
///   0,2: no change
///   1: XOR with all-ones mask
///   3: XOR only valid positions (j < len2)
#[allow(clippy::needless_range_loop)]
fn aggregate(bool_res: &[[u8; 16]; 16], len1: usize, len2: usize, imm: u8) -> u16 {
    let aggregation_operation = ((imm >> 2) & 3) as usize;
    let num_elements: usize = if imm & 0x1 != 0 { 8 } else { 16 };
    let polarity = ((imm >> 4) & 3) as usize;

    let mut result: u16 = 0;

    match aggregation_operation {
        0 => {
            // 'equal any'
            for j in 0..num_elements {
                let mut res = false;
                for i in 0..num_elements {
                    if override_if_data_invalid(
                        bool_res[j][i] != 0,
                        i < len1,
                        j < len2,
                        imm,
                    ) {
                        res = true;
                        break;
                    }
                }
                if res {
                    result |= 1 << j;
                }
            }
        }
        1 => {
            // 'ranges'
            for j in 0..num_elements {
                let mut res = false;
                let mut i = 0;
                while i < num_elements {
                    if override_if_data_invalid(
                        bool_res[j][i] != 0,
                        i < len1,
                        j < len2,
                        imm,
                    ) && override_if_data_invalid(
                        bool_res[j][i + 1] != 0,
                        (i + 1) < len1,
                        j < len2,
                        imm,
                    ) {
                        res = true;
                        break;
                    }
                    i += 2;
                }
                if res {
                    result |= 1 << j;
                }
            }
        }
        2 => {
            // 'equal each'
            for j in 0..num_elements {
                if override_if_data_invalid(
                    bool_res[j][j] != 0,
                    j < len1,
                    j < len2,
                    imm,
                ) {
                    result |= 1 << j;
                }
            }
        }
        3 => {
            // 'equal ordered'
            for j in 0..num_elements {
                let mut res = true;
                let mut i = 0;
                let mut k = j;
                while i < (num_elements - j) && k < num_elements {
                    if !override_if_data_invalid(
                        bool_res[k][i] != 0,
                        i < len1,
                        k < len2,
                        imm,
                    ) {
                        res = false;
                        break;
                    }
                    i += 1;
                    k += 1;
                }
                if res {
                    result |= 1 << j;
                }
            }
        }
        _ => unreachable!("aggregation_operation & 3 cannot exceed 3"),
    }

    // Apply polarity
    match polarity {
        0 | 2 => {
            // do nothing
        }
        1 => {
            result ^= if num_elements == 8 { 0xFF } else { 0xFFFF };
        }
        3 => {
            for j in 0..num_elements {
                if j < len2 {
                    result ^= 1 << j; // flip the bit
                }
            }
        }
        _ => unreachable!("polarity & 3 cannot exceed 3"),
    }

    result
}

// ============================================================================
// SSE4.2 instruction handlers
// ============================================================================

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// PCMPESTRM — Packed Compare Explicit-Length Strings, Return Mask (66 0F 3A 60)
    ///
    /// Lengths from EAX/RAX (op1 length) and EDX/RDX (op2 length).
    /// Result written to XMM0.
    /// Sets CF, ZF, SF, OF; clears AF, PF.
    pub(super) fn pcmpestrm_vdq_wdq_ib(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        let imm8 = instr.ib();

        // Compare all pairs of Ai, Bj
        let mut bool_res = [[0u8; 16]; 16];
        compare_strings(&mut bool_res, &op1, &op2, imm8);

        let num_elements: usize = if imm8 & 0x1 != 0 { 8 } else { 16 };

        let (len1, len2) = if instr.os64_l() != 0 {
            (
                find_eos64(self.rax() as i64, imm8),
                find_eos64(self.rdx() as i64, imm8),
            )
        } else {
            (
                find_eos32(self.eax() as i32, imm8),
                find_eos32(self.edx() as i32, imm8),
            )
        };

        let result2 = aggregate(&bool_res, len1, len2, imm8);

        // As defined by imm8[6], result2 is then either stored to the least
        // significant bits of XMM0 (zero extended to 128 bits) or expanded
        // into a byte/word-mask and then stored to XMM0
        let mut result = BxPackedXmmRegister::default();
        if imm8 & 0x40 != 0 {
            if num_elements == 8 {
                for index in 0..8 {
                        result.set_xmm16u(index, if result2 & (1 << index) != 0 { 0xFFFF } else { 0 });
                }
            } else {
                // num_elements = 16
                for index in 0..16 {
                        result.set_xmmubyte(index, if result2 & (1 << index) != 0 { 0xFF } else { 0 });
                }
            }
        } else {
                result.set_xmm64u(1, 0);
                result.set_xmm64u(0, result2 as u64);
        }

        // Set flags: CF, ZF, SF, OF; clear AF, PF
        self.set_of(false); self.set_sf(false); self.set_zf(false);
                self.set_af(false); self.set_pf(false); self.set_cf(false);
        if result2 != 0 {
            self.set_cf(true);
        }
        if len1 < num_elements {
            self.set_sf(true);
        }
        if len2 < num_elements {
            self.set_zf(true);
        }
        if result2 & 0x1 != 0 {
            self.set_of(true);
        }

        // Store result to XMM0
        self.write_xmm_reg_lo128(0, result);

        Ok(())
    }

    /// PCMPESTRI — Packed Compare Explicit-Length Strings, Return Index (66 0F 3A 61)
    ///
    /// Lengths from EAX/RAX (op1 length) and EDX/RDX (op2 length).
    /// Index of first/last set bit written to ECX/RCX.
    /// Sets CF, ZF, SF, OF; clears AF, PF.
    pub(super) fn pcmpestri_vdq_wdq_ib(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        let imm8 = instr.ib();

        // Compare all pairs of Ai, Bj
        let mut bool_res = [[0u8; 16]; 16];
        compare_strings(&mut bool_res, &op1, &op2, imm8);

        let num_elements: usize = if imm8 & 0x1 != 0 { 8 } else { 16 };

        let (len1, len2) = if instr.os64_l() != 0 {
            (
                find_eos64(self.rax() as i64, imm8),
                find_eos64(self.rdx() as i64, imm8),
            )
        } else {
            (
                find_eos32(self.eax() as i32, imm8),
                find_eos32(self.edx() as i32, imm8),
            )
        };

        let result2 = aggregate(&bool_res, len1, len2, imm8);

        // The index of the first (or last, according to imm8[6]) set bit of result2
        // is returned to ECX. If no bits are set in IntRes2, ECX is set to 16 (8)
        let index: usize;
        if imm8 & 0x40 != 0 {
            // The index returned to ECX is of the MSB in result2
            let mut idx: i32 = -1;
            for k in (0..num_elements as i32).rev() {
                if result2 & (1 << k) != 0 {
                    idx = k;
                    break;
                }
            }
            index = if idx < 0 {
                num_elements
            } else {
                idx as usize
            };
        } else {
            // The index returned to ECX is of the LSB in result2
            let mut idx = num_elements;
            for k in 0..num_elements {
                if result2 & (1 << k) != 0 {
                    idx = k;
                    break;
                }
            }
            index = idx;
        }
        self.set_rcx(index as u64);

        // Set flags: CF, ZF, SF, OF; clear AF, PF
        self.set_of(false); self.set_sf(false); self.set_zf(false);
                self.set_af(false); self.set_pf(false); self.set_cf(false);
        if result2 != 0 {
            self.set_cf(true);
        }
        if len1 < num_elements {
            self.set_sf(true);
        }
        if len2 < num_elements {
            self.set_zf(true);
        }
        if result2 & 0x1 != 0 {
            self.set_of(true);
        }

        Ok(())
    }

    /// PCMPISTRM — Packed Compare Implicit-Length Strings, Return Mask (66 0F 3A 62)
    ///
    /// Lengths determined by scanning operands for null terminators.
    /// Result written to XMM0.
    /// Sets CF, ZF, SF, OF; clears AF, PF.
    pub(super) fn pcmpistrm_vdq_wdq_ib(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        let imm8 = instr.ib();

        // Compare all pairs of Ai, Bj
        let mut bool_res = [[0u8; 16]; 16];
        compare_strings(&mut bool_res, &op1, &op2, imm8);

        let num_elements: usize = if imm8 & 0x1 != 0 { 8 } else { 16 };
        let len1 = find_eos(&op1, imm8);
        let len2 = find_eos(&op2, imm8);
        let result2 = aggregate(&bool_res, len1, len2, imm8);

        // As defined by imm8[6], result2 is then either stored to the least
        // significant bits of XMM0 (zero extended to 128 bits) or expanded
        // into a byte/word-mask and then stored to XMM0
        let mut result = BxPackedXmmRegister::default();
        if imm8 & 0x40 != 0 {
            if num_elements == 8 {
                for index in 0..8 {
                        result.set_xmm16u(index, if result2 & (1 << index) != 0 { 0xFFFF } else { 0 });
                }
            } else {
                // num_elements = 16
                for index in 0..16 {
                        result.set_xmmubyte(index, if result2 & (1 << index) != 0 { 0xFF } else { 0 });
                }
            }
        } else {
                result.set_xmm64u(1, 0);
                result.set_xmm64u(0, result2 as u64);
        }

        // Set flags: CF, ZF, SF, OF; clear AF, PF
        self.set_of(false); self.set_sf(false); self.set_zf(false);
                self.set_af(false); self.set_pf(false); self.set_cf(false);
        if result2 != 0 {
            self.set_cf(true);
        }
        if len1 < num_elements {
            self.set_sf(true);
        }
        if len2 < num_elements {
            self.set_zf(true);
        }
        if result2 & 0x1 != 0 {
            self.set_of(true);
        }

        // Store result to XMM0
        self.write_xmm_reg_lo128(0, result);

        Ok(())
    }

    /// PCMPISTRI — Packed Compare Implicit-Length Strings, Return Index (66 0F 3A 63)
    ///
    /// Lengths determined by scanning operands for null terminators.
    /// Index of first/last set bit written to ECX/RCX.
    /// Sets CF, ZF, SF, OF; clears AF, PF.
    pub(super) fn pcmpistri_vdq_wdq_ib(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        let imm8 = instr.ib();

        // Compare all pairs of Ai, Bj
        let mut bool_res = [[0u8; 16]; 16];
        compare_strings(&mut bool_res, &op1, &op2, imm8);

        let num_elements: usize = if imm8 & 0x1 != 0 { 8 } else { 16 };
        let len1 = find_eos(&op1, imm8);
        let len2 = find_eos(&op2, imm8);
        let result2 = aggregate(&bool_res, len1, len2, imm8);

        // The index of the first (or last, according to imm8[6]) set bit of result2
        // is returned to ECX. If no bits are set in IntRes2, ECX is set to 16 (8)
        let index: usize;
        if imm8 & 0x40 != 0 {
            // The index returned to ECX is of the MSB in result2
            let mut idx: i32 = -1;
            for k in (0..num_elements as i32).rev() {
                if result2 & (1 << k) != 0 {
                    idx = k;
                    break;
                }
            }
            index = if idx < 0 {
                num_elements
            } else {
                idx as usize
            };
        } else {
            // The index returned to ECX is of the LSB in result2
            let mut idx = num_elements;
            for k in 0..num_elements {
                if result2 & (1 << k) != 0 {
                    idx = k;
                    break;
                }
            }
            index = idx;
        }
        self.set_rcx(index as u64);

        // Set flags: CF, ZF, SF, OF; clear AF, PF
        self.set_of(false); self.set_sf(false); self.set_zf(false);
                self.set_af(false); self.set_pf(false); self.set_cf(false);
        if result2 != 0 {
            self.set_cf(true);
        }
        if len1 < num_elements {
            self.set_sf(true);
        }
        if len2 < num_elements {
            self.set_zf(true);
        }
        if result2 & 0x1 != 0 {
            self.set_of(true);
        }

        Ok(())
    }
}
