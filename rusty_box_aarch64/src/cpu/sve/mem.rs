//! The SVE loads, stores and prefetches FEAT_SME makes legal in Streaming
//! SVE mode: the contiguous accesses of one to four vectors, the
//! load-and-broadcasts, and LDR and STR of vector and predicate registers;
//! and FEAT_SME2's contiguous accesses of two or four vectors under a
//! predicate-as-counter, to consecutive or strided registers.
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

use super::counter::PredCounter;
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

/// What a multi-vector access starts with, in its pseudocode's order: the
/// element size `msz` (bits 14:13) and the [`MultiVectors`], the enable
/// check and `CurrentVL`, the governing predicate-as-counter `PNg` (PN8 to
/// PN15 by bits 12:10), and the base register `Xn|SP` (bits 9:5) with its
/// SP check; then the first address, `X[m]` elements or the immediate in
/// whole transfers past the base.
struct MultiAccess {
    vl: VecLen,
    es: Esize,
    regs: MultiVectors,
    counter: PredCounter,
    addr: u64,
}

/// The vectors a multi-vector access transfers: two or four by bit 15,
/// consecutive from `Zt:'0'` or `Zt:'00'` (bits 4:1 or 4:2) with bit 24
/// clear, else strided, from `T:'0':Zt` 8 apart or from `T:'00':Zt` 4
/// apart (`T` bit 4, `Zt` bits 2:0 or 1:0).
#[derive(Debug, Clone, Copy)]
struct MultiVectors {
    first: u32,
    nreg: usize,
    stride: u32,
}

impl MultiVectors {
    fn of(insn: u32) -> Self {
        let four = bit(insn, 15);
        let (nreg, (first, stride)) = match (four, bit(insn, 24)) {
            (false, false) => (2, (bits(insn, 4, 1) << 1, 1)),
            (true, false) => (4, (bits(insn, 4, 2) << 2, 1)),
            (false, true) => (2, ((bits(insn, 4, 4) << 4) | bits(insn, 2, 0), 8)),
            (true, true) => (4, ((bits(insn, 4, 4) << 4) | bits(insn, 1, 0), 4)),
        };
        Self {
            first,
            nreg,
            stride,
        }
    }

    /// The `r`th vector transferred.
    #[inline]
    fn register(self, r: usize) -> u32 {
        self.first + r as u32 * self.stride
    }
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
        Self {
            esize,
            msize,
            unsigned,
        }
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

    /// The [`MultiAccess`] start of the multi-vector access `insn`, scalar
    /// plus scalar or plus immediate by bit 22.
    fn sve_multi_access(&self, insn: u32) -> Result<MultiAccess, Fault> {
        let es = Esize::from_size(bits(insn, 14, 13));
        let regs = MultiVectors::of(insn);
        let offset = Offset::of(insn, bit(insn, 22));
        let (g, n) = (8 + bits(insn, 12, 10), bits(insn, 9, 5));
        let vl = self.sve_check()?;
        let counter = PredCounter::of(&self.p(g, vl), vl);
        let base = self.base_register(n)?;
        let mbytes = es.bytes();
        let addr =
            self.sve_first_address(base, offset, mbytes, es.elements(vl) * regs.nreg * mbytes);
        Ok(MultiAccess {
            vl,
            es,
            regs,
            counter,
            addr,
        })
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
    fn sve_load<B: PhysBus>(
        &mut self,
        bus: &mut B,
        insn: u32,
        el: Elements,
        nreg: usize,
        offset: Offset,
    ) -> ExecResult {
        let Predicated { vl, mask, base, t } = self.sve_predicated_access(insn)?;
        let count = el.esize.elements(vl);
        let mbytes = el.msize.bytes();
        let mut addr = self.sve_first_address(base, offset, mbytes, count * nreg * mbytes);
        let mut values = [Zv::ZERO; 4];
        for e in 0..count {
            let active = mask.active(e, el.esize);
            for value in &mut values[..nreg] {
                if active {
                    let data =
                        self.read_mem(bus, addr, el.msize.log2_bytes(), DataAccess::NORMAL)?;
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
    fn sve_store<B: PhysBus>(
        &mut self,
        bus: &mut B,
        insn: u32,
        el: Elements,
        nreg: usize,
        offset: Offset,
    ) -> ExecResult {
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
                    self.write_mem(
                        bus,
                        addr,
                        el.msize.log2_bytes(),
                        value.get(e, el.esize),
                        DataAccess::NORMAL,
                    )?;
                }
                addr = addr.wrapping_add(mbytes as u64);
            }
        }
        Ok(Flow::Next)
    }

    /// LD1B, LD1H, LD1W, LD1D and the sign-extending LD1SB, LD1SH, LD1SW
    /// (`ld1b_z_p_br` to `ld1sw_z_p_bi`: `1010010 dtype Rm|0imm4 010|101 Pg
    /// Rn Zt`, scalar plus scalar or plus immediate by bit 13).
    pub(crate) fn exec_sve_load_contiguous<B: PhysBus>(
        &mut self,
        bus: &mut B,
        insn: u32,
    ) -> ExecResult {
        let el = Elements::from_dtype(bits(insn, 24, 21));
        self.sve_load(bus, insn, el, 1, Offset::of(insn, bit(insn, 13)))
    }

    /// LDNT1B to LDNT1D and LD2, LD3, LD4 of bytes to doublewords
    /// (`ldnt1b_z_p_br` to `ld4d_z_p_bi`: `1010010 msz opc Rm|0imm4 110|111
    /// Pg Rn Zt`): `opc + 1` registers of `8 << msz`-bit elements,
    /// interleaved in memory; scalar plus scalar or plus immediate by bit
    /// 13. Non-temporal is a hint, which changes nothing here.
    pub(crate) fn exec_sve_load_structures<B: PhysBus>(
        &mut self,
        bus: &mut B,
        insn: u32,
    ) -> ExecResult {
        let el = Elements::same(Esize::from_size(bits(insn, 24, 23)));
        let nreg = bits(insn, 22, 21) as usize + 1;
        self.sve_load(bus, insn, el, nreg, Offset::of(insn, bit(insn, 13)))
    }

    /// ST1B, ST1H, ST1W, ST1D (`st1b_z_p_br` to `st1d_z_p_bi`: `1110010 msz
    /// size Rm|0imm4 010|111 Pg Rn Zt`, scalar plus scalar or plus
    /// immediate by bit 15): elements of `8 << size` bits stored to
    /// `8 << msz`. An element narrower than its memory is UNDEFINED, which
    /// of the encodings is ST1H's `size = 00`.
    pub(crate) fn exec_sve_store_contiguous<B: PhysBus>(
        &mut self,
        bus: &mut B,
        pc: u64,
        insn: u32,
    ) -> ExecResult {
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
    pub(crate) fn exec_sve_store_structures<B: PhysBus>(
        &mut self,
        bus: &mut B,
        insn: u32,
    ) -> ExecResult {
        let el = Elements::same(Esize::from_size(bits(insn, 24, 23)));
        let nreg = bits(insn, 22, 21) as usize + 1;
        self.sve_store(bus, insn, el, nreg, Offset::of(insn, bit(insn, 15)))
    }

    /// LD1B, LD1H, LD1W, LD1D and LDNT1B to LDNT1D of two or four vectors
    /// (`ld1b_mz_p_br_2` to `ldnt1d_mzx_p_bi_4x4`: `1010000 x 0 x 0
    /// Rm|0imm4 x msz PNg Rn Zt`, scalar plus scalar or plus immediate by
    /// bit 22, [`MultiVectors`] by bits 24, 15 and 4:0): `8 << msz`-bit
    /// elements, register after register in memory, element `e` of
    /// register `r` active as element `r * elements + e` of the
    /// predicate-as-counter, the others zero; the registers are written
    /// once every access is done. Non-temporal is a hint, which changes
    /// nothing here.
    pub(crate) fn exec_sve_load_multi<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let MultiAccess {
            vl,
            es,
            regs,
            counter,
            mut addr,
        } = self.sve_multi_access(insn)?;
        let elements = es.elements(vl);
        let mut values = [Zv::ZERO; 4];
        for (r, value) in values[..regs.nreg].iter_mut().enumerate() {
            for e in 0..elements {
                if counter.active(r * elements + e, es) {
                    value.set(
                        e,
                        es,
                        self.read_mem(bus, addr, es.log2_bytes(), DataAccess::NORMAL)?,
                    );
                }
                addr = addr.wrapping_add(es.bytes() as u64);
            }
        }
        for (r, value) in values[..regs.nreg].iter().enumerate() {
            self.set_z(regs.register(r), vl, value);
        }
        Ok(Flow::Next)
    }

    /// ST1B, ST1H, ST1W, ST1D and STNT1B to STNT1D of two or four vectors
    /// (`st1b_mz_p_br_2` to `stnt1d_mzx_p_bi_4x4`: `1010000 x 0 x 1
    /// Rm|0imm4 x msz PNg Rn Zt`), in [`Cpu::exec_sve_load_multi`]'s
    /// order: each active element.
    pub(crate) fn exec_sve_store_multi<B: PhysBus>(
        &mut self,
        bus: &mut B,
        insn: u32,
    ) -> ExecResult {
        let MultiAccess {
            vl,
            es,
            regs,
            counter,
            mut addr,
        } = self.sve_multi_access(insn)?;
        let elements = es.elements(vl);
        for r in 0..regs.nreg {
            let source = self.z(regs.register(r), vl);
            for e in 0..elements {
                if counter.active(r * elements + e, es) {
                    self.write_mem(
                        bus,
                        addr,
                        es.log2_bytes(),
                        source.get(e, es),
                        DataAccess::NORMAL,
                    )?;
                }
                addr = addr.wrapping_add(es.bytes() as u64);
            }
        }
        Ok(Flow::Next)
    }

    /// LD1RB, LD1RH, LD1RW, LD1RD and the sign-extending LD1RSB, LD1RSH,
    /// LD1RSW (`ld1rb_z_p_bi` to `ld1rsw_z_p_bi`: `1000010 dtypeh 1 imm6 1
    /// dtypel Pg Rn Zt`): one element at `X[n] + imm6 * mbytes`, read when
    /// an element is active, in each active element; the others zero.
    pub(crate) fn exec_sve_load_broadcast<B: PhysBus>(
        &mut self,
        bus: &mut B,
        insn: u32,
    ) -> ExecResult {
        let el = Elements::from_dtype((bits(insn, 24, 23) << 2) | bits(insn, 14, 13));
        let offset = u64::from(bits(insn, 21, 16));
        let Predicated { vl, mask, base, t } = self.sve_predicated_access(insn)?;
        let count = el.esize.elements(vl);
        let mut result = Zv::ZERO;
        if (0..count).any(|e| mask.active(e, el.esize)) {
            let addr = base.wrapping_add(offset * el.msize.bytes() as u64);
            let data =
                el.extend(self.read_mem(bus, addr, el.msize.log2_bytes(), DataAccess::NORMAL)?);
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
    pub(crate) fn exec_sve_load_broadcast_quadword<B: PhysBus>(
        &mut self,
        bus: &mut B,
        insn: u32,
    ) -> ExecResult {
        let es = Esize::from_size(bits(insn, 24, 23));
        let offset = Offset::of(insn, bit(insn, 13));
        let Predicated { vl, mask, base, t } = self.sve_predicated_access(insn)?;
        let mut addr = self.sve_first_address(base, offset, es.bytes(), 16);
        let mut quadword = Zv::ZERO;
        for e in 0..16 / es.bytes() {
            if mask.active(e, es) {
                quadword.set(
                    e,
                    es,
                    self.read_mem(bus, addr, es.log2_bytes(), DataAccess::NORMAL)?,
                );
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
        let t = if vector {
            bits(insn, 4, 0)
        } else {
            bits(insn, 3, 0)
        };
        let vl = self.sve_check()?;
        let (len, alignment) = if vector {
            (vl.bytes(), 16)
        } else {
            (vl.predicate_bytes(), 2)
        };
        let base = self.base_register(n)?;
        let addr = base.wrapping_add(imm.wrapping_mul(len as u64));
        let aligned = base % alignment == 0;
        if store {
            let source = if vector {
                self.z(t, vl)
            } else {
                Zv::from_bytes(self.p(t, vl).as_bytes())
            };
            self.spill_bytes(bus, addr, aligned, &source.as_bytes()[..len])?;
        } else {
            let loaded = self.fill_bytes(bus, addr, aligned, len)?;
            if vector {
                self.set_z(t, vl, &loaded);
            } else {
                self.set_p(t, vl, &Pv::from_bytes(&loaded.as_bytes()[..len]));
            }
        }
        Ok(Flow::Next)
    }

    /// The alignment check of an LDR or STR of a register, then the access
    /// each of its bytes makes: `aligned` when the transfer's base is
    /// aligned to the register's alignment; misaligned, the transfer
    /// faults here at `addr` when alignment is enforced, and otherwise
    /// Device memory refuses each of its bytes.
    fn transfer_access(&self, addr: u64, aligned: bool, store: bool) -> Result<DataAccess, Fault> {
        self.check_transfer_alignment(aligned, addr, store)?;
        Ok(DataAccess {
            part: if aligned {
                Part::Whole
            } else {
                Part::OfMisaligned
            },
            ..DataAccess::NORMAL
        })
    }

    /// The load of an LDR of a vector, a predicate or a ZA array vector,
    /// its alignment check included: `len` bytes from `addr` up, each an
    /// access of its own ([`Cpu::transfer_access`]).
    pub(crate) fn fill_bytes<B: PhysBus>(
        &mut self,
        bus: &mut B,
        addr: u64,
        aligned: bool,
        len: usize,
    ) -> Result<Zv, Fault> {
        let acc = self.transfer_access(addr, aligned, false)?;
        let mut loaded = Zv::ZERO;
        for i in 0..len {
            loaded.set_byte(
                i,
                self.read_mem(bus, addr.wrapping_add(i as u64), 0, acc)? as u8,
            );
        }
        Ok(loaded)
    }

    /// The store of an STR of a vector, a predicate or a ZA array vector:
    /// `bytes` to `addr` up, as [`Cpu::fill_bytes`] loads them.
    pub(crate) fn spill_bytes<B: PhysBus>(
        &mut self,
        bus: &mut B,
        addr: u64,
        aligned: bool,
        bytes: &[u8],
    ) -> Result<(), Fault> {
        let acc = self.transfer_access(addr, aligned, true)?;
        for (i, &byte) in (0u64..).zip(bytes) {
            self.write_mem(bus, addr.wrapping_add(i), 0, u64::from(byte), acc)?;
        }
        Ok(())
    }

    /// PRFB, PRFH, PRFW, PRFD (`prfb_i_p_bi` to `prfd_i_p_br`): a prefetch
    /// hint, after the enable check; it reads no memory and faults on no
    /// address.
    pub(crate) fn exec_sve_prefetch(&mut self) -> ExecResult {
        self.sve_check()?;
        Ok(Flow::Next)
    }
}
