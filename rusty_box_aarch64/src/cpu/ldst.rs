//! Loads and stores (Arm ARM C4.1.4 "Loads and Stores").
//!
//! Ordering of side effects follows the pseudocode: the address is formed
//! from the base before the access, the access happens (and may fault, in
//! which case nothing is written back), then the base is written back.
//! Where the architecture leaves a combination CONSTRAINED UNPREDICTABLE
//! (writeback into the transfer register, a load pair into one register),
//! the choice made is the one QEMU makes, so the reference and this
//! interpreter agree: loaded registers are written first, the base last.

use super::dp::extend_reg;
use super::exec::{bit, bits, sext, ExecResult, Flow};
use super::mmu::{DataAccess, ElementPair, Fit};
use super::{Cpu, ExclusiveMonitor};
use crate::bus::PhysBus;

/// How a single-register load/store forms its address: the three offset
/// forms of the Arm ARM's "Load/store register" classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SingleForm {
    /// Load/store register (unsigned immediate): `imm12` scaled by the size.
    UnsignedImmediate,
    /// The `imm9` classes, told apart by bits 11:10: unscaled (00),
    /// post-indexed (01), unprivileged (10), pre-indexed (11).
    Immediate9,
    /// Load/store register (register offset).
    RegisterOffset,
}

/// Where a single-register access goes and what happens to the base after.
#[derive(Debug, Clone, Copy)]
struct Addressing {
    address: u64,
    writeback: Option<u64>,
}

impl Cpu {
    /// The base register value for an access, with the SP alignment check
    /// applied when the base is SP.
    pub(super) fn base_register(&self, rn: u32) -> Result<u64, super::exec::Fault> {
        let base = self.x_or_sp(rn);
        self.check_sp_alignment(rn == 31, base)?;
        Ok(base)
    }

    /// LDR/STR and friends, every single-register form except the literal:
    /// `size 111 V 0x opc ...` with the offset form `form`.
    pub(crate) fn exec_ldst_single<B: PhysBus>(&mut self, bus: &mut B, pc: u64, insn: u32, form: SingleForm) -> ExecResult {
        let size = bits(insn, 31, 30);
        let vector = bit(insn, 26);
        let opc = bits(insn, 23, 22);
        let rn = bits(insn, 9, 5);
        let rt = bits(insn, 4, 0);

        // Element size: for SIMD&FP registers opc<1> selects the 128-bit form.
        let scale = if vector { ((opc >> 1) << 2) | size } else { size };
        if vector && scale > 4 {
            return self.undefined(pc, insn);
        }
        if vector && !self.fp_enabled() {
            return Err(self.fp_trap());
        }

        let mut acc = DataAccess::NORMAL;
        let addressing = match form {
            SingleForm::UnsignedImmediate => {
                let base = self.base_register(rn)?;
                Addressing {
                    address: base.wrapping_add(u64::from(bits(insn, 21, 10)) << scale),
                    writeback: None,
                }
            }
            SingleForm::Immediate9 => {
                let base = self.base_register(rn)?;
                let imm = sext(u64::from(bits(insn, 20, 12)), 9);
                match bits(insn, 11, 10) {
                    0b00 => Addressing {
                        address: base.wrapping_add(imm),
                        writeback: None,
                    },
                    0b01 => Addressing {
                        address: base,
                        writeback: Some(base.wrapping_add(imm)),
                    },
                    0b11 => {
                        let a = base.wrapping_add(imm);
                        Addressing {
                            address: a,
                            writeback: Some(a),
                        }
                    }
                    _ => {
                        if vector {
                            return self.undefined(pc, insn);
                        }
                        // LDTR/STTR: EL0 permissions when executed at EL1.
                        acc = DataAccess::UNPRIVILEGED;
                        Addressing {
                            address: base.wrapping_add(imm),
                            writeback: None,
                        }
                    }
                }
            }
            SingleForm::RegisterOffset => {
                let option = bits(insn, 15, 13);
                if option & 2 == 0 {
                    return self.undefined(pc, insn);
                }
                let shift = if bit(insn, 12) { scale } else { 0 };
                let offset = extend_reg(self.x_or_zr(bits(insn, 20, 16)), option, shift, 64);
                let base = self.base_register(rn)?;
                Addressing {
                    address: base.wrapping_add(offset),
                    writeback: None,
                }
            }
        };

        if vector {
            let load = opc & 1 == 1;
            if scale == 4 {
                if load {
                    let v = self.read_mem_u128(bus, addressing.address, acc)?;
                    self.v[rt as usize] = v;
                } else {
                    let v = self.v[rt as usize];
                    self.write_mem_u128(bus, addressing.address, v, acc)?;
                }
            } else if load {
                let v = self.read_mem(bus, addressing.address, scale, acc)?;
                self.v[rt as usize] = u128::from(v);
            } else {
                let v = self.v[rt as usize] as u64;
                self.write_mem(bus, addressing.address, scale, v, acc)?;
            }
        } else {
            let is_prefetch = size == 3 && opc == 2;
            match opc {
                0 => {
                    let v = self.x_or_zr(rt);
                    self.write_mem(bus, addressing.address, size, v, acc)?;
                }
                1 => {
                    let v = self.read_mem(bus, addressing.address, size, acc)?;
                    self.set_x_or_zr(rt, v);
                }
                2 if is_prefetch => {
                    // PRFM/PRFUM: a hint with no architectural effect. There is
                    // no pre/post-indexed or unprivileged prefetch.
                    if addressing.writeback.is_some() || acc.unprivileged {
                        return self.undefined(pc, insn);
                    }
                    return Ok(Flow::Next);
                }
                2 => {
                    // LDRSB/LDRSH/LDRSW to a 64-bit register.
                    let v = self.read_mem(bus, addressing.address, size, acc)?;
                    self.set_x_or_zr(rt, sext(v, 8 << size));
                }
                _ => {
                    // LDRSB/LDRSH to a 32-bit register; size 2 and 3 are not
                    // defined here.
                    if size >= 2 {
                        return self.undefined(pc, insn);
                    }
                    let v = self.read_mem(bus, addressing.address, size, acc)?;
                    self.set_x_or_zr(rt, sext(v, 8 << size) & 0xFFFF_FFFF);
                }
            }
        }

        if let Some(wb) = addressing.writeback {
            self.set_x_or_sp(rn, wb);
        }
        Ok(Flow::Next)
    }

    /// LDR (literal), LDRSW (literal), PRFM (literal) and the SIMD&FP
    /// literal loads: `opc 011 V 00 imm19 Rt`.
    pub(crate) fn exec_load_literal<B: PhysBus>(&mut self, bus: &mut B, pc: u64, insn: u32) -> ExecResult {
        let opc = bits(insn, 31, 30);
        let vector = bit(insn, 26);
        let rt = bits(insn, 4, 0);
        let address = pc.wrapping_add(sext(u64::from(bits(insn, 23, 5)) << 2, 21));
        if vector {
            if opc == 3 {
                return self.undefined(pc, insn);
            }
            if !self.fp_enabled() {
                return Err(self.fp_trap());
            }
            self.v[rt as usize] = match opc {
                0 => u128::from(self.read_mem(bus, address, 2, DataAccess::NORMAL)?),
                1 => u128::from(self.read_mem(bus, address, 3, DataAccess::NORMAL)?),
                _ => self.read_mem_u128(bus, address, DataAccess::NORMAL)?,
            };
            return Ok(Flow::Next);
        }
        match opc {
            0 => {
                let v = self.read_mem(bus, address, 2, DataAccess::NORMAL)?;
                self.set_x_or_zr(rt, v);
            }
            1 => {
                let v = self.read_mem(bus, address, 3, DataAccess::NORMAL)?;
                self.set_x_or_zr(rt, v);
            }
            2 => {
                let v = self.read_mem(bus, address, 2, DataAccess::NORMAL)?;
                self.set_x_or_zr(rt, sext(v, 32));
            }
            _ => {}
        }
        Ok(Flow::Next)
    }

    /// LDP/STP/LDPSW/LDNP/STNP, general and SIMD&FP:
    /// `opc 101 V 0 index L imm7 Rt2 Rn Rt`.
    pub(crate) fn exec_ldst_pair<B: PhysBus>(&mut self, bus: &mut B, pc: u64, insn: u32) -> ExecResult {
        let opc = bits(insn, 31, 30);
        let vector = bit(insn, 26);
        let index = bits(insn, 24, 23);
        let load = bit(insn, 22);
        let rt2 = bits(insn, 14, 10);
        let rn = bits(insn, 9, 5);
        let rt = bits(insn, 4, 0);

        // (log2 element bytes, sign-extending 32-bit load)
        let (scale, signed) = match (vector, opc) {
            (false, 0) => (2, false),
            (false, 1) if load && index != 0 => (2, true),
            (false, 2) => (3, false),
            (true, 0) => (2, false),
            (true, 1) => (3, false),
            (true, 2) => (4, false),
            _ => return self.undefined(pc, insn),
        };
        if vector && !self.fp_enabled() {
            return Err(self.fp_trap());
        }
        let offset = sext(u64::from(bits(insn, 21, 15)), 7) << scale;
        let base = self.base_register(rn)?;
        let (address, writeback) = match index {
            0b01 => (base, Some(base.wrapping_add(offset))),
            0b11 => {
                let a = base.wrapping_add(offset);
                (a, Some(a))
            }
            _ => (base.wrapping_add(offset), None),
        };
        let step = 1u64 << scale;
        let acc = DataAccess::NORMAL;
        let fit = Fit::of(address, step);

        if vector {
            if load {
                let (a, b) = if scale == 4 {
                    (
                        self.read_mem_u128(bus, address, acc)?,
                        self.read_mem_u128(bus, address.wrapping_add(step), acc)?,
                    )
                } else {
                    let pair = self.read_pair(bus, address, scale, fit, acc)?;
                    (u128::from(pair.first), u128::from(pair.second))
                };
                self.v[rt as usize] = a;
                self.v[rt2 as usize] = b;
            } else {
                let (a, b) = (self.v[rt as usize], self.v[rt2 as usize]);
                if scale == 4 {
                    self.write_mem_u128(bus, address, a, acc)?;
                    self.write_mem_u128(bus, address.wrapping_add(step), b, acc)?;
                } else {
                    let value = ElementPair {
                        first: a as u64,
                        second: b as u64,
                    };
                    self.write_pair(bus, address, scale, value, fit, acc)?;
                }
            }
        } else if load {
            let ElementPair { first: a, second: b } = self.read_pair(bus, address, scale, fit, acc)?;
            let (a, b) = if signed { (sext(a, 32), sext(b, 32)) } else { (a, b) };
            // With Rt == Rt2 the result is CONSTRAINED UNPREDICTABLE. QEMU
            // (translate-a64.c `trans_LDP`) loads a 32-bit pair as one
            // 64-bit value and writes Rt2 then Rt, but splits a 64-bit pair
            // writing Rt then Rt2; the same order is kept here.
            if scale == 2 {
                self.set_x_or_zr(rt2, b);
                self.set_x_or_zr(rt, a);
            } else {
                self.set_x_or_zr(rt, a);
                self.set_x_or_zr(rt2, b);
            }
        } else {
            let value = ElementPair {
                first: self.x_or_zr(rt),
                second: self.x_or_zr(rt2),
            };
            self.write_pair(bus, address, scale, value, fit, acc)?;
        }

        if let Some(wb) = writeback {
            self.set_x_or_sp(rn, wb);
        }
        Ok(Flow::Next)
    }

    /// Exclusive and acquire/release accesses:
    /// `size 001000 o2 L o1 Rs o0 Rt2 Rn Rt` (ARMv8.0 forms only; CAS and
    /// the LORegion forms belong to feature sets this core lacks).
    pub(crate) fn exec_ldst_exclusive<B: PhysBus>(&mut self, bus: &mut B, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 31, 30);
        let o2 = bit(insn, 23);
        let load = bit(insn, 22);
        let pair = bit(insn, 21);
        let rs = bits(insn, 20, 16);
        let o0 = bit(insn, 15);
        let rt2 = bits(insn, 14, 10);
        let rn = bits(insn, 9, 5);
        let rt = bits(insn, 4, 0);

        if o2 {
            // LDAR/STLR (o0 = 1). o0 = 0 is LDLAR/STLLR (LOR, absent).
            if pair || !o0 {
                return self.undefined(pc, insn);
            }
            let address = self.base_register(rn)?;
            if load {
                let v = self.read_mem(bus, address, size, DataAccess::ORDERED)?;
                self.set_x_or_zr(rt, v);
            } else {
                let v = self.x_or_zr(rt);
                self.write_mem(bus, address, size, v, DataAccess::ORDERED)?;
            }
            return Ok(Flow::Next);
        }

        // Pairs exist only for 32- and 64-bit registers.
        if pair && size < 2 {
            return self.undefined(pc, insn);
        }
        let address = self.base_register(rn)?;
        let total_log2 = if pair { size + 1 } else { size };
        let total = 1u64 << total_log2;

        if load {
            let pa = self.translate_for(bus, address, total, false, DataAccess::ORDERED)?;
            if pair {
                let elem = 1u64 << size;
                let a = self.read_mem(bus, address, size, DataAccess::ORDERED)?;
                let b = self.read_mem(bus, address.wrapping_add(elem), size, DataAccess::ORDERED)?;
                self.set_x_or_zr(rt, a);
                self.set_x_or_zr(rt2, b);
            } else {
                let v = self.read_mem(bus, address, size, DataAccess::ORDERED)?;
                self.set_x_or_zr(rt, v);
            }
            self.monitor = ExclusiveMonitor {
                armed: true,
                addr: pa,
                va: address,
                bytes: total,
            };
            return Ok(Flow::Next);
        }

        // Store-exclusive (Arm ARM `AArch64.ExclusiveMonitorsPass`): a
        // misaligned address faults first (`AArch64.CheckAlignment`). A store
        // to a virtual address other than the one the monitor holds then
        // fails without being translated: `AArch64.IsExclusiveVA`, which is
        // IMPLEMENTATION DEFINED, is permitted to return FALSE for it, and
        // here does, before `AArch64.TranslateAddress`, as QEMU compares the
        // address before any access (target/arm/tcg/translate-a64.c
        // `gen_store_exclusive`). Otherwise its translation and permission
        // faults are reported, and the physical address it reaches must be
        // the monitored one (`IsExclusiveLocal`).
        let m = self.monitor;
        let same_va = m.armed && m.va == address && m.bytes == total;
        let pass = if same_va || address & (total - 1) != 0 {
            let pa = self.translate_for(bus, address, total, true, DataAccess::ORDERED)?;
            same_va && m.addr == pa
        } else {
            false
        };
        self.monitor.armed = false;
        if pass {
            if pair {
                let elem = 1u64 << size;
                let (a, b) = (self.x_or_zr(rt), self.x_or_zr(rt2));
                self.write_mem(bus, address, size, a, DataAccess::ORDERED)?;
                self.write_mem(bus, address.wrapping_add(elem), size, b, DataAccess::ORDERED)?;
            } else {
                let v = self.x_or_zr(rt);
                self.write_mem(bus, address, size, v, DataAccess::ORDERED)?;
            }
        }
        self.set_x_or_zr(rs, u64::from(!pass));
        Ok(Flow::Next)
    }

    /// LD1-LD4/ST1-ST4 (multiple structures), with and without post-index:
    /// `0 Q 001100 post L 0 Rm opcode size Rn Rt`.
    pub(crate) fn exec_ldst_multiple<B: PhysBus>(&mut self, bus: &mut B, pc: u64, insn: u32) -> ExecResult {
        let q = bit(insn, 30);
        let post = bit(insn, 23);
        let load = bit(insn, 22);
        let rm = bits(insn, 20, 16);
        let opcode = bits(insn, 15, 12);
        let size = bits(insn, 11, 10);
        let rn = bits(insn, 9, 5);
        let rt = bits(insn, 4, 0);
        if !post && rm != 0 {
            return self.undefined(pc, insn);
        }
        let (rpt, selem) = match opcode {
            0b0000 => (1, 4),
            0b0010 => (4, 1),
            0b0100 => (1, 3),
            0b0110 => (3, 1),
            0b0111 => (1, 1),
            0b1000 => (1, 2),
            0b1010 => (2, 1),
            _ => return self.undefined(pc, insn),
        };
        if size == 3 && !q && selem != 1 {
            return self.undefined(pc, insn);
        }
        if !self.fp_enabled() {
            return Err(self.fp_trap());
        }
        let ebytes = 1u64 << size;
        let datasize_bytes = if q { 16 } else { 8 };
        let elements = datasize_bytes / ebytes;
        let address = self.base_register(rn)?;

        // Work on copies so a fault part-way leaves no register changed.
        let mut regs = self.v;
        let mut offs = 0u64;
        for r in 0..rpt {
            for e in 0..elements {
                let mut tt = (rt + r) % 32;
                for _ in 0..selem {
                    let shift = (e * ebytes * 8) as u32;
                    let mask = (u128::from(u64::MAX) >> (64 - ebytes * 8)) << shift;
                    let ea = address.wrapping_add(offs);
                    if load {
                        let v = self.read_mem(bus, ea, size, DataAccess::NORMAL)?;
                        let reg = &mut regs[tt as usize];
                        *reg = (*reg & !mask) | (u128::from(v) << shift);
                    } else {
                        let v = ((regs[tt as usize] & mask) >> shift) as u64;
                        self.write_mem(bus, ea, size, v, DataAccess::NORMAL)?;
                    }
                    offs += ebytes;
                    tt = (tt + 1) % 32;
                }
            }
        }
        if load {
            // Every register written is written at the operation's datasize:
            // a 64-bit form clears the upper half.
            for r in 0..(rpt * selem) {
                let t = ((rt + r) % 32) as usize;
                self.v[t] = if q { regs[t] } else { regs[t] & u128::from(u64::MAX) };
            }
        }
        if post {
            let step = if rm == 31 { offs } else { self.x_or_zr(rm) };
            self.set_x_or_sp(rn, address.wrapping_add(step));
        }
        Ok(Flow::Next)
    }

    /// LD1-LD4/ST1-ST4 (single structure) and LD1R-LD4R:
    /// `0 Q 001101 post L R Rm opcode S size Rn Rt`.
    pub(crate) fn exec_ldst_single_structure<B: PhysBus>(&mut self, bus: &mut B, pc: u64, insn: u32) -> ExecResult {
        let q = bit(insn, 30);
        let post = bit(insn, 23);
        let load = bit(insn, 22);
        let r = bit(insn, 21);
        let rm = bits(insn, 20, 16);
        let opcode = bits(insn, 15, 13);
        let s = bit(insn, 12);
        let size = bits(insn, 11, 10);
        let rn = bits(insn, 9, 5);
        let mut t = bits(insn, 4, 0);
        if !post && rm != 0 {
            return self.undefined(pc, insn);
        }
        let selem = (((opcode & 1) << 1) | u32::from(r)) + 1;
        let qs = (u32::from(q) << 1) | u32::from(s);
        let (scale, index, replicate) = match opcode >> 1 {
            0 => (0, (qs << 2) | size, false),
            1 => {
                if size & 1 != 0 {
                    return self.undefined(pc, insn);
                }
                (1, (qs << 1) | (size >> 1), false)
            }
            2 => {
                if size & 2 != 0 {
                    return self.undefined(pc, insn);
                }
                if size & 1 == 0 {
                    (2, qs, false)
                } else {
                    if s {
                        return self.undefined(pc, insn);
                    }
                    (3, u32::from(q), false)
                }
            }
            _ => {
                if !load || s {
                    return self.undefined(pc, insn);
                }
                (size, 0, true)
            }
        };
        if !self.fp_enabled() {
            return Err(self.fp_trap());
        }
        let ebytes = 1u64 << scale;
        let address = self.base_register(rn)?;
        let mut regs = self.v;
        let mut offs = 0u64;
        for _ in 0..selem {
            let ea = address.wrapping_add(offs);
            if replicate {
                let v = self.read_mem(bus, ea, scale, DataAccess::NORMAL)?;
                let mut full = 0u128;
                let mut pos = 0;
                while pos < 128 {
                    full |= u128::from(v) << pos;
                    pos += ebytes * 8;
                }
                regs[t as usize] = if q { full } else { full & u128::from(u64::MAX) };
            } else {
                let shift = (u64::from(index) * ebytes * 8) as u32;
                let mask = (u128::from(u64::MAX) >> (64 - ebytes * 8)) << shift;
                if load {
                    let v = self.read_mem(bus, ea, scale, DataAccess::NORMAL)?;
                    regs[t as usize] = (regs[t as usize] & !mask) | (u128::from(v) << shift);
                } else {
                    let v = ((regs[t as usize] & mask) >> shift) as u64;
                    self.write_mem(bus, ea, scale, v, DataAccess::NORMAL)?;
                }
            }
            offs += ebytes;
            t = (t + 1) % 32;
        }
        if load {
            self.v = regs;
        }
        if post {
            let step = if rm == 31 { offs } else { self.x_or_zr(rm) };
            self.set_x_or_sp(rn, address.wrapping_add(step));
        }
        Ok(Flow::Next)
    }
}
