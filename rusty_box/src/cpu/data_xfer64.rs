//! 64-bit data transfer instructions for x86 CPU emulation
//!
//! Based on Bochs data_xfer64.cc
//! Copyright (C) 2001-2018 The Bochs Project

use crate::cpu::decoder::{BxSegregs, Instruction};
use crate::cpu::{BxCpuC, BxCpuIdTrait};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // 64-bit MOV instructions
    // =========================================================================

    /// MOV r64, imm64 (register form)
    /// Matching C++ data_xfer64.cc:29-34 MOV_RRXIq
    pub fn mov_rrxiq(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let imm64 = instr.iq();

        self.set_gpr64(dst, imm64);
        tracing::trace!("MOV64: reg{} = {:#018x}", dst, imm64);
    }

    /// MOV r32, r/m32 (memory form, 64-bit addressing)
    /// Matching C++ data_xfer64.cc:36-43 MOV64_GdEdM
    pub fn mov64_gd_ed_m(&mut self, instr: &Instruction) {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let val32 = self.read_linear_dword(seg, laddr);
        let dst_reg = instr.dst() as usize;

        self.set_gpr32(dst_reg, val32);
        tracing::trace!(
            "MOV64 mem: reg{} = [{:?}:{:#x}] ({:#010x})",
            dst_reg,
            seg,
            eaddr,
            val32
        );
    }

    /// MOV r/m32, r32 (memory form, 64-bit addressing)
    /// Matching C++ data_xfer64.cc:45-52 MOV64_EdGdM
    pub fn mov64_ed_gd_m(&mut self, instr: &Instruction) {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let src_reg = instr.src() as usize;
        let val32 = self.get_gpr32(src_reg);

        self.write_linear_dword(seg, laddr, val32);
        tracing::trace!(
            "MOV64 mem: [{:?}:{:#x}] = reg{} ({:#010x})",
            seg,
            eaddr,
            src_reg,
            val32
        );
    }

    /// MOV r/m64, r64 (memory form)
    /// Matching C++ data_xfer64.cc:54-61 MOV_EqGqM
    pub fn mov_eq_gq_m(&mut self, instr: &Instruction) {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let src_reg = instr.src() as usize;
        let val64 = self.get_gpr64(src_reg);

        self.write_linear_qword(seg, laddr, val64);
        tracing::trace!(
            "MOV64 mem: [{:?}:{:#x}] = reg{} ({:#018x})",
            seg,
            eaddr,
            src_reg,
            val64
        );
    }

    /// MOV r/m64, r64 (stack form)
    /// Matching C++ data_xfer64.cc:63-70 MOV64S_EqGqM
    pub fn mov64s_eq_gq_m(&mut self, instr: &Instruction) {
        let eaddr = self.resolve_addr64(instr);
        let src_reg = instr.src() as usize;
        let val64 = self.get_gpr64(src_reg);

        self.stack_write_qword(eaddr, val64);
        tracing::trace!(
            "MOV64 stack: [{:#x}] = reg{} ({:#018x})",
            eaddr,
            src_reg,
            val64
        );
    }

    /// MOV r64, r/m64 (memory form)
    /// Matching C++ data_xfer64.cc:72-80 MOV_GqEqM
    pub fn mov_gq_eq_m(&mut self, instr: &Instruction) {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let val64 = self.read_linear_qword(seg, laddr);
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, val64);
        tracing::trace!(
            "MOV64 mem: reg{} = [{:?}:{:#x}] ({:#018x})",
            dst_reg,
            seg,
            eaddr,
            val64
        );
    }

    /// MOV r64, r/m64 (stack form)
    /// Matching C++ data_xfer64.cc:82-89 MOV64S_GqEqM
    pub fn mov64s_gq_eq_m(&mut self, instr: &Instruction) {
        let eaddr = self.resolve_addr64(instr);
        let val64 = self.stack_read_qword(eaddr);
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, val64);
        tracing::trace!(
            "MOV64 stack: reg{} = [{:#x}] ({:#018x})",
            dst_reg,
            eaddr,
            val64
        );
    }

    /// MOV r64, r64 (register form)
    /// Matching C++ data_xfer64.cc:91-96 MOV_GqEqR
    pub fn mov_gq_eq_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let val64 = self.get_gpr64(src_reg);
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, val64);
        tracing::trace!("MOV64: reg{} = reg{} ({:#018x})", dst_reg, src_reg, val64);
    }

    /// LEA r64, m - Load effective address into 64-bit register
    /// Matching C++ data_xfer64.cc:98-105 LEA_GqM
    pub fn lea_gq_m(&mut self, instr: &Instruction) {
        let eaddr = self.resolve_addr64(instr);
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, eaddr);
        tracing::trace!("LEA64: reg{} = {:#018x}", dst_reg, eaddr);
    }

    /// MOV AL, moffs64
    /// Matching C++ data_xfer64.cc:107-112 MOV_ALOq
    pub fn mov_aloq(&mut self, instr: &Instruction) {
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, instr.iq());
        let val8 = self.read_linear_byte(seg, laddr);

        self.set_gpr8(0, val8); // AL
        tracing::trace!("MOV AL, [{:?}:{:#018x}] = {:#04x}", seg, instr.iq(), val8);
    }

    /// MOV moffs64, AL
    /// Matching C++ data_xfer64.cc:114-119 MOV_OqAL
    pub fn mov_oq_al(&mut self, instr: &Instruction) {
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, instr.iq());
        let val8 = self.get_gpr8(0); // AL

        self.write_linear_byte(seg, laddr, val8);
        tracing::trace!("MOV [{:?}:{:#018x}], AL = {:#04x}", seg, instr.iq(), val8);
    }

    /// MOV AX, moffs64
    /// Matching C++ data_xfer64.cc:121-126 MOV_AXOq
    pub fn mov_ax_oq(&mut self, instr: &Instruction) {
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, instr.iq());
        let val16 = self.read_linear_word(seg, laddr);

        self.set_gpr16(0, val16); // AX
        tracing::trace!("MOV AX, [{:?}:{:#018x}] = {:#06x}", seg, instr.iq(), val16);
    }

    /// MOV moffs64, AX
    /// Matching C++ data_xfer64.cc:128-133 MOV_OqAX
    pub fn mov_oq_ax(&mut self, instr: &Instruction) {
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, instr.iq());
        let val16 = self.get_gpr16(0); // AX

        self.write_linear_word(seg, laddr, val16);
        tracing::trace!("MOV [{:?}:{:#018x}], AX = {:#06x}", seg, instr.iq(), val16);
    }

    /// MOV EAX, moffs64
    /// Matching C++ data_xfer64.cc:135-140 MOV_EAXOq
    pub fn mov_eax_oq(&mut self, instr: &Instruction) {
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, instr.iq());
        let val32 = self.read_linear_dword(seg, laddr);

        self.set_gpr32(0, val32); // EAX
        tracing::trace!(
            "MOV EAX, [{:?}:{:#018x}] = {:#010x}",
            seg,
            instr.iq(),
            val32
        );
    }

    /// MOV moffs64, EAX
    /// Matching C++ data_xfer64.cc:142-147 MOV_OqEAX
    pub fn mov_oq_eax(&mut self, instr: &Instruction) {
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, instr.iq());
        let val32 = self.get_gpr32(0); // EAX

        self.write_linear_dword(seg, laddr, val32);
        tracing::trace!(
            "MOV [{:?}:{:#018x}], EAX = {:#010x}",
            seg,
            instr.iq(),
            val32
        );
    }

    /// MOV RAX, moffs64
    /// Matching C++ data_xfer64.cc:149-154 MOV_RAXOq
    pub fn mov_rax_oq(&mut self, instr: &Instruction) {
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, instr.iq());
        let val64 = self.read_linear_qword(seg, laddr);

        self.set_gpr64(0, val64); // RAX
        tracing::trace!(
            "MOV RAX, [{:?}:{:#018x}] = {:#018x}",
            seg,
            instr.iq(),
            val64
        );
    }

    /// MOV moffs64, RAX
    /// Matching C++ data_xfer64.cc:156-161 MOV_OqRAX
    pub fn mov_oq_rax(&mut self, instr: &Instruction) {
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, instr.iq());
        let val64 = self.get_gpr64(0); // RAX

        self.write_linear_qword(seg, laddr, val64);
        tracing::trace!(
            "MOV [{:?}:{:#018x}], RAX = {:#018x}",
            seg,
            instr.iq(),
            val64
        );
    }

    /// MOV r/m64, imm32 (sign-extended to 64-bit) (memory form)
    /// Matching C++ data_xfer64.cc:163-172 MOV_EqIdM
    pub fn mov_eq_id_m(&mut self, instr: &Instruction) {
        let op_64 = instr.id() as i32 as u64; // sign extend imm32 to 64-bit
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);

        self.write_linear_qword(seg, laddr, op_64);
        tracing::trace!("MOV64 mem: [{:?}:{:#x}] = {:#018x}", seg, eaddr, op_64);
    }

    /// MOV r64, imm32 (sign-extended to 64-bit) (register form)
    /// Matching C++ data_xfer64.cc:174-180 MOV_EqIdR
    pub fn mov_eq_id_r(&mut self, instr: &Instruction) {
        let op_64 = instr.id() as i32 as u64; // sign extend imm32 to 64-bit
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, op_64);
        tracing::trace!("MOV64: reg{} = {:#018x}", dst_reg, op_64);
    }

    // =========================================================================
    // MOVZX - Zero extend
    // =========================================================================

    /// MOVZX r64, r/m8 (memory form)
    /// Matching C++ data_xfer64.cc:182-192 MOVZX_GqEbM
    /// Zero extend byte op2 into qword op1
    pub fn movzx_gq_eb_m(&mut self, instr: &Instruction) {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op2_8 = self.read_linear_byte(seg, laddr);
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, op2_8 as u64);
        tracing::trace!(
            "MOVZX64 mem: reg{} = [{:?}:{:#x}] ({:#04x})",
            dst_reg,
            seg,
            eaddr,
            op2_8
        );
    }

    /// MOVZX r64, r8 (register form)
    /// Matching C++ data_xfer64.cc:194-202 MOVZX_GqEbR
    /// Zero extend byte op2 into qword op1
    pub fn movzx_gq_eb_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op2_8 = self.read_8bit_regx(src_reg, extend8bit_l);
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, op2_8 as u64);
        tracing::trace!("MOVZX64: reg{} = reg{} ({:#04x})", dst_reg, src_reg, op2_8);
    }

    /// MOVZX r64, r/m16 (memory form)
    /// Matching C++ data_xfer64.cc:204-214 MOVZX_GqEwM
    /// Zero extend word op2 into qword op1
    pub fn movzx_gq_ew_m(&mut self, instr: &Instruction) {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op2_16 = self.read_linear_word(seg, laddr);
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, op2_16 as u64);
        tracing::trace!(
            "MOVZX64 mem: reg{} = [{:?}:{:#x}] ({:#06x})",
            dst_reg,
            seg,
            eaddr,
            op2_16
        );
    }

    /// MOVZX r64, r16 (register form)
    /// Matching C++ data_xfer64.cc:216-224 MOVZX_GqEwR
    /// Zero extend word op2 into qword op1
    pub fn movzx_gq_ew_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let op2_16 = self.get_gpr16(src_reg);
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, op2_16 as u64);
        tracing::trace!("MOVZX64: reg{} = reg{} ({:#06x})", dst_reg, src_reg, op2_16);
    }

    // =========================================================================
    // MOVSX - Sign extend
    // =========================================================================

    /// MOVSX r64, r/m8 (memory form)
    /// Matching C++ data_xfer64.cc:226-236 MOVSX_GqEbM
    /// Sign extend byte op2 into qword op1
    pub fn movsx_gq_eb_m(&mut self, instr: &Instruction) {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op2_8 = self.read_linear_byte(seg, laddr);
        let dst_reg = instr.dst() as usize;
        let val64 = (op2_8 as i8 as i64) as u64; // sign extend byte to qword

        self.set_gpr64(dst_reg, val64);
        tracing::trace!(
            "MOVSX64 mem: reg{} = [{:?}:{:#x}] ({:#04x} -> {:#018x})",
            dst_reg,
            seg,
            eaddr,
            op2_8,
            val64
        );
    }

    /// MOVSX r64, r8 (register form)
    /// Matching C++ data_xfer64.cc:238-246 MOVSX_GqEbR
    /// Sign extend byte op2 into qword op1
    pub fn movsx_gq_eb_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op2_8 = self.read_8bit_regx(src_reg, extend8bit_l);
        let dst_reg = instr.dst() as usize;
        let val64 = (op2_8 as i8 as i64) as u64; // sign extend byte to qword

        self.set_gpr64(dst_reg, val64);
        tracing::trace!(
            "MOVSX64: reg{} = reg{} ({:#04x} -> {:#018x})",
            dst_reg,
            src_reg,
            op2_8,
            val64
        );
    }

    /// MOVSX r64, r/m16 (memory form)
    /// Matching C++ data_xfer64.cc:248-258 MOVSX_GqEwM
    /// Sign extend word op2 into qword op1
    pub fn movsx_gq_ew_m(&mut self, instr: &Instruction) {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op2_16 = self.read_linear_word(seg, laddr);
        let dst_reg = instr.dst() as usize;
        let val64 = (op2_16 as i16 as i64) as u64; // sign extend word to qword

        self.set_gpr64(dst_reg, val64);
        tracing::trace!(
            "MOVSX64 mem: reg{} = [{:?}:{:#x}] ({:#06x} -> {:#018x})",
            dst_reg,
            seg,
            eaddr,
            op2_16,
            val64
        );
    }

    /// MOVSX r64, r16 (register form)
    /// Matching C++ data_xfer64.cc:260-268 MOVSX_GqEwR
    /// Sign extend word op2 into qword op1
    pub fn movsx_gq_ew_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let op2_16 = self.get_gpr16(src_reg);
        let dst_reg = instr.dst() as usize;
        let val64 = (op2_16 as i16 as i64) as u64; // sign extend word to qword

        self.set_gpr64(dst_reg, val64);
        tracing::trace!(
            "MOVSX64: reg{} = reg{} ({:#06x} -> {:#018x})",
            dst_reg,
            src_reg,
            op2_16,
            val64
        );
    }

    /// MOVSX r64, r/m32 (memory form)
    /// Matching C++ data_xfer64.cc:270-280 MOVSX_GqEdM
    /// Sign extend dword op2 into qword op1
    pub fn movsx_gq_ed_m(&mut self, instr: &Instruction) {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op2_32 = self.read_linear_dword(seg, laddr);
        let dst_reg = instr.dst() as usize;
        let val64 = (op2_32 as i32 as i64) as u64; // sign extend dword to qword

        self.set_gpr64(dst_reg, val64);
        tracing::trace!(
            "MOVSX64 mem: reg{} = [{:?}:{:#x}] ({:#010x} -> {:#018x})",
            dst_reg,
            seg,
            eaddr,
            op2_32,
            val64
        );
    }

    /// MOVSX r64, r32 (register form)
    /// Matching C++ data_xfer64.cc:282-290 MOVSX_GqEdR
    /// Sign extend dword op2 into qword op1
    pub fn movsx_gq_ed_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let op2_32 = self.get_gpr32(src_reg);
        let dst_reg = instr.dst() as usize;
        let val64 = (op2_32 as i32 as i64) as u64; // sign extend dword to qword

        self.set_gpr64(dst_reg, val64);
        tracing::trace!(
            "MOVSX64: reg{} = reg{} ({:#010x} -> {:#018x})",
            dst_reg,
            src_reg,
            op2_32,
            val64
        );
    }

    // =========================================================================
    // XCHG - Exchange
    // =========================================================================

    /// XCHG r/m64, r64 (memory form)
    /// Matching C++ data_xfer64.cc:292-300 XCHG_EqGqM
    /// Note: always locked (read_RMW_linear_qword)
    pub fn xchg_eq_gq_m(&mut self, instr: &Instruction) {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1_64, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr); // always locked
        let src_reg = instr.src() as usize;
        let op2_64 = self.get_gpr64(src_reg);

        self.write_rmw_linear_qword(rmw_laddr, op2_64);
        self.set_gpr64(src_reg, op1_64);
        tracing::trace!(
            "XCHG64 mem: [{:?}:{:#x}]={:#018x} <-> reg{}={:#018x}",
            seg,
            eaddr,
            op2_64,
            src_reg,
            op1_64
        );
    }

    /// XCHG r64, r64 (register form)
    /// Matching C++ data_xfer64.cc:302-311 XCHG_EqGqR
    pub fn xchg_eq_gq_r(&mut self, instr: &Instruction) {
        let dst_reg = instr.dst() as usize;
        let src_reg = instr.src() as usize;
        let op1_64 = self.get_gpr64(dst_reg);
        let op2_64 = self.get_gpr64(src_reg);

        self.set_gpr64(src_reg, op1_64);
        self.set_gpr64(dst_reg, op2_64);
        tracing::trace!(
            "XCHG64: reg{}={:#018x} <-> reg{}={:#018x}",
            dst_reg,
            op2_64,
            src_reg,
            op1_64
        );
    }

    // =========================================================================
    // CMOV - Conditional Move (64-bit)
    // =========================================================================
    // Note: CMOV accesses a memory source operand (read), regardless
    //       of whether condition is true or not.  Thus, exceptions may
    //       occur even if the MOV does not take place.
    // Matching C++ data_xfer64.cc:313-443

    /// Conditional move if overflow (OF=1)
    /// Matching C++ data_xfer64.cc:317-323 CMOVO_GqEqR
    pub fn cmovo_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_of() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not overflow (OF=0)
    /// Matching C++ data_xfer64.cc:325-331 CMOVNO_GqEqR
    pub fn cmovno_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_of() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if below/carry (CF=1)
    /// Matching C++ data_xfer64.cc:333-339 CMOVB_GqEqR
    pub fn cmovb_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_cf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not below/no carry (CF=0)
    /// Matching C++ data_xfer64.cc:341-347 CMOVNB_GqEqR
    pub fn cmovnb_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_cf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if zero/equal (ZF=1)
    /// Matching C++ data_xfer64.cc:349-355 CMOVZ_GqEqR
    pub fn cmovz_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_zf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not zero/not equal (ZF=0)
    /// Matching C++ data_xfer64.cc:357-363 CMOVNZ_GqEqR
    pub fn cmovnz_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_zf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if below or equal (CF=1 or ZF=1)
    /// Matching C++ data_xfer64.cc:365-371 CMOVBE_GqEqR
    pub fn cmovbe_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_cf() || self.get_zf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not below or equal/above (CF=0 and ZF=0)
    /// Matching C++ data_xfer64.cc:373-379 CMOVNBE_GqEqR
    pub fn cmovnbe_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_cf() && !self.get_zf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if sign (SF=1)
    /// Matching C++ data_xfer64.cc:381-387 CMOVS_GqEqR
    pub fn cmovs_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_sf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not sign (SF=0)
    /// Matching C++ data_xfer64.cc:389-395 CMOVNS_GqEqR
    pub fn cmovns_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_sf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if parity/parity even (PF=1)
    /// Matching C++ data_xfer64.cc:397-403 CMOVP_GqEqR
    pub fn cmovp_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_pf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if no parity/parity odd (PF=0)
    /// Matching C++ data_xfer64.cc:405-411 CMOVNP_GqEqR
    pub fn cmovnp_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_pf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if less (SF != OF)
    /// Matching C++ data_xfer64.cc:413-419 CMOVL_GqEqR
    pub fn cmovl_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_sf() != self.get_of() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not less/greater or equal (SF == OF)
    /// Matching C++ data_xfer64.cc:421-427 CMOVNL_GqEqR
    pub fn cmovnl_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_sf() == self.get_of() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if less or equal (ZF=1 or SF!=OF)
    /// Matching C++ data_xfer64.cc:429-435 CMOVLE_GqEqR
    pub fn cmovle_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_zf() || (self.get_sf() != self.get_of()) {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not less or equal/greater (ZF=0 and SF==OF)
    /// Matching C++ data_xfer64.cc:437-443 CMOVNLE_GqEqR
    pub fn cmovnle_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_zf() && (self.get_sf() == self.get_of()) {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    // =========================================================================
    // Unified dispatchers (mod_c0 routing for register vs memory)
    // =========================================================================

    pub fn mov_eq_gq(&mut self, instr: &Instruction) {
        if instr.mod_c0() {
            // Register form: MOV r64, r64 - use the same register form
            let src = instr.src() as usize;
            let dst = instr.dst() as usize;
            self.set_gpr64(dst, self.get_gpr64(src));
        } else {
            self.mov_eq_gq_m(instr);
        }
    }

    pub fn mov_gq_eq(&mut self, instr: &Instruction) {
        if instr.mod_c0() {
            self.mov_gq_eq_r(instr);
        } else {
            self.mov_gq_eq_m(instr);
        }
    }

    pub fn mov_eq_id(&mut self, instr: &Instruction) {
        if instr.mod_c0() {
            self.mov_eq_id_r(instr);
        } else {
            self.mov_eq_id_m(instr);
        }
    }

    pub fn xchg_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xchg_eq_gq_r(instr);
            Ok(())
        } else {
            self.xchg_eq_gq_m(instr);
            Ok(())
        }
    }

    pub fn movsx_gq_eb(&mut self, instr: &Instruction) {
        if instr.mod_c0() {
            self.movsx_gq_eb_r(instr);
        } else {
            self.movsx_gq_eb_m(instr);
        }
    }

    pub fn movsx_gq_ew(&mut self, instr: &Instruction) {
        if instr.mod_c0() {
            self.movsx_gq_ew_r(instr);
        } else {
            self.movsx_gq_ew_m(instr);
        }
    }

    pub fn movsxd_gq_ed(&mut self, instr: &Instruction) {
        if instr.mod_c0() {
            self.movsx_gq_ed_r(instr);
        } else {
            self.movsx_gq_ed_m(instr);
        }
    }

    pub fn movzx_gq_eb(&mut self, instr: &Instruction) {
        if instr.mod_c0() {
            self.movzx_gq_eb_r(instr);
        } else {
            self.movzx_gq_eb_m(instr);
        }
    }

    pub fn movzx_gq_ew(&mut self, instr: &Instruction) {
        if instr.mod_c0() {
            self.movzx_gq_ew_r(instr);
        } else {
            self.movzx_gq_ew_m(instr);
        }
    }

    // =========================================================================
    // Helper functions for 64-bit memory operations
    // =========================================================================

    /// Resolve effective address (64-bit addressing mode)
    /// Matching BX_CPU_RESOLVE_ADDR_64
    /// Made pub(crate) so it can be accessed from ctrl_xfer64.rs
    pub(crate) fn resolve_addr64(&self, instr: &Instruction) -> u64 {
        // Calculate: base + (index << scale) + displacement
        let base_reg = instr.sib_base() as usize;
        let mut eaddr = if base_reg < 16 {
            self.get_gpr64(base_reg)
        } else {
            0
        };

        eaddr = eaddr.wrapping_add(instr.displ32s() as u64);

        let index_reg = instr.sib_index();
        if index_reg != 4 {
            // 4 means no index
            let index_val = if index_reg < 16 {
                self.get_gpr64(index_reg as usize)
            } else {
                0
            };
            let scale = instr.sib_scale();
            eaddr = eaddr.wrapping_add(index_val << scale);
        }

        eaddr
    }

    // read_8bit_regx is defined in logical8.rs to avoid duplicate definitions

    /// Read byte from linear address (matches read_linear_byte)
    fn read_linear_byte(&self, _seg: BxSegregs, laddr: u64) -> u8 {
        self.mem_read_byte(laddr)
    }

    /// Read dword from linear address (matches read_linear_dword)
    fn read_linear_dword(&self, _seg: BxSegregs, laddr: u64) -> u32 {
        self.mem_read_dword(laddr)
    }

    /// Write byte to linear address (matches write_linear_byte)
    fn write_linear_byte(&mut self, _seg: BxSegregs, laddr: u64, val: u8) {
        self.mem_write_byte(laddr, val);
    }

    /// Write dword to linear address (matches write_linear_dword)
    fn write_linear_dword(&mut self, _seg: BxSegregs, laddr: u64, val: u32) {
        self.mem_write_dword(laddr, val);
    }

    /// Write qword to linear address (matches write_linear_qword)
    fn write_linear_qword(&mut self, _seg: BxSegregs, laddr: u64, val: u64) {
        self.mem_write_qword(laddr, val);
    }

    /// Read-Modify-Write: Read qword, return it and linear address for write back
    /// Matching read_RMW_linear_qword
    pub(super) fn read_rmw_linear_qword(&mut self, _seg: BxSegregs, laddr: u64) -> (u64, u64) {
        let val = self.mem_read_qword(laddr);
        (val, laddr)
    }

    /// Write qword to linear address (for RMW operations)
    /// Matching write_RMW_linear_qword
    pub(super) fn write_rmw_linear_qword(&mut self, laddr: u64, val: u64) {
        self.mem_write_qword(laddr, val);
    }
}
