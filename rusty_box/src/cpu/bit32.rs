//! 32-bit bit scan instructions: BSF, BSR
//! Matching Bochs bit32.cc
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // =========================================================================
    // BSF / BSR — Bit Scan Forward / Reverse (0F BC / 0F BD)
    // =========================================================================

    /// BSF r32, r/m32 — Bit Scan Forward (0F BC /r)
    /// Bochs bit32.cc: SET_FLAGS_OSZAPC_LOGIC_32(val_32); clear_ZF();
    pub fn bsf_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, eaddr)?
        };
        if op2 == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            let idx = op2.trailing_zeros();
            self.set_flags_oszapc_logic_32(idx);
            self.eflags.remove(EFlags::ZF);
            self.set_gpr32(instr.dst() as usize, idx);
        }
        Ok(())
    }

    /// BSR r32, r/m32 — Bit Scan Reverse (0F BD /r)
    /// Bochs bit32.cc: SET_FLAGS_OSZAPC_LOGIC_32(val_32); clear_ZF();
    pub fn bsr_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, eaddr)?
        };
        if op2 == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            let idx = 31 - op2.leading_zeros();
            self.set_flags_oszapc_logic_32(idx);
            self.eflags.remove(EFlags::ZF);
            self.set_gpr32(instr.dst() as usize, idx);
        }
        Ok(())
    }

    // =========================================================================
    // POPCNT — Population Count (F3 0F B8 /r)
    // Bochs: bit32.cc POPCNT_GdEdR / POPCNT_GdEdM
    // =========================================================================

    /// POPCNT r32, r/m32 — count set bits
    pub fn popcnt_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, eaddr)?
        };
        let result = op2.count_ones();
        self.set_gpr32(instr.dst() as usize, result);

        // POPCNT clears OF, SF, AF, CF, PF; sets ZF if result is 0
        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::AF | EFlags::CF | EFlags::PF);
        if result == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            self.eflags.remove(EFlags::ZF);
        }
        Ok(())
    }

    /// POPCNT r16, r/m16 — count set bits (16-bit)
    pub fn popcnt_gw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize) as u16
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_word(seg, eaddr)?
        };
        let result = op2.count_ones() as u16;
        // Write 16-bit result (preserve upper 16 bits)
        let dst = instr.dst() as usize;
        let current = self.get_gpr32(dst);
        self.set_gpr32(dst, (current & 0xFFFF0000) | result as u32);

        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::AF | EFlags::CF | EFlags::PF);
        if result == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            self.eflags.remove(EFlags::ZF);
        }
        Ok(())
    }

    // =========================================================================
    // LZCNT — Leading Zero Count (F3 0F BD /r)
    // =========================================================================

    /// LZCNT r32, r/m32 — count leading zeros
    pub fn lzcnt_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, eaddr)?
        };
        let result = op2.leading_zeros();
        self.set_gpr32(instr.dst() as usize, result);

        // CF = (op2 == 0), ZF = (result == 0 i.e. op2 has bit 31 set)
        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::AF | EFlags::PF);
        if op2 == 0 {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        if result == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            self.eflags.remove(EFlags::ZF);
        }
        Ok(())
    }

    /// LZCNT r16, r/m16 — count leading zeros (16-bit)
    pub fn lzcnt_gw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize) as u16
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_word(seg, eaddr)?
        };
        let result = op2.leading_zeros() as u16;
        let dst = instr.dst() as usize;
        let current = self.get_gpr32(dst);
        self.set_gpr32(dst, (current & 0xFFFF0000) | result as u32);

        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::AF | EFlags::PF);
        if op2 == 0 {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        if result == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            self.eflags.remove(EFlags::ZF);
        }
        Ok(())
    }

    // =========================================================================
    // TZCNT — Trailing Zero Count (F3 0F BC /r)
    // =========================================================================

    /// TZCNT r32, r/m32 — count trailing zeros
    pub fn tzcnt_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, eaddr)?
        };
        let result = op2.trailing_zeros();
        self.set_gpr32(instr.dst() as usize, result);

        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::AF | EFlags::PF);
        if op2 == 0 {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        if result == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            self.eflags.remove(EFlags::ZF);
        }
        Ok(())
    }

    /// TZCNT r16, r/m16 — count trailing zeros (16-bit)
    pub fn tzcnt_gw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize) as u16
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_word(seg, eaddr)?
        };
        let result = op2.trailing_zeros() as u16;
        let dst = instr.dst() as usize;
        let current = self.get_gpr32(dst);
        self.set_gpr32(dst, (current & 0xFFFF0000) | result as u32);

        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::AF | EFlags::PF);
        if op2 == 0 {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        if result == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            self.eflags.remove(EFlags::ZF);
        }
        Ok(())
    }

    // =========================================================================
    // CRC32 — CRC32C (Castagnoli) (F2 0F 38 F0/F1)
    // =========================================================================

    /// CRC32 r32, r/m8 — CRC32C accumulate byte
    pub fn crc32_gd_eb(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.read_8bit_regx(instr.src() as usize, instr.extend8bit_l())
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_byte(seg, eaddr)?
        };
        let crc = self.get_gpr32(instr.dst() as usize);
        let result = crc32c_byte(crc, op2);
        self.set_gpr32(instr.dst() as usize, result);
        Ok(())
    }

    /// CRC32 r32, r/m32 — CRC32C accumulate dword
    pub fn crc32_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, eaddr)?
        };
        let crc = self.get_gpr32(instr.dst() as usize);
        let result = crc32c_dword(crc, op2);
        self.set_gpr32(instr.dst() as usize, result);
        Ok(())
    }

    /// CRC32 r32, r/m16 — CRC32C accumulate word
    pub fn crc32_gd_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize) as u16
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_word(seg, eaddr)?
        };
        let crc = self.get_gpr32(instr.dst() as usize);
        let result = crc32c_word(crc, op2);
        self.set_gpr32(instr.dst() as usize, result);
        Ok(())
    }

    // =========================================================================
    // MOVBE — Move Big-Endian (0F 38 F0 / 0F 38 F1)
    // =========================================================================

    /// MOVBE r32, m32 — load with byte swap
    pub fn movbe_gd_md(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let val = self.v_read_dword(seg, eaddr)?;
        self.set_gpr32(instr.dst() as usize, val.swap_bytes());
        Ok(())
    }

    /// MOVBE m32, r32 — store with byte swap
    /// Decoder 0F38: dst()=nnn=register, resolve_addr32=memory
    pub fn movbe_md_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = self.get_gpr32(instr.dst() as usize);
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        self.v_write_dword(seg, eaddr, val.swap_bytes())?;
        Ok(())
    }

    /// MOVBE r16, m16 — load with byte swap (16-bit)
    pub fn movbe_gw_mw(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let val = self.v_read_word(seg, eaddr)?;
        let dst = instr.dst() as usize;
        let current = self.get_gpr32(dst);
        self.set_gpr32(dst, (current & 0xFFFF0000) | val.swap_bytes() as u32);
        Ok(())
    }

    /// MOVBE m16, r16 — store with byte swap (16-bit)
    /// Decoder 0F38: dst()=nnn=register, resolve_addr32=memory
    pub fn movbe_mw_gw(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = self.get_gpr32(instr.dst() as usize) as u16;
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        self.v_write_word(seg, eaddr, val.swap_bytes())?;
        Ok(())
    }
}

// =========================================================================
// CRC32C helper functions (Castagnoli polynomial 0x1EDC6F41)
// =========================================================================

/// CRC32C lookup table for single byte
const fn crc32c_table() -> [u32; 256] {
    let poly: u32 = 0x82F6_3B78; // Reversed Castagnoli polynomial
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ poly;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

static CRC32C_TABLE: [u32; 256] = crc32c_table();

fn crc32c_byte(crc: u32, b: u8) -> u32 {
    CRC32C_TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8)
}

fn crc32c_word(crc: u32, w: u16) -> u32 {
    let c = crc32c_byte(crc, w as u8);
    crc32c_byte(c, (w >> 8) as u8)
}

fn crc32c_dword(crc: u32, d: u32) -> u32 {
    let c = crc32c_byte(crc, d as u8);
    let c = crc32c_byte(c, (d >> 8) as u8);
    let c = crc32c_byte(c, (d >> 16) as u8);
    crc32c_byte(c, (d >> 24) as u8)
}
