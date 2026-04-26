//! AVX-512F gather instruction implementations.
//!
//! Mirrors Bochs `cpu/avx/gather.cc`. VPGATHER reads dword/qword elements
//! from memory at per-element addresses computed from a VSIB encoding:
//!
//!   `addr_n = base_gpr + (sext(index_vec[n]) << scale) + sext(disp32)`
//!
//! For each set bit `n` of the opmask `k1`, a single element is loaded
//! and the corresponding mask bit is cleared (architecturally observable
//! per element so the instruction is restartable on a per-element fault).
//! Elements with their mask bit clear are NOT modified (merge-masking).
//! After the loop, any element index above the vector-length bound is
//! zeroed (Bochs `BX_CLEAR_AVX_REGZ`).
//!
//! Bochs `BxResolveGatherD/Q` honour the address-size attribute (`as64L`):
//! 64-bit addressing reads the base GPR as 64 bits and uses 64-bit
//! arithmetic; 32-bit addressing truncates the effective address.
//!
//! ## Decoder pieces this depends on
//!
//! Real gather decoding requires reading the SIB index field as a 5-bit
//! VECTOR register index (0..31). Two pieces flow through the decoder:
//!   1. `EVEX.V'` — captured at `decode64.rs` and stashed onto the
//!      `Instruction` via `set_evex_v_prime` so the gather handler can
//!      combine `instr.sib_index() | (instr.get_evex_v_prime() << 4)`
//!      to recover the full 5-bit vmm index.
//!   2. `EVEX.R'` — applied to `nnn` during ModRM parsing so
//!      `instr.dst()` covers vmm0..31 directly.
//!
//! The default-segment computation in `decode64.rs` keys off the SIB
//! BASE register (a regular GPR), not the index, so it remains correct
//! for VSIB without further changes.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
};
use rusty_box_decoder::BX_NIL_REGISTER;

/// Number of 32-bit elements per vector length: VL=128→4, VL=256→8, VL=512→16
#[inline]
fn dword_elements(vl: u8) -> usize {
    match vl {
        0 => 4,
        1 => 8,
        _ => 16,
    }
}

/// Number of 64-bit elements per vector length: VL=128→2, VL=256→4, VL=512→8
#[inline]
fn qword_elements(vl: u8) -> usize {
    match vl {
        0 => 2,
        1 => 4,
        _ => 8,
    }
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // VSIB resolver helpers — mirror Bochs `BxResolveGatherD` /
    // `BxResolveGatherQ` (cpu/avx/gather.cc:31-49).
    // ========================================================================

    /// Read the VSIB base GPR per `as64L` addressing mode. Returns 0 when
    /// the base is `BX_NIL_REGISTER` (the `[disp32]` form encoded as
    /// mod=0/base=5).
    #[inline]
    fn vsib_base_gpr(&self, sib_base: u8, as64: bool) -> u64 {
        if sib_base as usize == BX_NIL_REGISTER {
            0
        } else if as64 {
            self.get_gpr64(sib_base as usize)
        } else {
            u64::from(self.get_gpr32(sib_base as usize))
        }
    }

    /// Compute the effective address for VSIB element `n` with a 32-bit
    /// (dword) index lane. Bochs `BxResolveGatherD`.
    #[inline]
    fn resolve_gather_d(&self, instr: &Instruction, sib_idx: u8, n: usize) -> u64 {
        let index = i64::from(self.vmm[sib_idx as usize].zmm32s(n));
        let scale = instr.sib_scale();
        let disp = i64::from(instr.displ32s());
        let base = self.vsib_base_gpr(instr.sib_base(), instr.as64_l() != 0);
        let off = (index << scale).wrapping_add(disp);
        let addr = base.wrapping_add(off as u64);
        if instr.as64_l() != 0 {
            addr
        } else {
            // 32-bit addressing: effective address truncated to 32 bits.
            addr as u32 as u64
        }
    }

    /// Compute the effective address for VSIB element `n` with a 64-bit
    /// (qword) index lane. Bochs `BxResolveGatherQ`.
    #[inline]
    fn resolve_gather_q(&self, instr: &Instruction, sib_idx: u8, n: usize) -> u64 {
        let index = self.vmm[sib_idx as usize].zmm64s(n);
        let scale = instr.sib_scale();
        let disp = i64::from(instr.displ32s());
        let base = self.vsib_base_gpr(instr.sib_base(), instr.as64_l() != 0);
        let off = (index << scale).wrapping_add(disp);
        let addr = base.wrapping_add(off as u64);
        if instr.as64_l() != 0 {
            addr
        } else {
            addr as u32 as u64
        }
    }

    /// Recover the V'-extended VSIB index register (5 bits, vmm0..31).
    #[inline]
    fn vsib_index_reg(instr: &Instruction) -> u8 {
        instr.sib_index() | (instr.get_evex_v_prime() << 4)
    }

    // ========================================================================
    // VPGATHERDD — Gather packed dwords using dword indices
    // EVEX.66.0F38.W0 90 /r  (vm32{x,y,z})
    // ========================================================================

    /// VPGATHERDD Vdq{k1}, vm32x — Bochs `VGATHERDPS_MASK_VpsVSib`
    /// (cpu/avx/gather.cc:258-297). Loads `nelements = dword_elements(VL)`
    /// dwords from memory at addresses computed from `[base + index*scale + disp]`
    /// where `index` is the corresponding 32-bit lane of the VSIB index
    /// register. Masked-off elements are unchanged; after the loop the
    /// opmask is fully cleared and bytes above VL are zeroed.
    pub fn evex_vpgatherdd(&mut self, instr: &Instruction) -> super::Result<()> {
        let dst = instr.dst();
        let sib_idx = Self::vsib_index_reg(instr);
        // Bochs cpu/avx/gather.cc:260-263: #UD if dst register collides with
        // the VSIB index register (would corrupt indices mid-gather).
        if sib_idx == dst {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        let k = instr.opmask();
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let seg = BxSegregs::from(instr.seg());

        let mut opmask = self.opmask[k as usize].rrx();
        for n in 0..nelements {
            let mask_bit = 1u64 << n;
            if opmask & mask_bit != 0 {
                let addr = self.resolve_gather_d(instr, sib_idx, n);
                let data = self.v_read_dword(seg, addr)?;
                self.vmm[dst as usize].set_zmm32u(n, data);
                opmask &= !mask_bit;
                self.bx_write_opmask(k as usize, opmask);
            }
        }
        // Final opmask clear + zero bytes above VL (Bochs BX_CLEAR_AVX_REGZ).
        self.bx_write_opmask(k as usize, 0);
        for i in nelements..16 {
            self.vmm[dst as usize].set_zmm32u(i, 0);
        }
        Ok(())
    }

    // ========================================================================
    // VPGATHERDQ — Gather packed qwords using dword indices
    // EVEX.66.0F38.W1 90 /r  (vm32{x,y})
    // ========================================================================

    /// VPGATHERDQ Vdq{k1}, vm32x — Bochs `VGATHERDPD_MASK_VpdVSib`
    /// (cpu/avx/gather.cc:344-383). Half as many indices as VPGATHERDD
    /// because each output qword consumes one dword index.
    pub fn evex_vpgatherdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let dst = instr.dst();
        let sib_idx = Self::vsib_index_reg(instr);
        if sib_idx == dst {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        let k = instr.opmask();
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let seg = BxSegregs::from(instr.seg());

        let mut opmask = self.opmask[k as usize].rrx();
        for n in 0..nelements {
            let mask_bit = 1u64 << n;
            if opmask & mask_bit != 0 {
                let addr = self.resolve_gather_d(instr, sib_idx, n);
                let data = self.v_read_qword(seg, addr)?;
                self.vmm[dst as usize].set_zmm64u(n, data);
                opmask &= !mask_bit;
                self.bx_write_opmask(k as usize, opmask);
            }
        }
        self.bx_write_opmask(k as usize, 0);
        for i in nelements..8 {
            self.vmm[dst as usize].set_zmm64u(i, 0);
        }
        Ok(())
    }

    // ========================================================================
    // VPGATHERQD — Gather packed dwords using qword indices
    // EVEX.66.0F38.W0 91 /r  (vm64{x,y,z})
    // ========================================================================

    /// VPGATHERQD Vdq{k1}, vm64z — Bochs `VGATHERQPS_MASK_VpsVSib`
    /// (cpu/avx/gather.cc:299-342). 64-bit indices, 32-bit data — gathered
    /// dwords pack into the LOWER half of the destination; the upper half
    /// is zeroed.
    pub fn evex_vpgatherqd(&mut self, instr: &Instruction) -> super::Result<()> {
        let dst = instr.dst();
        let sib_idx = Self::vsib_index_reg(instr);
        if sib_idx == dst {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        let k = instr.opmask();
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let seg = BxSegregs::from(instr.seg());

        let mut opmask = self.opmask[k as usize].rrx();
        for n in 0..nelements {
            let mask_bit = 1u64 << n;
            if opmask & mask_bit != 0 {
                let addr = self.resolve_gather_q(instr, sib_idx, n);
                let data = self.v_read_dword(seg, addr)?;
                self.vmm[dst as usize].set_zmm32u(n, data);
                opmask &= !mask_bit;
                self.bx_write_opmask(k as usize, opmask);
            }
        }
        self.bx_write_opmask(k as usize, 0);
        // Zero all dwords above the gathered range.
        for i in nelements..16 {
            self.vmm[dst as usize].set_zmm32u(i, 0);
        }
        Ok(())
    }

    // ========================================================================
    // VPGATHERQQ — Gather packed qwords using qword indices
    // EVEX.66.0F38.W1 91 /r  (vm64{x,y,z})
    // ========================================================================

    /// VPGATHERQQ Vdq{k1}, vm64z — Bochs `VGATHERQPD_MASK_VpdVSib`
    /// (cpu/avx/gather.cc:385-424).
    pub fn evex_vpgatherqq(&mut self, instr: &Instruction) -> super::Result<()> {
        let dst = instr.dst();
        let sib_idx = Self::vsib_index_reg(instr);
        if sib_idx == dst {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        let k = instr.opmask();
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let seg = BxSegregs::from(instr.seg());

        let mut opmask = self.opmask[k as usize].rrx();
        for n in 0..nelements {
            let mask_bit = 1u64 << n;
            if opmask & mask_bit != 0 {
                let addr = self.resolve_gather_q(instr, sib_idx, n);
                let data = self.v_read_qword(seg, addr)?;
                self.vmm[dst as usize].set_zmm64u(n, data);
                opmask &= !mask_bit;
                self.bx_write_opmask(k as usize, opmask);
            }
        }
        self.bx_write_opmask(k as usize, 0);
        for i in nelements..8 {
            self.vmm[dst as usize].set_zmm64u(i, 0);
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    //! Unit tests for the VPGATHER helpers.
    //!
    //! These cover the address-resolution, V'/R' decoder wiring, and
    //! #UD-on-alias logic without standing up the full memory bus.
    //! Memory-touching execution is exercised by guest code under alpine
    //! when AVX-512 paths are taken (none in alpine kernel itself).

    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::cpudb::amd::amd_ryzen::AmdRyzen;
    use crate::cpu::decoder::{BxSegregs, Instruction};
    use rusty_box_decoder::opcode::Opcode;
    use rusty_box_decoder::fetch_decode64;

    /// Build an EVEX.512 instruction with default sizes; tests fill in
    /// the VSIB pieces afterwards via public setters.
    fn make_evex_instr(opcode: Opcode, dst: u8, k: u8) -> Instruction {
        let mut i = Instruction::default();
        i.set_ia_opcode(opcode);
        i.set_src_reg(0, dst);
        i.set_opmask(k);
        i.set_vex(true);
        i.set_vl(2);
        i.set_seg(BxSegregs::Ds);
        i.init(0, 0, 1, 1);
        i
    }

    fn make_vsib_instr(
        opcode: Opcode,
        dst: u8,
        index_reg_full: u8,
        base_gpr: u8,
        scale: u8,
        disp: u32,
        k: u8,
    ) -> Instruction {
        let mut i = make_evex_instr(opcode, dst, k);
        i.set_evex_v_prime((index_reg_full >> 4) & 1);
        i.set_sib_index(index_reg_full & 0xF);
        i.set_sib_base(base_gpr);
        i.set_sib_scale(scale);
        i.set_displ32(disp);
        i
    }

    #[test]
    fn vsib_index_reg_combines_v_prime_with_sib_index() {
        for n in 0..16u8 {
            let i = make_vsib_instr(Opcode::EvexVgatherddVdqVsib, 0, n, 0, 0, 0, 1);
            assert_eq!(
                super::BxCpuC::<AmdRyzen>::vsib_index_reg(&i),
                n,
                "V'=0 must pass sib_index unchanged"
            );
        }
        for n in 0..16u8 {
            let i = make_vsib_instr(
                Opcode::EvexVgatherddVdqVsib,
                0,
                n + 16,
                0,
                0,
                0,
                1,
            );
            assert_eq!(
                super::BxCpuC::<AmdRyzen>::vsib_index_reg(&i),
                n + 16,
                "V'=1 must extend sib_index into vmm16..31"
            );
        }
    }

    #[test]
    fn resolve_gather_d_signed_index_as64() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.set_gpr64(rusty_box_decoder::instruction::GprIndex::Rbx as usize, 0x4000);
        cpu.vmm[5].set_zmm32s(0, -1);
        let i = make_vsib_instr(
            Opcode::EvexVgatherddVdqVsib,
            0,
            5,
            rusty_box_decoder::instruction::GprIndex::Rbx as usize as u8,
            2,
            0,
            1,
        );
        // 0x4000 + (-1 << 2) + 0 = 0x3FFC
        let addr = cpu.resolve_gather_d(&i, 5, 0);
        assert_eq!(addr, 0x3FFC, "signed index sign-extended before scaling");
    }

    #[test]
    fn resolve_gather_q_uses_qword_index() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.set_gpr64(rusty_box_decoder::instruction::GprIndex::Rax as usize, 0x10000);
        cpu.vmm[7].set_zmm64s(0, -2);
        let i = make_vsib_instr(
            Opcode::EvexVgatherqqVdqVsib,
            0,
            7,
            rusty_box_decoder::instruction::GprIndex::Rax as usize as u8,
            3,
            0x10,
            2,
        );
        // 0x10000 + (-2 << 3) + 0x10 = 0x10000 - 16 + 16 = 0x10000
        let addr = cpu.resolve_gather_q(&i, 7, 0);
        assert_eq!(addr, 0x10000);
    }

    #[test]
    fn vpgather_dst_index_alias_raises_ud() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        for op in [
            Opcode::EvexVgatherddVdqVsib,
            Opcode::EvexVgatherdqVdqVsib,
            Opcode::EvexVgatherqdVdqVsib,
            Opcode::EvexVgatherqqVdqVsib,
        ] {
            cpu.opmask[1].set_rrx(0);
            let i = make_vsib_instr(op, 5, 5, 0, 0, 0, 1);
            let res = match op {
                Opcode::EvexVgatherddVdqVsib => cpu.evex_vpgatherdd(&i),
                Opcode::EvexVgatherdqVdqVsib => cpu.evex_vpgatherdq(&i),
                Opcode::EvexVgatherqdVdqVsib => cpu.evex_vpgatherqd(&i),
                Opcode::EvexVgatherqqVdqVsib => cpu.evex_vpgatherqq(&i),
                _ => unreachable!(),
            };
            assert!(res.is_err(), "{:?} dst==sib_index must #UD", op);
        }
    }

    #[test]
    fn vpgatherdd_zero_opmask_skips_loads_and_clears_opmask() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        for n in 0..16 {
            cpu.vmm[10].set_zmm32u(n, 0xDEAD_BEEF);
        }
        cpu.opmask[1].set_rrx(0);
        let i = make_vsib_instr(
            Opcode::EvexVgatherddVdqVsib,
            10,
            5,
            rusty_box_decoder::instruction::GprIndex::Rbx as usize as u8,
            2,
            0,
            1,
        );
        cpu.evex_vpgatherdd(&i).unwrap();
        // No memory loads occurred; all 16 lanes within VL=512 untouched.
        for n in 0..16 {
            assert_eq!(cpu.vmm[10].zmm32u(n), 0xDEAD_BEEF);
        }
        assert_eq!(cpu.opmask[1].rrx(), 0);
    }

    #[test]
    fn fetch_decode64_vpgatherdd_low_half_index() {
        // EVEX.512.66.0F38.W0 90 /r  vpgatherdd zmm10{k1}, [rbx+zmm5*4+0x40]
        // P0=0x72: ~R=0 ~X=1 ~B=1 ~R'=1 mm=010 (R=1 → dst=10)
        // P1=0x7D: W=0 ~vvvv=1111 1 pp=01     (vvvv=0, 66 prefix)
        // P2=0x49: z=0 L'L=10 b=0 ~V'=1 aaa=001 (VL=512, V'=0, k1)
        // op=0x90, ModRM=0x94 (mod=10 reg=010 rm=100=SIB),
        // SIB=0xAB (scale=10 idx=101=zmm5 base=011=rbx), disp32=0x40.
        let bytes = [0x62, 0x72, 0x7D, 0x49, 0x90, 0x94, 0xAB, 0x40, 0x00, 0x00, 0x00];
        let i = fetch_decode64(&bytes).expect("fetch_decode64 should succeed");
        assert_eq!(i.get_ia_opcode(), Opcode::EvexVgatherddVdqVsib);
        assert_eq!(i.dst(), 10, "REX.R extends nnn=010 to dst=10");
        assert_eq!(i.sib_index(), 5, "sib.idx=101 → 5");
        assert_eq!(i.sib_base(), 3, "sib.base=011 → rbx");
        assert_eq!(i.sib_scale(), 2);
        assert_eq!(i.displ32u(), 0x40);
        assert_eq!(i.opmask(), 1);
        assert_eq!(i.get_evex_v_prime(), 0);
        assert_eq!(i.get_vl(), 2, "L'L=10 → VL=512");
    }

    #[test]
    fn fetch_decode64_vpgatherdd_v_prime_reaches_handler() {
        // Same encoding as above but with ~V'=0 → V'=1, naming zmm21 as
        // the VSIB index. Only decodable correctly when the decoder
        // captures V' and the gather handler combines it with sib_index.
        let bytes = [0x62, 0x72, 0x7D, 0x41, 0x90, 0x94, 0xAB, 0x40, 0x00, 0x00, 0x00];
        let i = fetch_decode64(&bytes).expect("fetch_decode64 should succeed");
        assert_eq!(i.get_evex_v_prime(), 1, "~V'=0 → V'=1");
        assert_eq!(i.sib_index(), 5);
        assert_eq!(
            super::BxCpuC::<AmdRyzen>::vsib_index_reg(&i),
            21,
            "combined V' || sib.idx must address zmm21"
        );
    }
}