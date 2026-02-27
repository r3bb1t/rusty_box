//! Instruction dispatcher for x86 CPU emulation
//!
//! Maps decoded opcodes to their implementation methods.
//! This is the central dispatch table, equivalent to Bochs cpu.cc's
//! BX_CPU_C::cpu_loop() switch statement.

use alloc::format;

use super::{
    cpuid::BxCpuIdTrait,
    decoder::{Instruction, Opcode},
    cpu::BxCpuC,
    Result,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn execute_instruction(&mut self, instr: &mut Instruction) -> Result<()> {
        use crate::cpu::arith8;
        use crate::cpu::arith16;
        use crate::cpu::arith32;
        use crate::cpu::data_xfer8;
        use crate::cpu::data_xfer16;
        use crate::cpu::data_xfer32;

        match instr.get_ia_opcode() {
            // =========================================================================
            // Data transfer (MOV) instructions - 32-bit
            // =========================================================================
            Opcode::MovOp32GdEd => { data_xfer32::MOV_GdEd(self, instr)?; Ok(()) }
            Opcode::MovOp32EdGd => { data_xfer32::MOV_EdGd(self, instr)?; Ok(()) }
            Opcode::MovEdId => { data_xfer32::MOV_EdId(self, instr)?; Ok(()) }

            // =========================================================================
            // Data transfer (MOV) instructions - 8-bit
            // =========================================================================
            Opcode::MovGbEb => { self.mov_gb_eb(instr)?; Ok(()) }
            Opcode::MovEbGb => { self.mov_eb_gb(instr)?; Ok(()) }
            Opcode::MovEbIb => { self.mov_eb_ib(instr)?; Ok(()) }

            // =========================================================================
            // 8-bit Arithmetic instructions (ADD, SUB, etc.)
            // =========================================================================
            Opcode::AddEbGb => arith8::ADD_EbGb(self, instr),
            Opcode::AddGbEb => arith8::ADD_GbEb(self, instr),
            Opcode::AdcEbGb => arith8::ADC_EbGb(self, instr),
            Opcode::AdcGbEb => arith8::ADC_GbEb(self, instr),
            Opcode::AdcGwEw => arith16::ADC_GwEw(self, instr),
            Opcode::AdcEwsIb => arith16::ADC_EwsIb(self, instr),
            Opcode::SubEbGb => arith8::SUB_EbGb(self, instr),
            Opcode::SubGbEb => arith8::SUB_GbEb(self, instr),
            Opcode::AndEbGb => { self.and_eb_gb(instr)?; Ok(()) }
            Opcode::AndGbEb => { self.and_gb_eb(instr)?; Ok(()) }
            Opcode::AndEbIb => { self.and_eb_ib(instr)?; Ok(()) }
            Opcode::OrEbGb => { self.or_eb_gb(instr)?; Ok(()) }
            Opcode::OrGbEb => { self.or_gb_eb(instr)?; Ok(()) }
            Opcode::OrEbIb => { self.or_eb_ib(instr)?; Ok(()) }
            Opcode::XorEbGb => { self.xor_eb_gb(instr)?; Ok(()) }
            Opcode::XorGbEb => { self.xor_gb_eb(instr)?; Ok(()) }
            Opcode::XorEbIb => { self.xor_eb_ib(instr)?; Ok(()) }
            Opcode::NotEb => { self.not_eb(instr)?; Ok(()) }
            Opcode::TestEbGb => { self.test_eb_gb(instr)?; Ok(()) }
            Opcode::TestEbIb => { self.test_eb_ib(instr)?; Ok(()) }

            // =========================================================================
            // Data transfer (MOV) instructions - 16-bit
            // =========================================================================
            Opcode::MovGwEw => { self.mov_gw_ew(instr)?; Ok(()) }
            Opcode::MovEwGw => { self.mov_ew_gw(instr)?; Ok(()) }
            Opcode::MovEwIw => { self.mov_ew_iw(instr)?; Ok(()) }

            // =========================================================================
            // Segment register MOV
            // =========================================================================
            Opcode::MovEwSw => { self.mov_ew_sw(instr)?; Ok(()) }
            Opcode::MovSwEw => { self.mov_sw_ew(instr)?; Ok(()) }

            // =========================================================================
            // MOV with direct memory offset
            // =========================================================================
            Opcode::MovAlod => data_xfer8::MOV_ALOd(self, instr),
            Opcode::MovAxod => data_xfer16::MOV_AXOd(self, instr),
            Opcode::MovOdAl => data_xfer8::MOV_OdAL(self, instr),
            Opcode::MovOdAx => data_xfer16::MOV_OdAX(self, instr),
            Opcode::MovEaxod => data_xfer32::MOV_EAXOd(self, instr),
            Opcode::MovOdEax => data_xfer32::MOV_OdEAX(self, instr),

            // =========================================================================
            // PUSH/POP segment registers
            // =========================================================================
            Opcode::PushOp16Sw => self.push_op16_sw(instr),
            Opcode::PopOp16Sw => self.pop_op16_sw(instr),
            Opcode::PushIw => { self.push_iw(instr)?; Ok(()) }
            Opcode::PushSIb16 => { self.push_sib16(instr)?; Ok(()) }

            // =========================================================================
            // Arithmetic (ADD) instructions
            // =========================================================================
            Opcode::AddGdEd => { arith32::ADD_GdEd(self, instr)?; Ok(()) }
            Opcode::AddEdGd => { arith32::ADD_EdGd(self, instr)?; Ok(()) }
            Opcode::AddEaxid => { arith32::ADD_EAX_Id(self, instr); Ok(()) }
            Opcode::AddAxiw => arith16::ADD_Axiw(self, instr),
            Opcode::AddAlib | Opcode::AddEbIb => arith8::ADD_EbIb(self, instr),
            Opcode::SubEbIb => arith8::SUB_EbIb(self, instr),
            Opcode::AdcEbIb => arith8::ADC_EbIb(self, instr),
            Opcode::SbbEbIb => arith8::SBB_EbIb(self, instr),
            Opcode::SbbEbGb => arith8::SBB_EbGb(self, instr),
            Opcode::SbbGbEb => arith8::SBB_GbEb(self, instr),
            Opcode::AddEwsIb => arith16::ADD_EwIbR(self, instr),
            Opcode::AddEwIw => arith16::ADD_EwIw(self, instr),
            Opcode::AddEwGw => arith16::ADD_EwGw(self, instr),
            Opcode::AddGwEw => arith16::ADD_GwEw(self, instr),
            Opcode::AddEdsIb | Opcode::AddEdId => { arith32::ADD_EdId(self, instr)?; Ok(()) }

            // =========================================================================
            // Arithmetic (SUB) instructions
            // =========================================================================
            Opcode::SubGdEd => { arith32::SUB_GdEd(self, instr)?; Ok(()) }
            Opcode::SubEdGd => { arith32::SUB_EdGd(self, instr)?; Ok(()) }
            Opcode::SubGwEw => arith16::SUB_GwEw(self, instr),
            Opcode::SubEwGw => arith16::SUB_EwGw(self, instr),
            Opcode::SbbGwEw => arith16::SBB_GwEw(self, instr),
            Opcode::SbbEwGw => arith16::SBB_EwGw(self, instr),
            Opcode::SbbEdGd => arith32::SBB_EdGd(self, instr),
            Opcode::SbbGdEd => arith32::SBB_GdEd(self, instr),
            Opcode::SbbEaxid => { arith32::SBB_EAX_Id(self, instr); Ok(()) }
            Opcode::SbbEdId => { arith32::SBB_EdId_R(self, instr); Ok(()) }
            Opcode::SbbEdsIb => { arith32::SBB_EdIb_R(self, instr); Ok(()) }
            Opcode::SubEaxid => { arith32::SUB_EAX_Id(self, instr); Ok(()) }
            Opcode::SubAlib => arith8::SUB_AL_Ib(self, instr),
            Opcode::SubAxiw => arith16::SUB_AX_Iw(self, instr),
            Opcode::SubEwIw => arith16::SUB_EwIw(self, instr),
            Opcode::SubEwsIb => arith16::SUB_EwsIb(self, instr),
            Opcode::SubEdsIb | Opcode::SubEdId => { arith32::SUB_EdId(self, instr)?; Ok(()) }
            // SUB zero idioms (SUB reg, reg where src==dst -> result is always 0)
            Opcode::SubEwGwZeroIdiom | Opcode::SubGwEwZeroIdiom => { self.zero_idiom_gw_r(instr); Ok(()) }
            Opcode::SubEdGdZeroIdiom | Opcode::SubGdEdZeroIdiom => { self.zero_idiom_gd_r(instr); Ok(()) }

            // =========================================================================
            // XOR instructions
            // =========================================================================
            Opcode::XorEdGd => { self.xor_ed_gd(instr)?; Ok(()) }
            Opcode::XorEdGdZeroIdiom | Opcode::XorGdEdZeroIdiom => { self.zero_idiom_gd_r(instr); Ok(()) }
            Opcode::XorGdEd => { self.xor_gd_ed(instr)?; Ok(()) }
            Opcode::XorEwGw => { self.xor_ew_gw(instr)?; Ok(()) }
            Opcode::XorGwEw => { self.xor_gw_ew(instr)?; Ok(()) }
            Opcode::XorEwGwZeroIdiom | Opcode::XorGwEwZeroIdiom => { self.zero_idiom_gw_r(instr); Ok(()) }
            Opcode::XorAlib => { self.xor_eb_ib_r(instr); Ok(()) }
            Opcode::XorAxiw => { self.xor_ew_iw_r(instr); Ok(()) }
            Opcode::XorEaxid => { self.xor_ed_id_r(instr); Ok(()) }

            // =========================================================================
            // FAR JMP
            // =========================================================================
            Opcode::JmpfAp => self.jmpf_ap(instr),

            // =========================================================================
            // Flag manipulation instructions
            // =========================================================================
            Opcode::Clc => self.clc(instr),
            Opcode::Stc => self.stc(instr),
            Opcode::Cmc => self.cmc(instr),
            Opcode::Cli => self.cli(instr),
            Opcode::Sti => self.sti(instr),

            // =========================================================================
            // Descriptor table / task register loads
            // =========================================================================
            Opcode::LidtMs => { self.lidt_ms(instr)?; Ok(()) }
            Opcode::LgdtMs => { self.lgdt_ms(instr)?; Ok(()) }
            Opcode::LldtEw => { self.lldt_ew(instr)?; Ok(()) }
            Opcode::LtrEw => { self.ltr_ew(instr)?; Ok(()) }

            // =========================================================================
            // Control Register Read Operations (MOV r32, CRx)
            // =========================================================================
            Opcode::MovRdCr0 => { self.mov_rd_cr0(instr)?; Ok(()) }
            Opcode::MovRdCr2 => { self.mov_rd_cr2(instr)?; Ok(()) }
            Opcode::MovRdCr3 => { self.mov_rd_cr3(instr)?; Ok(()) }
            Opcode::MovRdCr4 => { self.mov_rd_cr4(instr)?; Ok(()) }

            // =========================================================================
            // Control Register Write Operations (MOV CRx, r32)
            // =========================================================================
            Opcode::MovCr0rd => { self.mov_cr0_rd(instr)?; Ok(()) }
            Opcode::MovCr2rd => { self.mov_cr2_rd(instr)?; Ok(()) }
            Opcode::MovCr3rd => { self.mov_cr3_rd(instr)?; Ok(()) }
            Opcode::MovCr4rd => { self.mov_cr4_rd(instr)?; Ok(()) }

            // Debug Register Operations (0F 21 / 0F 23)
            Opcode::MovRdDd => self.mov_rd_dd(instr),
            Opcode::MovDdRd => self.mov_dd_rd(instr),

            Opcode::LmswEw => { self.lmsw_ew(instr)?; Ok(()) }

            Opcode::Cld => self.cld(instr),
            Opcode::Std => self.std_(instr),
            Opcode::Nop => Ok(()),

            // =========================================================================
            // I/O port instructions
            // =========================================================================
            Opcode::InAlib => { self.in_al_ib(instr); Ok(()) }
            Opcode::InAxib => { self.in_ax_ib(instr); Ok(()) }
            Opcode::InEaxib => { self.in_eax_ib(instr); Ok(()) }
            Opcode::OutIbAl => { self.out_ib_al(instr); Ok(()) }
            Opcode::OutIbAx => { self.out_ib_ax(instr); Ok(()) }
            Opcode::OutIbEax => { self.out_ib_eax(instr); Ok(()) }
            Opcode::InAlDx => { self.in_al_dx(instr); Ok(()) }
            Opcode::InAxDx => { self.in_ax_dx(instr); Ok(()) }
            Opcode::InEaxDx => { self.in_eax_dx(instr); Ok(()) }
            Opcode::OutDxAl => { self.out_dx_al(instr); Ok(()) }
            Opcode::OutDxAx => { self.out_dx_ax(instr); Ok(()) }
            Opcode::OutDxEax => { self.out_dx_eax(instr); Ok(()) }

            // INS/OUTS string I/O
            Opcode::RepInsbYbDx => self.insb_dispatch(instr),
            Opcode::RepInswYwDx => self.insw_dispatch(instr),
            Opcode::RepInsdYdDx => self.insd_dispatch(instr),
            Opcode::RepOutsbDxxb => self.outsb_dispatch(instr),
            Opcode::RepOutswDxxw => self.outsw_dispatch(instr),
            Opcode::RepOutsdDxxd => self.outsd_dispatch(instr),

            // =========================================================================
            // Conditional jumps (8-bit displacement, 16-bit mode)
            // =========================================================================
            Opcode::JoJbw => { self.jo_jb(instr); Ok(()) }
            Opcode::JnoJbw => { self.jno_jb(instr); Ok(()) }
            Opcode::JbJbw => { self.jb_jb(instr); Ok(()) }
            Opcode::JnbJbw => { self.jnb_jb(instr); Ok(()) }
            Opcode::JzJbw => { self.jz_jb(instr); Ok(()) }
            Opcode::JnzJbw => { self.jnz_jb(instr); Ok(()) }
            Opcode::JbeJbw => { self.jbe_jb(instr); Ok(()) }
            Opcode::JnbeJbw => { self.jnbe_jb(instr); Ok(()) }
            Opcode::JsJbw => { self.js_jb(instr); Ok(()) }
            Opcode::JnsJbw => { self.jns_jb(instr); Ok(()) }
            Opcode::JpJbw => { self.jp_jb(instr); Ok(()) }
            Opcode::JnpJbw => { self.jnp_jb(instr); Ok(()) }
            Opcode::JlJbw => { self.jl_jb(instr); Ok(()) }
            Opcode::JnlJbw => { self.jnl_jb(instr); Ok(()) }
            Opcode::JleJbw => { self.jle_jb(instr); Ok(()) }
            Opcode::JnleJbw => { self.jnle_jb(instr); Ok(()) }

            // Conditional jumps (16-bit displacement)
            Opcode::JzJw => { self.jz_jw(instr); Ok(()) }
            Opcode::JnzJw => { self.jnz_jw(instr); Ok(()) }

            // =========================================================================
            // JMP instructions
            // =========================================================================
            Opcode::JmpJbw => { self.jmp_jb(instr); Ok(()) }
            Opcode::JmpJw => { self.jmp_jw(instr); Ok(()) }
            Opcode::JmpJd => { self.jmp_jd(instr)?; Ok(()) }
            Opcode::JmpJbd => { self.jmp_jd(instr)?; Ok(()) }
            Opcode::JmpEw => { self.jmp_ew(instr)?; Ok(()) }
            Opcode::JmpEd => { self.jmp_ed(instr)?; Ok(()) }

            // =========================================================================
            // CALL instructions
            // =========================================================================
            Opcode::CallJw => { self.call_jw(instr)?; Ok(()) }
            Opcode::CallJd => { self.call_jd(instr)?; Ok(()) }
            Opcode::CallEw => { self.call_ew(instr)?; Ok(()) }
            Opcode::CallEd => { self.call_ed(instr)?; Ok(()) }

            // =========================================================================
            // RET instructions
            // =========================================================================
            Opcode::RetOp16 => { self.ret_near16(instr)?; Ok(()) }
            Opcode::RetOp16Iw => { self.ret_near16_iw(instr)?; Ok(()) }
            Opcode::RetOp32 => { self.ret_near32(instr)?; Ok(()) }
            Opcode::RetOp32Iw => { self.ret_near32_iw(instr)?; Ok(()) }

            // =========================================================================
            // LOOP instructions
            // =========================================================================
            Opcode::LoopJbw => { self.loop16_jb(instr); Ok(()) }
            Opcode::LoopeJbw => { self.loope16_jb(instr); Ok(()) }
            Opcode::LoopneJbw => { self.loopne16_jb(instr); Ok(()) }
            Opcode::JcxzJbw => { self.jcxz_jb(instr); Ok(()) }
            Opcode::JecxzJbd => { self.jecxz_jb(instr); Ok(()) }

            // =========================================================================
            // Far CALL instructions (32-bit)
            // =========================================================================
            Opcode::CallfOp32Ap => self.call32_ap(instr),
            Opcode::CallfOp32Ep => self.call32_ep(instr),

            // =========================================================================
            // Far JMP instructions (32-bit)
            // =========================================================================
            Opcode::JmpfOp32Ep => self.jmp32_ep(instr),

            // =========================================================================
            // Far RET instructions (32-bit)
            // =========================================================================
            Opcode::RetfOp32 => self.retfar32(instr),
            Opcode::RetfOp32Iw => self.retfar32_iw(instr),

            // =========================================================================
            // Conditional jumps with 32-bit displacement (Jd variants)
            // =========================================================================
            Opcode::JoJd | Opcode::JoJbd => { self.jo_jd(instr)?; Ok(()) }
            Opcode::JnoJd | Opcode::JnoJbd => { self.jno_jd(instr)?; Ok(()) }
            Opcode::JbJd | Opcode::JbJbd => { self.jb_jd(instr)?; Ok(()) }
            Opcode::JnbJd | Opcode::JnbJbd => { self.jnb_jd(instr)?; Ok(()) }
            Opcode::JzJd | Opcode::JzJbd => { self.jz_jd(instr)?; Ok(()) }
            Opcode::JnzJd | Opcode::JnzJbd => { self.jnz_jd(instr)?; Ok(()) }
            Opcode::JbeJd | Opcode::JbeJbd => { self.jbe_jd(instr)?; Ok(()) }
            Opcode::JnbeJd | Opcode::JnbeJbd => { self.jnbe_jd(instr)?; Ok(()) }
            Opcode::JsJd | Opcode::JsJbd => { self.js_jd(instr)?; Ok(()) }
            Opcode::JnsJd | Opcode::JnsJbd => { self.jns_jd(instr)?; Ok(()) }
            Opcode::JpJd | Opcode::JpJbd => { self.jp_jd(instr)?; Ok(()) }
            Opcode::JnpJd | Opcode::JnpJbd => { self.jnp_jd(instr)?; Ok(()) }
            Opcode::JlJd | Opcode::JlJbd => { self.jl_jd(instr)?; Ok(()) }
            Opcode::JnlJd | Opcode::JnlJbd => { self.jnl_jd(instr)?; Ok(()) }
            Opcode::JleJd | Opcode::JleJbd => { self.jle_jd(instr)?; Ok(()) }
            Opcode::JnleJd | Opcode::JnleJbd => { self.jnle_jd(instr)?; Ok(()) }

            // LOOP instructions: 32-bit variants
            Opcode::LoopJbd => { self.loop32_jb(instr)?; Ok(()) }
            Opcode::LoopeJbd => { self.loope32_jb(instr)?; Ok(()) }
            Opcode::LoopneJbd => { self.loopne32_jb(instr)?; Ok(()) }

            // =========================================================================
            // Far CALL instructions (16-bit)
            // =========================================================================
            Opcode::CallfOp16Ap => self.call16_ap(instr),
            Opcode::CallfOp16Ep => self.call16_ep(instr),

            // =========================================================================
            // Far JMP instructions (16-bit)
            // =========================================================================
            Opcode::JmpfOp16Ep => self.jmp16_ep(instr),

            // =========================================================================
            // Far RET instructions (16-bit)
            // =========================================================================
            Opcode::RetfOp16 => self.retfar16(instr),
            Opcode::RetfOp16Iw => self.retfar16_iw(instr),

            // =========================================================================
            // Conditional jumps with 16-bit displacement (Jw variants)
            // =========================================================================
            Opcode::JoJw => { self.jo_jw(instr); Ok(()) }
            Opcode::JnoJw => { self.jno_jw(instr); Ok(()) }
            Opcode::JbJw => { self.jb_jw(instr); Ok(()) }
            Opcode::JnbJw => { self.jnb_jw(instr); Ok(()) }
            Opcode::JbeJw => { self.jbe_jw(instr); Ok(()) }
            Opcode::JnbeJw => { self.jnbe_jw(instr); Ok(()) }
            Opcode::JsJw => { self.js_jw(instr); Ok(()) }
            Opcode::JnsJw => { self.jns_jw(instr); Ok(()) }
            Opcode::JpJw => { self.jp_jw(instr); Ok(()) }
            Opcode::JnpJw => { self.jnp_jw(instr); Ok(()) }
            Opcode::JlJw => { self.jl_jw(instr); Ok(()) }
            Opcode::JnlJw => { self.jnl_jw(instr); Ok(()) }
            Opcode::JleJw => { self.jle_jw(instr); Ok(()) }
            Opcode::JnleJw => { self.jnle_jw(instr); Ok(()) }

            // =========================================================================
            // CMP instructions
            // =========================================================================
            Opcode::CmpGbEb => { self.cmp_gb_eb(instr)?; Ok(()) }
            Opcode::CmpGwEw => { self.cmp_gw_ew(instr)?; Ok(()) }
            Opcode::CmpGdEd => { arith32::CMP_GdEd(self, instr)?; Ok(()) }
            Opcode::CmpEwGw => arith16::CMP_EwGw(self, instr),
            Opcode::CmpAlib => { self.cmp_al_ib(instr); Ok(()) }
            Opcode::CmpEbIb => { self.cmp_eb_ib(instr)?; Ok(()) }
            Opcode::CmpEbGb => { self.cmp_eb_gb(instr)?; Ok(()) }
            Opcode::CmpAxiw => { self.cmp_ax_iw(instr); Ok(()) }
            Opcode::CmpEaxid => { self.cmp_eax_id(instr); Ok(()) }
            Opcode::CmpEwIw | Opcode::CmpEwsIb => { self.cmp_ew_iw(instr)?; Ok(()) }
            Opcode::CmpEdId | Opcode::CmpEdsIb => { arith32::CMP_EdId(self, instr)?; Ok(()) }
            Opcode::CmpEdGd => { arith32::CMP_EdGd(self, instr)?; Ok(()) }

            // =========================================================================
            // TEST instructions
            // =========================================================================
            Opcode::TestEwGw => { self.test_ew_gw(instr)?; Ok(()) }
            Opcode::TestEdGd => { self.test_ed_gd(instr)?; Ok(()) }
            Opcode::TestAlib => { self.test_al_ib(instr); Ok(()) }
            Opcode::TestAxiw => { self.test_ax_iw(instr); Ok(()) }
            Opcode::TestEaxid => { self.test_eax_id(instr); Ok(()) }
            Opcode::TestEwIw => { self.test_ew_iw(instr)?; Ok(()) }
            Opcode::TestEdId => { self.test_ed_id(instr)?; Ok(()) }

            // =========================================================================
            // AND/OR/NOT instructions
            // =========================================================================
            Opcode::AndGwEw => { self.and_gw_ew(instr)?; Ok(()) }
            Opcode::AndEwGw => { self.and_ew_gw(instr)?; Ok(()) }
            Opcode::AndGdEd => { self.and_gd_ed(instr)?; Ok(()) }
            Opcode::AndEdGd => { self.and_ed_gd(instr)?; Ok(()) }
            Opcode::AndAlib => { self.and_al_ib(instr); Ok(()) }
            Opcode::AndAxiw => { self.and_ax_iw(instr); Ok(()) }
            Opcode::AndEaxid => { self.and_eax_id(instr); Ok(()) }
            Opcode::AndEwIw | Opcode::AndEwsIb => { self.and_ew_iw(instr)?; Ok(()) }
            Opcode::AndEdId | Opcode::AndEdsIb => { self.and_ed_id(instr)?; Ok(()) }

            Opcode::OrGwEw => { self.or_gw_ew(instr)?; Ok(()) }
            Opcode::OrEwGw => { self.or_ew_gw(instr)?; Ok(()) }
            Opcode::OrGdEd => { self.or_gd_ed(instr)?; Ok(()) }
            Opcode::OrEdGd => { self.or_ed_gd(instr)?; Ok(()) }
            Opcode::OrAlib => { self.or_al_ib(instr); Ok(()) }
            Opcode::OrAxiw => { self.or_ax_iw(instr); Ok(()) }
            Opcode::OrEaxid => { self.or_eax_id(instr); Ok(()) }
            Opcode::OrEwIw | Opcode::OrEwsIb => { self.or_ew_iw(instr)?; Ok(()) }
            Opcode::OrEdId | Opcode::OrEdsIb => { self.or_ed_id(instr)?; Ok(()) }
            Opcode::XorEwIw | Opcode::XorEwsIb => { self.xor_ew_iw(instr)?; Ok(()) }
            Opcode::XorEdId => { self.xor_ed_id(instr)?; Ok(()) }
            Opcode::NotEw => { self.not_ew(instr)?; Ok(()) }
            Opcode::NotEd => { self.not_ed(instr)?; Ok(()) }
            Opcode::NegEd => { arith32::NEG_Ed(self, instr)?; Ok(()) }

            // =========================================================================
            // Bit Test instructions (BT, BTS, BTR, BTC)
            // =========================================================================
            Opcode::BtEdIb => { self.bt_ed_ib(instr)?; Ok(()) }
            Opcode::BtsEdIb => { self.bts_ed_ib(instr)?; Ok(()) }
            Opcode::BtrEdIb => { self.btr_ed_ib(instr)?; Ok(()) }
            Opcode::BtcEdIb => { self.btc_ed_ib(instr)?; Ok(()) }
            Opcode::BtEdGd => { self.bt_ed_gd(instr)?; Ok(()) }
            Opcode::BtsEdGd => { self.bts_ed_gd(instr)?; Ok(()) }
            Opcode::BtrEdGd => { self.btr_ed_gd(instr)?; Ok(()) }
            Opcode::BtcEdGd => { self.btc_ed_gd(instr)?; Ok(()) }

            // =========================================================================
            // Bit Scan instructions (BSF, BSR)
            // =========================================================================
            Opcode::BsfGdEd => self.bsf_gd_ed(instr),
            Opcode::BsrGdEd => self.bsr_gd_ed(instr),
            Opcode::BsfGwEw => self.bsf_gw_ew(instr),
            Opcode::BsrGwEw => self.bsr_gw_ew(instr),

            // =========================================================================
            // Multiplication and Division instructions
            // =========================================================================
            Opcode::MulAleb => self.mul_al_eb(instr),
            Opcode::ImulAleb => self.imul_al_eb(instr),
            Opcode::DivAleb => self.div_al_eb(instr),
            Opcode::IdivAleb => self.idiv_al_eb(instr),
            Opcode::MulAxew => self.mul_ax_ew(instr),
            Opcode::ImulAxew => self.imul_ax_ew(instr),
            Opcode::DivAxew => self.div_ax_ew(instr),
            Opcode::IdivAxew => self.idiv_ax_ew(instr),
            Opcode::MulEaxed => self.mul_eax_ed(instr),
            Opcode::ImulEaxed => self.imul_eax_ed(instr),
            Opcode::ImulGdEdsIb => { self.imul_gd_ed_ib(instr)?; Ok(()) }
            Opcode::ImulGdEdId => self.imul_gd_ed_id(instr),
            Opcode::ImulGdEd => self.imul_gd_ed(instr),
            Opcode::ImulGwEw => self.imul_gw_ew(instr),
            Opcode::ImulGwEwIw => self.imul_gw_ew_iw(instr),
            Opcode::ImulGwEwsIb => self.imul_gw_ew_sib(instr),
            Opcode::DivEaxed => self.div_eax_ed(instr),
            Opcode::IdivEaxed => self.idiv_eax_ed(instr),

            // =========================================================================
            // INC/DEC instructions
            // =========================================================================
            Opcode::IncEb => arith8::inc_eb_dispatch(self, instr),
            Opcode::DecEb => arith8::dec_eb_dispatch(self, instr),
            Opcode::IncEw => self.inc_ew(instr),
            Opcode::IncEd => self.inc_ed(instr),
            Opcode::DecEw => self.dec_ew(instr),
            Opcode::DecEd => self.dec_ed(instr),

            // =========================================================================
            // PUSH/POP instructions
            // =========================================================================
            Opcode::PushEw => self.push_ew(instr),
            Opcode::PushEd => self.push_ed(instr),
            Opcode::PushId => { self.push_id(instr)?; Ok(()) }
            Opcode::PushSIb32 => { self.push_id(instr)?; Ok(()) }
            Opcode::PopEw => self.pop_ew(instr),
            Opcode::PopEd => self.pop_ed(instr),
            Opcode::PopOp32Sw => { self.pop32_sw(instr)?; Ok(()) }
            Opcode::LeaveOp32 => { self.leave_op32(instr)?; Ok(()) }
            Opcode::PushOp32Sw => { self.push_op32_sw(instr)?; Ok(()) }
            Opcode::PushaOp16 => { self.pusha16(instr)?; Ok(()) }
            Opcode::PushaOp32 => { self.pusha32(instr)?; Ok(()) }
            Opcode::PopaOp16 => { self.popa16(instr)?; Ok(()) }
            Opcode::PopaOp32 => { self.popa32(instr)?; Ok(()) }
            Opcode::PushfFw => { self.pushf_fw(instr)?; Ok(()) }
            Opcode::PopfFw => { self.popf_fw(instr)?; Ok(()) }
            Opcode::PushfFd => { self.pushf_fd(instr)?; Ok(()) }
            Opcode::PopfFd => { self.popf_fd(instr)?; Ok(()) }
            Opcode::LeaveOp16 => { self.leave16(instr)?; Ok(()) }

            // =========================================================================
            // String instructions
            // =========================================================================
            Opcode::RepMovsbYbXb => self.movsb_dispatch(instr),
            Opcode::RepMovswYwXw => self.movsw_dispatch(instr),
            Opcode::RepMovsdYdXd => self.movsd_dispatch(instr),
            Opcode::RepStosbYbAl => self.stosb_dispatch(instr),
            Opcode::RepStoswYwAx => self.stosw_dispatch(instr),
            Opcode::RepStosdYdEax => self.stosd_dispatch(instr),
            Opcode::RepLodsbAlxb => self.lodsb_dispatch(instr),
            Opcode::RepLodswAxxw => self.lodsw_dispatch(instr),
            Opcode::RepLodsdEaxxd => self.lodsd_dispatch(instr),
            Opcode::RepScasbAlyb => self.scasb_dispatch(instr),
            Opcode::RepScaswAxyw => self.scasw_dispatch(instr),
            Opcode::RepScasdEaxyd => self.scasd_dispatch(instr),
            Opcode::RepCmpsbXbYb => self.cmpsb_dispatch(instr),
            Opcode::RepCmpswXwYw => self.cmpsw_dispatch(instr),
            Opcode::RepCmpsdXdYd => self.cmpsd_dispatch(instr),

            // =========================================================================
            // Software interrupts
            // =========================================================================
            Opcode::IntIb => { self.int_ib(instr); Ok(()) }
            Opcode::INT3 => { self.int3(instr); Ok(()) }
            Opcode::INT1 => self.int1(instr),
            Opcode::IretOp16 => { self.iret16(instr)?; Ok(()) }
            Opcode::IretOp32 => { self.iret32(instr)?; Ok(()) }

            // =========================================================================
            // BOUND - Check Array Index Against Bounds
            // =========================================================================
            Opcode::BoundGwMa => { self.bound_gw_ma(instr)?; Ok(()) }
            Opcode::BoundGdMa => { self.bound_gd_ma(instr)?; Ok(()) }

            // =========================================================================
            // 64-bit control transfer instructions
            // =========================================================================
            Opcode::CallJq => self.call_jq(instr),
            Opcode::CallEq => self.call_eq_r(instr),
            Opcode::CallfOp64Ep => self.call64_ep(instr),
            Opcode::JmpJq => self.jmp_jq(instr),
            Opcode::JmpEq => self.jmp_eq_r(instr),
            Opcode::JmpfOp64Ep => self.jmp64_ep(instr),
            Opcode::RetOp64Iw => self.retnear64_iw(instr),
            Opcode::RetfOp64 => self.retfar64(instr),
            Opcode::RetfOp64Iw => self.retfar64_iw(instr),
            Opcode::IretOp64 => self.iret64(instr),
            Opcode::JrcxzJbq => { self.jrcxz_jb(instr); Ok(()) }

            // =========================================================================
            // Conditional jumps with 64-bit displacement (Jq variants)
            // =========================================================================
            Opcode::JoJq => { self.jo_jq(instr); Ok(()) }
            Opcode::JnoJq => { self.jno_jq(instr); Ok(()) }
            Opcode::JbJq => { self.jb_jq(instr); Ok(()) }
            Opcode::JnbJq => { self.jnb_jq(instr); Ok(()) }
            Opcode::JzJq => { self.jz_jq(instr); Ok(()) }
            Opcode::JnzJq => { self.jnz_jq(instr); Ok(()) }
            Opcode::JbeJq => { self.jbe_jq(instr); Ok(()) }
            Opcode::JnbeJq => { self.jnbe_jq(instr); Ok(()) }
            Opcode::JsJq => { self.js_jq(instr); Ok(()) }
            Opcode::JnsJq => { self.jns_jq(instr); Ok(()) }
            Opcode::JpJq => { self.jp_jq(instr); Ok(()) }
            Opcode::JnpJq => { self.jnp_jq(instr); Ok(()) }
            Opcode::JlJq => { self.jl_jq(instr); Ok(()) }
            Opcode::JnlJq => { self.jnl_jq(instr); Ok(()) }
            Opcode::JleJq => { self.jle_jq(instr); Ok(()) }
            Opcode::JnleJq => { self.jnle_jq(instr); Ok(()) }

            // =========================================================================
            // System instructions
            // =========================================================================
            Opcode::Hlt => { self.hlt(instr); Ok(()) }
            Opcode::Wbinvd => self.wbinvd(instr),
            Opcode::Invlpg => self.invlpg(instr),
            Opcode::Clts => self.clts(instr),

            // =========================================================================
            // LES/LDS/LSS/LFS/LGS - Load Far Pointer
            // =========================================================================
            Opcode::LesGwMp => self.les_gw_mp(instr),
            Opcode::LesGdMp => self.les_gd_mp(instr),
            Opcode::LdsGwMp => self.lds_gw_mp(instr),
            Opcode::LdsGdMp => self.lds_gd_mp(instr),
            Opcode::LssGwMp => self.lss_gw_mp(instr),
            Opcode::LssGdMp => self.lss_gd_mp(instr),
            Opcode::LfsGwMp => self.lfs_gw_mp(instr),
            Opcode::LfsGdMp => self.lfs_gd_mp(instr),
            Opcode::LgsGwMp => self.lgs_gw_mp(instr),
            Opcode::LgsGdMp => self.lgs_gd_mp(instr),

            Opcode::Cpuid => { self.cpuid(instr); Ok(()) }
            Opcode::Rdmsr => self.rdmsr(instr),
            Opcode::Wrmsr => self.wrmsr(instr),

            // =========================================================================
            // Shift/Rotate instructions
            // =========================================================================
            Opcode::ShlEbI1 => { self.shl_eb_1(instr)?; Ok(()) }
            Opcode::ShlEb => { self.shl_eb_cl(instr)?; Ok(()) }
            Opcode::ShlEbIb => { self.shl_eb_ib(instr)?; Ok(()) }
            Opcode::ShlEwI1 => { self.shl_ew_1(instr)?; Ok(()) }
            Opcode::ShlEw => { self.shl_ew_cl(instr)?; Ok(()) }
            Opcode::ShlEwIb => { self.shl_ew_ib(instr)?; Ok(()) }
            Opcode::ShlEdI1 => { self.shl_ed_1(instr)?; Ok(()) }
            Opcode::ShlEd => { self.shl_ed_cl(instr)?; Ok(()) }
            Opcode::ShlEdIb => { self.shl_ed_ib(instr)?; Ok(()) }
            Opcode::ShldEdGdIb => { self.shld_ed_gd_ib(instr)?; Ok(()) }
            Opcode::ShldEdGd => { self.shld_ed_gd_cl(instr)?; Ok(()) }
            Opcode::ShrdEdGdIb => { self.shrd_ed_gd_ib(instr)?; Ok(()) }
            Opcode::ShrdEdGd => { self.shrd_ed_gd_cl(instr)?; Ok(()) }
            Opcode::SarEbIb => { self.sar_eb_ib(instr)?; Ok(()) }

            Opcode::ShrEbI1 => { self.shr_eb_1(instr)?; Ok(()) }
            Opcode::ShrEb => { self.shr_eb_cl(instr)?; Ok(()) }
            Opcode::ShrEbIb => { self.shr_eb_ib(instr)?; Ok(()) }
            Opcode::ShrEwI1 => { self.shr_ew_1(instr)?; Ok(()) }
            Opcode::ShrEw => { self.shr_ew_cl(instr)?; Ok(()) }
            Opcode::ShrEwIb => { self.shr_ew_ib(instr)?; Ok(()) }
            Opcode::ShrEdI1 => { self.shr_ed_1(instr)?; Ok(()) }
            Opcode::ShrEd => { self.shr_ed_cl(instr)?; Ok(()) }
            Opcode::ShrEdIb => { self.shr_ed_ib(instr)?; Ok(()) }

            // ROL - Rotate Left
            Opcode::RolEbI1 => { self.rol_eb_1(instr)?; Ok(()) }
            Opcode::RolEb => { self.rol_eb_cl(instr)?; Ok(()) }
            Opcode::RolEbIb => { self.rol_eb_ib(instr)?; Ok(()) }
            Opcode::RolEwI1 => { self.rol_ew_1(instr)?; Ok(()) }
            Opcode::RolEw => { self.rol_ew_cl(instr)?; Ok(()) }
            Opcode::RolEwIb => { self.rol_ew_ib(instr)?; Ok(()) }
            Opcode::RolEdI1 => { self.rol_ed_1(instr)?; Ok(()) }
            Opcode::RolEd => { self.rol_ed_cl(instr)?; Ok(()) }
            Opcode::RolEdIb => { self.rol_ed_ib(instr)?; Ok(()) }
            Opcode::RorEbI1 => { self.ror_eb_1(instr)?; Ok(()) }
            Opcode::RorEb => { self.ror_eb_cl(instr)?; Ok(()) }
            Opcode::RorEbIb => { self.ror_eb_ib(instr)?; Ok(()) }
            Opcode::RorEwI1 => { self.ror_ew_1(instr)?; Ok(()) }
            Opcode::RorEw => { self.ror_ew_cl(instr)?; Ok(()) }
            Opcode::RorEwIb => { self.ror_ew_ib(instr)?; Ok(()) }
            Opcode::RorEdI1 => { self.ror_ed_1(instr)?; Ok(()) }
            Opcode::RorEd => { self.ror_ed_cl(instr)?; Ok(()) }
            Opcode::RorEdIb => { self.ror_ed_ib(instr)?; Ok(()) }
            Opcode::SarEbI1 => { self.sar_eb_1(instr)?; Ok(()) }
            Opcode::SarEb => { self.sar_eb_cl(instr)?; Ok(()) }
            Opcode::SarEwI1 => { self.sar_ew_1(instr)?; Ok(()) }
            Opcode::SarEw => { self.sar_ew_cl(instr)?; Ok(()) }
            Opcode::SarEwIb => { self.sar_ew_ib(instr)?; Ok(()) }
            Opcode::SarEdI1 => { self.sar_ed_1(instr)?; Ok(()) }
            Opcode::SarEd => { self.sar_ed_cl(instr)?; Ok(()) }
            Opcode::SarEdIb => { self.sar_ed_ib(instr)?; Ok(()) }

            // =========================================================================
            // Data transfer extensions
            // =========================================================================
            Opcode::LeaGwM => { self.lea_gw_m(instr); Ok(()) }
            Opcode::LeaGdM => { self.lea_gd_m(instr); Ok(()) }
            Opcode::XchgEbGb => self.xchg_eb_gb_dispatch(instr),
            Opcode::XchgEwGw => self.xchg_ew_gw_dispatch(instr),
            Opcode::XchgEdGd => { self.xchg_ed_gd(instr); Ok(()) }
            Opcode::XchgErxEax => { self.xchg_eax_rd(instr); Ok(()) }
            Opcode::XchgRxax => { self.xchg_ax_rw(instr); Ok(()) }

            // =========================================================================
            // SETcc Eb - Set byte on condition
            // =========================================================================
            Opcode::SetoEb   => self.seto_eb(instr),
            Opcode::SetnoEb  => self.setno_eb(instr),
            Opcode::SetbEb   => self.setb_eb(instr),
            Opcode::SetnbEb  => self.setnb_eb(instr),
            Opcode::SetzEb   => self.setz_eb(instr),
            Opcode::SetnzEb  => self.setnz_eb(instr),
            Opcode::SetbeEb  => self.setbe_eb(instr),
            Opcode::SetnbeEb => self.setnbe_eb(instr),
            Opcode::SetsEb   => self.sets_eb(instr),
            Opcode::SetnsEb  => self.setns_eb(instr),
            Opcode::SetpEb   => self.setp_eb(instr),
            Opcode::SetnpEb  => self.setnp_eb(instr),
            Opcode::SetlEb   => self.setl_eb(instr),
            Opcode::SetnlEb  => self.setnl_eb(instr),
            Opcode::SetleEb  => self.setle_eb(instr),
            Opcode::SetnleEb => self.setnle_eb(instr),

            Opcode::Cbw => { self.cbw(instr); Ok(()) }
            Opcode::MovsxGdEb => { self.movsx_gd_eb(instr)?; Ok(()) }
            Opcode::MovsxGdEw => { self.movsx_gd_ew(instr)?; Ok(()) }
            Opcode::MovzxGdEb => { data_xfer32::MOVZX_GdEb_unified(self, instr)?; Ok(()) }
            Opcode::MovzxGdEw => { data_xfer32::MOVZX_GdEw_unified(self, instr)?; Ok(()) }
            Opcode::MovzxGwEb => self.movzx_gw_eb(instr),
            Opcode::MovsxGwEb => self.movsx_gw_eb(instr),
            Opcode::Cwd => { self.cwd(instr); Ok(()) }
            Opcode::Cwde => { self.cwde(instr); Ok(()) }
            Opcode::Cdq => { self.cdq(instr); Ok(()) }
            Opcode::Xlat => { self.xlat(instr); Ok(()) }
            Opcode::Lahf => { self.lahf(instr); Ok(()) }
            Opcode::Sahf => { self.sahf(instr); Ok(()) }

            // =========================================================================
            // Data transfer (64-bit) instructions
            // =========================================================================
            Opcode::MovRrxiq => { self.mov_rrxiq(instr); Ok(()) }
            Opcode::MovOp64GdEd => { self.mov64_gd_ed_m(instr); Ok(()) }
            Opcode::MovOp64EdGd => { self.mov64_ed_gd_m(instr); Ok(()) }
            Opcode::MovEqGq => { self.mov_eq_gq_m(instr); Ok(()) }
            Opcode::MovGqEq => { self.mov_gq_eq_m(instr); Ok(()) }
            Opcode::LeaGqM => { self.lea_gq_m(instr); Ok(()) }

            // =========================================================================
            // CMOVcc (Conditional Move) instructions - 32-bit
            // =========================================================================
            Opcode::CmovoGdEd => { self.cmovo_gd_ed_r(instr); Ok(()) }
            Opcode::CmovnoGdEd => { self.cmovno_gd_ed_r(instr); Ok(()) }
            Opcode::CmovbGdEd => { self.cmovb_gd_ed_r(instr); Ok(()) }
            Opcode::CmovnbGdEd => { self.cmovnb_gd_ed_r(instr); Ok(()) }
            Opcode::CmovzGdEd => { self.cmovz_gd_ed_r(instr); Ok(()) }
            Opcode::CmovnzGdEd => { self.cmovnz_gd_ed_r(instr); Ok(()) }
            Opcode::CmovbeGdEd => { self.cmovbe_gd_ed_r(instr); Ok(()) }
            Opcode::CmovnbeGdEd => { self.cmovnbe_gd_ed_r(instr); Ok(()) }
            Opcode::CmovsGdEd => { self.cmovs_gd_ed_r(instr); Ok(()) }
            Opcode::CmovnsGdEd => { self.cmovns_gd_ed_r(instr); Ok(()) }
            Opcode::CmovpGdEd => { self.cmovp_gd_ed_r(instr); Ok(()) }
            Opcode::CmovnpGdEd => { self.cmovnp_gd_ed_r(instr); Ok(()) }
            Opcode::CmovlGdEd => { self.cmovl_gd_ed_r(instr); Ok(()) }
            Opcode::CmovnlGdEd => { self.cmovnl_gd_ed_r(instr); Ok(()) }
            Opcode::CmovleGdEd => { self.cmovle_gd_ed_r(instr); Ok(()) }
            Opcode::CmovnleGdEd => { self.cmovnle_gd_ed_r(instr); Ok(()) }

            // =========================================================================
            // CMOVcc (Conditional Move) instructions - 64-bit
            // =========================================================================
            Opcode::CmovoGqEq => { self.cmovo_gq_eq_r(instr); Ok(()) }
            Opcode::CmovnoGqEq => { self.cmovno_gq_eq_r(instr); Ok(()) }
            Opcode::CmovbGqEq => { self.cmovb_gq_eq_r(instr); Ok(()) }
            Opcode::CmovnbGqEq => { self.cmovnb_gq_eq_r(instr); Ok(()) }
            Opcode::CmovzGqEq => { self.cmovz_gq_eq_r(instr); Ok(()) }
            Opcode::CmovnzGqEq => { self.cmovnz_gq_eq_r(instr); Ok(()) }
            Opcode::CmovbeGqEq => { self.cmovbe_gq_eq_r(instr); Ok(()) }
            Opcode::CmovnbeGqEq => { self.cmovnbe_gq_eq_r(instr); Ok(()) }
            Opcode::CmovsGqEq => { self.cmovs_gq_eq_r(instr); Ok(()) }
            Opcode::CmovnsGqEq => { self.cmovns_gq_eq_r(instr); Ok(()) }
            Opcode::CmovpGqEq => { self.cmovp_gq_eq_r(instr); Ok(()) }
            Opcode::CmovnpGqEq => { self.cmovnp_gq_eq_r(instr); Ok(()) }
            Opcode::CmovlGqEq => { self.cmovl_gq_eq_r(instr); Ok(()) }
            Opcode::CmovnlGqEq => { self.cmovnl_gq_eq_r(instr); Ok(()) }
            Opcode::CmovleGqEq => { self.cmovle_gq_eq_r(instr); Ok(()) }
            Opcode::CmovnleGqEq => { self.cmovnle_gq_eq_r(instr); Ok(()) }

            // =========================================================================
            // BCD (Binary Coded Decimal) instructions
            // =========================================================================
            Opcode::Das => crate::cpu::bcd::DAS(self, instr),

            // =========================================================================
            // x87 FPU instructions — Core (fpu.rs)
            // =========================================================================
            Opcode::Fninit => self.fninit(instr),
            Opcode::Fnclex => self.fnclex(instr),
            Opcode::Fnop => self.fnop(instr),
            Opcode::Fplegacy => self.fplegacy(instr),
            Opcode::Fpuesc | Opcode::Fwait => Ok(()),
            Opcode::Fldcw => self.fldcw(instr),
            Opcode::Fnstcw => self.fnstcw(instr),
            Opcode::Fnstsw => self.fnstsw(instr),
            Opcode::FnstswAx => self.fnstsw_ax(instr),
            Opcode::Fnstenv => self.fnstenv(instr),
            Opcode::Fldenv => self.fldenv(instr),
            Opcode::Fnsave => self.fnsave(instr),
            Opcode::Frstor => self.frstor(instr),

            // =========================================================================
            // x87 FPU — Load/Store (fpu_load_store.rs)
            // =========================================================================
            Opcode::FldSti => self.fld_sti(instr),
            Opcode::FldSingleReal => self.fld_single_real(instr),
            Opcode::FldDoubleReal => self.fld_double_real(instr),
            Opcode::FldExtendedReal => self.fld_extended_real(instr),
            Opcode::FildWordInteger => self.fild_word_integer(instr),
            Opcode::FildDwordInteger => self.fild_dword_integer(instr),
            Opcode::FildQwordInteger => self.fild_qword_integer(instr),
            Opcode::FbldPackedBcd => self.fbld_packed_bcd(instr),
            Opcode::FstSti | Opcode::FstpSti | Opcode::FstpSpecialSti => self.fst_sti(instr),
            Opcode::FstSingleReal => self.fst_single_real(instr),
            Opcode::FstpSingleReal => self.fstp_single_real(instr),
            Opcode::FstDoubleReal => self.fst_double_real(instr),
            Opcode::FstpDoubleReal => self.fstp_double_real(instr),
            Opcode::FstpExtendedReal => self.fstp_extended_real(instr),
            Opcode::FistWordInteger => self.fist_word_integer(instr),
            Opcode::FistpWordInteger => self.fistp_word_integer(instr),
            Opcode::FistDwordInteger => self.fist_dword_integer(instr),
            Opcode::FistpDwordInteger => self.fistp_dword_integer(instr),
            Opcode::FistpQwordInteger => self.fistp_qword_integer(instr),
            Opcode::FbstpPackedBcd => self.fbstp_packed_bcd(instr),
            Opcode::FisttpMw => self.fisttp16(instr),
            Opcode::FisttpMd => self.fisttp32(instr),
            Opcode::FisttpMq => self.fisttp64(instr),

            // =========================================================================
            // x87 FPU — Arithmetic (fpu_arith.rs)
            // =========================================================================
            Opcode::FaddSt0Stj => self.fadd_st0_stj(instr),
            Opcode::FaddStiSt0 => self.fadd_sti_st0(instr),
            Opcode::FaddpStiSt0 => self.faddp_sti_st0(instr),
            Opcode::FaddSingleReal => self.fadd_single_real(instr),
            Opcode::FaddDoubleReal => self.fadd_double_real(instr),
            Opcode::FiaddWordInteger => self.fiadd_word_integer(instr),
            Opcode::FiaddDwordInteger => self.fiadd_dword_integer(instr),
            Opcode::FmulSt0Stj => self.fmul_st0_stj(instr),
            Opcode::FmulStiSt0 => self.fmul_sti_st0(instr),
            Opcode::FmulpStiSt0 => self.fmulp_sti_st0(instr),
            Opcode::FmulSingleReal => self.fmul_single_real(instr),
            Opcode::FmulDoubleReal => self.fmul_double_real(instr),
            Opcode::FimulWordInteger => self.fimul_word_integer(instr),
            Opcode::FimulDwordInteger => self.fimul_dword_integer(instr),
            Opcode::FsubSt0Stj => self.fsub_st0_stj(instr),
            Opcode::FsubrSt0Stj => self.fsubr_st0_stj(instr),
            Opcode::FsubStiSt0 => self.fsub_sti_st0(instr),
            Opcode::FsubpStiSt0 => self.fsubp_sti_st0(instr),
            Opcode::FsubrStiSt0 => self.fsubr_sti_st0(instr),
            Opcode::FsubrpStiSt0 => self.fsubrp_sti_st0(instr),
            Opcode::FsubSingleReal => self.fsub_single_real(instr),
            Opcode::FsubrSingleReal => self.fsubr_single_real(instr),
            Opcode::FsubDoubleReal => self.fsub_double_real(instr),
            Opcode::FsubrDoubleReal => self.fsubr_double_real(instr),
            Opcode::FisubWordInteger => self.fisub_word_integer(instr),
            Opcode::FisubrWordInteger => self.fisubr_word_integer(instr),
            Opcode::FisubDwordInteger => self.fisub_dword_integer(instr),
            Opcode::FisubrDwordInteger => self.fisubr_dword_integer(instr),
            Opcode::FdivSt0Stj => self.fdiv_st0_stj(instr),
            Opcode::FdivrSt0Stj => self.fdivr_st0_stj(instr),
            Opcode::FdivStiSt0 => self.fdiv_sti_st0(instr),
            Opcode::FdivpStiSt0 => self.fdivp_sti_st0(instr),
            Opcode::FdivrStiSt0 => self.fdivr_sti_st0(instr),
            Opcode::FdivrpStiSt0 => self.fdivrp_sti_st0(instr),
            Opcode::FdivSingleReal => self.fdiv_single_real(instr),
            Opcode::FdivrSingleReal => self.fdivr_single_real(instr),
            Opcode::FdivDoubleReal => self.fdiv_double_real(instr),
            Opcode::FdivrDoubleReal => self.fdivr_double_real(instr),
            Opcode::FidivWordInteger => self.fidiv_word_integer(instr),
            Opcode::FidivrWordInteger => self.fidivr_word_integer(instr),
            Opcode::FidivDwordInteger => self.fidiv_dword_integer(instr),
            Opcode::FidivrDwordInteger => self.fidivrp_dword_integer(instr),
            Opcode::Fsqrt => self.fsqrt(instr),
            Opcode::Frndint => self.frndint(instr),

            // =========================================================================
            // x87 FPU — Compare (fpu_compare.rs)
            // =========================================================================
            Opcode::FcomSti => self.fcom_sti(instr),
            Opcode::FcompSti => self.fcomp_sti(instr),
            Opcode::FucomSti => self.fucom_sti(instr),
            Opcode::FucompSti => self.fucomp_sti(instr),
            Opcode::FcomSingleReal => self.fcom_single_real(instr),
            Opcode::FcompSingleReal => self.fcomp_single_real(instr),
            Opcode::FcomDoubleReal => self.fcom_double_real(instr),
            Opcode::FcompDoubleReal => self.fcomp_double_real(instr),
            Opcode::FicomWordInteger => self.ficom_word_integer(instr),
            Opcode::FicompWordInteger => self.ficomp_word_integer(instr),
            Opcode::FicomDwordInteger => self.ficom_dword_integer(instr),
            Opcode::FicompDwordInteger => self.ficomp_dword_integer(instr),
            Opcode::Fcompp | Opcode::Fucompp => self.fcompp(instr),
            Opcode::FcomiSt0Stj => self.fcomi_st0_stj(instr),
            Opcode::FcomipSt0Stj => self.fcomip_st0_stj(instr),
            Opcode::FucomiSt0Stj => self.fucomi_st0_stj(instr),
            Opcode::FucomipSt0Stj => self.fucomip_st0_stj(instr),
            Opcode::Ftst => self.ftst(instr),
            Opcode::Fxam => self.fxam(instr),

            // =========================================================================
            // x87 FPU — Misc (fpu_misc.rs)
            // =========================================================================
            Opcode::FxchSti => self.fxch_sti(instr),
            Opcode::Fchs => self.fchs(instr),
            Opcode::Fabs => self.fabs_(instr),
            Opcode::Fdecstp => self.fdecstp(instr),
            Opcode::Fincstp => self.fincstp(instr),
            Opcode::FfreeSti => self.ffree_sti(instr),
            Opcode::FfreepSti => self.ffreep_sti(instr),

            // =========================================================================
            // x87 FPU — Constants (fpu_const.rs)
            // =========================================================================
            Opcode::FLD1 => self.fld1(instr),
            Opcode::Fldl2t => self.fldl2t(instr),
            Opcode::Fldl2e => self.fldl2e(instr),
            Opcode::Fldpi => self.fldpi(instr),
            Opcode::Fldlg2 => self.fldlg2(instr),
            Opcode::Fldln2 => self.fldln2(instr),
            Opcode::Fldz => self.fldz(instr),

            // =========================================================================
            // x87 FPU — Conditional Move (fpu_cmov.rs)
            // =========================================================================
            Opcode::FcmovbSt0Stj => self.fcmovb_st0_stj(instr),
            Opcode::FcmoveSt0Stj => self.fcmove_st0_stj(instr),
            Opcode::FcmovbeSt0Stj => self.fcmovbe_st0_stj(instr),
            Opcode::FcmovuSt0Stj => self.fcmovu_st0_stj(instr),
            Opcode::FcmovnbSt0Stj => self.fcmovnb_st0_stj(instr),
            Opcode::FcmovneSt0Stj => self.fcmovne_st0_stj(instr),
            Opcode::FcmovnbeSt0Stj => self.fcmovnbe_st0_stj(instr),
            Opcode::FcmovnuSt0Stj => self.fcmovnu_st0_stj(instr),

            // =========================================================================
            // x87 FPU — Transcendentals (fpu_trans.rs)
            // =========================================================================
            Opcode::Fscale => self.fscale(instr),
            Opcode::Fxtract => self.fxtract(instr),
            Opcode::Fprem => self.fprem(instr),
            Opcode::FPREM1 => self.fprem1(instr),
            Opcode::F2XM1 => self.f2xm1(instr),
            Opcode::FYL2X => self.fyl2x(instr),
            Opcode::FYL2XP1 => self.fyl2xp1(instr),
            Opcode::Fptan => self.fptan(instr),
            Opcode::Fpatan => self.fpatan(instr),
            Opcode::Fsin => self.fsin(instr),
            Opcode::Fcos => self.fcos(instr),
            Opcode::Fsincos => self.fsincos(instr),

            _ => {
                tracing::error!("Unimplemented opcode: {:?}", instr.get_ia_opcode());
                Err(crate::cpu::CpuError::UnimplementedOpcode {
                    opcode: format!("{:?}", instr.get_ia_opcode()),
                })
            }
        }
    }
}
