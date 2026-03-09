//! 32-bit logical and comparison instructions for x86 CPU emulation
//!
//! Based on Bochs logical32.cc

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Flag update helpers
    // =========================================================================

    /// Update flags for 32-bit logical operations
    pub fn set_flags_oszapc_logic_32(&mut self, result: u32) {
        let sf = (result & 0x80000000) != 0;
        let zf = result == 0;
        let pf = (result as u8).count_ones() % 2 == 0;

        self.eflags.remove(EFlags::LOGIC_MASK);

        if pf {
            self.eflags.insert(EFlags::PF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
    }

    /// Update flags for 32-bit subtraction
    pub fn set_flags_oszapc_sub_32(&mut self, op1: u32, op2: u32, result: u32) {
        let cf = op1 < op2;
        let zf = result == 0;
        let sf = (result & 0x80000000) != 0;
        let of = ((op1 ^ op2) & (op1 ^ result) & 0x80000000) != 0;
        let af = ((op1 ^ op2 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones() % 2 == 0;

        self.eflags.remove(EFlags::OSZAPC);

        if cf {
            self.eflags.insert(EFlags::CF);
        }
        if pf {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }
    }

    // =========================================================================
    // ZERO_IDIOM - XOR register with itself (optimization: set to 0)
    // =========================================================================

    /// ZERO_IDIOM_GdR: XOR r32, r32 (zero idiom - set register to 0)
    /// Matches BX_CPU_C::ZERO_IDIOM_GdR
    pub fn zero_idiom_gd_r(&mut self, instr: &Instruction) {
        let dst = instr.operands.dst as usize;
        self.set_gpr32(dst, 0);
        self.set_flags_oszapc_logic_32(0);
    }

    // =========================================================================
    // CMP instructions
    // =========================================================================

    /// CMP r32, r32
    pub fn cmp_gd_ed_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = self.get_gpr32(src);
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_32(op1, op2, result);
        tracing::trace!(
            "CMP r32, r32: {:#010x} - {:#010x} = {:#010x}",
            op1,
            op2,
            result
        );
    }

    /// CMP EAX, imm32
    pub fn cmp_eax_id(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr32(0); // EAX
        let op2 = instr.id();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_32(op1, op2, result);
        // Trace '%' comparisons in kernel space
        if op2 == 0x25 && op1 == 0x25 && self.rip() > 0xC0000000 {
            let zf = (self.eflags.bits() >> 6) & 1;
            tracing::warn!(
                "CMP EAX=0x25, Id=0x25 at RIP={:#x} ZF={} eflags={:#x} icount={}",
                self.rip(),
                zf,
                self.eflags.bits(),
                self.icount
            );
        }
        tracing::trace!("CMP EAX, imm32: {:#010x} - {:#010x}", op1, op2);
    }

    /// CMP r/m32, imm32
    pub fn cmp_ed_id_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = instr.id();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_32(op1, op2, result);
        // vsprintf trace removed (using printk breakpoint instead)
        tracing::trace!("CMP r32, imm32: {:#010x} - {:#010x}", op1, op2);
    }

    // =========================================================================
    // TEST instructions
    // =========================================================================

    /// TEST r32, r32
    pub fn test_ed_gd_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = self.get_gpr32(src);
        let result = op1 & op2;
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!(
            "TEST r32, r32: {:#010x} & {:#010x} = {:#010x}",
            op1,
            op2,
            result
        );
    }

    /// TEST EAX, imm32
    pub fn test_eax_id(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr32(0); // EAX
        let op2 = instr.id();
        let result = op1 & op2;
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("TEST EAX, imm32: {:#010x} & {:#010x}", op1, op2);
    }

    /// TEST r/m32, imm32
    pub fn test_ed_id_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = instr.id();
        let result = op1 & op2;
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("TEST r32, imm32: {:#010x} & {:#010x}", op1, op2);
    }

    // =========================================================================
    // AND instructions
    // =========================================================================

    /// AND_EdGdR: AND r/m32, r32 (register form, store-direction)
    /// Opcode 0x21: decoder swaps: [0]=rm=DEST, [1]=nnn=SOURCE
    pub fn and_ed_gd_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr32(instr.operands.dst as usize); // rm = destination
        let op2 = self.get_gpr32(instr.operands.src1 as usize); // nnn = source
        let result = op1 & op2;
        self.set_gpr32(instr.operands.dst as usize, result);
        self.set_flags_oszapc_logic_32(result);
    }

    /// AND r32, r32 (load-direction, opcode 0x23)
    pub fn and_gd_ed_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = self.get_gpr32(src);
        let result = op1 & op2;
        self.set_gpr32(dst, result);
        self.set_flags_oszapc_logic_32(result);
    }

    /// AND EAX, imm32
    pub fn and_eax_id(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr32(0);
        let op2 = instr.id();
        let result = op1 & op2;
        self.set_gpr32(0, result);
        self.set_flags_oszapc_logic_32(result);
    }

    /// AND r/m32, imm32
    pub fn and_ed_id_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = instr.id();
        let result = op1 & op2;
        self.set_gpr32(dst, result);
        self.set_flags_oszapc_logic_32(result);
    }

    // =========================================================================
    // XOR instructions
    // =========================================================================

    /// XOR_GdEdR: XOR r32, r32 (register form)
    /// Matches BX_CPU_C::XOR_GdEdR
    pub fn xor_gd_ed_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = self.get_gpr32(src);
        let result = op1 ^ op2;
        self.set_gpr32(dst, result);
        self.set_flags_oszapc_logic_32(result);
    }

    /// XOR_EdGdR: XOR r/m32, r32 (register form, store-direction)
    /// Opcode 0x31: decoder swaps: [0]=rm=DEST, [1]=nnn=SOURCE
    pub fn xor_ed_gd_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr32(instr.operands.dst as usize); // rm = destination
        let op2 = self.get_gpr32(instr.operands.src1 as usize); // nnn = source
        let result = op1 ^ op2;
        self.set_gpr32(instr.operands.dst as usize, result);
        self.set_flags_oszapc_logic_32(result);
    }

    /// XOR_EdIdR: XOR r/m32, imm32 (register form)
    /// Matches BX_CPU_C::XOR_EdIdR
    pub fn xor_ed_id_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = instr.id();
        let result = op1 ^ op2;
        self.set_gpr32(dst, result);
        self.set_flags_oszapc_logic_32(result);
    }

    // =========================================================================
    // OR instructions
    // =========================================================================

    /// OR_EdGdR: OR r/m32, r32 (register form, store-direction)
    /// Opcode 0x09: decoder swaps: [0]=rm=DEST, [1]=nnn=SOURCE
    pub fn or_ed_gd_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr32(instr.operands.dst as usize); // rm = destination
        let op2 = self.get_gpr32(instr.operands.src1 as usize); // nnn = source
        let result = op1 | op2;
        self.set_gpr32(instr.operands.dst as usize, result);
        self.set_flags_oszapc_logic_32(result);
    }

    /// OR r32, r32 (load-direction, opcode 0x0B)
    pub fn or_gd_ed_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = self.get_gpr32(src);
        let result = op1 | op2;
        self.set_gpr32(dst, result);
        self.set_flags_oszapc_logic_32(result);
    }

    /// OR EAX, imm32
    pub fn or_eax_id(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr32(0);
        let op2 = instr.id();
        let result = op1 | op2;
        self.set_gpr32(0, result);
        self.set_flags_oszapc_logic_32(result);
    }

    /// OR_EdIdR: OR r/m32, imm32 (register form)
    /// Matches BX_CPU_C::OR_EdIdR
    pub fn or_ed_id_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = instr.id();
        let result = op1 | op2;
        self.set_gpr32(dst, result);
        self.set_flags_oszapc_logic_32(result);
    }

    // =========================================================================
    // NOT instructions
    // =========================================================================

    /// NOT r32
    pub fn not_ed_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        self.set_gpr32(dst, !op1);
    }

    // =========================================================================
    // INC/DEC instructions
    // =========================================================================

    /// INC r32
    pub fn inc_ed_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let result = op1.wrapping_add(1);
        self.set_gpr32(dst, result);

        let zf = result == 0;
        let sf = (result & 0x80000000) != 0;
        let of = result == 0x80000000;
        let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones() % 2 == 0;

        const OSZAP: EFlags = EFlags::PF
            .union(EFlags::AF)
            .union(EFlags::ZF)
            .union(EFlags::SF)
            .union(EFlags::OF);
        self.eflags.remove(OSZAP);
        if pf {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }

        tracing::trace!("INC r32: {:#010x} + 1 = {:#010x}", op1, result);
    }

    /// DEC r32
    pub fn dec_ed_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let result = op1.wrapping_sub(1);
        self.set_gpr32(dst, result);

        let zf = result == 0;
        let sf = (result & 0x80000000) != 0;
        let of = op1 == 0x80000000;
        let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones() % 2 == 0;

        const OSZAP: EFlags = EFlags::PF
            .union(EFlags::AF)
            .union(EFlags::ZF)
            .union(EFlags::SF)
            .union(EFlags::OF);
        self.eflags.remove(OSZAP);
        if pf {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }

        tracing::trace!("DEC r32: {:#010x} - 1 = {:#010x}", op1, result);
    }

    /// INC r/m32 (memory form) — matches Bochs INC_EdM
    pub fn inc_ed_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_rmw_dword(seg, eaddr)?;
        let result = op1.wrapping_add(1);
        self.write_rmw_linear_dword(result);

        let zf = result == 0;
        let sf = (result & 0x80000000) != 0;
        let of = result == 0x80000000;
        let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones() % 2 == 0;

        const OSZAP: EFlags = EFlags::PF
            .union(EFlags::AF)
            .union(EFlags::ZF)
            .union(EFlags::SF)
            .union(EFlags::OF);
        self.eflags.remove(OSZAP);
        if pf {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }

        Ok(())
    }

    /// DEC r/m32 (memory form) — matches Bochs DEC_EdM
    pub fn dec_ed_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_rmw_dword(seg, eaddr)?;
        let result = op1.wrapping_sub(1);
        self.write_rmw_linear_dword(result);

        let zf = result == 0;
        let sf = (result & 0x80000000) != 0;
        let of = op1 == 0x80000000;
        let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones() % 2 == 0;

        const OSZAP: EFlags = EFlags::PF
            .union(EFlags::AF)
            .union(EFlags::ZF)
            .union(EFlags::SF)
            .union(EFlags::OF);
        self.eflags.remove(OSZAP);
        if pf {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }

        Ok(())
    }

    /// INC r/m32 — unified dispatch
    pub fn inc_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.inc_ed_r(instr);
            Ok(())
        } else {
            self.inc_ed_m(instr)
        }
    }

    /// DEC r/m32 — unified dispatch
    pub fn dec_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.dec_ed_r(instr);
            Ok(())
        } else {
            self.dec_ed_m(instr)
        }
    }

    // =========================================================================
    // Helper functions for memory operations
    // =========================================================================

    // resolve_addr32 is defined in logical8.rs to avoid duplicate definitions

    // get_laddr32_seg is defined in logical8.rs to avoid duplicate definitions

    /// Write-back phase of a read-modify-write dword access.
    /// Uses address_xlation populated by read_rmw_virtual_dword.
    /// Bochs: write_RMW_linear_dword (access2.cc:802)
    #[inline]
    pub fn write_rmw_linear_dword(&mut self, val: u32) {
        if self.address_xlation.pages > 2 {
            // Host pointer cached from TLB hit — direct write
            unsafe { (self.address_xlation.pages as *mut u32).write_unaligned(val) };
        } else if self.address_xlation.pages == 1 {
            // Single-page physical write
            self.mem_write_dword(self.address_xlation.paddress1, val);
        } else {
            // Cross-page (pages == 2): split write (little-endian)
            let bytes = val.to_le_bytes();
            let len1 = self.address_xlation.len1 as usize;
            for i in 0..len1 {
                self.mem_write_byte(self.address_xlation.paddress1 + i as u64, bytes[i]);
            }
            let len2 = self.address_xlation.len2 as usize;
            for i in 0..len2 {
                self.mem_write_byte(self.address_xlation.paddress2 + i as u64, bytes[len1 + i]);
            }
        }
    }

    // =========================================================================
    // Memory-form instructions
    // =========================================================================

    /// XOR_EdGdM: XOR r/m32, r32 (memory form)
    /// Decoder swaps: src() = [1] = nnn = SOURCE register
    pub fn xor_ed_gd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.v_read_rmw_dword(seg, eaddr)?;
        let src_reg = instr.src() as usize;
        let op2_32 = self.get_gpr32(src_reg);
        let result = op1_32 ^ op2_32;

        self.write_rmw_linear_dword(result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!(
            "XOR32 mem: [{:?}:{:#x}] = {:#010x} ^ {:#010x} = {:#010x}",
            seg,
            eaddr,
            op1_32,
            op2_32,
            result
        );
        Ok(())
    }

    /// XOR_GdEdM: XOR r32, r/m32 (memory form)
    /// Matches BX_CPU_C::XOR_GdEdM
    pub fn xor_gd_ed_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_32 = self.v_read_dword(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let op1_32 = self.get_gpr32(dst_reg);
        let result = op1_32 ^ op2_32;

        self.set_gpr32(dst_reg, result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!(
            "XOR32 mem: reg{} = {:#010x} ^ {:#010x} = {:#010x}",
            dst_reg,
            op1_32,
            op2_32,
            result
        );
        Ok(())
    }

    /// XOR_EdIdM: XOR r/m32, imm32 (memory form)
    /// Matches BX_CPU_C::XOR_EdIdM
    pub fn xor_ed_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.v_read_rmw_dword(seg, eaddr)?;
        let op2_32 = instr.id();
        let result = op1_32 ^ op2_32;

        self.write_rmw_linear_dword(result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!(
            "XOR32 mem: [{:?}:{:#x}] = {:#010x} ^ {:#010x} = {:#010x}",
            seg,
            eaddr,
            op1_32,
            op2_32,
            result
        );
        Ok(())
    }

    /// OR_EdGdM: OR r/m32, r32 (memory form)
    /// Decoder swaps: src() = [1] = nnn = SOURCE register
    pub fn or_ed_gd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.v_read_rmw_dword(seg, eaddr)?;
        let src_reg = instr.src() as usize;
        let op2_32 = self.get_gpr32(src_reg);
        let result = op1_32 | op2_32;

        self.write_rmw_linear_dword(result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!(
            "OR32 mem: [{:?}:{:#x}] = {:#010x} | {:#010x} = {:#010x}",
            seg,
            eaddr,
            op1_32,
            op2_32,
            result
        );
        Ok(())
    }

    /// OR_GdEdM: OR r32, r/m32 (memory form)
    /// Matches BX_CPU_C::OR_GdEdM
    pub fn or_gd_ed_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_32 = self.v_read_dword(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let op1_32 = self.get_gpr32(dst_reg);
        let result = op1_32 | op2_32;

        self.set_gpr32(dst_reg, result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!(
            "OR32 mem: reg{} = {:#010x} | {:#010x} = {:#010x}",
            dst_reg,
            op1_32,
            op2_32,
            result
        );
        Ok(())
    }

    /// OR_EdIdM: OR r/m32, imm32 (memory form)
    /// Matches BX_CPU_C::OR_EdIdM
    pub fn or_ed_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.v_read_rmw_dword(seg, eaddr)?;
        let op2_32 = instr.id();
        let result = op1_32 | op2_32;

        self.write_rmw_linear_dword(result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!(
            "OR32 mem: [{:?}:{:#x}] = {:#010x} | {:#010x} = {:#010x}",
            seg,
            eaddr,
            op1_32,
            op2_32,
            result
        );
        Ok(())
    }

    /// AND_EdGdM: AND r/m32, r32 (memory form)
    /// Decoder swaps: src() = [1] = nnn = SOURCE register
    pub fn and_ed_gd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.v_read_rmw_dword(seg, eaddr)?;
        let src_reg = instr.src() as usize;
        let op2_32 = self.get_gpr32(src_reg);
        let result = op1_32 & op2_32;

        self.write_rmw_linear_dword(result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!(
            "AND32 mem: [{:?}:{:#x}] = {:#010x} & {:#010x} = {:#010x}",
            seg,
            eaddr,
            op1_32,
            op2_32,
            result
        );
        Ok(())
    }

    /// AND_GdEdM: AND r32, r/m32 (memory form)
    /// Matches BX_CPU_C::AND_GdEdM
    pub fn and_gd_ed_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_32 = self.v_read_dword(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let op1_32 = self.get_gpr32(dst_reg);
        let result = op1_32 & op2_32;

        self.set_gpr32(dst_reg, result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!(
            "AND32 mem: reg{} = {:#010x} & {:#010x} = {:#010x}",
            dst_reg,
            op1_32,
            op2_32,
            result
        );
        Ok(())
    }

    /// AND_EdIdM: AND r/m32, imm32 (memory form)
    /// Matches BX_CPU_C::AND_EdIdM
    pub fn and_ed_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.v_read_rmw_dword(seg, eaddr)?;
        let op2_32 = instr.id();
        let result = op1_32 & op2_32;

        self.write_rmw_linear_dword(result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!(
            "AND32 mem: [{:?}:{:#x}] = {:#010x} & {:#010x} = {:#010x}",
            seg,
            eaddr,
            op1_32,
            op2_32,
            result
        );
        Ok(())
    }

    /// NOT_EdM: NOT r/m32 (memory form)
    /// Matches BX_CPU_C::NOT_EdM
    pub fn not_ed_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.v_read_rmw_dword(seg, eaddr)?;
        let result = !op1_32;

        self.write_rmw_linear_dword(result);
        tracing::trace!(
            "NOT32 mem: [{:?}:{:#x}] = !{:#010x} = {:#010x}",
            seg,
            eaddr,
            op1_32,
            result
        );
        Ok(())
    }

    /// TEST_EdGdM: TEST r/m32, r32 (memory form)
    /// Opcode 0x85 is NOT store-direction, so dst() = nnn = register operand
    pub fn test_ed_gd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.v_read_dword(seg, eaddr)?;
        let src_reg = instr.dst() as usize;
        let op2_32 = self.get_gpr32(src_reg);
        let result = op1_32 & op2_32;

        self.set_flags_oszapc_logic_32(result);
        tracing::trace!(
            "TEST32 mem: [{:?}:{:#x}] & reg{} = {:#010x} & {:#010x} = {:#010x}",
            seg,
            eaddr,
            src_reg,
            op1_32,
            op2_32,
            result
        );
        Ok(())
    }

    /// TEST_EdIdM: TEST r/m32, imm32 (memory form)
    /// Matches BX_CPU_C::TEST_EdIdM
    pub fn test_ed_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.v_read_dword(seg, eaddr)?;
        let op2_32 = instr.id();
        let result = op1_32 & op2_32;

        self.set_flags_oszapc_logic_32(result);
        tracing::trace!(
            "TEST32 mem: [{:?}:{:#x}] & {:#010x} = {:#010x} & {:#010x} = {:#010x}",
            seg,
            eaddr,
            op2_32,
            op1_32,
            op2_32,
            result
        );
        Ok(())
    }

    /// CMP_GdEdM: CMP r32, r/m32 (memory form)
    /// Matches BX_CPU_C::CMP_GdEdM
    pub fn cmp_gd_ed_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_32 = self.v_read_dword(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let op1_32 = self.get_gpr32(dst_reg);
        let result = op1_32.wrapping_sub(op2_32);
        self.set_flags_oszapc_sub_32(op1_32, op2_32, result);
        Ok(())
    }

    /// CMP_EdGdR: CMP r/m32, r32 (register form)
    /// Matches BX_CPU_C::CMP_EdGdR
    pub fn cmp_ed_gd_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = self.get_gpr32(src);
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_32(op1, op2, result);
    }

    /// CMP_EdGdM: CMP r/m32, r32 (memory form)
    /// Matches BX_CPU_C::CMP_EdGdM
    pub fn cmp_ed_gd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.v_read_rmw_dword(seg, eaddr)?;
        let src_reg = instr.src() as usize;
        let op2_32 = self.get_gpr32(src_reg);
        let result = op1_32.wrapping_sub(op2_32);
        self.set_flags_oszapc_sub_32(op1_32, op2_32, result);
        Ok(())
    }

    /// CMP_EdIdM: CMP r/m32, imm32 (memory form)
    /// Matches BX_CPU_C::CMP_EdIdM
    pub fn cmp_ed_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.v_read_dword(seg, eaddr)?;
        let op2_32 = instr.id();
        let result = op1_32.wrapping_sub(op2_32);
        self.set_flags_oszapc_sub_32(op1_32, op2_32, result);
        Ok(())
    }

    // =========================================================================
    // Unified handlers: dispatch R/M based on instr.mod_c0()
    // =========================================================================

    /// XOR r/m32, r32 - unified (EdGd: memory is read-modify-write, register is commutative)
    pub fn xor_ed_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xor_ed_gd_r(instr);
            Ok(())
        } else {
            self.xor_ed_gd_m(instr)
        }
    }

    /// XOR r32, r/m32 - unified (GdEd: register dest, memory is read-only source)
    pub fn xor_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xor_gd_ed_r(instr);
            Ok(())
        } else {
            self.xor_gd_ed_m(instr)
        }
    }

    /// XOR r/m32, imm32 - unified
    pub fn xor_ed_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xor_ed_id_r(instr);
            Ok(())
        } else {
            self.xor_ed_id_m(instr)
        }
    }

    /// AND r/m32, r32 - unified (EdGd: memory is read-modify-write, register is commutative)
    pub fn and_ed_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.and_ed_gd_r(instr);
            Ok(())
        } else {
            self.and_ed_gd_m(instr)
        }
    }

    /// AND r32, r/m32 - unified (GdEd: register dest, memory is read-only source)
    pub fn and_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.and_gd_ed_r(instr);
            Ok(())
        } else {
            self.and_gd_ed_m(instr)
        }
    }

    /// AND r/m32, imm32 - unified (handles both AndEdId and AndEdsIb opcodes)
    pub fn and_ed_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.and_ed_id_r(instr);
            Ok(())
        } else {
            self.and_ed_id_m(instr)
        }
    }

    /// OR r/m32, r32 - unified (EdGd: memory is read-modify-write, register is commutative)
    pub fn or_ed_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.or_ed_gd_r(instr);
            Ok(())
        } else {
            self.or_ed_gd_m(instr)
        }
    }

    /// OR r32, r/m32 - unified (GdEd: register dest, memory is read-only source)
    pub fn or_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.or_gd_ed_r(instr);
            Ok(())
        } else {
            self.or_gd_ed_m(instr)
        }
    }

    /// OR r/m32, imm32 - unified
    pub fn or_ed_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.or_ed_id_r(instr);
            Ok(())
        } else {
            self.or_ed_id_m(instr)
        }
    }

    /// NOT r/m32 - unified
    pub fn not_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.not_ed_r(instr);
            Ok(())
        } else {
            self.not_ed_m(instr)
        }
    }

    /// TEST r/m32, r32 - unified
    pub fn test_ed_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.test_ed_gd_r(instr);
            Ok(())
        } else {
            self.test_ed_gd_m(instr)
        }
    }

    /// TEST r/m32, imm32 - unified
    pub fn test_ed_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.test_ed_id_r(instr);
            Ok(())
        } else {
            self.test_ed_id_m(instr)
        }
    }

    /// CMP r32, r/m32 - unified (GdEd: register dest compares with reg or memory)
    pub fn cmp_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.cmp_gd_ed_r(instr);
            Ok(())
        } else {
            self.cmp_gd_ed_m(instr)
        }
    }

    /// CMP r/m32, r32 - unified (EdGd: memory or register compared with register)
    pub fn cmp_ed_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.cmp_ed_gd_r(instr);
            Ok(())
        } else {
            self.cmp_ed_gd_m(instr)
        }
    }

    /// CMP r/m32, imm32 - unified (handles both CmpEdId and CmpEdsIb opcodes)
    pub fn cmp_ed_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.cmp_ed_id_r(instr);
            Ok(())
        } else {
            self.cmp_ed_id_m(instr)
        }
    }

    // =========================================================================
    // Bit Test instructions (BT, BTS, BTR, BTC) — 32-bit
    // Based on Bochs cpu/bit.cc
    // =========================================================================

    /// BT r/m32, imm8 — Bit Test (0F BA /4 ib)
    /// Tests bit `imm8 % 32` of r/m32, sets CF to that bit.
    pub fn bt_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = if instr.mod_c0() {
            self.get_gpr32(instr.dst() as usize)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, eaddr)?
        };
        let bit = (instr.ib() & 0x1F) as u32;
        let cf = (op1 >> bit) & 1;
        self.eflags.set(EFlags::CF, cf != 0);
        Ok(())
    }

    /// BTS r/m32, imm8 — Bit Test and Set (0F BA /5 ib)
    /// Tests bit, sets CF, then sets the bit to 1.
    pub fn bts_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let bit = (instr.ib() & 0x1F) as u32;
        if instr.mod_c0() {
            let dst = instr.dst() as usize;
            let op1 = self.get_gpr32(dst);
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.set_gpr32(dst, op1 | (1 << bit));
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.v_read_rmw_dword(seg, eaddr)?;
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.write_rmw_linear_dword(op1 | (1 << bit));
        }
        Ok(())
    }

    /// BTR r/m32, imm8 — Bit Test and Reset (0F BA /6 ib)
    /// Tests bit, sets CF, then clears the bit to 0.
    pub fn btr_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let bit = (instr.ib() & 0x1F) as u32;
        if instr.mod_c0() {
            let dst = instr.dst() as usize;
            let op1 = self.get_gpr32(dst);
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.set_gpr32(dst, op1 & !(1 << bit));
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.v_read_rmw_dword(seg, eaddr)?;
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.write_rmw_linear_dword(op1 & !(1 << bit));
        }
        Ok(())
    }

    /// BTC r/m32, imm8 — Bit Test and Complement (0F BA /7 ib)
    /// Tests bit, sets CF, then complements (toggles) the bit.
    pub fn btc_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let bit = (instr.ib() & 0x1F) as u32;
        if instr.mod_c0() {
            let dst = instr.dst() as usize;
            let op1 = self.get_gpr32(dst);
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.set_gpr32(dst, op1 ^ (1 << bit));
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.v_read_rmw_dword(seg, eaddr)?;
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.write_rmw_linear_dword(op1 ^ (1 << bit));
        }
        Ok(())
    }

    /// BT r/m32, r32 — Bit Test (0F A3)
    /// Tests bit specified by r32 in r/m32, sets CF.
    pub fn bt_ed_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.get_gpr32(instr.src() as usize);
        let op1 = if instr.mod_c0() {
            self.get_gpr32(instr.dst() as usize)
        } else {
            let eaddr = self.resolve_addr(instr);
            // For memory form, bit offset is full op2 (not masked to 31).
            // Bochs: displacement = ((s32)op2 >> 5) << 2
            let displacement = ((op2 as i32) >> 5) << 2;
            let addr = (eaddr as i32).wrapping_add(displacement) as u32;
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, addr)?
        };
        let bit = op2 & 0x1F;
        let cf = (op1 >> bit) & 1;
        self.eflags.set(EFlags::CF, cf != 0);
        Ok(())
    }

    /// BTS r/m32, r32 — Bit Test and Set (0F AB)
    pub fn bts_ed_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.get_gpr32(instr.src() as usize);
        let bit = op2 & 0x1F;
        if instr.mod_c0() {
            let dst = instr.dst() as usize;
            let op1 = self.get_gpr32(dst);
            {
                let cf = (op1 >> bit) & 1;
                self.eflags.set(EFlags::CF, cf != 0);
            }
            self.set_gpr32(dst, op1 | (1 << bit));
        } else {
            let eaddr = self.resolve_addr(instr);
            let displacement = ((op2 as i32) >> 5) << 2;
            let addr = (eaddr as i32).wrapping_add(displacement) as u32;
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.v_read_rmw_dword(seg, addr)?;
            {
                let cf = (op1 >> bit) & 1;
                self.eflags.set(EFlags::CF, cf != 0);
            }
            self.write_rmw_linear_dword(op1 | (1 << bit));
        }
        Ok(())
    }

    /// BTR r/m32, r32 — Bit Test and Reset (0F B3)
    pub fn btr_ed_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.get_gpr32(instr.src() as usize);
        let bit = op2 & 0x1F;
        if instr.mod_c0() {
            let dst = instr.dst() as usize;
            let op1 = self.get_gpr32(dst);
            {
                let cf = (op1 >> bit) & 1;
                self.eflags.set(EFlags::CF, cf != 0);
            }
            self.set_gpr32(dst, op1 & !(1 << bit));
        } else {
            let eaddr = self.resolve_addr(instr);
            let displacement = ((op2 as i32) >> 5) << 2;
            let addr = (eaddr as i32).wrapping_add(displacement) as u32;
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.v_read_rmw_dword(seg, addr)?;
            {
                let cf = (op1 >> bit) & 1;
                self.eflags.set(EFlags::CF, cf != 0);
            }
            self.write_rmw_linear_dword(op1 & !(1 << bit));
        }
        Ok(())
    }

    /// BTC r/m32, r32 — Bit Test and Complement (0F BB)
    pub fn btc_ed_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.get_gpr32(instr.src() as usize);
        let bit = op2 & 0x1F;
        if instr.mod_c0() {
            let dst = instr.dst() as usize;
            let op1 = self.get_gpr32(dst);
            {
                let cf = (op1 >> bit) & 1;
                self.eflags.set(EFlags::CF, cf != 0);
            }
            self.set_gpr32(dst, op1 ^ (1 << bit));
        } else {
            let eaddr = self.resolve_addr(instr);
            let displacement = ((op2 as i32) >> 5) << 2;
            let addr = (eaddr as i32).wrapping_add(displacement) as u32;
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.v_read_rmw_dword(seg, addr)?;
            {
                let cf = (op1 >> bit) & 1;
                self.eflags.set(EFlags::CF, cf != 0);
            }
            self.write_rmw_linear_dword(op1 ^ (1 << bit));
        }
        Ok(())
    }
}
