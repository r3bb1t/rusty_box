//! The SVE loads, stores and prefetches FEAT_SME makes legal in Streaming
//! SVE mode: the contiguous accesses of one to four vectors, the
//! load-and-broadcasts, and LDR and STR of vector and predicate registers.
//!
//! An element access is `Mem[addr, mbytes]` of the element's memory size,
//! checked for alignment by that size. A fault ends the instruction: a load
//! has written no register, a store the elements before the faulting one,
//! in the pseudocode's order. A predicated access with no element active
//! reads and writes no memory; its base, if SP, still takes the SP
//! alignment check (`Unpredictable_CHECKSPNONEACTIVE` taken as TRUE: the
//! check depends on the base register alone, as for LDR and STR). The
//! prefetches are hints, which this processor, with no cache, takes as no
//! operation once the enable check has passed.
//!
//! Provenance (R7): the Arm A64 ISA XML, release 2026-09, the instruction
//! pages each handler names.

use super::{Esize, Pv, Zv};
use crate::bus::PhysBus;
use crate::cpu::exec::{bit, bits, sext, ExecResult, Fault, Flow};
use crate::cpu::features::VecLen;
use crate::cpu::mmu::{DataAccess, Part};
use crate::cpu::{Cpu, Levels};

/// What a predicated access starts with, in its pseudocode's order: the
/// enable check and `CurrentVL`, the governing predicate `Pg` (bits 12:10),
/// and the base register `Xn|SP` (bits 9:5) with its SP check.
struct Predicated {
    vl: VecLen,
    mask: Pv,
    base: u64,
    /// `Zt` (bits 4:0): the first register the access transfers.
    t: u32,
}

/// The elements a contiguous access moves: `esize` bits in the register,
/// `msize` in memory, and how a loaded element widens from one to the
/// other.
#[derive(Debug, Clone, Copy)]
struct Elements {
    esize: Esize,
    msize: Esize,
    /// Zero-extended (`unsigned`), else sign-extended.
    unsigned: bool,
}

impl Elements {
    /// A load's elements by its `dtype` (`sve_mem_cld_ss`, `_cld_si`, and
    /// `sve_mem_ld_dup`'s `dtypeh:dtypel`): the sixteen encodings of LD1
    /// and of LD1R, which Arm numbers alike.
    const fn from_dtype(dtype: u32) -> Self {
        let (esize, msize, unsigned) = match dtype & 0xF {
            0b0000 => (Esize::B, Esize::B, true),
            0b0001 => (Esize::H, Esize::B, true),
            0b0010 => (Esize::S, Esize::B, true),
            0b0011 => (Esize::D, Esize::B, true),
            0b0100 => (Esize::D, Esize::S, false),
            0b0101 => (Esize::H, Esize::H, true),
            0b0110 => (Esize::S, Esize::H, true),
            0b0111 => (Esize::D, Esize::H, true),
            0b1000 => (Esize::D, Esize::H, false),
            0b1001 => (Esize::S, Esize::H, false),
            0b1010 => (Esize::S, Esize::S, true),
            0b1011 => (Esize::D, Esize::S, true),
            0b1100 => (Esize::D, Esize::B, false),
            0b1101 => (Esize::S, Esize::B, false),
            0b1110 => (Esize::H, Esize::B, false),
            _ => (Esize::D, Esize::D, true),
        };
        Self { esize, msize, unsigned }
    }

    /// Elements of one size in register and in memory.
    const fn same(size: Esize) -> Self {
        Self {
            esize: size,
            msize: size,
            unsigned: true,
        }
    }

    /// The register element an `msize` value from memory makes:
    /// `Extend(data, esize, unsigned)`.
    #[inline]
    fn extend(self, data: u64) -> u64 {
        if self.unsigned {
            data
        } else {
            sext(data, self.msize.bits()) & self.esize.mask()
        }
    }
}

/// Where a contiguous access starts, past its base register.
///
/// Closed (R5): SVE's two contiguous addressing forms.
#[derive(Debug, Clone, Copy)]
enum Offset {
    /// Scalar plus scalar: `X[m]` memory elements.
    Register(u32),
    /// Scalar plus immediate: a signed multiple of the bytes the access
    /// spans, which is `elements * nreg * mbytes` (16 for LD1RQ).
    Multiple(i64),
}

impl Offset {
    /// The offset field of a contiguous access: `imm4` (bits 19:16) when
    /// `immediate`, else `Rm` (bits 20:16).
    fn of(insn: u32, immediate: bool) -> Self {
        if immediate {
            Self::Multiple(sext(u64::from(bits(insn, 19, 16)), 4) as i64)
        } else {
            Self::Register(bits(insn, 20, 16))
        }
    }
}

impl<L: Levels> Cpu<L> {
    /// The [`Predicated`] start of the access `insn`.
    fn sve_predicated_access(&self, insn: u32) -> Result<Predicated, Fault> {
        let (g, n, t) = (bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let mask = self.p(g, vl);
        let base = self.base_register(n)?;
        Ok(Predicated { vl, mask, base, t })
    }

    /// The first address of a contiguous access from `base`: `X[m] *
    /// mbytes` past it, or `imm * span` (`AddressAdd`, which wraps).
    fn sve_first_address(&self, base: u64, offset: Offset, mbytes: usize, span: usize) -> u64 {
        match offset {
            Offset::Register(m) => base.wrapping_add(self.x_or_zr(m).wrapping_mul(mbytes as u64)),
            Offset::Multiple(imm) => base.wrapping_add((imm as u64).wrapping_mul(span as u64)),
        }
    }

    /// The contiguous load of `nreg` registers from `Zt` up (modulo 32):
    /// element `e` of register `r` from `(e * nreg + r) * mbytes` past the
    /// first address, zero when inactive; the registers are written once
    /// every access is done.
    fn sve_load<B: PhysBus>(&mut self, bus: &mut B, insn: u32, el: Elements, nreg: usize, offset: Offset) -> ExecResult {
        let Predicated { vl, mask, base, t } = self.sve_predicated_access(insn)?;
        let count = el.esize.elements(vl);
        let mbytes = el.msize.bytes();
        let mut addr = self.sve_first_address(base, offset, mbytes, count * nreg * mbytes);
        let mut values = [Zv::ZERO; 4];
        for e in 0..count {
            let active = mask.active(e, el.esize);
            for value in &mut values[..nreg] {
                if active {
                    let data = self.read_mem(bus, addr, el.msize.log2_bytes(), DataAccess::NORMAL)?;
                    value.set(e, el.esize, el.extend(data));
                }
                addr = addr.wrapping_add(mbytes as u64);
            }
        }
        for (r, value) in (0u32..).zip(&values[..nreg]) {
            self.set_z((t + r) % 32, vl, value);
        }
        Ok(Flow::Next)
    }

    /// The contiguous store of `nreg` registers from `Zt` up (modulo 32),
    /// in [`Cpu::sve_load`]'s order: the low `msize` bits of each active
    /// element.
    fn sve_store<B: PhysBus>(&mut self, bus: &mut B, insn: u32, el: Elements, nreg: usize, offset: Offset) -> ExecResult {
        let Predicated { vl, mask, base, t } = self.sve_predicated_access(insn)?;
        let count = el.esize.elements(vl);
        let mbytes = el.msize.bytes();
        let mut addr = self.sve_first_address(base, offset, mbytes, count * nreg * mbytes);
        let mut values = [Zv::ZERO; 4];
        for (r, value) in (0u32..).zip(&mut values[..nreg]) {
            *value = self.z((t + r) % 32, vl);
        }
        for e in 0..count {
            let active = mask.active(e, el.esize);
            for value in &values[..nreg] {
                if active {
                    self.write_mem(bus, addr, el.msize.log2_bytes(), value.get(e, el.esize), DataAccess::NORMAL)?;
                }
                addr = addr.wrapping_add(mbytes as u64);
            }
        }
        Ok(Flow::Next)
    }

    /// LD1B, LD1H, LD1W, LD1D and the sign-extending LD1SB, LD1SH, LD1SW
    /// (`ld1b_z_p_br` to `ld1sw_z_p_bi`: `1010010 dtype Rm|0imm4 010|101 Pg
    /// Rn Zt`, scalar plus scalar or plus immediate by bit 13).
    pub(crate) fn exec_sve_load_contiguous<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let el = Elements::from_dtype(bits(insn, 24, 21));
        self.sve_load(bus, insn, el, 1, Offset::of(insn, bit(insn, 13)))
    }

    /// LDNT1B to LDNT1D and LD2, LD3, LD4 of bytes to doublewords
    /// (`ldnt1b_z_p_br` to `ld4d_z_p_bi`: `1010010 msz opc Rm|0imm4 110|111
    /// Pg Rn Zt`): `opc + 1` registers of `8 << msz`-bit elements,
    /// interleaved in memory; scalar plus scalar or plus immediate by bit
    /// 13. Non-temporal is a hint, which changes nothing here.
    pub(crate) fn exec_sve_load_structures<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let el = Elements::same(Esize::from_size(bits(insn, 24, 23)));
        let nreg = bits(insn, 22, 21) as usize + 1;
        self.sve_load(bus, insn, el, nreg, Offset::of(insn, bit(insn, 13)))
    }

    /// ST1B, ST1H, ST1W, ST1D (`st1b_z_p_br` to `st1d_z_p_bi`: `1110010 msz
    /// size Rm|0imm4 010|111 Pg Rn Zt`, scalar plus scalar or plus
    /// immediate by bit 15): elements of `8 << size` bits stored to
    /// `8 << msz`. An element narrower than its memory is UNDEFINED, which
    /// of the encodings is ST1H's `size = 00`.
    pub(crate) fn exec_sve_store_contiguous<B: PhysBus>(&mut self, bus: &mut B, pc: u64, insn: u32) -> ExecResult {
        let msize = Esize::from_size(bits(insn, 24, 23));
        let esize = Esize::from_size(bits(insn, 22, 21));
        if esize.bits() < msize.bits() {
            return self.undefined(pc, insn);
        }
        let el = Elements {
            esize,
            msize,
            unsigned: true,
        };
        self.sve_store(bus, insn, el, 1, Offset::of(insn, bit(insn, 15)))
    }

    /// STNT1B to STNT1D and ST2, ST3, ST4 of bytes to doublewords
    /// (`stnt1b_z_p_br` to `st4d_z_p_bi`: `1110010 msz opc Rm|1imm4
    /// 011|111 Pg Rn Zt`), as [`Cpu::exec_sve_load_structures`]; scalar
    /// plus scalar or plus immediate by bit 15.
    pub(crate) fn exec_sve_store_structures<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let el = Elements::same(Esize::from_size(bits(insn, 24, 23)));
        let nreg = bits(insn, 22, 21) as usize + 1;
        self.sve_store(bus, insn, el, nreg, Offset::of(insn, bit(insn, 15)))
    }

    /// LD1RB, LD1RH, LD1RW, LD1RD and the sign-extending LD1RSB, LD1RSH,
    /// LD1RSW (`ld1rb_z_p_bi` to `ld1rsw_z_p_bi`: `1000010 dtypeh 1 imm6 1
    /// dtypel Pg Rn Zt`): one element at `X[n] + imm6 * mbytes`, read when
    /// an element is active, in each active element; the others zero.
    pub(crate) fn exec_sve_load_broadcast<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let el = Elements::from_dtype((bits(insn, 24, 23) << 2) | bits(insn, 14, 13));
        let offset = u64::from(bits(insn, 21, 16));
        let Predicated { vl, mask, base, t } = self.sve_predicated_access(insn)?;
        let count = el.esize.elements(vl);
        let mut result = Zv::ZERO;
        if (0..count).any(|e| mask.active(e, el.esize)) {
            let addr = base.wrapping_add(offset * el.msize.bytes() as u64);
            let data = el.extend(self.read_mem(bus, addr, el.msize.log2_bytes(), DataAccess::NORMAL)?);
            for e in (0..count).filter(|&e| mask.active(e, el.esize)) {
                result.set(e, el.esize, data);
            }
        }
        self.set_z(t, vl, &result);
        Ok(Flow::Next)
    }

    /// LD1RQB, LD1RQH, LD1RQW, LD1RQD (`ld1rqb_z_p_br` to `ld1rqd_z_p_bi`:
    /// `1010010 msz 00 Rm|0imm4 000|001 Pg Rn Zt`): a quadword of
    /// `8 << msz`-bit elements under the low 16 bits of `Pg`, at
    /// `X[n] + X[m] * mbytes` or `X[n] + imm4 * 16` by bit 13, its inactive
    /// elements zero, in every quadword of the vector.
    pub(crate) fn exec_sve_load_broadcast_quadword<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 24, 23));
        let offset = Offset::of(insn, bit(insn, 13));
        let Predicated { vl, mask, base, t } = self.sve_predicated_access(insn)?;
        let mut addr = self.sve_first_address(base, offset, es.bytes(), 16);
        let mut quadword = Zv::ZERO;
        for e in 0..16 / es.bytes() {
            if mask.active(e, es) {
                quadword.set(e, es, self.read_mem(bus, addr, es.log2_bytes(), DataAccess::NORMAL)?);
            }
            addr = addr.wrapping_add(es.bytes() as u64);
        }
        let mut result = Zv::ZERO;
        for i in 0..vl.bytes() {
            result.set_byte(i, quadword.byte(i % 16));
        }
        self.set_z(t, vl, &result);
        Ok(Flow::Next)
    }

    /// LDR and STR of a vector or a predicate register (`ldr_z_bi`,
    /// `ldr_p_bi`, `str_z_bi`, `str_p_bi`: `1x0|1x1 0010 1 imm9h 0x0 imm9l
    /// Rn Zt|0Pt`, a store by bit 30, of a vector by bit 14): the register's
    /// `VL DIV 8` or `PL DIV 8` bytes at `X[n] + imm9` times that length,
    /// each byte an access of its own. The transfer is aligned when its
    /// base is, to 16 bytes for a vector and 2 for a predicate; misaligned,
    /// it faults when alignment is enforced, and Device memory refuses each
    /// of its bytes.
    pub(crate) fn exec_sve_fill_spill<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let store = bit(insn, 30);
        let vector = bit(insn, 14);
        let imm = sext(u64::from((bits(insn, 21, 16) << 3) | bits(insn, 12, 10)), 9);
        let n = bits(insn, 9, 5);
        let t = if vector { bits(insn, 4, 0) } else { bits(insn, 3, 0) };
        let vl = self.sve_check()?;
        let (len, alignment) = if vector {
            (vl.bytes(), 16)
        } else {
            (vl.predicate_bytes(), 2)
        };
        let base = self.base_register(n)?;
        let addr = base.wrapping_add(imm.wrapping_mul(len as u64));
        let aligned = base % alignment == 0;
        self.check_transfer_alignment(aligned, addr, store)?;
        let acc = DataAccess {
            part: if aligned { Part::Whole } else { Part::OfMisaligned },
            ..DataAccess::NORMAL
        };
        if store {
            let source = if vector {
                self.z(t, vl)
            } else {
                Zv::from_bytes(self.p(t, vl).as_bytes())
            };
            for (i, &byte) in (0u64..).zip(&source.as_bytes()[..len]) {
                self.write_mem(bus, addr.wrapping_add(i), 0, u64::from(byte), acc)?;
            }
        } else {
            let mut loaded = Zv::ZERO;
            for i in 0..len {
                loaded.set_byte(i, self.read_mem(bus, addr.wrapping_add(i as u64), 0, acc)? as u8);
            }
            if vector {
                self.set_z(t, vl, &loaded);
            } else {
                self.set_p(t, vl, &Pv::from_bytes(&loaded.as_bytes()[..len]));
            }
        }
        Ok(Flow::Next)
    }

    /// PRFB, PRFH, PRFW, PRFD (`prfb_i_p_bi` to `prfd_i_p_br`): a prefetch
    /// hint, after the enable check; it reads no memory and faults on no
    /// address.
    pub(crate) fn exec_sve_prefetch(&mut self) -> ExecResult {
        self.sve_check()?;
        Ok(Flow::Next)
    }
}
