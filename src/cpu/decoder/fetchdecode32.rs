use crate::cpu::{
    cpuid::BxCpuIdTrait,
    decoder::{
        fetchdecode::{fetch_dword, fetch_word},
        fetchdecode_generated::{
            BxDecodeError, BxModrm, AS32_OFFSET, MODC0_OFFSET, NNN_OFFSET, OS32_OFFSET, RRR_OFFSET,
            SRC_EQ_DST_OFFSET, SSE_PREFIX_OFFSET,
        },
        fetchdecode_opmap_0f38::BxOpcodeTable0F38,
        fetchdecode_x87::Bx3DNowOpcode,
        DecodeError, BX_NIL_REGISTER,
    },
    BxCpuC,
};

use super::{
    fetchdecode::SsePrefix, fetchdecode_opmap::*, fetchdecode_opmap_0f3a::BxOpcodeTable0F3A,
    ia_opcodes::Opcode, instr::MetaInfoFlags, instr_generated::BxInstructionGenerated, BxRegs16,
    BxSegregs, DecodeResult,
};

// Define the function pointer type
type BxFetchDecode32Ptr = for<'a> fn(
    iptr: &'a [u8], // Slice of Bit8u
    //remain: &mut usize,                   // Mutable reference to unsigned (using u32 for unsigned)
    //i: &mut BxInstruction,         // Mutable reference to bxInstruction_c
    i: &mut BxInstructionGenerated,
    b1: u32,                              // Unsigned integer (using u32)
    sse_prefix: Option<SsePrefix>,        // Unsigned integer (using u32)
    opcode_table: Option<&'static [u64]>, // Slice of u8 instead of a pointer to void
) -> DecodeResult<(Opcode, &'a [u8])>;

// Some info on the opcodes at {0F A6} and {0F A7}
//
// On 386 steps A0-B0:
//   {OF A6} = XBTS
//   {OF A7} = IBTS
// On 486 steps A0-B0:
//   {OF A6} = CMPXCHG 8
//   {OF A7} = CMPXCHG 16|32
//
// On 486 >= B steps, and further processors, the
// CMPXCHG instructions were moved to opcodes:
//   {OF B0} = CMPXCHG 8
//   {OF B1} = CMPXCHG 16|32

pub(super) const DECODE32_DESCRIPTOR: [BxOpcodeDecodeDescriptor32; 512] = [
    /*    00 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable00),
    },
    /*    01 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable01),
    },
    /*    02 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable02),
    },
    /*    03 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable03),
    },
    /*    04 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable04),
    },
    /*    05 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable05),
    },
    /*    06 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable06),
    },
    /*    07 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable07),
    },
    /*    08 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable08),
    },
    /*    09 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable09),
    },
    /*    0A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0A),
    },
    /*    0B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0B),
    },
    /*    0C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0C),
    },
    /*    0D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0D),
    },
    /*    0E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0E),
    },
    /*    0F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    }, // 2-byte escape
    /*    10 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable10),
    },
    /*    11 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable11),
    },
    /*    12 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable12),
    },
    /*    13 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable13),
    },
    /*    14 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable14),
    },
    /*    15 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable15),
    },
    /*    16 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable16),
    },
    /*    17 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable17),
    },
    /*    18 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable18),
    },
    /*    19 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable19),
    },
    /*    1A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable1A),
    },
    /*    1B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable1B),
    },
    /*    1C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable1C),
    },
    /*    1D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable1D),
    },
    /*    1E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable1E),
    },
    /*    1F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable1F),
    },
    /*    20 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable20),
    },
    /*    21 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable21),
    },
    /*    22 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable22),
    },
    /*    23 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable23),
    },
    /*    24 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable24),
    },
    /*    25 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable25),
    },
    /*    26 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    }, // ES:
    /*    27 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable27),
    },
    /*    28 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable28),
    },
    /*    29 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable29),
    },
    /*    2A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable2A),
    },
    /*    2B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable2B),
    },
    /*    2C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable2C),
    },
    /*    2D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable2D),
    },
    /*    2E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    }, // CS:
    /*    2F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable2F),
    },
    /*    30 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable30),
    },
    /*    31 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable31),
    },
    /*    32 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable32),
    },
    /*    33 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable33),
    },
    /*    34 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable34),
    },
    /*    35 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable35),
    },
    /*    36 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    }, // SS:
    /*    37 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable37),
    },
    /*    38 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable38),
    },
    /*    39 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable39),
    },
    /*    3A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable3A),
    },
    /*    3B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable3B),
    },
    /*    3C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable3C),
    },
    /*    3D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable3D),
    },
    /*    3E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    }, // DS:
    /*    3F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable3F),
    },
    /*    40 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable40x47),
    },
    /*    41 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable40x47),
    },
    /*    42 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable40x47),
    },
    /*    43 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable40x47),
    },
    /*    44 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable40x47),
    },
    /*    45 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable40x47),
    },
    /*    46 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable40x47),
    },
    /*    47 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable40x47),
    },
    /*    48 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable48x4F),
    },
    /*    49 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable48x4F),
    },
    /*    4A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable48x4F),
    },
    /*    4B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable48x4F),
    },
    /*    4C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable48x4F),
    },
    /*    4D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable48x4F),
    },
    /*    4E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable48x4F),
    },
    /*    4F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable48x4F),
    },
    /*    50 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable50x57),
    },
    /*    51 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable50x57),
    },
    /*    52 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable50x57),
    },
    /*    53 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable50x57),
    },
    /*    54 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable50x57),
    },
    /*    55 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable50x57),
    },
    /*    56 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable50x57),
    },
    /*    57 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable50x57),
    },
    /*    58 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable58x5F),
    },
    /*    59 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable58x5F),
    },
    /*    5A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable58x5F),
    },
    /*    5B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable58x5F),
    },
    /*    5C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable58x5F),
    },
    /*    5D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable58x5F),
    },
    /*    5E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable58x5F),
    },
    /*    5F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable58x5F),
    },
    /*    60 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable60),
    },
    /*    61 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable61),
    },
    /*    62 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_evex32,
        opcode_table: &Some(&BxOpcodeTable62),
    }, // EVEX prefix
    /*    63 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable63_32),
    },
    /*    64 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    }, // FS:
    /*    65 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    }, // GS:
    /*    66 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    }, // OSIZE:
    /*    67 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    }, // ASIZE:
    /*    68 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable68),
    },
    /*    69 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable69),
    },
    /*    6A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable6A),
    },
    /*    6B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable6B),
    },
    /*    6C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable6C),
    },
    /*    6D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable6D),
    },
    /*    6E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable6E),
    },
    /*    6F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable6F),
    },
    /*    70 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable70_32),
    },
    /*    71 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable71_32),
    },
    /*    72 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable72_32),
    },
    /*    73 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable73_32),
    },
    /*    74 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable74_32),
    },
    /*    75 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable75_32),
    },
    /*    76 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable76_32),
    },
    /*    77 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable77_32),
    },
    /*    78 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable78_32),
    },
    /*    79 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable79_32),
    },
    /*    7A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable7A_32),
    },
    /*    7B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable7B_32),
    },
    /*    7C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable7C_32),
    },
    /*    7D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable7D_32),
    },
    /*    7E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable7E_32),
    },
    /*    7F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable7F_32),
    },
    /*    80 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable80),
    },
    /*    81 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable81),
    },
    /*    82 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable80),
    }, // opcode 0x82 is copy of the 0x80
    /*    83 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable83),
    },
    /*    84 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable84),
    },
    /*    85 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable85),
    },
    /*    86 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable86),
    },
    /*    87 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable87),
    },
    /*    88 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable88),
    },
    /*    89 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable89),
    },
    /*    8A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable8A),
    },
    /*    8B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable8B),
    },
    /*    8C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable8C),
    },
    /*    8D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable8D),
    },
    /*    8E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable8E),
    },
    /*    8F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_xop32,
        opcode_table: &Some(&BxOpcodeTable8F),
    }, // XOP prefix
    /*    90 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_nop,
        opcode_table: &None,
    },
    /*    91 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable90x97),
    },
    /*    92 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable90x97),
    },
    /*    93 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable90x97),
    },
    /*    94 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable90x97),
    },
    /*    95 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable90x97),
    },
    /*    96 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable90x97),
    },
    /*    97 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable90x97),
    },
    /*    98 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable98),
    },
    /*    99 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable99),
    },
    /*    9A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable9A),
    },
    /*    9B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable9B),
    },
    /*    9C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable9C),
    },
    /*    9D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable9D),
    },
    /*    9E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable9E_32),
    },
    /*    9F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable9F_32),
    },
    /*    A0 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableA0_32),
    },
    /*    A1 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableA1_32),
    },
    /*    A2 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableA2_32),
    },
    /*    A3 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableA3_32),
    },
    /*    A4 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableA4),
    },
    /*    A5 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableA5),
    },
    /*    A6 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableA6),
    },
    /*    A7 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableA7),
    },
    /*    A8 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableA8),
    },
    /*    A9 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableA9),
    },
    /*    AA */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableAA),
    },
    /*    AB */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableAB),
    },
    /*    AC */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableAC),
    },
    /*    AD */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableAD),
    },
    /*    AE */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableAE),
    },
    /*    AF */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableAF),
    },
    /*    B0 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB0xB7),
    },
    /*    B1 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB0xB7),
    },
    /*    B2 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB0xB7),
    },
    /*    B3 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB0xB7),
    },
    /*    B4 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB0xB7),
    },
    /*    B5 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB0xB7),
    },
    /*    B6 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB0xB7),
    },
    /*    B7 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB0xB7),
    },
    /*    B8 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB8xBF),
    },
    /*    B9 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB8xBF),
    },
    /*    BA */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB8xBF),
    },
    /*    BB */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB8xBF),
    },
    /*    BC */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB8xBF),
    },
    /*    BD */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB8xBF),
    },
    /*    BE */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB8xBF),
    },
    /*    BF */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableB8xBF),
    },
    /*    C0 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableC0),
    },
    /*    C1 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableC1),
    },
    /*    C2 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableC2_32),
    },
    /*    C3 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableC3_32),
    },
    /*    C4 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_vex32,
        opcode_table: &Some(&BxOpcodeTableC4_32),
    }, // VEX prefix
    /*    C5 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_vex32,
        opcode_table: &Some(&BxOpcodeTableC5_32),
    }, // VEX prefix
    /*    C6 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableC6),
    },
    /*    C7 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableC7),
    },
    /*    C8 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableC8_32),
    },
    /*    C9 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableC9_32),
    },
    /*    CA */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableCA),
    },
    /*    CB */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableCB),
    },
    /*    CC */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTableCC),
    },
    /*    CD */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableCD),
    },
    /*    CE */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTableCE),
    },
    /*    CF */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableCF_32),
    },
    /*    D0 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableD0),
    },
    /*    D1 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableD1),
    },
    /*    D2 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableD2),
    },
    /*    D3 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableD3),
    },
    /*    D4 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableD4),
    },
    /*    D5 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableD5),
    },
    /*    D6 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTableD6),
    },
    /*    D7 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTableD7),
    },
    /*    D8 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_fp_escape,
        opcode_table: &None,
    },
    /*    D9 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_fp_escape,
        opcode_table: &None,
    },
    /*    DA */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_fp_escape,
        opcode_table: &None,
    },
    /*    DB */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_fp_escape,
        opcode_table: &None,
    },
    /*    DC */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_fp_escape,
        opcode_table: &None,
    },
    /*    DD */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_fp_escape,
        opcode_table: &None,
    },
    /*    DE */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_fp_escape,
        opcode_table: &None,
    },
    /*    DF */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_fp_escape,
        opcode_table: &None,
    },
    /*    E0 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableE0_32),
    },
    /*    E1 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableE1_32),
    },
    /*    E2 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableE2_32),
    },
    /*    E3 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableE3_32),
    },
    /*    E4 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableE4),
    },
    /*    E5 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableE5),
    },
    /*    E6 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableE6),
    },
    /*    E7 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableE7),
    },
    /*    E8 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableE8_32),
    },
    /*    E9 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableE9_32),
    },
    /*    EA */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableEA_32),
    },
    /*    EB */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableEB_32),
    },
    /*    EC */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableEC),
    },
    /*    ED */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableED),
    },
    /*    EE */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableEE),
    },
    /*    EF */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTableEF),
    },
    /*    F0 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    }, // LOCK:
    /*    F1 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTableF1),
    },
    /*    F2 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    }, // REPNE/REPNZ
    /*    F3 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    }, // REP, REPE/REPZ
    /*    F4 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTableF4),
    },
    /*    F5 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTableF5),
    },
    /*    F6 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableF6),
    },
    /*    F7 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableF7),
    },
    /*    F8 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTableF8),
    },
    /*    F9 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTableF9),
    },
    /*    FA */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTableFA),
    },
    /*    FB */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTableFB),
    },
    /*    FC */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTableFC),
    },
    /*    FD */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTableFD),
    },
    /*    FE */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableFE),
    },
    /*    FF */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableFF),
    },
    /* 0F 00 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F00),
    },
    /* 0F 01 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F01),
    },
    /* 0F 02 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F02),
    },
    /* 0F 03 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F03),
    },
    /* 0F 04 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 05 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0F05_32),
    },
    /* 0F 06 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0F06),
    },
    /* 0F 07 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0F07_32),
    },
    /* 0F 08 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0F08),
    },
    /* 0F 09 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0F09),
    },
    /* 0F 0A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 0B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0F0B),
    },
    /* 0F 0C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 0D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F0D),
    },
    /* 0F 0E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0F0E),
    },
    /* 0F 0F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_3dnow,
        opcode_table: &None,
    },
    /* 0F 10 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F10),
    },
    /* 0F 11 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F11),
    },
    /* 0F 12 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F12),
    },
    /* 0F 13 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F13),
    },
    /* 0F 14 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F14),
    },
    /* 0F 15 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F15),
    },
    /* 0F 16 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F16),
    },
    /* 0F 17 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F17),
    },
    /* 0F 18 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F18),
    },
    /* 0F 19 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableMultiByteNOP),
    },
    /* 0F 1A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableMultiByteNOP),
    },
    /* 0F 1B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableMultiByteNOP),
    },
    /* 0F 1C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableMultiByteNOP),
    },
    /* 0F 1D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableMultiByteNOP),
    },
    /* 0F 1E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F1E),
    },
    /* 0F 1F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTableMultiByteNOP),
    },
    /* 0F 20 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_creg32,
        opcode_table: &Some(&BxOpcodeTable0F20_32),
    },
    /* 0F 21 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_creg32,
        opcode_table: &Some(&BxOpcodeTable0F21_32),
    },
    /* 0F 22 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_creg32,
        opcode_table: &Some(&BxOpcodeTable0F22_32),
    },
    /* 0F 23 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_creg32,
        opcode_table: &Some(&BxOpcodeTable0F23_32),
    },
    /* 0F 24 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_creg32,
        opcode_table: &Some(&BxOpcodeTable0F24),
    },
    /* 0F 25 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 26 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_creg32,
        opcode_table: &Some(&BxOpcodeTable0F26),
    },
    /* 0F 27 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 28 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F28),
    },
    /* 0F 29 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F29),
    },
    /* 0F 2A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F2A),
    },
    /* 0F 2B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F2B),
    },
    /* 0F 2C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F2C),
    },
    /* 0F 2D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F2D),
    },
    /* 0F 2E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F2E),
    },
    /* 0F 2F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F2F),
    },
    /* 0F 30 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0F30),
    },
    /* 0F 31 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0F31),
    },
    /* 0F 32 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0F32),
    },
    /* 0F 33 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0F33),
    },
    /* 0F 34 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0F34),
    },
    /* 0F 35 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0F35),
    },
    /* 0F 36 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 37 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F37),
    },
    /* 0F 38 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &None,
    }, // 3-byte escape
    /* 0F 39 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 3A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &None,
    }, // 3-byte escape
    /* 0F 3B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 3C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 3D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 3E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 3F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 40 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F40),
    },
    /* 0F 41 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F41),
    },
    /* 0F 42 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F42),
    },
    /* 0F 43 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F43),
    },
    /* 0F 44 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F44),
    },
    /* 0F 45 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F45),
    },
    /* 0F 46 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F46),
    },
    /* 0F 47 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F47),
    },
    /* 0F 48 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F48),
    },
    /* 0F 49 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F49),
    },
    /* 0F 4A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F4A),
    },
    /* 0F 4B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F4B),
    },
    /* 0F 4C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F4C),
    },
    /* 0F 4D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F4D),
    },
    /* 0F 4E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F4E),
    },
    /* 0F 4F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F4F),
    },
    /* 0F 50 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F50),
    },
    /* 0F 51 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F51),
    },
    /* 0F 52 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F52),
    },
    /* 0F 53 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F53),
    },
    /* 0F 54 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F54),
    },
    /* 0F 55 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F55),
    },
    /* 0F 56 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F56),
    },
    /* 0F 57 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F57),
    },
    /* 0F 58 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F58),
    },
    /* 0F 59 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F59),
    },
    /* 0F 5A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F5A),
    },
    /* 0F 5B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F5B),
    },
    /* 0F 5C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F5C),
    },
    /* 0F 5D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F5D),
    },
    /* 0F 5E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F5E),
    },
    /* 0F 5F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F5F),
    },
    /* 0F 60 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F60),
    },
    /* 0F 61 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F61),
    },
    /* 0F 62 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F62),
    },
    /* 0F 63 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F63),
    },
    /* 0F 64 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F64),
    },
    /* 0F 65 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F65),
    },
    /* 0F 66 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F66),
    },
    /* 0F 67 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F67),
    },
    /* 0F 68 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F68),
    },
    /* 0F 69 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F69),
    },
    /* 0F 6A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F6A),
    },
    /* 0F 6B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F6B),
    },
    /* 0F 6C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F6C),
    },
    /* 0F 6D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F6D),
    },
    /* 0F 6E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F6E),
    },
    /* 0F 6F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F6F),
    },
    /* 0F 70 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F70),
    },
    /* 0F 71 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F71),
    },
    /* 0F 72 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F72),
    },
    /* 0F 73 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F73),
    },
    /* 0F 74 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F74),
    },
    /* 0F 75 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F75),
    },
    /* 0F 76 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F76),
    },
    /* 0F 77 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F77),
    },
    /* 0F 78 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F78),
    },
    /* 0F 79 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F79),
    },
    /* 0F 7A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 7B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F 7C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F7C),
    },
    /* 0F 7D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F7D),
    },
    /* 0F 7E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F7E),
    },
    /* 0F 7F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F7F),
    },
    /* 0F 80 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F80_32),
    },
    /* 0F 81 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F81_32),
    },
    /* 0F 82 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F82_32),
    },
    /* 0F 83 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F83_32),
    },
    /* 0F 84 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F84_32),
    },
    /* 0F 85 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F85_32),
    },
    /* 0F 86 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F86_32),
    },
    /* 0F 87 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F87_32),
    },
    /* 0F 88 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F88_32),
    },
    /* 0F 89 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F89_32),
    },
    /* 0F 8A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F8A_32),
    },
    /* 0F 8B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F8B_32),
    },
    /* 0F 8C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F8C_32),
    },
    /* 0F 8D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F8D_32),
    },
    /* 0F 8E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F8E_32),
    },
    /* 0F 8F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0F8F_32),
    },
    /* 0F 90 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F90),
    },
    /* 0F 91 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F91),
    },
    /* 0F 92 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F92),
    },
    /* 0F 93 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F93),
    },
    /* 0F 94 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F94),
    },
    /* 0F 95 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F95),
    },
    /* 0F 96 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F96),
    },
    /* 0F 97 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F97),
    },
    /* 0F 98 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F98),
    },
    /* 0F 99 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F99),
    },
    /* 0F 9A */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F9A),
    },
    /* 0F 9B */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F9B),
    },
    /* 0F 9C */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F9C),
    },
    /* 0F 9D */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F9D),
    },
    /* 0F 9E */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F9E),
    },
    /* 0F 9F */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0F9F),
    },
    /* 0F A0 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0FA0),
    },
    /* 0F A1 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0FA1),
    },
    /* 0F A2 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0FA2),
    },
    /* 0F A3 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FA3),
    },
    /* 0F A4 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FA4),
    },
    /* 0F A5 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FA5),
    },
    /* 0F A6 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F A7 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_ud32,
        opcode_table: &None,
    },
    /* 0F A8 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0FA8),
    },
    /* 0F A9 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0FA9),
    },
    /* 0F AA */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0FAA),
    },
    /* 0F AB */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FAB),
    },
    /* 0F AC */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FAC),
    },
    /* 0F AD */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FAD),
    },
    /* 0F AE */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FAE),
    },
    /* 0F AF */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FAF),
    },
    /* 0F B0 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FB0),
    },
    /* 0F B1 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FB1),
    },
    /* 0F B2 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FB2),
    },
    /* 0F B3 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FB3),
    },
    /* 0F B4 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FB4),
    },
    /* 0F B5 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FB5),
    },
    /* 0F B6 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FB6),
    },
    /* 0F B7 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FB7),
    },
    /* 0F B8 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FB8),
    },
    /* 0F B9 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FB9),
    },
    /* 0F BA */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FBA),
    },
    /* 0F BB */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FBB),
    },
    /* 0F BC */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FBC),
    },
    /* 0F BD */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FBD),
    },
    /* 0F BE */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FBE),
    },
    /* 0F BF */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FBF),
    },
    /* 0F C0 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FC0),
    },
    /* 0F C1 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FC1),
    },
    /* 0F C2 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FC2),
    },
    /* 0F C3 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FC3),
    },
    /* 0F C4 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FC4),
    },
    /* 0F C5 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FC5),
    },
    /* 0F C6 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FC6),
    },
    /* 0F C7 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FC7),
    },
    /* 0F C8 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0FC8x0FCF),
    },
    /* 0F C9 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0FC8x0FCF),
    },
    /* 0F CA */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0FC8x0FCF),
    },
    /* 0F CB */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0FC8x0FCF),
    },
    /* 0F CC */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0FC8x0FCF),
    },
    /* 0F CD */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0FC8x0FCF),
    },
    /* 0F CE */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0FC8x0FCF),
    },
    /* 0F CF */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32,
        opcode_table: &Some(&BxOpcodeTable0FC8x0FCF),
    },
    /* 0F D0 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FD0),
    },
    /* 0F D1 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FD1),
    },
    /* 0F D2 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FD2),
    },
    /* 0F D3 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FD3),
    },
    /* 0F D4 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FD4),
    },
    /* 0F D5 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FD5),
    },
    /* 0F D6 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FD6),
    },
    /* 0F D7 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FD7),
    },
    /* 0F D8 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FD8),
    },
    /* 0F D9 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FD9),
    },
    /* 0F DA */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FDA),
    },
    /* 0F DB */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FDB),
    },
    /* 0F DC */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FDC),
    },
    /* 0F DD */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FDD),
    },
    /* 0F DE */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FDE),
    },
    /* 0F DF */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FDF),
    },
    /* 0F E0 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FE0),
    },
    /* 0F E1 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FE1),
    },
    /* 0F E2 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FE2),
    },
    /* 0F E3 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FE3),
    },
    /* 0F E4 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FE4),
    },
    /* 0F E5 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FE5),
    },
    /* 0F E6 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FE6),
    },
    /* 0F E7 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FE7),
    },
    /* 0F E8 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FE8),
    },
    /* 0F E9 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FE9),
    },
    /* 0F EA */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FEA),
    },
    /* 0F EB */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FEB),
    },
    /* 0F EC */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FEC),
    },
    /* 0F ED */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FED),
    },
    /* 0F EE */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FEE),
    },
    /* 0F EF */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FEF),
    },
    /* 0F F0 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FF0),
    },
    /* 0F F1 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FF1),
    },
    /* 0F F2 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FF2),
    },
    /* 0F F3 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FF3),
    },
    /* 0F F4 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FF4),
    },
    /* 0F F5 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FF5),
    },
    /* 0F F6 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FF6),
    },
    /* 0F F7 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FF7),
    },
    /* 0F F8 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FF8),
    },
    /* 0F F9 */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FF9),
    },
    /* 0F FA */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FFA),
    },
    /* 0F FB */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FFB),
    },
    /* 0F FC */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FFC),
    },
    /* 0F FD */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FFD),
    },
    /* 0F FE */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder32_modrm,
        opcode_table: &Some(&BxOpcodeTable0FFE),
    },
    /* 0F FF */
    BxOpcodeDecodeDescriptor32 {
        decode_method: decoder_simple32,
        opcode_table: &Some(&BxOpcodeTable0FFF),
    },
];

// Define the BxOpcodeDecodeDescriptor32 struct
pub(super) struct BxOpcodeDecodeDescriptor32 {
    pub(super) decode_method: BxFetchDecode32Ptr, // Function pointer
    pub(super) opcode_table: &'static Option<&'static [u64]>, // Use Vec<u8> for dynamic array
}

const RESOLVE16_BASE_REG: [BxRegs16; 8] = [
    BxRegs16::Bx,
    BxRegs16::Bx,
    BxRegs16::Bp,
    BxRegs16::Bp,
    BxRegs16::Si,
    BxRegs16::Di,
    BxRegs16::Bp,
    BxRegs16::Bx,
];

const RESOLVE16_INDEX_REG: [u32; 8] = [
    BxRegs16::Si as _,
    BxRegs16::Di as _,
    BxRegs16::Si as _,
    BxRegs16::Di as _,
    BX_NIL_REGISTER as _,
    BX_NIL_REGISTER as _,
    BX_NIL_REGISTER as _,
    BX_NIL_REGISTER as _,
];

// decoding instructions; accessing seg reg's by index
const SREG_MOD00_RM16: [BxSegregs; 8] = [
    BxSegregs::Ds,
    BxSegregs::Ds,
    BxSegregs::Ss,
    BxSegregs::Ss,
    BxSegregs::Ds,
    BxSegregs::Ds,
    BxSegregs::Ds,
    BxSegregs::Ds,
];

const SREG_MOD01OR10_RM16: [BxSegregs; 8] = [
    BxSegregs::Ds,
    BxSegregs::Ds,
    BxSegregs::Ss,
    BxSegregs::Ss,
    BxSegregs::Ds,
    BxSegregs::Ds,
    BxSegregs::Ss,
    BxSegregs::Ds,
];

// decoding instructions; accessing seg reg's by index
const SREG_MOD0_BASE32: [BxSegregs; 8] = [
    BxSegregs::Ds,
    BxSegregs::Ds,
    BxSegregs::Ds,
    BxSegregs::Ds,
    BxSegregs::Ss,
    BxSegregs::Ds,
    BxSegregs::Ds,
    BxSegregs::Ds,
];

const SREG_MOD1OR2_BASE32: [BxSegregs; 8] = [
    BxSegregs::Ds,
    BxSegregs::Ds,
    BxSegregs::Ds,
    BxSegregs::Ds,
    BxSegregs::Ss,
    BxSegregs::Ss,
    BxSegregs::Ds,
    BxSegregs::Ds,
];

//fn decoder32_modrm(
//    iptr: &[u8],
//    remain: &mut usize,
//    i: &mut BxInstruction,
//    b1: u32,
//    sse_prefix: Option<SsePrefix>,
//    opcode_table: Option<&'static [u64]>,
//) -> DecodeResult<Opcode> {
//    tracing::info!("in decoder32_modrm");
//
//    let modrm = BxModrm::default();
//
//    unimplemented!()
//}

fn decodeModrm32<'a>(
    mut iptr: &'a [u8],
    // remain: &mut usize,
    i: &mut BxInstructionGenerated,
    r#mod: u32,
    nnn: u32, // Why is it unused in bochs???
    rm: u32,
) -> DecodeResult<&'a [u8]> {
    let mut seg = BxSegregs::Ds;

    // initialize displ32 with zero to include cases with no diplacement
    i.modrm_form.displacement.set_displ32u(0);

    // note that mod==11b handled outside

    if i.as32_l() != 0 {
        // rm is a 3-bit field (0-7), safe to convert to u8
        // This conversion should never fail as rm is extracted from ModRM byte with & 0x7
        i.set_sib_base(rm.try_into()?);
        i.set_sib_index(4); // no Index encoding by default

        // no s-i-b byte
        if rm != 4 {
            if r#mod == 0x00 {
                if rm == 5 {
                    // BX_NIL_REGISTER is 19, fits in u8
                    // This conversion should never fail as BX_NIL_REGISTER = BX_GENERAL_REGISTERS + 3 = 19
                    i.set_sib_base(BX_NIL_REGISTER.try_into()?);
                    if iptr.len() > 3 {
                        i.modrm_form.displacement.set_displ32u(fetch_dword(iptr));
                        iptr = &iptr[4..];
                    } else {
                        return Err(BxDecodeError::ModRmParseFail.into());
                    }
                }
                // mod==00b, rm!=4, rm!=5
                // Goto modrm_done
                i.set_seg(seg);
                return Ok(iptr);
            }
            seg = SREG_MOD1OR2_BASE32[usize::try_from(rm).unwrap()];
        }
        // mod!=11b, rm==4, s-i-b byte follows
        else {
            let mut sib = if !iptr.is_empty() {
                let value = iptr[0];
                iptr = &iptr[1..];
                value
            } else {
                return Err(BxDecodeError::ModRmParseFail.into());
            };

            //let base = sib & 0x7;
            //let index = (sib >> 3) & 0x7;
            //let scale = sib >> 6;

            let base = sib & 0x7;
            sib >>= 3;

            let index = sib & 0x7;
            sib >>= 3;

            let scale = sib;

            i.set_sib_scale(scale.into());
            i.set_sib_base(base.into());

            // this part is a little tricky - assign index value always,
            // it will be really used if the instruction is Gather. Others
            // assume that resolve function will do the right thing.
            i.set_sib_index(index.into());

            if r#mod == 0x00 {
                // mod==00b, rm==4
                seg = SREG_MOD1OR2_BASE32[usize::from(base)];
                if base == 5 {
                    // BX_NIL_REGISTER is 19, fits in u8
                    // This conversion should never fail as BX_NIL_REGISTER = BX_GENERAL_REGISTERS + 3 = 19
                    i.set_sib_base(BX_NIL_REGISTER.try_into()?);

                    if iptr.len() > 3 {
                        i.modrm_form.displacement.set_displ32u(fetch_dword(iptr));
                        iptr = &iptr[4..];
                        //*remain -= 4;
                    } else {
                        return Err(BxDecodeError::ModRmParseFail.into());
                    }
                }
                // mod==00b, rm==4, base!=5
                // Goto modrm_done
                i.set_seg(seg);
                return Ok(iptr);
            } else {
                seg = SREG_MOD1OR2_BASE32[usize::try_from(rm).unwrap()];
            }
        }

        if r#mod == 0x40 {
            if !iptr.is_empty() {
                // 8 sign extended to 32
                // 8-bit value, zero-extend to 32-bit
                i.modrm_form.displacement.set_displ32u(u32::from(iptr[0]));
                iptr = &iptr[1..];
                //*remain -= 1;
            } else {
                return Err(BxDecodeError::ModRmParseFail.into());
            }
        } else {
            // (mod == 0x80), mod==10b
            if iptr.len() > 3 {
                i.modrm_form.displacement.set_displ32u(fetch_dword(iptr));
                iptr = &iptr[4..];
                //remain -= 4;
            } else {
                return Err(BxDecodeError::ModRmParseFail.into());
            }
        }
    } else {
        // 16-bit addressing modes, mod==11b handled above
        // rm is 0-7, safe to index array
        // rm is extracted from ModRM byte with & 0x7, so it's always 0-7
        let rm_idx = usize::try_from(rm)?;
        // RESOLVE16_BASE_REG and RESOLVE16_INDEX_REG contain register enum values that should fit in u8
        let base_reg = RESOLVE16_BASE_REG[rm_idx];
        let index_reg = RESOLVE16_INDEX_REG[rm_idx];
        // Register enum values are small, should fit in u8
        i.set_sib_base((base_reg as u32).try_into()?);
        i.set_sib_index(index_reg.try_into()?);
        i.set_sib_scale(0);

        if r#mod == 0x00 {
            // mod == 00b
            seg = SREG_MOD00_RM16[usize::try_from(rm).unwrap()];
            if rm == 6 {
                i.set_sib_base(BX_NIL_REGISTER as _);
                if !iptr.is_empty() {
                    i.modrm_form
                        .displacement
                        .set_displ32u(u32::from(fetch_word(iptr)));
                    iptr = &iptr[2..];
                    //*remain -= 2;
                } else {
                    return Err(BxDecodeError::ModRmParseFail.into());
                }
            }
            // modrm_done
            i.set_seg(seg);
            return Ok(iptr);
        } else {
            seg = SREG_MOD00_RM16[usize::try_from(rm).unwrap()];
        }

        if r#mod == 0x40 {
            // mod == 01b

            if !iptr.is_empty() {
                // 8 sign extended to 16
                // 8-bit sign-extended to 32-bit: interpret u8 as signed i8, then widen to i32, then to u32
                // Using 'as i8' is safe here as it performs sign extension (bit 7 becomes sign bit)
                let signed_val = iptr[0] as i8;
                i.modrm_form
                    .displacement
                    .set_displ32u(u32::try_from(i32::from(signed_val)).unwrap_or(0));
            // BOCHS, what??
            //*remain -= 1;
            } else {
                return Err(BxDecodeError::ModRmParseFail.into());
            }
        } else {
            // (mod == 0x80)      mod == 10b

            if iptr.len() > 1 {
                i.modrm_form
                    .displacement
                    .set_displ32u(u32::from(fetch_word(iptr)));
                iptr = &iptr[2..];
                //*remain -= 2;
            } else {
                return Err(BxDecodeError::ModRmParseFail.into());
            }
        }
    }

    i.set_seg(seg);
    Ok(iptr)
}

fn parse_modrm32<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
) -> DecodeResult<(BxModrm, &'a [u8])> {
    tracing::info!("in parse_modrm32");
    let mut modrm = BxModrm::default();

    if iptr.is_empty() {
        return Err(BxDecodeError::ParseModrm32.into());
    }

    let b2 = u32::from(iptr[0]);
    iptr = &iptr[1..];

    // Keep original modrm byte
    modrm.modrm = b2;

    // Parse mod-nnn-rm and related bytes
    modrm.mod_ = b2 & 0xc0; // leave unshifted
    modrm.nnn = (b2 >> 3) & 0x7;
    modrm.rm = b2 & 0x7;

    if modrm.mod_ == 0xc0 { // mod == 11b
    } else {
        iptr = decodeModrm32(iptr, i, modrm.mod_, modrm.nnn, modrm.rm)?;
    }

    tracing::info!("exting parse_modrm32");
    Ok((modrm, iptr))
}

/// Get source operand descriptors for an opcode
///
/// Returns [src0, src1, src2, src3] where each is encoded as BX_FORM_SRC(type, src_origin)
///
/// TODO: This should be replaced with a generated table from ia_opcodes.def
/// For now, we use a match statement for common opcodes
fn get_opcode_srcs(opcode: Opcode) -> [u8; 4] {
    use super::fetchdecode_generated::*;

    // Helper to form src descriptor: (type << 4) | src_origin
    const fn form_src(type_val: u8, src_val: u8) -> u8 {
        (type_val << 4) | src_val
    }

    match opcode {
        // ADD instructions
        Opcode::AddGdEd | Opcode::AddGwEw | Opcode::AddGqEq => [OP_ED, OP_GD, OP_NONE, OP_NONE],
        Opcode::AddEdGd | Opcode::AddEwGw | Opcode::AddEqGq => [OP_ED, OP_GD, OP_NONE, OP_NONE],
        Opcode::AddEaxid | Opcode::AddAxiw | Opcode::AddRaxid => {
            [OP_EAXREG, OP_ID, OP_NONE, OP_NONE]
        }
        Opcode::AddAlib | Opcode::AddEbIb => [OP_ALREG, OP_IB, OP_NONE, OP_NONE],
        Opcode::AddEbGb => [OP_EB, OP_GB, OP_NONE, OP_NONE],
        Opcode::AddGbEb => [OP_GB, OP_EB, OP_NONE, OP_NONE],

        // SUB instructions
        Opcode::SubGdEd | Opcode::SubGwEw | Opcode::SubGqEq => [OP_ED, OP_GD, OP_NONE, OP_NONE],
        Opcode::SubEdGd | Opcode::SubEwGw | Opcode::SubEqGq => [OP_ED, OP_GD, OP_NONE, OP_NONE],
        Opcode::SubEaxid | Opcode::SubAxiw | Opcode::SubRaxid => {
            [OP_EAXREG, OP_ID, OP_NONE, OP_NONE]
        }

        // MOV instructions
        Opcode::MovOp32GdEd | Opcode::MovOp64GdEd => [OP_ED, OP_GD, OP_NONE, OP_NONE],
        Opcode::MovOp32EdGd | Opcode::MovOp64EdGd => [OP_ED, OP_GD, OP_NONE, OP_NONE],
        Opcode::MovEdId | Opcode::MovEwIw | Opcode::MovEqId => [OP_ED, OP_ID, OP_NONE, OP_NONE],
        Opcode::MovAlod => [OP_ALREG, OP_OB, OP_NONE, OP_NONE],
        Opcode::MovAloq => [OP_OB, OP_ALREG, OP_NONE, OP_NONE],

        // Jump instructions with 8-bit relative offset (Jbw forms)
        // These use sign-extended 8-bit immediate: BX_IMMBW_SE (3) with BX_SRC_BRANCH_OFFSET (9)
        Opcode::JnbJbw
        | Opcode::JbJbw
        | Opcode::JbeJbw
        | Opcode::JzJbw
        | Opcode::JnzJbw
        | Opcode::JnbeJbw
        | Opcode::JsJbw
        | Opcode::JnsJbw
        | Opcode::JpJbw
        | Opcode::JnpJbw
        | Opcode::JlJbw
        | Opcode::JnlJbw
        | Opcode::JleJbw
        | Opcode::JnleJbw
        | Opcode::JoJbw
        | Opcode::JnoJbw => {
            // 8-bit sign-extended branch offset: form_src(BX_IMMBW_SE, BX_SRC_BRANCH_OFFSET)
            // BX_IMMBW_SE = 3, BX_SRC_BRANCH_OFFSET = 9
            // form_src(3, 9) = (3 << 4) | 9 = 48 | 9 = 57
            [form_src(3, 9), OP_NONE, OP_NONE, OP_NONE]
        }

        // Jump instructions with 16-bit relative offset (Jw forms)
        Opcode::JnbJw
        | Opcode::JbJw
        | Opcode::JbeJw
        | Opcode::JzJw
        | Opcode::JnzJw
        | Opcode::JnbeJw
        | Opcode::JsJw
        | Opcode::JnsJw
        | Opcode::JpJw
        | Opcode::JnpJw
        | Opcode::JlJw
        | Opcode::JnlJw
        | Opcode::JleJw
        | Opcode::JnleJw
        | Opcode::JoJw
        | Opcode::JnoJw => {
            // 16-bit branch offset: form_src(BX_IMMW, BX_SRC_BRANCH_OFFSET)
            // BX_IMMW = 5, BX_SRC_BRANCH_OFFSET = 9
            // form_src(5, 9) = (5 << 4) | 9 = 80 | 9 = 89
            [form_src(5, 9), OP_NONE, OP_NONE, OP_NONE]
        }

        // Jump instructions with 32-bit relative offset (Jd forms)
        Opcode::JnbJd
        | Opcode::JbJd
        | Opcode::JbeJd
        | Opcode::JzJd
        | Opcode::JnzJd
        | Opcode::JnbeJd
        | Opcode::JsJd
        | Opcode::JnsJd
        | Opcode::JpJd
        | Opcode::JnpJd
        | Opcode::JlJd
        | Opcode::JnlJd
        | Opcode::JleJd
        | Opcode::JnleJd
        | Opcode::JoJd
        | Opcode::JnoJd => {
            // 32-bit branch offset: form_src(BX_IMMD, BX_SRC_BRANCH_OFFSET)
            // BX_IMMD = 6, BX_SRC_BRANCH_OFFSET = 9
            // form_src(6, 9) = (6 << 4) | 9 = 96 | 9 = 105
            [form_src(6, 9), OP_NONE, OP_NONE, OP_NONE]
        }

        // Default: no sources
        _ => [OP_NONE, OP_NONE, OP_NONE, OP_NONE],
    }
}

/// Extract source type from encoded src descriptor
///
/// BX_DISASM_SRC_TYPE(src) = src >> 4
#[inline]
const fn get_src_type(src: u8) -> u8 {
    src >> 4
}

/// Extract source origin from encoded src descriptor
///
/// BX_DISASM_SRC_ORIGIN(src) = src & 0xf
#[inline]
const fn get_src_origin(src: u8) -> u8 {
    src & 0xf
}

/// Fetch immediate values from instruction stream
///
/// Based on the opcode's source operand definitions, fetches immediate values
/// of various sizes (Ib, Iw, Id, Iq, etc.) and stores them in the instruction structure.
///
/// Returns the updated instruction pointer slice after consuming immediate bytes,
/// or an error if insufficient bytes are available.
///
/// This function matches the C++ `fetchImmediate` implementation.
pub(super) fn fetch_immediate<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    ia_opcode: Opcode,
    is_64: bool,
) -> DecodeResult<&'a [u8]> {
    use super::fetchdecode::*;
    use super::fetchdecode_generated::*;

    let srcs = get_opcode_srcs(ia_opcode);

    for n in 0..=3 {
        let src_encoded = srcs[n];
        if src_encoded == OP_NONE {
            continue;
        }

        let src_type = get_src_type(src_encoded);
        let src_origin = get_src_origin(src_encoded);

        // Only fetch immediates (not register operands)
        if src_origin == BX_SRC_IMM as u8 || src_origin == BX_SRC_BRANCH_OFFSET as u8 {
            // src_type is u8 (from src >> 4), safe to widen to u32
            let imm_type = u32::from(src_type);

            match imm_type {
                x if x == BX_IMM1 as u32 => {
                    // Constant 1
                    i.modrm_form.operand_data.set_ib([1, 0, 0, 0]);
                }
                x if x == BX_IMMB as u32 => {
                    // 8-bit immediate
                    if iptr.is_empty() {
                        return Err(BxDecodeError::NoMoreLen.into());
                    }
                    let val = iptr[0];
                    iptr = &iptr[1..];
                    i.modrm_form.operand_data.set_ib([val, 0, 0, 0]);
                }
                x if x == BX_IMMBW_SE as u32 => {
                    // Sign-extended 8-bit to 16-bit
                    if iptr.is_empty() {
                        return Err(BxDecodeError::NoMoreLen.into());
                    }
                    // Sign-extend 8-bit to 16-bit: interpret u8 as signed i8, then widen to i16, then to u16
                    // Using 'as i8' is safe here as it performs sign extension (bit 7 becomes sign bit)
                    let signed_byte = iptr[0] as i8;
                    let val = u16::try_from(i16::from(signed_byte)).unwrap_or(0);
                    iptr = &iptr[1..];
                    i.modrm_form.operand_data.set_iw([val, 0]);
                }
                x if x == BX_IMMBD_SE as u32 => {
                    // Sign-extended 8-bit to 32-bit
                    if iptr.is_empty() {
                        return Err(BxDecodeError::NoMoreLen.into());
                    }
                    // Sign-extend 8-bit to 32-bit: interpret u8 as signed i8, then widen to i32, then to u32
                    // Using 'as i8' is safe here as it performs sign extension (bit 7 becomes sign bit)
                    let signed_byte = iptr[0] as i8;
                    let val = u32::try_from(i32::from(signed_byte)).unwrap_or(0);
                    iptr = &iptr[1..];
                    i.modrm_form.operand_data.set_id(val);
                }
                x if x == BX_IMMB2 as u32 => {
                    // Second 8-bit immediate
                    if iptr.is_empty() {
                        return Err(BxDecodeError::NoMoreLen.into());
                    }
                    let val = iptr[0];
                    iptr = &iptr[1..];
                    let mut ib2 = i.modrm_form.displacement.ib2();
                    ib2[0] = val;
                    i.modrm_form
                        .displacement
                        .set_id2(unsafe { core::mem::transmute(ib2) });
                }
                x if x == BX_IMMW as u32 => {
                    // 16-bit immediate
                    if iptr.len() < 2 {
                        return Err(BxDecodeError::NoMoreLen.into());
                    }
                    let val = fetch_word(iptr);
                    iptr = &iptr[2..];
                    i.modrm_form.operand_data.set_iw([val, 0]);
                }
                x if x == BX_IMMD as u32 => {
                    // 32-bit immediate
                    if iptr.len() < 4 {
                        return Err(BxDecodeError::NoMoreLen.into());
                    }
                    let val = fetch_dword(iptr);
                    iptr = &iptr[4..];
                    i.modrm_form.operand_data.set_id(val);
                }
                #[cfg(feature = "x86_64")]
                x if x == BX_IMMQ as u32 => {
                    // 64-bit immediate (x86-64 only)
                    if iptr.len() < 8 {
                        return Err(BxDecodeError::NoMoreLen.into());
                    }
                    let val = fetch_qword(iptr);
                    iptr = &iptr[8..];
                    i.set_iq(val);
                }
                x if x == BX_DIRECT_PTR as u32 => {
                    // Direct pointer: IdIw2 in 32-bit mode, IwIw2 in 16-bit mode
                    if i.os32_l() != 0 {
                        // 32-bit mode: fetch 32-bit offset
                        if iptr.len() < 4 {
                            return Err(BxDecodeError::NoMoreLen.into());
                        }
                        let val = fetch_dword(iptr);
                        iptr = &iptr[4..];
                        i.modrm_form.operand_data.set_id(val);
                    } else {
                        // 16-bit mode: fetch 16-bit offset
                        if iptr.len() < 2 {
                            return Err(BxDecodeError::NoMoreLen.into());
                        }
                        let val = fetch_word(iptr);
                        iptr = &iptr[2..];
                        i.modrm_form.operand_data.set_iw([val, 0]);
                    }

                    // Then fetch 16-bit segment selector
                    if iptr.len() < 2 {
                        return Err(BxDecodeError::NoMoreLen.into());
                    }
                    let val = fetch_word(iptr);
                    iptr = &iptr[2..];
                    i.modrm_form
                        .displacement
                        .set_id2(unsafe { core::mem::transmute([val, 0u16]) });
                }
                x if x == BX_DIRECT_MEMREF_B as u32
                    || x == BX_DIRECT_MEMREF_W as u32
                    || x == BX_DIRECT_MEMREF_D as u32
                    || x == BX_DIRECT_MEMREF_Q as u32 =>
                {
                    // Direct memory reference - address embedded in opcode
                    #[cfg(feature = "x86_64")]
                    if is_64 {
                        if i.as64_l() != 0 {
                            // 64-bit addressing
                            if iptr.len() < 8 {
                                return Err(BxDecodeError::NoMoreLen.into());
                            }
                            let val = fetch_qword(iptr);
                            iptr = &iptr[8..];
                            i.set_iq(val);
                        } else {
                            // 32-bit addressing (zero-extended to 64)
                            if iptr.len() < 4 {
                                return Err(BxDecodeError::NoMoreLen.into());
                            }
                            let val = u64::from(fetch_dword(iptr));
                            iptr = &iptr[4..];
                            i.set_iq(val);
                        }
                    } else {
                        // 32-bit mode
                        if i.as32_l() != 0 {
                            // 32-bit addressing
                            if iptr.len() < 4 {
                                return Err(BxDecodeError::NoMoreLen.into());
                            }
                            let val = fetch_dword(iptr);
                            iptr = &iptr[4..];
                            i.modrm_form.operand_data.set_id(val);
                        } else {
                            // 16-bit addressing (zero-extended to 32)
                            if iptr.len() < 2 {
                                return Err(BxDecodeError::NoMoreLen.into());
                            }
                            let val = u32::from(fetch_word(iptr));
                            iptr = &iptr[2..];
                            i.modrm_form.operand_data.set_id(val);
                        }
                    }
                }
                _ => {
                    // Unknown immediate type - skip
                }
            }
        } else if src_origin == BX_SRC_VIB as u8 {
            // Vector immediate byte
            if iptr.is_empty() {
                return Err(BxDecodeError::NoMoreLen.into());
            }
            let val = iptr[0];
            iptr = &iptr[1..];
            i.modrm_form.operand_data.set_ib([val, 0, 0, 0]);
        }
    }

    Ok(iptr)
}

/// EVEX displacement 8 compression helper
///
/// Calculates the memory operand size for EVEX compressed displacement encoding.
/// This is used when the EVEX.b bit indicates compressed 8-bit displacement.
fn evex_displ8_compression(
    _i: &BxInstructionGenerated,
    _ia_opcode: Opcode,
    _src: u8,
    _type: u8,
    _vex_w: u8,
) -> u32 {
    // TODO: Implement full EVEX displacement compression logic
    // For now, return default size based on type
    match _type {
        x if x == super::fetchdecode_generated::BX_GPR64 as u8 => 8,
        x if x == super::fetchdecode_generated::BX_GPR32 as u8 => 4,
        x if x == super::fetchdecode_generated::BX_GPR16 as u8 => 2,
        _ => 1,
    }
}

/// Assign source registers based on opcode and ModRM
///
/// This function assigns the source register indices to the instruction's meta_data
/// array based on the opcode's source operand definitions and the ModRM byte.
///
/// This matches the C++ `assign_srcs` implementation (non-AVX version).
pub(super) fn assign_srcs(
    i: &mut BxInstructionGenerated,
    ia_opcode: Opcode,
    nnn: u32,
    rm: u32,
) -> Result<(), BxDecodeError> {
    use super::fetchdecode_generated::*;
    use super::BX_TMP_REGISTER;

    let srcs = get_opcode_srcs(ia_opcode);

    for n in 0..=3 {
        let src_encoded = srcs[n];
        if src_encoded == OP_NONE {
            continue;
        }

        let src_type = get_src_type(src_encoded);
        let src_origin = get_src_origin(src_encoded);

        match src_origin {
            x if x == BX_SRC_NONE as u8
                || x == BX_SRC_IMM as u8
                || x == BX_SRC_BRANCH_OFFSET as u8
                || x == BX_SRC_IMPLICIT as u8 =>
            {
                // No register assignment needed
            }
            x if x == BX_SRC_EAX as u8 => {
                // Source is AL/AX/EAX/RAX
                i.set_src_reg(n, 0);
            }
            x if x == BX_SRC_NNN as u8 => {
                // Source is from ModRM reg field (nnn)
                i.set_src_reg(n, u8::try_from(nnn).unwrap_or(0));
            }
            x if x == BX_SRC_RM as u8 => {
                // Source is from ModRM r/m field
                if i.mod_c0() {
                    // Register mode: use rm directly
                    i.set_src_reg(n, u8::try_from(rm).unwrap_or(0));
                } else {
                    // Memory mode: use temporary register
                    let tmpreg = if src_type == BX_VMM_REG as u8 {
                        super::BX_XMM_REGISTERS // BX_VECTOR_TMP_REGISTER
                    } else {
                        BX_TMP_REGISTER
                    };
                    i.set_src_reg(n, u8::try_from(tmpreg).unwrap_or(0));
                }
            }
            x if x == BX_SRC_VECTOR_RM as u8 => {
                // Source is vector register or memory
                if i.mod_c0() {
                    i.set_src_reg(n, u8::try_from(rm).unwrap_or(0));
                } else {
                    i.set_src_reg(n, super::BX_XMM_REGISTERS as u8); // BX_VECTOR_TMP_REGISTER
                }
            }
            _ => {
                // Unknown source origin - this shouldn't happen for basic opcodes
                // For now, we'll skip it rather than panic
                tracing::warn!(
                    "assign_srcs: unknown source origin {} for opcode {:?} src {}",
                    src_origin,
                    ia_opcode,
                    n
                );
            }
        }
    }

    Ok(())
}

/// Assign source registers for AVX/EVEX/XOP instructions
///
/// This is the AVX version of assign_srcs that handles VEX/EVEX/XOP-specific
/// source operands like VVV (vector register from VEX.vvvv field).
///
/// This matches the C++ `assign_srcs` implementation (AVX version).
#[cfg(feature = "avx")]
fn assign_srcs_avx(
    i: &mut BxInstructionGenerated,
    ia_opcode: Opcode,
    _is_64: bool,
    nnn: u32,
    rm: u32,
    vvv: u32,
    _vex_w: u32,
    _had_evex: bool,
    _displ8: bool,
) -> Result<(), BxDecodeError> {
    use super::fetchdecode_generated::*;
    use super::BX_TMP_REGISTER;

    let srcs = get_opcode_srcs(ia_opcode);

    for n in 0..=3 {
        let src_encoded = srcs[n];
        if src_encoded == OP_NONE {
            continue;
        }

        let src_type = get_src_type(src_encoded);
        let src_origin = get_src_origin(src_encoded);

        match src_origin {
            x if x == BX_SRC_NONE as u8
                || x == BX_SRC_IMM as u8
                || x == BX_SRC_BRANCH_OFFSET as u8
                || x == BX_SRC_IMPLICIT as u8 =>
            {
                // No register assignment needed
            }
            x if x == BX_SRC_EAX as u8 => {
                i.set_src_reg(n, 0);
            }
            x if x == BX_SRC_NNN as u8 => {
                i.set_src_reg(n, u8::try_from(nnn).unwrap_or(0));
                // TODO: Add EVEX/AMX register validation
            }
            x if x == BX_SRC_RM as u8 => {
                if i.mod_c0() {
                    i.set_src_reg(n, u8::try_from(rm).unwrap_or(0));
                    // TODO: Add EVEX/AMX register validation
                } else {
                    let tmpreg = if src_type == BX_VMM_REG as u8 {
                        super::BX_XMM_REGISTERS // BX_VECTOR_TMP_REGISTER
                    } else {
                        BX_TMP_REGISTER
                    };
                    i.set_src_reg(n, u8::try_from(tmpreg).unwrap_or(0));
                }
            }
            x if x == BX_SRC_VECTOR_RM as u8 => {
                if i.mod_c0() {
                    i.set_src_reg(n, u8::try_from(rm).unwrap_or(0));
                } else {
                    i.set_src_reg(n, super::BX_XMM_REGISTERS as u8); // BX_VECTOR_TMP_REGISTER
                }
            }
            x if x == BX_SRC_VVV as u8 => {
                // Source is from VEX/EVEX vvvv field
                i.set_src_reg(n, u8::try_from(vvv).unwrap_or(0));
            }
            _ => {
                tracing::warn!(
                    "assign_srcs_avx: unknown source origin {} for opcode {:?} src {}",
                    src_origin,
                    ia_opcode,
                    n
                );
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "avx"))]
fn assign_srcs_avx(
    _i: &mut BxInstructionGenerated,
    _ia_opcode: Opcode,
    _is_64: bool,
    _nnn: u32,
    _rm: u32,
    _vvv: u32,
    _vex_w: u32,
    _had_evex: bool,
    _displ8: bool,
) -> Result<(), BxDecodeError> {
    // AVX not supported
    Err(BxDecodeError::BxIllegalOpcode.into())
}

/// Decode EVEX-prefixed AVX-512 instructions
///
/// EVEX prefix is 0x62 followed by 3 bytes.
/// Based on the C++ decoder_evex32 implementation.
#[cfg(not(feature = "evex"))]
fn decoder_evex32<'a>(
    _iptr: &'a [u8],
    _i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    // EVEX not supported
    Err(BxDecodeError::BxIllegalOpcode.into())
}

#[cfg(feature = "evex")]
fn decoder_evex32<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    b1: u32,
    sse_prefix: Option<SsePrefix>,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    use super::fetchdecode::fetch_dword;
    use super::fetchdecode_generated::*;

    // make sure EVEX 0x62 prefix
    assert_eq!(b1, 0x62, "decoder_evex32: invalid b1 value");

    if iptr.is_empty() {
        return Err(BxDecodeError::NoMoreLen.into());
    }

    // If mod field is not 11b (register form), fall back to regular modrm decoder
    if (iptr[0] & 0xc0) != 0xc0 {
        // TODO: Call decoder32_modrm - for now return error
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    if sse_prefix.is_some() {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    if iptr.len() < 4 {
        return Err(BxDecodeError::NoMoreLen.into());
    }

    let evex = fetch_dword(iptr);
    iptr = &iptr[4..];

    // EVEX format: 0x62 P0 P1 P2
    // Check for reserved EVEX bits
    if (evex & 0x08) != 0 {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    // EVEX.U must be '1
    if (evex & 0x400) == 0 {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    let evex_opc_map = u32::from(evex & 0x7);
    if evex_opc_map == 0 || evex_opc_map == 4 || evex_opc_map == 7 {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }
    let evex_opc_map = if evex_opc_map >= 4 {
        evex_opc_map - 1
    } else {
        evex_opc_map
    };

    let sse_prefix_raw = u32::from((evex >> 8) & 0x3);
    let vvv = 15u32 - u32::from((evex >> 11) & 0xf);
    if vvv >= 8 {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    let vex_w = u32::from((evex >> 15) & 0x1);
    let opmask = u8::try_from((evex >> 16) & 0x7).unwrap_or(0);
    i.set_opmask(opmask);
    let evex_v = ((evex >> 19) & 0x1) ^ 0x1;
    if evex_v != 0 {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }
    let evex_b = u8::try_from((evex >> 20) & 0x1).unwrap_or(0);
    i.set_evex_b(evex_b);

    let evex_vl_rc = u8::try_from((evex >> 21) & 0x3).unwrap_or(0);
    i.set_rc(evex_vl_rc);
    // VL: 0 -> 128, 1 -> 256, 2 -> 512, 3 -> reserved
    let vl_value = match evex_vl_rc {
        0 => 128u8,
        1 => 256u8,
        2 => 512u8,
        _ => return Err(BxDecodeError::BxIllegalOpcode.into()),
    };
    i.set_vl(vl_value);
    i.set_vex_w(vex_w as u8);

    let evex_z = u8::try_from((evex >> 23) & 0x1).unwrap_or(0);
    i.set_zero_masking(evex_z);

    if evex_z != 0 && opmask == 0 {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    let opcode_byte = u32::from((evex >> 24) & 0xff);
    let opcode_byte = opcode_byte + 256 * (evex_opc_map - 1);

    let (modrm, mut updated_iptr) = parse_modrm32(iptr, i)?;
    iptr = updated_iptr;

    let displ8 = (modrm.mod_ == 0x40);

    if modrm.mod_ == 0xc0 {
        // EVEX.b in reg form implies 512-bit vector length
        if i.get_evex_b() != 0 {
            i.set_vl(512u8); // BX_VL512
        }
    }

    let vl = i.get_vl() - 1; // 0: VL128, 1: VL256, 3: VL512
    let mut decmask = ((i.osize() as u32) << OS32_OFFSET)
        | ((i.asize() as u32) << AS32_OFFSET)
        | (sse_prefix_raw << SSE_PREFIX_OFFSET)
        | (if i.mod_c0() { 1 << MODC0_OFFSET } else { 0 })
        | (modrm.nnn << NNN_OFFSET)
        | (modrm.rm << RRR_OFFSET)
        | (vex_w << VEX_W_OFFSET)
        | (vl << VEX_VL_128_256_OFFSET);

    if i.mod_c0() && modrm.nnn == modrm.rm {
        decmask |= 1 << SRC_EQ_DST_OFFSET;
    }
    if opmask == 0 {
        decmask |= 1 << MASK_K0_OFFSET;
    }

    // TODO: Use BxOpcodeTableEVEX[opcode_byte] to find opcode
    let _ia_opcode = Opcode::IaError;

    // Check for immediate
    let has_immediate = (opcode_byte >= 0x70 && opcode_byte <= 0x73)
        || (opcode_byte >= 0xC2 && opcode_byte <= 0xC6)
        || (opcode_byte >= 0x200 && opcode_byte < 0x300);

    if has_immediate {
        if iptr.is_empty() {
            return Err(BxDecodeError::NoMoreLen.into());
        }
        let mut ib = i.modrm_form.operand_data.ib();
        ib[0] = iptr[0];
        i.modrm_form.operand_data.set_ib(ib);
        iptr = &iptr[1..];
    }

    // TODO: Call assign_srcs_avx with proper parameters
    // assign_srcs_avx(i, ia_opcode, false, modrm.nnn, modrm.rm, vvv, vex_w, true, displ8)?;

    // EVEX specific #UD conditions
    if i.get_vl() > 512 {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    Err(BxDecodeError::BxIllegalOpcode.into()) // Not fully implemented yet
}

/// Decode VEX-prefixed AVX instructions
///
/// VEX prefix can be 2-byte (0xC5) or 3-byte (0xC4).
/// Based on the C++ decoder_vex32 implementation.
#[cfg(not(feature = "avx"))]
fn decoder_vex32<'a>(
    _iptr: &'a [u8],
    _i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    // AVX not supported
    Err(BxDecodeError::BxIllegalOpcode.into())
}

#[cfg(feature = "avx")]
fn decoder_vex32<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    b1: u32,
    sse_prefix: Option<SsePrefix>,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    use super::fetchdecode_generated::*;

    // make sure VEX 0xC4 or VEX 0xC5
    assert!((b1 & !0x1) == 0xc4, "decoder_vex32: invalid b1 value");

    if iptr.is_empty() {
        return Err(BxDecodeError::NoMoreLen.into());
    }

    // If mod field is not 11b (register form), fall back to regular modrm decoder
    if (iptr[0] & 0xc0) != 0xc0 {
        // TODO: Call decoder32_modrm - for now return error
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    if sse_prefix.is_some() {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    let mut rm = 0u32;
    let mut nnn = 0u32;
    let mut vex_w = 0u32;
    let mut vex_opc_map = 1u32;

    let vex = iptr[0];
    iptr = &iptr[1..];

    if b1 == 0xc4 {
        // decode 3-byte VEX prefix
        vex_opc_map = u32::from(vex & 0x1f);
        if iptr.is_empty() {
            return Err(BxDecodeError::NoMoreLen.into());
        }
        let vex3 = iptr[0];
        iptr = &iptr[1..];
        vex_w = u32::from((vex3 >> 7) & 0x1);
        let vvv = 15u32 - u32::from((vex3 >> 3) & 0xf);
        let vex_l = (vex3 >> 2) & 0x1;
        i.set_vl(128u8.saturating_add((vex_l * 128).try_into().unwrap_or(0))); // BX_VL128 + vex_l
        i.set_vex_w(vex_w.try_into().unwrap_or(0));
        let sse_prefix_raw = u32::from(vex3 & 0x3);

        if iptr.is_empty() {
            return Err(BxDecodeError::NoMoreLen.into());
        }
        let opcode_byte = u32::from(iptr[0]);
        iptr = &iptr[1..];

        // there are instructions only from maps 1,2,3 for now in 32-bit mode
        if vex_opc_map < 1 || vex_opc_map >= 4 {
            return Err(BxDecodeError::BxIllegalOpcode.into());
        }

        let has_modrm = opcode_byte != 0x177; // if not VZEROUPPER/VZEROALL opcode
        let opcode_byte = opcode_byte.wrapping_sub(256);

        if has_modrm {
            // opcode requires modrm byte
            let (modrm, updated_iptr) = parse_modrm32(iptr, i)?;
            iptr = updated_iptr;
            nnn = modrm.nnn;
            rm = modrm.rm;
        } else {
            // Opcode does not require a MODRM byte
            rm = u32::from(b1 & 0x7);
            nnn = u32::from((b1 >> 3) & 0x7);
            i.assert_mod_c0();
        }

        let mut decmask = (u32::from(i.osize()) << OS32_OFFSET)
            | (u32::from(i.asize()) << AS32_OFFSET)
            | (sse_prefix_raw << SSE_PREFIX_OFFSET)
            | (if i.mod_c0() { 1 << MODC0_OFFSET } else { 0 })
            | (nnn << NNN_OFFSET)
            | (rm << RRR_OFFSET)
            | (vex_w << VEX_W_OFFSET)
            | (vex_l << VEX_VL_128_256_OFFSET);

        if i.mod_c0() && nnn == rm {
            decmask |= 1 << SRC_EQ_DST_OFFSET;
        }

        // TODO: Use BxOpcodeTableVEX[opcode_byte] to find opcode
        // For now, return error
        let _ia_opcode = Opcode::IaError;

        // Check for immediate
        let has_immediate = (opcode_byte >= 0x70 && opcode_byte <= 0x73)
            || (opcode_byte >= 0xC2 && opcode_byte <= 0xC6)
            || (opcode_byte >= 0x200);

        if has_immediate {
            if iptr.is_empty() {
                return Err(BxDecodeError::NoMoreLen.into());
            }
            let mut ib = i.modrm_form.operand_data.ib();
            ib[0] = iptr[0];
            i.modrm_form.operand_data.set_ib(ib);
            iptr = &iptr[1..];
        }

        // TODO: Call assign_srcs_avx with proper parameters
        // assign_srcs_avx(i, ia_opcode, false, nnn, rm, vvv, vex_w)?;

        return Err(BxDecodeError::BxIllegalOpcode.into()); // Not fully implemented yet
    } else {
        // 2-byte VEX (0xC5) - not fully implemented
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }
}

/// Decode XOP-prefixed instructions
///
/// XOP prefix is 0x8F followed by 2 bytes.
/// Based on the C++ decoder_xop32 implementation.
#[cfg(not(feature = "avx"))]
fn decoder_xop32<'a>(
    _iptr: &'a [u8],
    _i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    // AVX/XOP not supported
    Err(BxDecodeError::BxIllegalOpcode.into())
}

#[cfg(feature = "avx")]
fn decoder_xop32<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    b1: u32,
    sse_prefix: Option<SsePrefix>,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    use super::fetchdecode_generated::*;

    // make sure XOP 0x8f prefix
    assert_eq!(b1, 0x8f, "decoder_xop32: invalid b1 value");

    if iptr.is_empty() {
        return Err(BxDecodeError::NoMoreLen.into());
    }

    // Check if this is actually an XOP prefix
    if (iptr[0] & 0xc8) != 0xc8 {
        // not XOP prefix, decode regular opcode
        // TODO: Call decoder32_modrm - for now return error
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    if sse_prefix.is_some() {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    // 3 byte XOP prefix
    if iptr.len() < 3 {
        return Err(BxDecodeError::NoMoreLen.into());
    }

    let xop2 = iptr[0];
    iptr = &iptr[1..];

    let xop_opcext = i32::from(xop2 & 0x1f) - 8;
    if xop_opcext < 0 || xop_opcext >= 3 {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    let xop3 = iptr[0];
    iptr = &iptr[1..];

    let vex_w = u32::from((xop3 >> 7) & 0x1);
    let vvv = 15u32 - u32::from((xop3 >> 3) & 0xf);
    let vex_l = (xop3 >> 2) & 0x1;
    i.set_vl(128u8.saturating_add((vex_l * 128).try_into().unwrap_or(0))); // BX_VL128 + vex_l
    i.set_vex_w(vex_w.try_into().unwrap_or(0));
    let sse_prefix_raw = u32::from(xop3 & 0x3);

    if sse_prefix_raw != 0 {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    if iptr.is_empty() {
        return Err(BxDecodeError::NoMoreLen.into());
    }
    let opcode_byte = u32::from(iptr[0]);
    iptr = &iptr[1..];
    let opcode_byte = opcode_byte + 256 * u32::try_from(xop_opcext).unwrap_or(0);

    let (modrm, updated_iptr) = parse_modrm32(iptr, i)?;
    iptr = updated_iptr;

    let mut decmask = ((i.osize() as u32) << OS32_OFFSET)
        | ((i.asize() as u32) << AS32_OFFSET)
        | (if i.mod_c0() { 1 << MODC0_OFFSET } else { 0 })
        | (modrm.nnn << NNN_OFFSET)
        | (modrm.rm << RRR_OFFSET)
        | (vex_w << VEX_W_OFFSET)
        | (vex_l << VEX_VL_128_256_OFFSET);

    if i.mod_c0() && modrm.nnn == modrm.rm {
        decmask |= 1 << SRC_EQ_DST_OFFSET;
    }

    // TODO: Use BxOpcodeTableXOP[opcode_byte] to find opcode
    let _ia_opcode = Opcode::IaError;

    // Fetch immediate if needed
    // TODO: Call fetch_immediate(iptr, i, ia_opcode, false)?;

    // TODO: Call assign_srcs_avx with proper parameters
    // assign_srcs_avx(i, ia_opcode, false, modrm.nnn, modrm.rm, vvv, vex_w)?;

    Err(BxDecodeError::BxIllegalOpcode.into()) // Not fully implemented yet
}

/// Decode x87 FPU escape instructions (D8-DF)
///
/// Based on the C++ decoder32_fp_escape implementation.
/// x87 instructions use escape opcodes D8-DF followed by ModRM byte.
fn decoder32_fp_escape<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    use super::fetchdecode_x87::*;

    // b1 should be 0xD8-0xDF
    assert!(
        b1 >= 0xd8 && b1 <= 0xdf,
        "decoder32_fp_escape: invalid b1 value"
    );

    // opcode requires modrm byte
    if iptr.is_empty() {
        return Err(BxDecodeError::ModRmParseFail.into());
    }

    let modrm_byte = iptr[0];
    iptr = &iptr[1..];

    // Parse mod-nnn-rm
    let mod_field = modrm_byte & 0xc0;
    let nnn = (modrm_byte >> 3) & 0x7;
    let rm = modrm_byte & 0x7;

    // Store foo field for x87: (modrm | (b1 << 8)) & 0x7ff
    let foo = ((u16::from(modrm_byte)) | (u16::try_from(b1).unwrap_or(0) << 8)) & 0x7ff;
    i.set_foo(foo);

    // Select the appropriate x87 opcode table based on b1
    let x87_table = match b1 {
        0xd8 => &BxOpcodeInfo_FloatingPointD8[..],
        0xd9 => &BxOpcodeInfo_FloatingPointD9[..],
        0xda => &BxOpcodeInfo_FloatingPointDA[..],
        0xdb => &BxOpcodeInfo_FloatingPointDB[..],
        0xdc => &BxOpcodeInfo_FloatingPointDC[..],
        0xdd => &BxOpcodeInfo_FloatingPointDD[..],
        0xde => &BxOpcodeInfo_FloatingPointDE[..],
        0xdf => &BxOpcodeInfo_FloatingPointDF[..],
        _ => return Err(BxDecodeError::BxIllegalOpcode.into()),
    };

    // Determine opcode index
    let opcode_idx = if mod_field != 0xc0 {
        // /m form: use nnn directly (0-7)
        usize::try_from(nnn).unwrap_or(0)
    } else {
        // /r form: use (modrm & 0x3f) + 8
        usize::try_from(modrm_byte & 0x3f).unwrap_or(0) + 8
    };

    if opcode_idx >= x87_table.len() {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }

    let ia_opcode = x87_table[opcode_idx];

    // Assign sources
    assign_srcs(i, ia_opcode, u32::from(nnn), u32::from(rm))?;

    Ok((ia_opcode, iptr))
}

pub fn decoder32_modrm<'a>(
    mut iptr: &'a [u8],
    //remain: &mut usize,
    //i: &mut BxInstruction,
    i: &mut BxInstructionGenerated,
    _b1: u32,
    sse_prefix: Option<SsePrefix>,
    opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    tracing::info!("in decoder32_modrm");
    // opcode requires modrm byte

    let Some(opcode_table) = opcode_table else {
        unreachable!()
    };

    //let mut generated_i = BxInstructionGenerated::try_from(i.clone())?;
    let (modrm, updated_iptr) = parse_modrm32(iptr, i)?; // Handle error if parsing fails
                                                         //
    iptr = updated_iptr;

    // Construct the decmask
    let mut decmask = (u32::from(i.osize()) << OS32_OFFSET)
        | (u32::from(i.asize()) << AS32_OFFSET)
        | (sse_prefix.map_or(0u32, |sp| sp as u32) << SSE_PREFIX_OFFSET)
        | if i.mod_c0() { 1 << MODC0_OFFSET } else { 0 }
        | (modrm.nnn << NNN_OFFSET)
        | (modrm.rm << RRR_OFFSET);

    if i.mod_c0() && modrm.nnn == modrm.rm {
        decmask |= 1 << SRC_EQ_DST_OFFSET;
    }

    // Find the opcode
    let ia_opcode = find_opcode(opcode_table, decmask)?;

    // Fetch immediate value
    iptr = fetch_immediate(iptr, i, ia_opcode, false)?;

    // Assign sources
    assign_srcs(i, ia_opcode, modrm.nnn, modrm.rm)?;

    Ok((ia_opcode, iptr))
}

/// Decode control register instructions (MOV CRx, DRx)
///
/// MOVs with CRx and DRx always use register ops and ignore the mod field.
/// Based on the C++ decoder_creg32 implementation.
fn decoder_creg32<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    b1: u32,
    sse_prefix: Option<SsePrefix>,
    opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    use super::fetchdecode_generated::*;

    // MOVs with CRx and DRx always use register ops and ignore the mod field.
    // b1 should be 0x120-0x127 (0x120 | nnn)
    assert!((b1 & !7) == 0x120, "decoder_creg32: invalid b1 value");

    // opcode requires modrm byte
    if iptr.is_empty() {
        return Err(BxDecodeError::ModRmParseFail.into());
    }

    let b2 = u32::from(iptr[0]);
    iptr = &iptr[1..];

    // Parse mod-nnn-rm and related bytes
    let nnn = (b2 >> 3) & 0x7;
    let rm = b2 & 0x7;

    i.assert_mod_c0();

    let sse_prefix_raw = match sse_prefix {
        Some(prefix) => prefix as u32,
        None => 0,
    };

    let mut decmask = ((i.osize() as u32) << OS32_OFFSET)
        | ((i.asize() as u32) << AS32_OFFSET)
        | (sse_prefix_raw << SSE_PREFIX_OFFSET)
        | (1 << MODC0_OFFSET)
        | (nnn << NNN_OFFSET)
        | (rm << RRR_OFFSET);

    let Some(opcode_table) = opcode_table else {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    };

    let ia_opcode = find_opcode(opcode_table, decmask)?;

    // Assign sources
    assign_srcs(i, ia_opcode, nnn, rm)?;

    Ok((ia_opcode, iptr))
}

fn decoder32_3dnow<'a>(
    iptr: &'a [u8],
    //remain: &mut usize,
    //i: &mut BxInstruction,
    i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    tracing::info!("in decoder32_3dnow");

    let opcode = Opcode::IaError;

    let (modrm, mut iptr) = parse_modrm32(iptr, i)?;

    if !iptr.is_empty() {
        let mut ib = i.modrm_form.operand_data.ib();
        ib[0] = iptr[0];
        i.modrm_form.operand_data.set_ib(ib);
        iptr = &iptr[1..];
    } else {
        return Err(BxDecodeError::ThreeDNow.into());
    }

    let ib_val = i.modrm_form.operand_data.ib()[0];
    let opcode = Bx3DNowOpcode[usize::from(ib_val)];

    Ok((opcode, iptr))
}

fn decoder32_nop<'a>(
    iptr: &'a [u8],
    //remain: &mut usize,
    //i: &mut BxInstruction,
    i: &mut BxInstructionGenerated,
    b1: u32,
    sse_prefix: Option<SsePrefix>,
    opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    tracing::info!("in decoder32_nop");

    assert_eq!(b1, 0x90);

    i.assert_mod_c0();

    if let Some(sse_prefix) = sse_prefix {
        if sse_prefix == SsePrefix::PrefixF3 {
            return Ok((Opcode::Pause, iptr));
        }
    }

    Ok((Opcode::Nop, iptr))
}

fn decoder_simple32<'a>(
    iptr: &'a [u8],
    //remain: &mut usize,
    //i: &mut BxInstruction,
    i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    tracing::info!("in decoder_simple32");

    i.assert_mod_c0();

    let Some(opcode_table) = opcode_table else {
        unreachable!()
    };

    let op = opcode_table[0];

    // no immediate expected, no sources expected, take first opcode
    // check attributes ?
    // Extract the opcode from the upper bits
    let ia_opcode: u16 = u16::try_from((op >> 48) & 0x7FFF_u64).unwrap_or(0); // Extracting the opcode

    // Create the Opcode from the extracted value
    let opcode = Opcode::try_from(ia_opcode)?;
    Ok((opcode, iptr))
}

fn decoder_ud32<'a>(
    _iptr: &'a [u8],
    //_remain: &mut usize,
    //_i: &mut BxInstruction,
    i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    //tracing::info!("in decoder_ud32");

    Err(BxDecodeError::Ud32.into())
}

pub(super) fn find_opcode(op_map: &'static [u64], decmask: u32) -> DecodeResult<Opcode> {
    let mut ia_opcode_raw = Opcode::IaError as _;

    for &op in op_map {
        let ignmsk = u32::try_from(op & 0xFFFFFF_u64).unwrap_or(0);
        let opmsk = u32::try_from((op >> 24) & 0xFFFFFF_u64).unwrap_or(0);

        if (opmsk & ignmsk) == (decmask & ignmsk) {
            ia_opcode_raw = u16::try_from((op >> 48) & 0x7FFF_u64).unwrap_or(0); // Masking to get the opcode
            break;
        }
    }

    tracing::warn!("opcode number: {ia_opcode_raw}");
    Opcode::try_from(ia_opcode_raw)
}

fn decoder32<'a>(
    iptr: &'a [u8],
    //i: &mut BxInstruction,
    i: &mut BxInstructionGenerated,
    b1: u32,
    sse_prefix: Option<SsePrefix>,
    opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    tracing::info!("in decoder32");

    let Some(opcode_table) = opcode_table else {
        unreachable!()
    };
    let rm: u32 = b1 & 0x7;
    let nnn: u32 = (b1 >> 3) & 0x7;

    //i->assertModC0();
    //

    let sse_prefix_raw = match sse_prefix {
        Some(prefix) => prefix as u32,
        None => 0,
    };
    let mut decmask = (u32::from(i.osize()) << OS32_OFFSET)
        | (u32::from(i.asize()) << AS32_OFFSET)
        | (sse_prefix_raw << SSE_PREFIX_OFFSET)
        | (1 << MODC0_OFFSET);

    if nnn == rm {
        decmask |= 1 << SRC_EQ_DST_OFFSET;
    }

    let ia_opcode = find_opcode(opcode_table, decmask)?;

    // Fetch immediate value if needed
    let mut updated_iptr = fetch_immediate(iptr, i, ia_opcode, false)?;

    // Assign sources (decoder32 is for register-only instructions, so modC0 is always true)
    i.assert_mod_c0();
    assign_srcs(i, ia_opcode, nnn, rm)?;

    tracing::warn!("decoder32(): Opcode: {ia_opcode:?}");

    Ok((ia_opcode, updated_iptr))
}

pub fn fetch_decode32_chatgpt_generated_instr(
    mut iptr: &[u8],
    is_32: bool,
) -> DecodeResult<BxInstructionGenerated> {
    let remaining_in_page = iptr.len().min(15);
    iptr = &iptr[0..remaining_in_page];

    let mut instruction = BxInstructionGenerated::default(); // Initialize the instruction
                                                             //instruction.set_il_len(remaining_in_page);

    //let mut remain = remaining_in_page;
    //let mut b1: u32;
    let mut b1: u32;
    let mut ia_opcode = Opcode::IaError;
    let mut seg_override: Option<BxSegregs> = None;
    let mut os_32 = is_32;
    let mut lock = false;

    let mut sse_prefix: Option<SsePrefix> = None;

    let meta_info1_raw = (u8::from(is_32) << 2) | (u8::from(is_32) << 3);
    let mut meta_info_1 = MetaInfoFlags::from_bits_truncate(meta_info1_raw);

    instruction.init(u8::from(is_32), u8::from(is_32), 0, 0);

    if iptr.is_empty() {
        return Err(BxDecodeError::NoMoreLen.into());
    }

    //instruction.init(is_32, is_32, 0, 0);

    loop {
        if !iptr.is_empty() {
            b1 = u32::from(iptr[0]);
            //remain -= 1;
            iptr = &iptr[1..];

            if b1 == 0x3e {
                // Handle DS prefix for CET
                // seg_override_cet = BX_SEG_REG_DS; // Removed as per your request
            }

            match b1 {
                0x0f => {
                    if iptr.is_empty() {
                        return Err(BxDecodeError::NoMoreLen.into());
                    }
                    b1 = 0x100 | u32::from(iptr[0]);
                    iptr = &iptr[1..];
                    //remain -= 1;
                }
                0x66 => {
                    os_32 = !is_32;
                    if sse_prefix.is_none() {
                        sse_prefix = Some(SsePrefix::Prefix66);
                    }
                    meta_info_1.set_os32_b(os_32);
                    //instruction.set_os32_b(os_32);
                }
                0x67 => {
                    instruction.set_as32_b(!is_32);
                }
                // REPNE/REPNZ
                0xf2 => {
                    sse_prefix = Some(SsePrefix::PrefixF2);
                    meta_info_1.set_lock_rep_used(b1 & 3);
                    //instruction.set_lock_rep_used(b1 & 3);
                }
                // REP/REPE/REPZ
                0xf3 => {
                    sse_prefix = Some(SsePrefix::PrefixF3);
                    //instruction.set_lock_rep_used(b1 & 3);
                }
                // Segment override
                0x26 => {
                    seg_override = Some(BxSegregs::Es);
                }
                0x2e => {
                    seg_override = Some(BxSegregs::Cs);
                }
                0x36 => {
                    seg_override = Some(BxSegregs::Ss);
                }
                0x3e => {
                    seg_override = Some(BxSegregs::Ds);
                }
                0x64 => {
                    seg_override = Some(BxSegregs::Fs);
                }
                0x65 => seg_override = Some(BxSegregs::Gs),
                0xf0 => {
                    lock = true;
                }
                _ => {
                    break;
                }
            }

            tracing::warn!("seg_override: {seg_override:?} sse_prefix: {sse_prefix:?}");

            if iptr.is_empty() {
                return Err(BxDecodeError::NoMoreLen.into());
            }
        }
    }

    tracing::info!("Passed the loop");

    let decode_descriptor =
        &DECODE32_DESCRIPTOR[usize::try_from(b1).map_err(|_| BxDecodeError::U32toUsize)?];
    let decode_method = decode_descriptor.decode_method;
    let mut opcode_table = *decode_descriptor.opcode_table;

    if b1 == 0x138 || b1 == 0x13a {
        if iptr.is_empty() {
            return Err(BxDecodeError::NoMoreLen.into());
        }
        let opcode = iptr[0];
        iptr = &iptr[1..];
        //remain -= 1;

        if b1 == 0x138 {
            opcode_table = Some(BxOpcodeTable0F38[opcode as usize]);
            b1 = 0x200 | u32::from(opcode);
        } else if b1 == 0x13a {
            opcode_table = Some(BxOpcodeTable0F3A[opcode as usize]);
            b1 = 0x300 | u32::from(opcode);
        }
    }

    instruction.set_seg(BxSegregs::Ds); // default segment is DS:
    instruction.set_cet_seg_override(BxSegregs::Null);
    //seg_override_cet handling removed

    instruction.modrm_form.operand_data.set_id(0);

    //instruction.metainfo.meta_info1 = meta_info_1;

    (ia_opcode, iptr) = decode_method(iptr, &mut instruction, b1, sse_prefix, opcode_table)?;

    instruction.meta_info.metainfo1 = meta_info_1;
    instruction.meta_info.ia_opcode = ia_opcode;
    instruction.meta_info.ilen = u8::try_from(remaining_in_page)
        .unwrap_or(0)
        .saturating_sub(u8::try_from(iptr.len()).unwrap_or(0));

    if lock {
        tracing::info!("We have a lock btw");
    }

    //if ia_opcode < 0 {
    //    return Err(DecodeError::NoMoreLen);
    //}

    //instruction.set_il_len(remaining_in_page - remain);
    //instruction.set_ia_opcode(ia_opcode);
    //
    //if seg_override.is_some() {
    //    instruction.set_seg(seg_override);
    //}

    //let op_flags = BxOpcodesTable[ia_opcode].opflags;
    //
    //if lock {
    //    instruction.set_lock();
    //    if instruction.mod_c0() || !(op_flags & BX_LOCKABLE) != 0 {
    //        // Handle lock errors
    //        instruction.set_ia_opcode(BX_IA_ERROR);
    //    }
    //}

    Ok(instruction) // Return the initialized instruction
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(in crate::cpu) fn init_fetch_decode_tables(&mut self) -> crate::cpu::Result<()> {
        // TODO: implement this in future
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::cpu::decoder::fetchdecode32::fetch_decode32_chatgpt_generated_instr;

    // #[test]
    // fn example_decode() {
    //     tracing_subscriber::fmt()
    //         .without_time()
    //         .with_target(false)
    //         .init();
    //
    //     //let buf = [0x90];
    //     let buf = [
    //         0x31, 0xff, 0x48, 0x31, 0xf6, 0x48, 0x31, 0xd2, 0x48, 0x31, 0xc0, 0x50, 0x48, 0xbb,
    //         0x2f, 0x62, 0x69, 0x6e, 0x2f, 0x2f, 0x73, 0x68, 0x53, 0x48, 0x89, 0xe7, 0xb0, 0x3b,
    //         0x0f, 0x05,
    //     ];
    //     //let buf = [0x8B, 0x03]; //  mov    eax,DWORD PTR [ebx]
    //     //let buf = [0x8b, 0x40, 0x10]; // mov eax, [eax+0x10]
    //     let buf = [0x8B, 0x80, 0x45, 0x23, 0x01, 0x00]; // mov eax, [eax+0x12345]
    //                                                     //let buf = [0x0f, 0x93, 0xc0]; //  setnb al
    //                                                     //let buf = [0xc3]; // ret
    //                                                     //let buf = [0xe9, 0xfc, 0xff, 0xff, 0xff]; // jmp rcx
    //                                                     //let buf = [0x98]; // cwde
    //                                                     // let buf = [0x91]; // xchg eax, ecx
    //                                                     //let buf = [0x0f, 0xcb]; // bswap ebx (not working)
    //
    //     //let buf = [0x48]; // dec eax
    //
    //     match fetch_decode32_chatgpt_generated_instr(&buf, true) {
    //         Ok(instr) => tracing::info!("{:#?}", instr),
    //         Err(e) => tracing::error!("Error when decoding: {e:?}"),
    //     }
    // }
}
