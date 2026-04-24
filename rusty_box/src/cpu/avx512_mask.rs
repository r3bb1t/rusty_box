

//! AVX-512 Opmask (k-register) instruction handlers
//!
//! Implements KMOV, KAND, KOR, KXOR, KXNOR, KNOT, KADD, KUNPCK,
//! KORTEST, KTEST, KSHIFT for all widths (B/W/D/Q).
//!
//! Mirrors Bochs `cpu/avx/avx512_mask{8,16,32,64}.cc`.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
};

/// Helper: read opmask register value (full 64-bit)
#[inline]
fn read_opmask<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(cpu: &BxCpuC<'_, I, T>, idx: u8) -> u64 {
    cpu.opmask_rrx(idx as usize)
}

/// Helper: write opmask register with width mask
#[inline]
fn write_opmask_masked<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(cpu: &mut BxCpuC<'_, I, T>, idx: u8, val: u64, mask: u64) {
    cpu.bx_write_opmask(idx as usize, val & mask);
}

// Width masks
const MASK_B: u64 = 0xFF;
const MASK_W: u64 = 0xFFFF;
const MASK_D: u64 = 0xFFFF_FFFF;
const MASK_Q: u64 = u64::MAX;

// ========================================================================
// KMOV — Move opmask register
// ========================================================================

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // --- KMOV register-to-register ---

    /// KMOVB KGb, KEb (VEX.L0.66.0F.W0 90 /r) — register form
    pub fn kmovb_kgb_keb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src()) & MASK_B;
        write_opmask_masked(self, instr.dst(), src, MASK_B);
        Ok(())
    }

    /// KMOVW KGw, KEw (VEX.L0.0F.W0 90 /r) — register form
    pub fn kmovw_kgw_kew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src()) & MASK_W;
        write_opmask_masked(self, instr.dst(), src, MASK_W);
        Ok(())
    }

    /// KMOVD KGd, KEd (VEX.L0.66.0F.W1 90 /r) — register form
    pub fn kmovd_kgd_ked_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src()) & MASK_D;
        write_opmask_masked(self, instr.dst(), src, MASK_D);
        Ok(())
    }

    /// KMOVQ KGq, KEq (VEX.L0.0F.W1 90 /r) — register form
    pub fn kmovq_kgq_keq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src());
        self.bx_write_opmask(instr.dst() as usize, src);
        Ok(())
    }

    // --- KMOV memory load ---

    /// KMOVB KGb, Mb (VEX.L0.66.0F.W0 90 /r) — memory form
    pub fn kmovb_kgb_keb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let laddr = self.resolve_addr(instr);
        let val = self.v_read_byte(BxSegregs::from(instr.seg()), laddr)? as u64;
        write_opmask_masked(self, instr.dst(), val, MASK_B);
        Ok(())
    }

    /// KMOVW KGw, Mw (VEX.L0.0F.W0 90 /r) — memory form
    pub fn kmovw_kgw_kew_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let laddr = self.resolve_addr(instr);
        let val = self.v_read_word(BxSegregs::from(instr.seg()), laddr)? as u64;
        write_opmask_masked(self, instr.dst(), val, MASK_W);
        Ok(())
    }

    /// KMOVD KGd, Md (VEX.L0.66.0F.W1 90 /r) — memory form
    pub fn kmovd_kgd_ked_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let laddr = self.resolve_addr(instr);
        let val = self.v_read_dword(BxSegregs::from(instr.seg()), laddr)? as u64;
        write_opmask_masked(self, instr.dst(), val, MASK_D);
        Ok(())
    }

    /// KMOVQ KGq, Mq (VEX.L0.0F.W1 90 /r) — memory form
    pub fn kmovq_kgq_keq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let laddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let val = if self.long64_mode() {
            self.read_virtual_qword_64(seg, laddr)?
        } else {
            self.v_read_dword(seg, laddr)? as u64
        };
        self.bx_write_opmask(instr.dst() as usize, val);
        Ok(())
    }

    // --- KMOV memory store ---

    /// KMOVB Mb, KGb (VEX.L0.66.0F.W0 91 /r) — memory store
    pub fn kmovb_keb_kgb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = read_opmask(self, instr.src()) as u8;
        let laddr = self.resolve_addr(instr);
        self.v_write_byte(BxSegregs::from(instr.seg()), laddr, val)?;
        Ok(())
    }

    /// KMOVW Mw, KGw (VEX.L0.0F.W0 91 /r) — memory store
    pub fn kmovw_kew_kgw_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = read_opmask(self, instr.src()) as u16;
        let laddr = self.resolve_addr(instr);
        self.v_write_word(BxSegregs::from(instr.seg()), laddr, val)?;
        Ok(())
    }

    /// KMOVD Md, KGd (VEX.L0.66.0F.W1 91 /r) — memory store
    pub fn kmovd_ked_kgd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = read_opmask(self, instr.src()) as u32;
        let laddr = self.resolve_addr(instr);
        self.v_write_dword(BxSegregs::from(instr.seg()), laddr, val)?;
        Ok(())
    }

    /// KMOVQ Mq, KGq (VEX.L0.0F.W1 91 /r) — memory store
    pub fn kmovq_keq_kgq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = read_opmask(self, instr.src());
        let laddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        if self.long64_mode() {
            self.write_virtual_qword_64(seg, laddr, val)?;
        } else {
            self.v_write_dword(seg, laddr, val as u32)?;
        }
        Ok(())
    }

    // --- KMOV GPR↔opmask ---

    /// KMOVB KGb, Rd (VEX.L0.66.0F.W0 92 /r) — GPR to opmask
    pub fn kmovb_kgb_eb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = self.get_gpr32(instr.src() as usize) as u64;
        write_opmask_masked(self, instr.dst(), val, MASK_B);
        Ok(())
    }

    /// KMOVW KGw, Rd (VEX.L0.0F.W0 92 /r) — GPR to opmask
    pub fn kmovw_kgw_ew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = self.get_gpr32(instr.src() as usize) as u64;
        write_opmask_masked(self, instr.dst(), val, MASK_W);
        Ok(())
    }

    /// KMOVD KGd, Rd (VEX.L0.F2.0F.W0 92 /r) — GPR to opmask
    pub fn kmovd_kgd_ed_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = self.get_gpr32(instr.src() as usize) as u64;
        write_opmask_masked(self, instr.dst(), val, MASK_D);
        Ok(())
    }

    /// KMOVQ KGq, Rq (VEX.L0.F2.0F.W1 92 /r) — GPR to opmask (64-bit)
    pub fn kmovq_kgq_eq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = self.get_gpr64(instr.src() as usize);
        self.bx_write_opmask(instr.dst() as usize, val);
        Ok(())
    }

    /// KMOVB Gd, KEb (VEX.L0.66.0F.W0 93 /r) — opmask to GPR
    pub fn kmovb_gd_keb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = read_opmask(self, instr.src()) & MASK_B;
        self.set_gpr64(instr.dst() as usize, val);
        Ok(())
    }

    /// KMOVW Gd, KEw (VEX.L0.0F.W0 93 /r) — opmask to GPR
    pub fn kmovw_gd_kew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = read_opmask(self, instr.src()) & MASK_W;
        self.set_gpr64(instr.dst() as usize, val);
        Ok(())
    }

    /// KMOVD Gd, KEd (VEX.L0.F2.0F.W0 93 /r) — opmask to GPR
    pub fn kmovd_gd_ked_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = read_opmask(self, instr.src()) & MASK_D;
        self.set_gpr64(instr.dst() as usize, val);
        Ok(())
    }

    /// KMOVQ Gq, KEq (VEX.L0.F2.0F.W1 93 /r) — opmask to GPR
    pub fn kmovq_gq_keq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = read_opmask(self, instr.src());
        self.set_gpr64(instr.dst() as usize, val);
        Ok(())
    }

    // ========================================================================
    // KAND/KANDN/KOR/KXOR/KXNOR — Opmask logical operations
    // ========================================================================

    /// KANDB KGb, KHb, KEb
    pub fn kandb_kgb_khb_keb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), s1 & s2, MASK_B);
        Ok(())
    }
    /// KANDW KGw, KHw, KEw
    pub fn kandw_kgw_khw_kew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), s1 & s2, MASK_W);
        Ok(())
    }
    /// KANDD KGd, KHd, KEd
    pub fn kandd_kgd_khd_ked_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), s1 & s2, MASK_D);
        Ok(())
    }
    /// KANDQ KGq, KHq, KEq
    pub fn kandq_kgq_khq_keq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        self.bx_write_opmask(instr.dst() as usize, s1 & s2);
        Ok(())
    }

    /// KANDNB KGb, KHb, KEb
    pub fn kandnb_kgb_khb_keb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), (!s1) & s2, MASK_B);
        Ok(())
    }
    /// KANDNW KGw, KHw, KEw
    pub fn kandnw_kgw_khw_kew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), (!s1) & s2, MASK_W);
        Ok(())
    }
    /// KANDND KGd, KHd, KEd
    pub fn kandnd_kgd_khd_ked_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), (!s1) & s2, MASK_D);
        Ok(())
    }
    /// KANDNQ KGq, KHq, KEq
    pub fn kandnq_kgq_khq_keq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        self.bx_write_opmask(instr.dst() as usize, (!s1) & s2);
        Ok(())
    }

    /// KORB KGb, KHb, KEb
    pub fn korb_kgb_khb_keb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), s1 | s2, MASK_B);
        Ok(())
    }
    /// KORW KGw, KHw, KEw
    pub fn korw_kgw_khw_kew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), s1 | s2, MASK_W);
        Ok(())
    }
    /// KORD KGd, KHd, KEd
    pub fn kord_kgd_khd_ked_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), s1 | s2, MASK_D);
        Ok(())
    }
    /// KORQ KGq, KHq, KEq
    pub fn korq_kgq_khq_keq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        self.bx_write_opmask(instr.dst() as usize, s1 | s2);
        Ok(())
    }

    /// KXORB KGb, KHb, KEb
    pub fn kxorb_kgb_khb_keb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), s1 ^ s2, MASK_B);
        Ok(())
    }
    /// KXORW KGw, KHw, KEw
    pub fn kxorw_kgw_khw_kew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), s1 ^ s2, MASK_W);
        Ok(())
    }
    /// KXORD KGd, KHd, KEd
    pub fn kxord_kgd_khd_ked_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), s1 ^ s2, MASK_D);
        Ok(())
    }
    /// KXORQ KGq, KHq, KEq
    pub fn kxorq_kgq_khq_keq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        self.bx_write_opmask(instr.dst() as usize, s1 ^ s2);
        Ok(())
    }

    /// KXNORB KGb, KHb, KEb
    pub fn kxnorb_kgb_khb_keb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), !(s1 ^ s2), MASK_B);
        Ok(())
    }
    /// KXNORW KGw, KHw, KEw
    pub fn kxnorw_kgw_khw_kew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), !(s1 ^ s2), MASK_W);
        Ok(())
    }
    /// KXNORD KGd, KHd, KEd
    pub fn kxnord_kgd_khd_ked_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        write_opmask_masked(self, instr.dst(), !(s1 ^ s2), MASK_D);
        Ok(())
    }
    /// KXNORQ KGq, KHq, KEq
    pub fn kxnorq_kgq_khq_keq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        self.bx_write_opmask(instr.dst() as usize, !(s1 ^ s2));
        Ok(())
    }

    // ========================================================================
    // KNOT — Opmask NOT
    // ========================================================================

    /// KNOTB KGb, KEb
    pub fn knotb_kgb_keb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src());
        write_opmask_masked(self, instr.dst(), !src, MASK_B);
        Ok(())
    }
    /// KNOTW KGw, KEw
    pub fn knotw_kgw_kew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src());
        write_opmask_masked(self, instr.dst(), !src, MASK_W);
        Ok(())
    }
    /// KNOTD KGd, KEd
    pub fn knotd_kgd_ked_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src());
        write_opmask_masked(self, instr.dst(), !src, MASK_D);
        Ok(())
    }
    /// KNOTQ KGq, KEq
    pub fn knotq_kgq_keq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src());
        self.bx_write_opmask(instr.dst() as usize, !src);
        Ok(())
    }

    // ========================================================================
    // KADD — Opmask addition (modular)
    // ========================================================================

    /// KADDB KGb, KHb, KEb
    pub fn kaddb_kgb_khb_keb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1()) as u8;
        let s2 = read_opmask(self, instr.src2()) as u8;
        write_opmask_masked(self, instr.dst(), s1.wrapping_add(s2) as u64, MASK_B);
        Ok(())
    }
    /// KADDW KGw, KHw, KEw
    pub fn kaddw_kgw_khw_kew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1()) as u16;
        let s2 = read_opmask(self, instr.src2()) as u16;
        write_opmask_masked(self, instr.dst(), s1.wrapping_add(s2) as u64, MASK_W);
        Ok(())
    }
    /// KADDD KGd, KHd, KEd
    pub fn kaddd_kgd_khd_ked_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1()) as u32;
        let s2 = read_opmask(self, instr.src2()) as u32;
        write_opmask_masked(self, instr.dst(), s1.wrapping_add(s2) as u64, MASK_D);
        Ok(())
    }
    /// KADDQ KGq, KHq, KEq
    pub fn kaddq_kgq_khq_keq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1());
        let s2 = read_opmask(self, instr.src2());
        self.bx_write_opmask(instr.dst() as usize, s1.wrapping_add(s2));
        Ok(())
    }

    // ========================================================================
    // KORTEST — OR-test opmask (sets EFLAGS)
    // ========================================================================

    /// KORTESTB KGb, KEb — Bochs KORTESTB_KGbKEbR
    pub fn kortestb_kgb_keb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let tmp = (read_opmask(self, instr.dst()) | read_opmask(self, instr.src())) & MASK_B;
        self.oszapc.set_oszapc_logic_32(1); // clearEFlagsOSZAPC
        if tmp == 0 { self.oszapc.set_zf(true); }
        else if tmp == MASK_B { self.oszapc.set_cf(true); }
        Ok(())
    }
    /// KORTESTW KGw, KEw — Bochs KORTESTW_KGwKEwR
    pub fn kortestw_kgw_kew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let tmp = (read_opmask(self, instr.dst()) | read_opmask(self, instr.src())) & MASK_W;
        self.oszapc.set_oszapc_logic_32(1);
        if tmp == 0 { self.oszapc.set_zf(true); }
        else if tmp == MASK_W { self.oszapc.set_cf(true); }
        Ok(())
    }
    /// KORTESTD KGd, KEd — Bochs KORTESTD_KGdKEdR
    pub fn kortestd_kgd_ked_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let tmp = (read_opmask(self, instr.dst()) | read_opmask(self, instr.src())) & MASK_D;
        self.oszapc.set_oszapc_logic_32(1);
        if tmp == 0 { self.oszapc.set_zf(true); }
        else if tmp == MASK_D { self.oszapc.set_cf(true); }
        Ok(())
    }
    /// KORTESTQ KGq, KEq — Bochs KORTESTQ_KGqKEqR
    pub fn kortestq_kgq_keq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let tmp = read_opmask(self, instr.dst()) | read_opmask(self, instr.src());
        self.oszapc.set_oszapc_logic_32(1);
        if tmp == 0 { self.oszapc.set_zf(true); }
        else if tmp == MASK_Q { self.oszapc.set_cf(true); }
        Ok(())
    }

    // ========================================================================
    // KTEST — Test opmask (sets EFLAGS based on AND)
    // ========================================================================

    /// KTESTB KGb, KEb — Bochs KTESTB_KGbKEbR
    pub fn ktestb_kgb_keb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = read_opmask(self, instr.dst()) & MASK_B;
        let op2 = read_opmask(self, instr.src()) & MASK_B;
        self.oszapc.set_oszapc_logic_32(1); // clearEFlagsOSZAPC
        if (op1 & op2) == 0 { self.oszapc.set_zf(true); }
        if ((!op1) & op2 & MASK_B) == 0 { self.oszapc.set_cf(true); }
        Ok(())
    }
    /// KTESTW KGw, KEw — Bochs KTESTW_KGwKEwR
    pub fn ktestw_kgw_kew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = read_opmask(self, instr.dst()) & MASK_W;
        let op2 = read_opmask(self, instr.src()) & MASK_W;
        self.oszapc.set_oszapc_logic_32(1);
        if (op1 & op2) == 0 { self.oszapc.set_zf(true); }
        if ((!op1) & op2 & MASK_W) == 0 { self.oszapc.set_cf(true); }
        Ok(())
    }
    /// KTESTD KGd, KEd — Bochs KTESTD_KGdKEdR
    pub fn ktestd_kgd_ked_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = read_opmask(self, instr.dst()) & MASK_D;
        let op2 = read_opmask(self, instr.src()) & MASK_D;
        self.oszapc.set_oszapc_logic_32(1);
        if (op1 & op2) == 0 { self.oszapc.set_zf(true); }
        if ((!op1) & op2 & MASK_D) == 0 { self.oszapc.set_cf(true); }
        Ok(())
    }
    /// KTESTQ KGq, KEq — Bochs KTESTQ_KGqKEqR
    pub fn ktestq_kgq_keq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = read_opmask(self, instr.dst());
        let op2 = read_opmask(self, instr.src());
        self.oszapc.set_oszapc_logic_32(1);
        if (op1 & op2) == 0 { self.oszapc.set_zf(true); }
        if ((!op1) & op2) == 0 { self.oszapc.set_cf(true); }
        Ok(())
    }

    // ========================================================================
    // KSHIFT — Shift opmask left/right
    // ========================================================================

    /// KSHIFTLB KGb, KEb, Ib
    pub fn kshiftlb_kgb_keb_ib_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src()) & MASK_B;
        let count = instr.ib() as u32;
        let result = if count >= 8 { 0 } else { (src as u8).wrapping_shl(count) as u64 };
        write_opmask_masked(self, instr.dst(), result, MASK_B);
        Ok(())
    }
    /// KSHIFTLW KGw, KEw, Ib
    pub fn kshiftlw_kgw_kew_ib_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src()) & MASK_W;
        let count = instr.ib() as u32;
        let result = if count >= 16 { 0 } else { (src as u16).wrapping_shl(count) as u64 };
        write_opmask_masked(self, instr.dst(), result, MASK_W);
        Ok(())
    }
    /// KSHIFTLD KGd, KEd, Ib
    pub fn kshiftld_kgd_ked_ib_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src()) & MASK_D;
        let count = instr.ib() as u32;
        let result = if count >= 32 { 0 } else { (src as u32).wrapping_shl(count) as u64 };
        write_opmask_masked(self, instr.dst(), result, MASK_D);
        Ok(())
    }
    /// KSHIFTLQ KGq, KEq, Ib
    pub fn kshiftlq_kgq_keq_ib_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src());
        let count = instr.ib() as u32;
        let result = if count >= 64 { 0 } else { src.wrapping_shl(count) };
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// KSHIFTRB KGb, KEb, Ib
    pub fn kshiftrb_kgb_keb_ib_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src()) & MASK_B;
        let count = instr.ib() as u32;
        let result = if count >= 8 { 0 } else { (src as u8).wrapping_shr(count) as u64 };
        write_opmask_masked(self, instr.dst(), result, MASK_B);
        Ok(())
    }
    /// KSHIFTRW KGw, KEw, Ib
    pub fn kshiftrw_kgw_kew_ib_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src()) & MASK_W;
        let count = instr.ib() as u32;
        let result = if count >= 16 { 0 } else { (src as u16).wrapping_shr(count) as u64 };
        write_opmask_masked(self, instr.dst(), result, MASK_W);
        Ok(())
    }
    /// KSHIFTRD KGd, KEd, Ib
    pub fn kshiftrd_kgd_ked_ib_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src()) & MASK_D;
        let count = instr.ib() as u32;
        let result = if count >= 32 { 0 } else { (src as u32).wrapping_shr(count) as u64 };
        write_opmask_masked(self, instr.dst(), result, MASK_D);
        Ok(())
    }
    /// KSHIFTRQ KGq, KEq, Ib
    pub fn kshiftrq_kgq_keq_ib_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_opmask(self, instr.src());
        let count = instr.ib() as u32;
        let result = if count >= 64 { 0 } else { src.wrapping_shr(count) };
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // KUNPCK — Unpack opmask halves
    // ========================================================================

    /// KUNPCKBW KGw, KHb, KEb — unpack two 8-bit masks into one 16-bit mask
    pub fn kunpckbw_kgw_khb_keb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1()) & MASK_B;
        let s2 = read_opmask(self, instr.src2()) & MASK_B;
        let result = (s1 << 8) | s2;
        write_opmask_masked(self, instr.dst(), result, MASK_W);
        Ok(())
    }
    /// KUNPCKWD KGd, KHw, KEw — unpack two 16-bit masks into one 32-bit mask
    pub fn kunpckwd_kgd_khw_kew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1()) & MASK_W;
        let s2 = read_opmask(self, instr.src2()) & MASK_W;
        let result = (s1 << 16) | s2;
        write_opmask_masked(self, instr.dst(), result, MASK_D);
        Ok(())
    }
    /// KUNPCKDQ KGq, KHd, KEd — unpack two 32-bit masks into one 64-bit mask
    pub fn kunpckdq_kgq_khd_ked_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let s1 = read_opmask(self, instr.src1()) & MASK_D;
        let s2 = read_opmask(self, instr.src2()) & MASK_D;
        let result = (s1 << 32) | s2;
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }
}
