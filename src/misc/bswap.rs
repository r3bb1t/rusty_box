use byteorder::{ByteOrder, LittleEndian};

#[cfg(target_os = "android")]
compile_error! { "Android is not supported yet. Particularly writing to memory is not implemented" }

#[inline]
pub fn write_host_word_to_little_endian(host_ptr: &mut [u8], native_var16: u16) {
    LittleEndian::write_u16(host_ptr, native_var16);
}

#[inline]
pub fn write_host_dword_to_little_endian(host_ptr: &mut [u8], native_var32: u32) {
    LittleEndian::write_u32(host_ptr, native_var32);
}

#[inline]
pub fn write_host_qword_to_little_endian(host_ptr: &mut [u8], native_var64: u64) {
    LittleEndian::write_u64(host_ptr, native_var64);
}

#[inline]
pub fn read_host_word_to_little_endian(host_ptr: &mut [u8]) -> u16 {
    LittleEndian::read_u16(host_ptr)
}

#[inline]
pub fn read_host_dword_to_little_endian(host_ptr: &mut [u8]) -> u32 {
    LittleEndian::read_u32(host_ptr)
}

#[inline]
pub fn read_host_qword_to_little_endian(host_ptr: &mut [u8]) -> u64 {
    LittleEndian::read_u64(host_ptr)
}
