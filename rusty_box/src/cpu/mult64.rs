//! 64-bit multiplication and division instructions for x86 CPU emulation
//!
//! Based on Bochs mult64.cc
//! Copyright (C) 2001-2018 The Bochs Project

use super::{
    cpu::{BxCpuC, Exception},
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
    error::Result,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // 64-bit Multiplication and Division
    // =========================================================================

    /// MUL r/m64 - Unsigned multiply RAX by r/m64, result in RDX:RAX (register form)
    /// Matching C++ mult64.cc:MUL_RAXEqR
    pub fn mul_rax_eq_r(&mut self, instr: &Instruction) -> Result<()> {
        let op1 = self.rax(); // RAX
        let src_reg = instr.dst() as usize; // Group 3: rm field is in dst()
        let op2 = self.get_gpr64(src_reg);

        let product_128 = (op1 as u128) * (op2 as u128);
        let product_64l = product_128 as u64;
        let product_64h = (product_128 >> 64) as u64;

        // Write product back to RAX:RDX
        self.set_rax(product_64l);
        self.set_rdx(product_64h);

        // SET_FLAGS_OSZAPC_LOGIC_64: clears OF and CF, sets SF/ZF/PF from product_64l
        self.set_flags_oszapc_logic_64(product_64l);
        if product_64h != 0 {
            // assert CF and OF
            self.eflags.insert(EFlags::CF.union(EFlags::OF));
        }

        Ok(())
    }

    /// MUL r/m64 - Unsigned multiply RAX by r/m64, result in RDX:RAX (memory form)
    /// Matching Bochs LOAD_Eq + MUL_RAXEqR
    pub fn mul_rax_eq_m(&mut self, instr: &Instruction) -> Result<()> {
        let op1 = self.rax(); // RAX
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op2 = self.read_linear_qword(seg, laddr)?;

        let product_128 = (op1 as u128) * (op2 as u128);
        let product_64l = product_128 as u64;
        let product_64h = (product_128 >> 64) as u64;

        // Write product back to RAX:RDX
        self.set_rax(product_64l);
        self.set_rdx(product_64h);

        // SET_FLAGS_OSZAPC_LOGIC_64: clears OF and CF, sets SF/ZF/PF from product_64l
        self.set_flags_oszapc_logic_64(product_64l);
        if product_64h != 0 {
            // assert CF and OF
            self.eflags.insert(EFlags::CF.union(EFlags::OF));
        }

        Ok(())
    }

    /// IMUL r/m64 - Signed multiply RAX by r/m64, result in RDX:RAX (register form)
    /// Matching C++ mult64.cc:IMUL_RAXEqR
    pub fn imul_rax_eq_r(&mut self, instr: &Instruction) -> Result<()> {
        let op1 = self.rax() as i64;
        let src_reg = instr.dst() as usize; // Group 3: rm field is in dst()
        let op2 = self.get_gpr64(src_reg) as i64;

        let product_128 = (op1 as i128) * (op2 as i128);
        let product_64l = product_128 as u64;
        let product_64h = (product_128 >> 64) as u64;

        // Write product back to RAX:RDX
        self.set_rax(product_64l);
        self.set_rdx(product_64h);

        // SET_FLAGS_OSZAPC_LOGIC_64: clears OF and CF, sets SF/ZF/PF from product_64l
        self.set_flags_oszapc_logic_64(product_64l);

        // IMUL r/m64: CF and OF set if result doesn't fit in signed 64-bit
        // Bochs: if (((Bit64u)(product_128.hi) + (product_128.lo >> 63)) != 0)
        // This checks: does hi equal the sign-extension of lo's sign bit?
        if (product_64h).wrapping_add(product_64l >> 63) != 0 {
            self.eflags.insert(EFlags::CF.union(EFlags::OF));
        }

        Ok(())
    }

    /// IMUL r/m64 - Signed multiply RAX by r/m64, result in RDX:RAX (memory form)
    /// Matching Bochs LOAD_Eq + IMUL_RAXEqR
    pub fn imul_rax_eq_m(&mut self, instr: &Instruction) -> Result<()> {
        let op1 = self.rax() as i64;
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op2 = self.read_linear_qword(seg, laddr)? as i64;

        let product_128 = (op1 as i128) * (op2 as i128);
        let product_64l = product_128 as u64;
        let product_64h = (product_128 >> 64) as u64;

        // Write product back to RAX:RDX
        self.set_rax(product_64l);
        self.set_rdx(product_64h);

        // SET_FLAGS_OSZAPC_LOGIC_64: clears OF and CF, sets SF/ZF/PF from product_64l
        self.set_flags_oszapc_logic_64(product_64l);

        // IMUL r/m64: CF and OF set if result doesn't fit in signed 64-bit
        // Bochs: if (((Bit64u)(product_128.hi) + (product_128.lo >> 63)) != 0)
        if (product_64h).wrapping_add(product_64l >> 63) != 0 {
            self.eflags.insert(EFlags::CF.union(EFlags::OF));
        }

        Ok(())
    }

    /// DIV r/m64 - Unsigned divide RDX:RAX by r/m64, quotient in RAX, remainder in RDX
    /// (register form)
    /// Matching C++ mult64.cc:DIV_RAXEqR
    pub fn div_rax_eq_r(&mut self, instr: &Instruction) -> Result<()> {
        let src_reg = instr.dst() as usize; // Group 3: rm field is in dst()
        let op2 = self.get_gpr64(src_reg);

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let rax = self.rax();
        let rdx = self.rdx();
        let op1_128 = ((rdx as u128) << 64) | (rax as u128);

        let quotient_128 = op1_128 / (op2 as u128);
        let remainder_64 = (op1_128 % (op2 as u128)) as u64;
        let quotient_64l = quotient_128 as u64;

        // If quotient doesn't fit in 64-bit, #DE
        if (quotient_128 >> 64) != 0 {
            return self.exception(Exception::De, 0);
        }

        // DIV: O,S,Z,A,P,C are undefined — we leave them unchanged

        // Write quotient to RAX, remainder to RDX
        self.set_rax(quotient_64l);
        self.set_rdx(remainder_64);

        Ok(())
    }

    /// DIV r/m64 - Unsigned divide RDX:RAX by r/m64, quotient in RAX, remainder in RDX
    /// (memory form)
    /// Matching Bochs LOAD_Eq + DIV_RAXEqR
    pub fn div_rax_eq_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op2 = self.read_linear_qword(seg, laddr)?;

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let rax = self.rax();
        let rdx = self.rdx();
        let op1_128 = ((rdx as u128) << 64) | (rax as u128);

        let quotient_128 = op1_128 / (op2 as u128);
        let remainder_64 = (op1_128 % (op2 as u128)) as u64;
        let quotient_64l = quotient_128 as u64;

        // If quotient doesn't fit in 64-bit, #DE
        if (quotient_128 >> 64) != 0 {
            return self.exception(Exception::De, 0);
        }

        // Write quotient to RAX, remainder to RDX
        self.set_rax(quotient_64l);
        self.set_rdx(remainder_64);

        Ok(())
    }

    /// IDIV r/m64 - Signed divide RDX:RAX by r/m64, quotient in RAX, remainder in RDX
    /// (register form)
    /// Matching C++ mult64.cc:IDIV_RAXEqR
    pub fn idiv_rax_eq_r(&mut self, instr: &Instruction) -> Result<()> {
        let rax = self.rax();
        let rdx = self.rdx();

        let op1_128_lo = rax;
        let op1_128_hi = rdx as i64;

        // Check MIN_INT case: RDX=0x8000_0000_0000_0000, RAX=0
        // Bochs: if ((op1_128.hi == (Bit64s) 0x8000000000000000) && (!op1_128.lo))
        if op1_128_hi == i64::MIN && op1_128_lo == 0 {
            return self.exception(Exception::De, 0);
        }

        let src_reg = instr.dst() as usize; // Group 3: rm field is in dst()
        let op2 = self.get_gpr64(src_reg) as i64;

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        // Construct signed 128-bit dividend from RDX:RAX
        let op1_128 = ((rdx as i128) << 64) | (rax as i128);

        let quotient_128 = op1_128 / (op2 as i128);
        let remainder_64 = (op1_128 % (op2 as i128)) as i64;
        let quotient_64l = quotient_128 as i64;

        // Bochs overflow check:
        // if ((!(quotient_128.lo & 0x8000...) && quotient_128.hi != 0) ||
        //      ((quotient_128.lo & 0x8000...) && quotient_128.hi != 0xffff...))
        // In i128 terms: quotient must fit in i64 (sign-extend check)
        if quotient_128 != (quotient_64l as i128) {
            return self.exception(Exception::De, 0);
        }

        // IDIV: O,S,Z,A,P,C are undefined — leave unchanged

        // Write quotient to RAX, remainder to RDX
        self.set_rax(quotient_64l as u64);
        self.set_rdx(remainder_64 as u64);

        Ok(())
    }

    /// IDIV r/m64 - Signed divide RDX:RAX by r/m64, quotient in RAX, remainder in RDX
    /// (memory form)
    /// Matching Bochs LOAD_Eq + IDIV_RAXEqR
    pub fn idiv_rax_eq_m(&mut self, instr: &Instruction) -> Result<()> {
        let rax = self.rax();
        let rdx = self.rdx();

        let op1_128_lo = rax;
        let op1_128_hi = rdx as i64;

        // Check MIN_INT case
        if op1_128_hi == i64::MIN && op1_128_lo == 0 {
            return self.exception(Exception::De, 0);
        }

        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op2 = self.read_linear_qword(seg, laddr)? as i64;

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let op1_128 = ((rdx as i128) << 64) | (rax as i128);

        let quotient_128 = op1_128 / (op2 as i128);
        let remainder_64 = (op1_128 % (op2 as i128)) as i64;
        let quotient_64l = quotient_128 as i64;

        if quotient_128 != (quotient_64l as i128) {
            return self.exception(Exception::De, 0);
        }

        self.set_rax(quotient_64l as u64);
        self.set_rdx(remainder_64 as u64);

        Ok(())
    }

    /// IMUL Gq, Eq - Two-operand signed multiply (register form)
    /// dst = dst * src, only lower 64 bits stored
    /// Matching C++ mult64.cc:IMUL_GqEqR
    pub fn imul_gq_eq_r(&mut self, instr: &Instruction) -> Result<()> {
        let dst_reg = instr.dst() as usize;
        let src_reg = instr.src() as usize;

        let op1 = self.get_gpr64(dst_reg) as i64;
        let op2 = self.get_gpr64(src_reg) as i64;

        let product_128 = (op1 as i128) * (op2 as i128);
        let product_64l = product_128 as u64;
        let product_64h = (product_128 >> 64) as u64;

        self.set_gpr64(dst_reg, product_64l);

        self.set_flags_oszapc_logic_64(product_64l);
        // Bochs: if (((Bit64u)(product_128.hi) + (product_128.lo >> 63)) != 0)
        if product_64h.wrapping_add(product_64l >> 63) != 0 {
            self.eflags.insert(EFlags::CF.union(EFlags::OF));
        }

        Ok(())
    }

    /// IMUL Gq, Eq - Two-operand signed multiply (memory form)
    /// Matching Bochs LOAD_Eq + IMUL_GqEqR
    pub fn imul_gq_eq_m(&mut self, instr: &Instruction) -> Result<()> {
        let dst_reg = instr.dst() as usize;

        let op1 = self.get_gpr64(dst_reg) as i64;
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op2 = self.read_linear_qword(seg, laddr)? as i64;

        let product_128 = (op1 as i128) * (op2 as i128);
        let product_64l = product_128 as u64;
        let product_64h = (product_128 >> 64) as u64;

        self.set_gpr64(dst_reg, product_64l);

        self.set_flags_oszapc_logic_64(product_64l);
        if product_64h.wrapping_add(product_64l >> 63) != 0 {
            self.eflags.insert(EFlags::CF.union(EFlags::OF));
        }

        Ok(())
    }

    /// IMUL Gq, Eq, Id - Three-operand signed multiply with sign-extended 32-bit immediate
    /// (register form)
    /// Opcode: REX.W 69 /r id  or  REX.W 6B /r ib
    /// Matching C++ mult64.cc:IMUL_GqEqIdR (also used for IMUL_GqEqsIb)
    pub fn imul_gq_eq_id_r(&mut self, instr: &Instruction) -> Result<()> {
        let dst_reg = instr.dst() as usize;
        let src_reg = instr.src() as usize;

        let op1 = self.get_gpr64(src_reg) as i64;
        // op2 is sign-extended 32-bit immediate (id() is already sign-extended in the decoder)
        let op2 = instr.id() as i32 as i64;

        let product_128 = (op1 as i128) * (op2 as i128);
        let product_64l = product_128 as u64;
        let product_64h = (product_128 >> 64) as u64;

        self.set_gpr64(dst_reg, product_64l);

        self.set_flags_oszapc_logic_64(product_64l);
        // Bochs: if (((Bit64u)(product_128.hi) + (product_128.lo >> 63)) != 0)
        if product_64h.wrapping_add(product_64l >> 63) != 0 {
            self.eflags.insert(EFlags::CF.union(EFlags::OF));
        } else {
            self.eflags.remove(EFlags::CF.union(EFlags::OF));
        }

        Ok(())
    }

    /// IMUL Gq, Eq, Id - Three-operand signed multiply with sign-extended 32-bit immediate
    /// (memory form)
    /// Matching Bochs LOAD_Eq + IMUL_GqEqIdR
    pub fn imul_gq_eq_id_m(&mut self, instr: &Instruction) -> Result<()> {
        let dst_reg = instr.dst() as usize;

        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op1 = self.read_linear_qword(seg, laddr)? as i64;
        let op2 = instr.id() as i32 as i64;

        let product_128 = (op1 as i128) * (op2 as i128);
        let product_64l = product_128 as u64;
        let product_64h = (product_128 >> 64) as u64;

        self.set_gpr64(dst_reg, product_64l);

        self.set_flags_oszapc_logic_64(product_64l);
        if product_64h.wrapping_add(product_64l >> 63) != 0 {
            self.eflags.insert(EFlags::CF.union(EFlags::OF));
        } else {
            self.eflags.remove(EFlags::CF.union(EFlags::OF));
        }

        Ok(())
    }

    /// IMUL Gq, Eq, sIb - Three-operand signed multiply with sign-extended 8-bit immediate
    /// (register form)
    /// Opcode: REX.W 6B /r ib
    /// Note: Bochs uses IMUL_GqEqIdR for both sId and sIb forms (immediate is pre-sign-extended
    /// by the decoder into the Id field). This separate function handles the 8-bit immediate
    /// case where instr.ib() is used.
    pub fn imul_gq_eq_sib_r(&mut self, instr: &Instruction) -> Result<()> {
        let dst_reg = instr.dst() as usize;
        let src_reg = instr.src() as usize;

        let op1 = self.get_gpr64(src_reg) as i64;
        // 8-bit immediate, sign-extended to 64-bit
        let op2 = instr.ib() as i8 as i64;

        let product_128 = (op1 as i128) * (op2 as i128);
        let product_64l = product_128 as u64;
        let product_64h = (product_128 >> 64) as u64;

        self.set_gpr64(dst_reg, product_64l);

        self.set_flags_oszapc_logic_64(product_64l);
        if product_64h.wrapping_add(product_64l >> 63) != 0 {
            self.eflags.insert(EFlags::CF.union(EFlags::OF));
        } else {
            self.eflags.remove(EFlags::CF.union(EFlags::OF));
        }

        Ok(())
    }

    /// IMUL Gq, Eq, sIb - Three-operand signed multiply with sign-extended 8-bit immediate
    /// (memory form)
    pub fn imul_gq_eq_sib_m(&mut self, instr: &Instruction) -> Result<()> {
        let dst_reg = instr.dst() as usize;

        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op1 = self.read_linear_qword(seg, laddr)? as i64;
        let op2 = instr.ib() as i8 as i64;

        let product_128 = (op1 as i128) * (op2 as i128);
        let product_64l = product_128 as u64;
        let product_64h = (product_128 >> 64) as u64;

        self.set_gpr64(dst_reg, product_64l);

        self.set_flags_oszapc_logic_64(product_64l);
        if product_64h.wrapping_add(product_64l >> 63) != 0 {
            self.eflags.insert(EFlags::CF.union(EFlags::OF));
        } else {
            self.eflags.remove(EFlags::CF.union(EFlags::OF));
        }

        Ok(())
    }

    // =========================================================================
    // Unified wrappers (dispatch register vs memory form based on mod_c0)
    // =========================================================================

    /// MUL RAX, r/m64 - Unified wrapper
    pub fn mul_rax_eq(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.mul_rax_eq_r(instr)
        } else {
            self.mul_rax_eq_m(instr)
        }
    }

    /// IMUL RAX, r/m64 - Unified wrapper
    pub fn imul_rax_eq(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.imul_rax_eq_r(instr)
        } else {
            self.imul_rax_eq_m(instr)
        }
    }

    /// DIV RAX, r/m64 - Unified wrapper
    pub fn div_rax_eq(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.div_rax_eq_r(instr)
        } else {
            self.div_rax_eq_m(instr)
        }
    }

    /// IDIV RAX, r/m64 - Unified wrapper
    pub fn idiv_rax_eq(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.idiv_rax_eq_r(instr)
        } else {
            self.idiv_rax_eq_m(instr)
        }
    }

    /// IMUL Gq, Eq - Two-operand signed multiply - Unified wrapper
    pub fn imul_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.imul_gq_eq_r(instr)
        } else {
            self.imul_gq_eq_m(instr)
        }
    }

    /// IMUL Gq, Eq, Id - Three-operand signed multiply with 32-bit immediate - Unified wrapper
    pub fn imul_gq_eq_id(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.imul_gq_eq_id_r(instr)
        } else {
            self.imul_gq_eq_id_m(instr)
        }
    }

    /// IMUL Gq, Eq, sIb - Three-operand signed multiply with 8-bit immediate - Unified wrapper
    pub fn imul_gq_eq_sib(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.imul_gq_eq_sib_r(instr)
        } else {
            self.imul_gq_eq_sib_m(instr)
        }
    }

}
