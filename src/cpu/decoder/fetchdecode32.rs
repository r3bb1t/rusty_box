use alloc::borrow::ToOwned;

use crate::cpu::{
    decoder::{fetchdecode_generated::BxDecodeError, DecodeError},
    descriptor::{BxSegmentReg, SegmentType},
    CpuError,
};

use super::{
    fetchdecode::SsePrefix,
    fetchdecode_opmap::*,
    ia_opcodes::Opcode,
    instr::{BxInstruction, MetaInfoFlags},
    BxSegregs,
};

// Define the function pointer type
type BxFetchDecode32Ptr = fn(
    iptr: &[u8],                                  // Slice of Bit8u
    remain: &mut u32,      // Mutable reference to unsigned (using u32 for unsigned)
    i: &mut BxInstruction, // Mutable reference to bxInstruction_c
    b1: u32,               // Unsigned integer (using u32)
    sse_prefix: u32,       // Unsigned integer (using u32)
    opcode_table: &'static Option<&'static [u8]>, // Slice of u8 instead of a pointer to void
) -> i32; // Return type is int (using i32 in Rust)

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

const DECODE32_DESCRIPTOR: [BxOpcodeDecodeDescriptor32; 512] = [
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
struct BxOpcodeDecodeDescriptor32 {
    decode_method: BxFetchDecode32Ptr,             // Function pointer
    opcode_table: &'static Option<&'static [u64]>, // Use Vec<u8> for dynamic array
}

fn decoder32_modrm(
    iptr: &[u8],
    remain: &mut u32,
    i: &mut BxInstruction,
    b1: u32,
    sse_prefix: u32,
    opcode_table: &'static Option<&'static [u8]>,
) -> i32 {
    unimplemented!()
}

fn decoder32(
    iptr: &[u8],
    remain: &mut u32,
    i: &mut BxInstruction,
    b1: u32,
    sse_prefix: u32,
    opcode_table: &'static Option<&'static [u8]>,
) -> i32 {
    unimplemented!()
}

fn decoder_ud32(
    iptr: &[u8],
    remain: &mut u32,
    i: &mut BxInstruction,
    b1: u32,
    sse_prefix: u32,
    opcode_table: &'static Option<&'static [u8]>,
) -> i32 {
    unimplemented!()
}

fn decoder_simple32(
    iptr: &[u8],
    remain: &mut u32,
    i: &mut BxInstruction,
    b1: u32,
    sse_prefix: u32,
    opcode_table: &'static Option<&'static [u8]>,
) -> i32 {
    unimplemented!()
}

fn decoder32_fp_escape(
    iptr: &[u8],
    remain: &mut u32,
    i: &mut BxInstruction,
    b1: u32,
    sse_prefix: u32,
    opcode_table: &'static Option<&'static [u8]>,
) -> i32 {
    unimplemented!()
}

fn decoder_evex32(
    iptr: &[u8],
    remain: &mut u32,
    i: &mut BxInstruction,
    b1: u32,
    sse_prefix: u32,
    opcode_table: &'static Option<&'static [u8]>,
) -> i32 {
    unimplemented!()
}

fn decoder32_3dnow(
    iptr: &[u8],
    remain: &mut u32,
    i: &mut BxInstruction,
    b1: u32,
    sse_prefix: u32,
    opcode_table: &'static Option<&'static [u8]>,
) -> i32 {
    unimplemented!()
}

fn decoder_creg32(
    iptr: &[u8],
    remain: &mut u32,
    i: &mut BxInstruction,
    b1: u32,
    sse_prefix: u32,
    opcode_table: &'static Option<&'static [u8]>,
) -> i32 {
    unimplemented!()
}

fn decoder_xop32(
    iptr: &[u8],
    remain: &mut u32,
    i: &mut BxInstruction,
    b1: u32,
    sse_prefix: u32,
    opcode_table: &'static Option<&'static [u8]>,
) -> i32 {
    unimplemented!()
}

fn decoder32_nop(
    iptr: &[u8],
    remain: &mut u32,
    i: &mut BxInstruction,
    b1: u32,
    sse_prefix: u32,
    opcode_table: &'static Option<&'static [u8]>,
) -> i32 {
    unimplemented!()
}

fn decoder_vex32(
    iptr: &[u8],
    remain: &mut u32,
    i: &mut BxInstruction,
    b1: u32,
    sse_prefix: u32,
    opcode_table: &'static Option<&'static [u8]>,
) -> i32 {
    unimplemented!()
}

type Result<T> = core::result::Result<T, DecodeError>;

pub fn fetch_decode32(mut iptr: &[u8], is_32: bool, mut remaining_in_page: u32) -> Result<()> {
    if remaining_in_page > 15 {
        remaining_in_page = 15;
    }

    if iptr.len() > 15 {
        iptr = &iptr[..15];
    }

    let mut iptr_offset = 0;
    let ilen = remaining_in_page;

    let mut remain = remaining_in_page; // remain must be at least 1
    let mut b1 = iptr[0];

    let mut ia_opcode = Opcode::Error;
    let mut seg_override = BxSegregs::Null;
    let mut seg_override_cet = BxSegregs::Null;

    let mut os_32 = is_32;
    let mut lock = false;

    let mut sse_prefix: Option<SsePrefix> = None;

    let mut meta_info_1 = MetaInfoFlags::default();

    let mut meta_info1_raw = ((is_32 as u8) << 2) | ((is_32 as u8) << 3) | (0 << 0) | (0 << 1);
    let mut meta_info_1 = MetaInfoFlags::from_bits_truncate(meta_info1_raw);

    // First, start with fetching prefixes
    // There are maximum 4 bytes of such prefixes
    let mut b1 = u32::from(iptr[0]);
    for _ in 0..4 {
        remain -= 1;

        if b1 == 0x3e {
            seg_override_cet = BxSegregs::Ds;
        }
        match b1 {
            // Lock
            0x0f => {
                if iptr.len() > 1 {
                    b1 = 0x100 | u32::from(iptr[0]);
                    iptr = &iptr[1..];
                    break;
                }
            }
            // OpSize
            0x66 => {
                os_32 = !is_32;
                if sse_prefix.is_none() {
                    sse_prefix = Some(SsePrefix::Prefix66);
                }
            }
            // Segment override
            0x26 => {
                seg_override = BxSegregs::Es;
            }
            0x2e => {
                seg_override = BxSegregs::Cs;
            }
            0x36 => {
                seg_override = BxSegregs::Ss;
            }
            0x3e => {
                seg_override = BxSegregs::Ds;
            }
            0x64 => {
                seg_override = BxSegregs::Fs;
            }
            0x65 => seg_override = BxSegregs::Gs,
            _ => {
                // Handle default case...
            }
        }

        b1 = u32::from(iptr[0]);
        if iptr.len() < 2 {
            return Err(BxDecodeError::Other);
        }
        iptr = &iptr[1..];
    }

    todo!()
}

//pub fn fetch_decode32_v2(iptr: &[u8], is_32: bool, mut remaining_in_page: u32) -> Result<()> {
//    if remaining_in_page > 15 {
//        remaining_in_page = 15;
//    }
//    i.set_il_len(remaining_in_page);
//
//    let mut remain = remaining_in_page; // remain must be at least 1
//    let mut b1: u8;
//    let mut ia_opcode = Opcode::Error;
//    let mut seg_override = Some(BxSegregs::Null as u8);
//    let mut seg_override_cet = Some(BxSegregs::Null as u8);
//    let mut os_32 = is_32;
//    let mut lock = false;
//    let mut sse_prefix: Option<u8> = None;
//
//    i.init(is_32, is_32, 0, 0);
//
//    while remain > 0 {
//        b1 = iptr[0]; // Fetch the byte
//        iptr = &iptr[1..]; // Move the pointer forward
//        remain -= 1;
//
//        if b1 == 0x3e {
//            seg_override_cet = Some(BxSegregs::Ds as u8);
//        }
//
//        match b1 {
//            0x0f => {
//                // 2-byte escape
//                if remain == 0 {
//                    return Err(CpuError::Decoder(Error));
//                }
//                remain -= 1;
//                b1 = 0x100 | iptr[0]; // Fetch the next byte
//                iptr = &iptr[1..]; // Move the pointer forward
//                break;
//            }
//            0x66 => {
//                // OpSize
//                os_32 = !is_32;
//                if sse_prefix.is_none() {
//                    sse_prefix = Some(SsePrefix::Prefix66 as u8);
//                }
//                i.set_os32_b(os_32);
//            }
//            0x67 => {
//                // AddrSize
//                i.set_as32_b(!is_32);
//            }
//            0xf2 | 0xf3 => {
//                // REPNE/REPNZ or REP/REPE/REPZ
//                sse_prefix = Some((b1 & 3) ^ 1);
//                i.set_lock_rep_used(b1 & 3);
//            }
//            0x26 | 0x2e | 0x36 | 0x3e => {
//                // Segment overrides
//                seg_override = Some((b1 >> 3) & 3);
//            }
//            0x64 | 0x65 => {
//                // FS: or GS:
//                seg_override = Some(b1 & 0xf);
//            }
//            0xf0 => {
//                // LOCK:
//                lock = true;
//            }
//            _ => break,
//        }
//
//        if remain == 0 {
//            return Err(CpuError::Decode);
//        }
//    }
//
//    let decode_descriptor = DECODE32_DESCRIPTOR[usize::from(b1)];
//
//    //let decode_descriptor = &decode32_descriptor[b1 as usize];
//    //let decode_method = decode_descriptor.decode_method;
//    //let mut opcode_table = decode_descriptor.opcode_table;
//    //
//    //if b1 == 0x138 || b1 == 0x13a {
//    //    if remain == 0 {
//    //        return Err(CpuError::Decoder);
//    //    }
//    //    let opcode = iptr[0]; // Fetch the next byte
//    //    iptr = &iptr[1..]; // Move the pointer forward
//    //    remain -= 1;
//    //
//    //    if b1 == 0x138 {
//    //        opcode_table = BxOpcodeTable0F38[opcode as usize];
//    //        b1 = 0x200 | opcode;
//    //    } else if b1 == 0x13a {
//    //        opcode_table = BxOpcodeTable0F3A[opcode as usize];
//    //        b1 = 0x300 | opcode;
//    //    }
//    //}
//    //
//    //i.set_seg(BX_SEG_REG_DS); // Default segment is DS
//    //i.set_cet_seg_override(seg_override_cet);
//    //i.mod_rm_form.id = 0;
//    //
//    //ia_opcode = decode_method(iptr, remain, i, b1, sse_prefix, opcode_table)?;
//    //if ia_opcode < 0 {
//    //    return Err(CpuError::Decoder);
//    //}
//    //
//    //i.set_il_len(remaining_in_page - remain);
//    //i.set_ia_opcode(ia_opcode);
//    //
//    //if !BX_NULL_SEG_REG(seg_override) {
//    //    i.set_seg(seg_override);
//    //}
//    //
//    //let op_flags = BxOpcodesTable[ia_opcode].opflags;
//    //if lock {
//    //    i.set_lock();
//    //    if i.mod_c0() || !(op_flags & BX_LOCKABLE) != 0 {
//    //        #[cfg(BX_CPU_LEVEL >= 6)]
//    //        if (op_flags & BX_LOCKABLE) != 0 {
//    //            if ia_opcode == BX_IA_MOV_CR0Rd {
//    //                i.set_src_reg(0, 8); // extend CR0 -> CR8
//    //            } else if ia_opcode == BX_IA_MOV_RdCR0 {
//    //                i.set_src_reg(1, 8); // extend CR0 -> CR8
//    //            } else {
//    //                i.set_ia_opcode(BX_IA_ERROR); // replace execution function with undefined-opcode
//    //            }
//    //        } else {
//    //            i.set_ia_opcode(BX_IA_ERROR); // replace execution function with undefined-opcode
//    //        }
//    //    }
//    //}
//    //
//    //Ok(())
//}
