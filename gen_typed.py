#!/usr/bin/env python3
"""Generate TypedInstruction enum variants and match arms for SIMD opcodes."""

import re
import sys

# Read unmatched opcodes from stdin or file
opcodes = []
with open(sys.argv[1]) as f:
    for line in f:
        line = line.strip()
        if line and not line.startswith('Self::') and line[0].isupper():
            opcodes.append(line)

# Classification rules: (suffix_pattern, macro_name, variant_fields_R, variant_fields_M)
# Order matters - first match wins

# Patterns for operand suffixes
patterns = []

def add_pattern(regex, macro, fields_r, fields_m, r_only=False, m_only=False):
    patterns.append((re.compile(regex + '$'), macro, fields_r, fields_m, r_only, m_only))

# === AMX/Tile opcodes ===
add_pattern(r'TnnnTrmTreg', 'tile3', '{ dst: u8, src1: u8, src2: u8 }', None, r_only=True)
add_pattern(r'TnnnMdq', 'tile_m', None, '{ dst: u8, src: MemoryOperand }', m_only=True)
add_pattern(r'MdqTnnn', 'tile_st_m', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'Tnnn', 'tile1', '{ reg: u8 }', None, r_only=True)

# === Extracts: Ed/Eq,Vdq,Ib with separate R/M variants ===
# PextrbEdVdqIbR, PextrbMbVdqIbM — already split in opcode enum
add_pattern(r'EdVdqIbR', 'none', '{ dst: GprIndex, src: u8, imm: u8 }', None, r_only=True)
add_pattern(r'MbVdqIbM', 'none', None, '{ dst: MemoryOperand, src: u8, imm: u8 }', m_only=True)
add_pattern(r'EdVdqIbM', 'none', None, '{ dst: MemoryOperand, src: u8, imm: u8 }', m_only=True)
add_pattern(r'MwVdqIbM', 'none', None, '{ dst: MemoryOperand, src: u8, imm: u8 }', m_only=True)
add_pattern(r'EdVdqIb', 'simd_extract_ib', '{ dst: GprIndex, src: u8, imm: u8 }',
            '{ dst: MemoryOperand, src: u8, imm: u8 }')
add_pattern(r'EqVdqIb', 'simd_extract_ib', '{ dst: GprIndex, src: u8, imm: u8 }',
            '{ dst: MemoryOperand, src: u8, imm: u8 }')
add_pattern(r'EdVpsIb', 'simd_extract_ib', '{ dst: GprIndex, src: u8, imm: u8 }',
            '{ dst: MemoryOperand, src: u8, imm: u8 }')
# PextrwEdVdqIbR — already split
add_pattern(r'EdVdqIbR', 'none', '{ dst: GprIndex, src: u8, imm: u8 }', None, r_only=True)

# === Inserts: Vdq,Ed/Eb/Ew/Eq,Ib ===
add_pattern(r'VdqEbIb', 'simd_insert_ib', '{ dst: u8, src: GprIndex, imm: u8 }',
            '{ dst: u8, src: MemoryOperand, imm: u8 }')
add_pattern(r'VdqEwIb', 'simd_insert_ib', '{ dst: u8, src: GprIndex, imm: u8 }',
            '{ dst: u8, src: MemoryOperand, imm: u8 }')
add_pattern(r'VdqEdIb', 'simd_insert_ib', '{ dst: u8, src: GprIndex, imm: u8 }',
            '{ dst: u8, src: MemoryOperand, imm: u8 }')
add_pattern(r'VdqEqIb', 'simd_insert_ib', '{ dst: u8, src: GprIndex, imm: u8 }',
            '{ dst: u8, src: MemoryOperand, imm: u8 }')

# === Register-only with immediate (shift by imm) ===
add_pattern(r'NqIb', 'reg_ib', '{ dst: u8, imm: u8 }', None, r_only=True)
add_pattern(r'UdqIb', 'reg_ib', '{ dst: u8, imm: u8 }', None, r_only=True)

# === GPR ← SIMD (movmsk, pextrw old form, etc.) ===
add_pattern(r'GdNqIb', 'gpr_simd_ib', '{ dst: GprIndex, src: u8, imm: u8 }', None, r_only=True)
add_pattern(r'GdUdqIb', 'gpr_simd_ib', '{ dst: GprIndex, src: u8, imm: u8 }', None, r_only=True)
add_pattern(r'GdNq', 'gpr_simd', '{ dst: GprIndex, src: u8 }', None, r_only=True)
add_pattern(r'GdUdq', 'gpr_simd', '{ dst: GprIndex, src: u8 }', None, r_only=True)
add_pattern(r'GdUps', 'gpr_simd', '{ dst: GprIndex, src: u8 }', None, r_only=True)
add_pattern(r'GdUpd', 'gpr_simd', '{ dst: GprIndex, src: u8 }', None, r_only=True)

# === GPR ← SIMD r/m (cvt) ===
add_pattern(r'GdWss', 'simd_gpr_src', '{ dst: GprIndex, src: u8 }', '{ dst: GprIndex, src: MemoryOperand }')
add_pattern(r'GdWsd', 'simd_gpr_src', '{ dst: GprIndex, src: u8 }', '{ dst: GprIndex, src: MemoryOperand }')
add_pattern(r'GqWss', 'simd_gpr_src', '{ dst: GprIndex, src: u8 }', '{ dst: GprIndex, src: MemoryOperand }')
add_pattern(r'GqWsd', 'simd_gpr_src', '{ dst: GprIndex, src: u8 }', '{ dst: GprIndex, src: MemoryOperand }')

# === SIMD ← GPR (cvtsi2ss, movd) ===
add_pattern(r'VssEd', 'simd_dst_gpr', '{ dst: u8, src: GprIndex }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VsdEd', 'simd_dst_gpr', '{ dst: u8, src: GprIndex }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VssEq', 'simd_dst_gpr', '{ dst: u8, src: GprIndex }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VsdEq', 'simd_dst_gpr', '{ dst: u8, src: GprIndex }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VdqEd', 'simd_dst_gpr', '{ dst: u8, src: GprIndex }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VdEd', 'simd_dst_gpr', '{ dst: u8, src: GprIndex }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'PqEd', 'simd_dst_gpr', '{ dst: u8, src: GprIndex }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'EdPq', 'simd_extract', '{ dst: GprIndex, src: u8 }', '{ dst: MemoryOperand, src: u8 }')
add_pattern(r'EdVd', 'simd_extract', '{ dst: GprIndex, src: u8 }', '{ dst: MemoryOperand, src: u8 }')
add_pattern(r'EqPq', 'simd_extract', '{ dst: GprIndex, src: u8 }', '{ dst: MemoryOperand, src: u8 }')

# === Memory-only stores ===
add_pattern(r'MdqVdq', 'simd_m', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'MpdVpd', 'simd_m', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'MpsVps', 'simd_m', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'MqVsd', 'simd_m', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'MqVps', 'simd_m', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'MqPq', 'simd_m', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'MsdVsd', 'simd_m', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'MssVss', 'simd_m', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'MbVdqIb', 'simd_m_ib', None, '{ dst: MemoryOperand, src: u8, imm: u8 }', m_only=True)

# === Memory-only loads ===
add_pattern(r'VdqMdq', 'simd_load_m', None, '{ dst: u8, src: MemoryOperand }', m_only=True)
add_pattern(r'VpdMq', 'simd_load_m', None, '{ dst: u8, src: MemoryOperand }', m_only=True)
add_pattern(r'VsdMq', 'simd_load_m', None, '{ dst: u8, src: MemoryOperand }', m_only=True)
add_pattern(r'VpsMq', 'simd_load_m', None, '{ dst: u8, src: MemoryOperand }', m_only=True)

# === Store direction: W←V, Q←P ===
add_pattern(r'WpdVpd', 'simd_st', '{ dst: u8, src: u8 }', '{ dst: MemoryOperand, src: u8 }')
add_pattern(r'WpsVps', 'simd_st', '{ dst: u8, src: u8 }', '{ dst: MemoryOperand, src: u8 }')
add_pattern(r'WdqVdq', 'simd_st', '{ dst: u8, src: u8 }', '{ dst: MemoryOperand, src: u8 }')
add_pattern(r'WsdVsd', 'simd_st', '{ dst: u8, src: u8 }', '{ dst: MemoryOperand, src: u8 }')
add_pattern(r'WssVss', 'simd_st', '{ dst: u8, src: u8 }', '{ dst: MemoryOperand, src: u8 }')
add_pattern(r'WqVq', 'simd_st', '{ dst: u8, src: u8 }', '{ dst: MemoryOperand, src: u8 }')
add_pattern(r'QqPq', 'simd_st', '{ dst: u8, src: u8 }', '{ dst: MemoryOperand, src: u8 }')

# === SIMD with immediate ===
add_pattern(r'VdqWdqIb', 'simd_ib', '{ dst: u8, src: u8, imm: u8 }', '{ dst: u8, src: MemoryOperand, imm: u8 }')
add_pattern(r'VpsWpsIb', 'simd_ib', '{ dst: u8, src: u8, imm: u8 }', '{ dst: u8, src: MemoryOperand, imm: u8 }')
add_pattern(r'VpdWpdIb', 'simd_ib', '{ dst: u8, src: u8, imm: u8 }', '{ dst: u8, src: MemoryOperand, imm: u8 }')
add_pattern(r'VssWssIb', 'simd_ib', '{ dst: u8, src: u8, imm: u8 }', '{ dst: u8, src: MemoryOperand, imm: u8 }')
add_pattern(r'VsdWsdIb', 'simd_ib', '{ dst: u8, src: u8, imm: u8 }', '{ dst: u8, src: MemoryOperand, imm: u8 }')
add_pattern(r'PqQqIb', 'simd_ib', '{ dst: u8, src: u8, imm: u8 }', '{ dst: u8, src: MemoryOperand, imm: u8 }')

# === Register-only SIMD ===
add_pattern(r'VdqUdq', 'simd_r', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'VdqUq', 'simd_r', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'PqUdq', 'simd_r', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'VdqQq', 'simd_r', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'PqNq', 'simd_r', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'UdqIbIb', 'reg_ib_ib', '{ dst: u8, imm1: u8, imm2: u8 }', None, r_only=True)

# === Standard SIMD 2-operand (load direction) ===
add_pattern(r'PqQq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'PqQd', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VdqWdq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpsWps', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpdWpd', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VssWss', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VsdWsd', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VqWq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpdWq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VqWpd', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpsWdq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VdqWps', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpsWq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VdqWq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VdqWw', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VdqWd', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpsWss', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VsdWss', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VssWsd', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpdWps', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpsWpd', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VdqMq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpdQq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpdWdq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpsWdq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpsQq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')

# === 3-operand VEX: VpsHpsWps, VpdHpdWpd, etc. ===
add_pattern(r'VpsHpsWps', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VpdHpdWpd', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VssHssWss', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VsdHsdWsd', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VdqHdqWdq', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VqqHqqWqq', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VphHphWph', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')

# === 3-operand VEX with immediate ===
add_pattern(r'VpsHpsWpsIb', 'simd3_ib', '{ dst: u8, src1: u8, src2: u8, imm: u8 }',
            '{ dst: u8, src1: u8, src2: MemoryOperand, imm: u8 }')
add_pattern(r'VpdHpdWpdIb', 'simd3_ib', '{ dst: u8, src1: u8, src2: u8, imm: u8 }',
            '{ dst: u8, src1: u8, src2: MemoryOperand, imm: u8 }')
add_pattern(r'VssHssWssIb', 'simd3_ib', '{ dst: u8, src1: u8, src2: u8, imm: u8 }',
            '{ dst: u8, src1: u8, src2: MemoryOperand, imm: u8 }')
add_pattern(r'VsdHsdWsdIb', 'simd3_ib', '{ dst: u8, src1: u8, src2: u8, imm: u8 }',
            '{ dst: u8, src1: u8, src2: MemoryOperand, imm: u8 }')
add_pattern(r'VdqHdqWdqIb', 'simd3_ib', '{ dst: u8, src1: u8, src2: u8, imm: u8 }',
            '{ dst: u8, src1: u8, src2: MemoryOperand, imm: u8 }')

# === 3-operand VEX with GPR source (cvtsi2ss/sd) ===
add_pattern(r'VssHssEd', 'simd3_gpr', '{ dst: u8, src1: u8, src2: GprIndex }',
            '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VsdHsdEd', 'simd3_gpr', '{ dst: u8, src1: u8, src2: GprIndex }',
            '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VssHssEq', 'simd3_gpr', '{ dst: u8, src1: u8, src2: GprIndex }',
            '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VsdHsdEq', 'simd3_gpr', '{ dst: u8, src1: u8, src2: GprIndex }',
            '{ dst: u8, src1: u8, src2: MemoryOperand }')

# === AVX VEX move with H (3-op scalar move) ===
add_pattern(r'VssHssWss', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VsdHsdWsd', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')

# === VEX store: WpsVps, WdqVdq, etc. ===
add_pattern(r'WpsHpsVps', 'simd3_st', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: MemoryOperand, src1: u8, src2: u8 }')

# === VEX memory-only stores ===
add_pattern(r'MpsVps', 'simd_m', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'MdqVdq', 'simd_m', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)

# === VEX MOVD/MOVQ with H ===
add_pattern(r'VdqHdqEd', 'simd3_gpr', '{ dst: u8, src1: u8, src2: GprIndex }',
            '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VdqHdqEq', 'simd3_gpr', '{ dst: u8, src1: u8, src2: GprIndex }',
            '{ dst: u8, src1: u8, src2: MemoryOperand }')

# === VEX memory-only loads ===
add_pattern(r'VdqMdq', 'simd_load_m', None, '{ dst: u8, src: MemoryOperand }', m_only=True)

# === VEX pblendvps etc (4-operand: dst, src1(vvvv), src2(rm), src3(Is4)) ===
# These use imm8 upper bits for 4th operand register
add_pattern(r'VpsHpsWpsIs4', 'simd4', '{ dst: u8, src1: u8, src2: u8, src3: u8 }',
            '{ dst: u8, src1: u8, src2: MemoryOperand, src3: u8 }')
add_pattern(r'VpdHpdWpdIs4', 'simd4', '{ dst: u8, src1: u8, src2: u8, src3: u8 }',
            '{ dst: u8, src1: u8, src2: MemoryOperand, src3: u8 }')
add_pattern(r'VdqHdqWdqIs4', 'simd4', '{ dst: u8, src1: u8, src2: u8, src3: u8 }',
            '{ dst: u8, src1: u8, src2: MemoryOperand, src3: u8 }')

# === VEX GPR ← SIMD ===
add_pattern(r'GdUps', 'gpr_simd', '{ dst: GprIndex, src: u8 }', None, r_only=True)
add_pattern(r'GdUpd', 'gpr_simd', '{ dst: GprIndex, src: u8 }', None, r_only=True)

# === K-mask opcodes ===
add_pattern(r'KgbKhbKeb', 'kmask3', '{ dst: u8, src1: u8, src2: u8 }', None, r_only=True)
add_pattern(r'KgwKhwKew', 'kmask3', '{ dst: u8, src1: u8, src2: u8 }', None, r_only=True)
add_pattern(r'KgdKhdKed', 'kmask3', '{ dst: u8, src1: u8, src2: u8 }', None, r_only=True)
add_pattern(r'KgqKhqKeq', 'kmask3', '{ dst: u8, src1: u8, src2: u8 }', None, r_only=True)
add_pattern(r'KgbKeb', 'kmask2_r', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'KgwKew', 'kmask2_r', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'KgdKed', 'kmask2_r', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'KgqKeq', 'kmask2_r', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'KgbMb', 'kmask_load', None, '{ dst: u8, src: MemoryOperand }', m_only=True)
add_pattern(r'KgwMw', 'kmask_load', None, '{ dst: u8, src: MemoryOperand }', m_only=True)
add_pattern(r'KgdMd', 'kmask_load', None, '{ dst: u8, src: MemoryOperand }', m_only=True)
add_pattern(r'KgqMq', 'kmask_load', None, '{ dst: u8, src: MemoryOperand }', m_only=True)
add_pattern(r'MbKgb', 'kmask_store', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'MwKgw', 'kmask_store', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'MdKgd', 'kmask_store', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'MqKgq', 'kmask_store', None, '{ dst: MemoryOperand, src: u8 }', m_only=True)
add_pattern(r'GdKeb', 'kmask_to_gpr', '{ dst: GprIndex, src: u8 }', None, r_only=True)
add_pattern(r'GdKew', 'kmask_to_gpr', '{ dst: GprIndex, src: u8 }', None, r_only=True)
add_pattern(r'GdKed', 'kmask_to_gpr', '{ dst: GprIndex, src: u8 }', None, r_only=True)
add_pattern(r'GqKeq', 'kmask_to_gpr', '{ dst: GprIndex, src: u8 }', None, r_only=True)
add_pattern(r'KgbGd', 'kmask_from_gpr', '{ dst: u8, src: GprIndex }', None, r_only=True)
add_pattern(r'KgwGd', 'kmask_from_gpr', '{ dst: u8, src: GprIndex }', None, r_only=True)
add_pattern(r'KgdGd', 'kmask_from_gpr', '{ dst: u8, src: GprIndex }', None, r_only=True)
add_pattern(r'KgqGq', 'kmask_from_gpr', '{ dst: u8, src: GprIndex }', None, r_only=True)
add_pattern(r'KgbKebIb', 'kmask_ib', '{ dst: u8, src: u8, imm: u8 }', None, r_only=True)
add_pattern(r'KgwKewIb', 'kmask_ib', '{ dst: u8, src: u8, imm: u8 }', None, r_only=True)
add_pattern(r'KgdKedIb', 'kmask_ib', '{ dst: u8, src: u8, imm: u8 }', None, r_only=True)
add_pattern(r'KgqKeqIb', 'kmask_ib', '{ dst: u8, src: u8, imm: u8 }', None, r_only=True)
add_pattern(r'KgwKhbKeb', 'kmask3', '{ dst: u8, src1: u8, src2: u8 }', None, r_only=True)
add_pattern(r'KgqKhdKed', 'kmask3', '{ dst: u8, src1: u8, src2: u8 }', None, r_only=True)
add_pattern(r'KgdKhwKew', 'kmask3', '{ dst: u8, src1: u8, src2: u8 }', None, r_only=True)
# Kmov reg-to-reg: KebKgb, KgbEb, etc.
add_pattern(r'KebKgb', 'kmask2_r', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'KewKgw', 'kmask2_r', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'KedKgd', 'kmask2_r', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'KeqKgq', 'kmask2_r', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'KgbEb', 'kmask_from_gpr', '{ dst: u8, src: GprIndex }', None, r_only=True)
add_pattern(r'KgwEw', 'kmask_from_gpr', '{ dst: u8, src: GprIndex }', None, r_only=True)
add_pattern(r'KgdEd', 'kmask_from_gpr', '{ dst: u8, src: GprIndex }', None, r_only=True)
add_pattern(r'KgqEq', 'kmask_from_gpr', '{ dst: u8, src: GprIndex }', None, r_only=True)

# === EVEX patterns (with Kmask suffix) ===
# Most EVEX are 3-operand with mask: VpsHpsWps[Kmask]
add_pattern(r'VpsHpsWpsKmask', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VpdHpdWpdKmask', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VssHssWssKmask', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VsdHsdWsdKmask', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VdqHdqWdqKmask', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'Kmask$', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')

# === Mixed scalar 3-operand VEX: VsdHpdWsd, VssHpsWss (scalar ops with packed H) ===
add_pattern(r'VsdHpdWsd', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VssHpsWss', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
# FMA scalar: VpdHsdWsd, VpsHssWss
add_pattern(r'VpdHsdWsd', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VpsHssWss', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
# FMA3 with Ib
add_pattern(r'VsdHpdWsdIb', 'simd3_ib', '{ dst: u8, src1: u8, src2: u8, imm: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand, imm: u8 }')
add_pattern(r'VssHpsWssIb', 'simd3_ib', '{ dst: u8, src1: u8, src2: u8, imm: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand, imm: u8 }')

# === V128/V256 prefixed opcodes (length-specific variants) ===
# These have the V128/V256 prefix but otherwise follow normal VEX patterns.
# The prefix is part of the name — treat as a single opcode name.
# V128VmovhpdVpdHpdMq — memory-only load with 3 operands
add_pattern(r'VpdHpdMq', 'simd3_m', None, '{ dst: u8, src1: u8, src2: MemoryOperand }', m_only=True)
add_pattern(r'VpsHpsMq', 'simd3_m', None, '{ dst: u8, src1: u8, src2: MemoryOperand }', m_only=True)
add_pattern(r'EqVq', 'simd_extract', '{ dst: GprIndex, src: u8 }', '{ dst: MemoryOperand, src: u8 }')
# V128VmovsdVsdHpdWsd — 3-operand scalar move
add_pattern(r'VsdHpdWsd$', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VssHpsWss$', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
# V128VmovsdWsdHpdVsd — 3-operand scalar store
add_pattern(r'WsdHpdVsd', 'simd3_st', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: MemoryOperand, src1: u8, src2: u8 }')
add_pattern(r'WssHpsVss', 'simd3_st', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: MemoryOperand, src1: u8, src2: u8 }')
# V256VbroadcastsdVpdMsd/Wsd
add_pattern(r'VpdMsd', 'simd_load_m', None, '{ dst: u8, src: MemoryOperand }', m_only=True)
add_pattern(r'VpdWsd', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpsWss', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')

# === FMA4 4-operand VEX (XOP encoding) ===
# VfmaddpdVpdHpdVibWpd: dst=nnn, src1=vvvv, src2=Is4(imm8[7:4]), src3=rm
add_pattern(r'VpdHpdVibWpd', 'simd4_vibw', '{ dst: u8, src1: u8, src2: u8, src3: u8 }', '{ dst: u8, src1: u8, src2: u8, src3: MemoryOperand }')
add_pattern(r'VpsHpsVibWps', 'simd4_vibw', '{ dst: u8, src1: u8, src2: u8, src3: u8 }', '{ dst: u8, src1: u8, src2: u8, src3: MemoryOperand }')
add_pattern(r'VsdHsdVibWsd', 'simd4_vibw', '{ dst: u8, src1: u8, src2: u8, src3: u8 }', '{ dst: u8, src1: u8, src2: u8, src3: MemoryOperand }')
add_pattern(r'VssHssVibWss', 'simd4_vibw', '{ dst: u8, src1: u8, src2: u8, src3: u8 }', '{ dst: u8, src1: u8, src2: u8, src3: MemoryOperand }')
# VfmaddpdVpdHpdWpdVib: dst=nnn, src1=vvvv, src2=rm, src3=Is4(imm8[7:4])
add_pattern(r'VpdHpdWpdVib', 'simd4_wvib', '{ dst: u8, src1: u8, src2: u8, src3: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand, src3: u8 }')
add_pattern(r'VpsHpsWpsVib', 'simd4_wvib', '{ dst: u8, src1: u8, src2: u8, src3: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand, src3: u8 }')
add_pattern(r'VsdHsdWsdVib', 'simd4_wvib', '{ dst: u8, src1: u8, src2: u8, src3: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand, src3: u8 }')
add_pattern(r'VssHssWssVib', 'simd4_wvib', '{ dst: u8, src1: u8, src2: u8, src3: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand, src3: u8 }')

# === XOP/FMA4 4-operand (VdqHdqVibWdq / VdqHdqWdqVib) ===
add_pattern(r'VdqHdqVibWdq', 'simd4', '{ dst: u8, src1: u8, src2: u8, src3: u8 }', '{ dst: u8, src1: u8, src2: u8, src3: MemoryOperand }')
add_pattern(r'VdqHdqWdqVib', 'simd4_rev', '{ dst: u8, src1: u8, src2: u8, src3: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand, src3: u8 }')
# V128VmovqVdqEq
add_pattern(r'VdqEq', 'simd_dst_gpr', '{ dst: u8, src: GprIndex }', '{ dst: u8, src: MemoryOperand }')

# === Gather (VEX VSIB): 3 operands — dst=nnn, mask=vvvv, addr(vsib)=rm ===
add_pattern(r'VdqHdq$', 'gather', '{ dst: u8, mask: u8 }', None, r_only=True)
add_pattern(r'VpdHpd$', 'gather', '{ dst: u8, mask: u8 }', None, r_only=True)
add_pattern(r'VpsHps$', 'gather', '{ dst: u8, mask: u8 }', None, r_only=True)

# === Masked move (VEX): VmaskmovdVdqHdqMdq, VmaskmovdMdqHdqVdq ===
add_pattern(r'VdqHdqMdq', 'maskmov_load', None, '{ dst: u8, mask: u8, src: MemoryOperand }', m_only=True)
add_pattern(r'MdqHdqVdq', 'maskmov_store', None, '{ dst: MemoryOperand, mask: u8, src: u8 }', m_only=True)
add_pattern(r'VpdHpdMpd', 'maskmov_load', None, '{ dst: u8, mask: u8, src: MemoryOperand }', m_only=True)
add_pattern(r'MpdHpdVpd', 'maskmov_store', None, '{ dst: MemoryOperand, mask: u8, src: u8 }', m_only=True)
add_pattern(r'VpsHpsMps', 'maskmov_load', None, '{ dst: u8, mask: u8, src: MemoryOperand }', m_only=True)
add_pattern(r'MpsHpsVps', 'maskmov_store', None, '{ dst: MemoryOperand, mask: u8, src: u8 }', m_only=True)

# === XOP (vprot/vpsha/vpshl): VdqWdqHdq = dst=nnn, src1=rm, src2=vvvv ===
add_pattern(r'VdqWdqHdq', 'xop3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: MemoryOperand, src2: u8 }')

# === Broadcast ===
add_pattern(r'VpsMss', 'simd_load_m', None, '{ dst: u8, src: MemoryOperand }', m_only=True)
add_pattern(r'VdqWb', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')

# === Convert ===
add_pattern(r'VdqWpd', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VphWps', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpsWph', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpsWw', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpsWsh', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')

# === VEX GPR ← SIMD conversions ===
add_pattern(r'VssHssEd', 'simd3_gpr', '{ dst: u8, src1: u8, src2: GprIndex }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VsdHsdEd', 'simd3_gpr', '{ dst: u8, src1: u8, src2: GprIndex }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VssHssEq', 'simd3_gpr', '{ dst: u8, src1: u8, src2: GprIndex }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VsdHsdEq', 'simd3_gpr', '{ dst: u8, src1: u8, src2: GprIndex }', '{ dst: u8, src1: u8, src2: MemoryOperand }')

# === FP16 (half-precision) EVEX patterns ===
# FMA scalar half: VphHshWsh (3 operands)
add_pattern(r'VphHshWsh', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VshHphWsh', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'VshWsh', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'WshHphVsh', 'simd3_st', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: MemoryOperand, src1: u8, src2: u8 }')
add_pattern(r'WshVsh', 'simd_st', '{ dst: u8, src: u8 }', '{ dst: MemoryOperand, src: u8 }')
add_pattern(r'VshEd', 'simd_dst_gpr', '{ dst: u8, src: GprIndex }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VshEq', 'simd_dst_gpr', '{ dst: u8, src: GprIndex }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VshEw', 'simd_dst_gpr', '{ dst: u8, src: GprIndex }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'EdVsh', 'simd_extract', '{ dst: GprIndex, src: u8 }', '{ dst: MemoryOperand, src: u8 }')

# === EVEX conversion patterns ===
add_pattern(r'V8bWph', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'V8bWps', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VphWph', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VdqWph', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpdWph', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VphWdq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VssWsh', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VsdWsh', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')

# === EVEX VSIB (gather/scatter) ===
add_pattern(r'VdqVsib', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpdVsib', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VpsVsib', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VsibVdq', 'simd_st', '{ dst: u8, src: u8 }', '{ dst: MemoryOperand, src: u8 }')
add_pattern(r'VsibVpd', 'simd_st', '{ dst: u8, src: u8 }', '{ dst: MemoryOperand, src: u8 }')
add_pattern(r'VsibVps', 'simd_st', '{ dst: u8, src: u8 }', '{ dst: MemoryOperand, src: u8 }')

# === EVEX mask compare (result to K register) ===
add_pattern(r'KgbHdqWdq', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'KgwHdqWdq', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'KgdHdqWdq', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')
add_pattern(r'KgqHdqWdq', 'simd3', '{ dst: u8, src1: u8, src2: u8 }', '{ dst: u8, src1: u8, src2: MemoryOperand }')

# === EVEX insert GPR → XMM ===
add_pattern(r'VdqEb', 'simd_dst_gpr', '{ dst: u8, src: GprIndex }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VdqEw', 'simd_dst_gpr', '{ dst: u8, src: GprIndex }', '{ dst: u8, src: MemoryOperand }')

# === EVEX 32-bit data move ===
add_pattern(r'VdWd', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'WdVd', 'simd_st', '{ dst: u8, src: u8 }', '{ dst: MemoryOperand, src: u8 }')

# === EVEX mask extract: Vpmovb2m/d2m/q2m/w2m → KgqWdq ===
add_pattern(r'KgqWdq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'KgwWdq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'KgbWdq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'KgdWdq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')

# === EVEX mask expand ===
add_pattern(r'VdqKeb', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VdqKew', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VdqKed', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')
add_pattern(r'VdqKeq', 'simd', '{ dst: u8, src: u8 }', '{ dst: u8, src: MemoryOperand }')

# === EVEX AMX tile ops ===
add_pattern(r'VpsTrmBd', 'tile_row', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'VphTrmBd', 'tile_row', '{ dst: u8, src: u8 }', None, r_only=True)
add_pattern(r'VdqTrmBd', 'tile_row', '{ dst: u8, src: u8 }', None, r_only=True)

# === Catch-all for remaining SIMD with Ib ===
add_pattern(r'Ib$', 'simd_ib', '{ dst: u8, src: u8, imm: u8 }', '{ dst: u8, src: MemoryOperand, imm: u8 }')

# === Catch-all for GwEw (non-SIMD like LAR, LSL) ===
add_pattern(r'GwEw', 'gx_ex', '{ dst: GprIndex, src: GprIndex }', '{ dst: GprIndex, src: MemoryOperand }')

# === No-operand ===
# These are identified by name, not suffix

NO_OPERAND = {
    'Femms', 'Emms', 'Sfence', 'Lfence', 'Mfence',
    'Tilerelease', 'Vzeroall', 'Vzeroupper',
}

MEMORY_ONLY_SINGLE = {
    'Ldmxcsr': '{ src: MemoryOperand }',
    'Stmxcsr': '{ dst: MemoryOperand }',
    'Ldtilecfg': '{ src: MemoryOperand }',
    'Sttilecfg': '{ dst: MemoryOperand }',
    'Vldmxcsr': '{ src: MemoryOperand }',
    'Vstmxcsr': '{ dst: MemoryOperand }',
}

MEMORY_ONLY_PREFETCH = {
    'PrefetchMb', 'PrefetchntaMb', 'Prefetcht0Mb', 'Prefetcht1Mb', 'Prefetcht2Mb',
}

MEMORY_ONLY_LOAD = {
    'LddquVdqMdq', 'MovntdqaVdqMdq',
}

def classify(opcode):
    """Classify an opcode and return (macro, fields_r, fields_m, r_only, m_only)."""
    if opcode in NO_OPERAND:
        return ('no_op', None, None, False, False)
    if opcode in MEMORY_ONLY_LOAD:
        return ('simd_load_m', None, '{ dst: u8, src: MemoryOperand }', False, True)
    if opcode in MEMORY_ONLY_SINGLE:
        return ('mem_single', None, MEMORY_ONLY_SINGLE[opcode], False, True)
    if opcode in MEMORY_ONLY_PREFETCH:
        return ('prefetch', None, '{ mem: MemoryOperand }', False, True)

    for regex, macro, fields_r, fields_m, r_only, m_only in patterns:
        if regex.search(opcode):
            return (macro, fields_r, fields_m, r_only, m_only)

    return ('UNKNOWN', None, None, False, False)

# Generate output
enum_variants = []
match_arms = []
unknown = []

for opcode in opcodes:
    macro, fields_r, fields_m, r_only, m_only = classify(opcode)

    if macro == 'UNKNOWN':
        unknown.append(opcode)
        continue

    if macro == 'no_op':
        enum_variants.append(f"    {opcode},")
        match_arms.append(f"            O::{opcode} => T::{opcode},")
        continue

    if r_only:
        # Single variant, register-only
        enum_variants.append(f"    {opcode} {fields_r},")
        # Match arm depends on macro type
        if macro == 'tile3':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, src1: self.operands.src2, src2: self.operands.src1 }},")
        elif macro == 'tile1':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ reg: self.operands.dst }},")
        elif macro == 'reg_ib':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, imm: self.ib() }},")
        elif macro == 'reg_ib_ib':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, imm1: self.ib(), imm2: self.ib2() }},")
        elif macro == 'gpr_simd':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.dst_reg(), src: self.operands.src1 }},")
        elif macro == 'gpr_simd_ib':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.dst_reg(), src: self.operands.src1, imm: self.ib() }},")
        elif macro == 'simd_r':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, src: self.operands.src1 }},")
        elif macro == 'kmask3':
            match_arms.append(f"            O::{opcode} => kmask3!({opcode}, self),")
        elif macro == 'kmask2_r':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, src: self.operands.src1 }},")
        elif macro == 'kmask_to_gpr':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.dst_reg(), src: self.operands.src1 }},")
        elif macro == 'kmask_from_gpr':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, src: self.src1_reg() }},")
        elif macro == 'kmask_ib':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, src: self.operands.src1, imm: self.ib() }},")
        elif macro == 'gather':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, mask: self.operands.src2 }},")
        elif macro == 'tile_row':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, src: self.operands.src1 }},")
        elif macro == 'none':
            # Already split R variant
            if 'GprIndex' in fields_r:
                match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.dst_reg(), src: self.operands.src1, imm: self.ib() }},")
            else:
                match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, src: self.operands.src1, imm: self.ib() }},")
        else:
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, src: self.operands.src1 }},")
    elif m_only:
        # Single variant, memory-only
        enum_variants.append(f"    {opcode} {fields_m},")
        if macro == 'prefetch':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ mem: self.memory_operand() }},")
        elif macro == 'mem_single':
            if 'src' in fields_m:
                match_arms.append(f"            O::{opcode} => T::{opcode} {{ src: self.memory_operand() }},")
            else:
                match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.memory_operand() }},")
        elif macro in ('simd_m', 'kmask_store', 'tile_st_m'):
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.memory_operand(), src: self.operands.src1 }},")
        elif macro in ('simd_load_m', 'kmask_load', 'tile_m'):
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, src: self.memory_operand() }},")
        elif macro == 'simd_m_ib':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.memory_operand(), src: self.operands.src1, imm: self.ib() }},")
        elif macro == 'simd3_m':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, src1: self.operands.src2, src2: self.memory_operand() }},")
        elif macro == 'maskmov_load':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, mask: self.operands.src2, src: self.memory_operand() }},")
        elif macro == 'maskmov_store':
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.memory_operand(), mask: self.operands.src2, src: self.operands.src1 }},")
        elif macro == 'none':
            # Already split M variant (extract to memory)
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.memory_operand(), src: self.operands.src1, imm: self.ib() }},")
        else:
            match_arms.append(f"            O::{opcode} => T::{opcode} {{ dst: self.operands.dst, src: self.memory_operand() }},")
    else:
        # Has both R and M forms — generate two variants
        r_name = f"{opcode}R"
        m_name = f"{opcode}M"
        enum_variants.append(f"    {r_name} {fields_r},")
        enum_variants.append(f"    {m_name} {fields_m},")

        if macro == 'simd':
            match_arms.append(f"            O::{opcode} => simd!({r_name}, {m_name}, self),")
        elif macro == 'simd_st':
            match_arms.append(f"            O::{opcode} => simd_st!({r_name}, {m_name}, self),")
        elif macro == 'simd_ib':
            match_arms.append(f"            O::{opcode} => simd_ib!({r_name}, {m_name}, self),")
        elif macro == 'simd3':
            match_arms.append(f"            O::{opcode} => simd3!({r_name}, {m_name}, self),")
        elif macro == 'simd3_ib':
            match_arms.append(f"            O::{opcode} => simd3_ib!({r_name}, {m_name}, self),")
        elif macro == 'simd3_gpr':
            match_arms.append(f"            O::{opcode} => simd3_gpr!({r_name}, {m_name}, self),")
        elif macro == 'simd_gpr_src':
            match_arms.append(f"            O::{opcode} => simd_gpr_src!({r_name}, {m_name}, self),")
        elif macro == 'simd_dst_gpr':
            match_arms.append(f"            O::{opcode} => simd_dst_gpr!({r_name}, {m_name}, self),")
        elif macro == 'simd_extract':
            match_arms.append(f"            O::{opcode} => if self.mod_c0() {{ T::{r_name} {{ dst: self.dst_reg(), src: self.operands.src1 }} }} else {{ T::{m_name} {{ dst: self.memory_operand(), src: self.operands.src1 }} }},")
        elif macro == 'simd_extract_ib':
            match_arms.append(f"            O::{opcode} => if self.mod_c0() {{ T::{r_name} {{ dst: self.dst_reg(), src: self.operands.src1, imm: self.ib() }} }} else {{ T::{m_name} {{ dst: self.memory_operand(), src: self.operands.src1, imm: self.ib() }} }},")
        elif macro == 'simd_insert_ib':
            match_arms.append(f"            O::{opcode} => if self.mod_c0() {{ T::{r_name} {{ dst: self.operands.dst, src: self.src1_reg(), imm: self.ib() }} }} else {{ T::{m_name} {{ dst: self.operands.dst, src: self.memory_operand(), imm: self.ib() }} }},")
        elif macro == 'gx_ex':
            match_arms.append(f"            O::{opcode} => gx_ex!({r_name}, {m_name}, self),")
        elif macro in ('simd4', 'simd4_vibw'):
            # VibW: dst=nnn, src1=vvvv, src2=Is4(src3), src3=rm(src1) — M form: src3=memory
            match_arms.append(f"            O::{opcode} => if self.mod_c0() {{ T::{r_name} {{ dst: self.operands.dst, src1: self.operands.src2, src2: self.operands.src3, src3: self.operands.src1 }} }} else {{ T::{m_name} {{ dst: self.operands.dst, src1: self.operands.src2, src2: self.operands.src3, src3: self.memory_operand() }} }},")
        elif macro in ('simd4_rev', 'simd4_wvib'):
            # WVib: dst=nnn, src1=vvvv, src2=rm(src1), src3=Is4(src3) — M form: src2=memory
            match_arms.append(f"            O::{opcode} => if self.mod_c0() {{ T::{r_name} {{ dst: self.operands.dst, src1: self.operands.src2, src2: self.operands.src1, src3: self.operands.src3 }} }} else {{ T::{m_name} {{ dst: self.operands.dst, src1: self.operands.src2, src2: self.memory_operand(), src3: self.operands.src3 }} }},")
        elif macro == 'simd3_st':
            match_arms.append(f"            O::{opcode} => if self.mod_c0() {{ T::{r_name} {{ dst: self.operands.dst, src1: self.operands.src2, src2: self.operands.src1 }} }} else {{ T::{m_name} {{ dst: self.memory_operand(), src1: self.operands.src2, src2: self.operands.src1 }} }},")
        elif macro == 'xop3':
            match_arms.append(f"            O::{opcode} => if self.mod_c0() {{ T::{r_name} {{ dst: self.operands.dst, src1: self.operands.src1, src2: self.operands.src2 }} }} else {{ T::{m_name} {{ dst: self.operands.dst, src1: self.memory_operand(), src2: self.operands.src2 }} }},")
        elif macro == 'kmask2':
            match_arms.append(f"            O::{opcode} => kmask2!({r_name}, {m_name}, self),")
        else:
            match_arms.append(f"            // TODO: {opcode} ({macro})")

# Output
print("// ========== ENUM VARIANTS ==========")
for v in enum_variants:
    print(v)

print("\n// ========== MATCH ARMS ==========")
for a in match_arms:
    print(a)

if unknown:
    print(f"\n// ========== UNKNOWN ({len(unknown)}) ==========")
    for u in unknown:
        print(f"// UNKNOWN: {u}")
