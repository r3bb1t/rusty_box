/////////////////////////////////////////////////////////////////////////
// $Id$
/////////////////////////////////////////////////////////////////////////
//
//  I grabbed these CRC routines from the following source:
//    http://www.landfield.com/faqs/compression-faq/part1/section-25.html
//
//  These routines are very useful, so I'm including them in bochs.
//  They are not covered by the license, as they are not my doing.
//  My gratitude to the author for offering them on the 'net.
//
//  I only changed the u_long to Bit32u, and u_char to Bit8u, and gave
//  the functions prototypes.
//
//  -Kevin
//
//  **************************************************************************
//  The following C code (by Rob Warnock <rpw3@sgi.com>) does CRC-32 in
//  BigEndian/BigEndian byte/bit order.  That is, the data is sent most
//  significant byte first, and each of the bits within a byte is sent most
//  significant bit first, as in FDDI. You will need to twiddle with it to do
//  Ethernet CRC, i.e., BigEndian/LittleEndian byte/bit order. [Left as an
//  exercise for the reader.]
//
//  The CRCs this code generates agree with the vendor-supplied Verilog models
//  of several of the popular FDDI "MAC" chips.
//  **************************************************************************

use std::sync::OnceLock;

/// Initialized first time "crc32()" is called. If you prefer, you can
/// statically initialize it at compile time. [Another exercise.]
static CRC32_TABLE: OnceLock<[u32; 256]> = OnceLock::new();

pub fn crc32_table() -> &'static [u32; 256] {
    CRC32_TABLE.get_or_init(init_crc32_table)
}

// Build auxiliary table for parallel byte-at-a-time CRC-32.
/// AUTODIN II, Ethernet, & FDDI
const CRC32_POLY: u32 = 0x04c11db7;

fn init_crc32_table() -> [u32; 256] {
    let mut crc32_table = [0u32; 256]; // Create a vector with 256 elements initialized to 0

    (0..256usize).for_each(|i| {
        // FIXME: don't unwrap
        let mut c: u32 = (i << 24).try_into().unwrap(); // Shift the byte value to the left by 24 bits
        for _ in 0..8 {
            c = if c & 0x80000000 != 0 {
                (c << 1) ^ CRC32_POLY // If the MSB is set, shift left and XOR with the polynomial
            } else {
                c << 1 // Otherwise, just shift left
            };
        }
        crc32_table[i] = c; // Store the result in the table
    });

    crc32_table // Return the initialized table
}

// NOTE: i'm not 100% sure that it's correct. But most likely, it is
// TODO: Revisit it because of "as" casts
pub fn crc32(buf: &[u8]) -> u32 {
    let crc32_table = crc32_table();
    let mut crc: u32 = 0xffffffff; // preload shift register, per CRC-32 spec

    for byte in buf {
        crc = (crc << 8) ^ crc32_table[(crc >> 24) as usize ^ (*byte as usize)];
    }

    !crc // transmit complement, per CRC-32 spec
}
