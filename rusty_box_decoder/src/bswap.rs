use byteorder::{ByteOrder, LittleEndian};

#[inline]
pub(super) fn read_host_word_to_little_endian(host_ptr: &[u8]) -> u16 {
    LittleEndian::read_u16(host_ptr)
}

#[inline]
pub(super) fn read_host_dword_to_little_endian(host_ptr: &[u8]) -> u32 {
    LittleEndian::read_u32(host_ptr)
}

#[inline]
pub(super) fn read_host_qword_to_little_endian(host_ptr: &[u8]) -> u64 {
    LittleEndian::read_u64(host_ptr)
}
