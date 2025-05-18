use core::ffi::c_uint;

const BX_TILE_REGISTERS: usize = 8;

#[derive(Debug, Default)]
pub struct TILECFG {
    rows: c_uint,
    bytes_per_row: c_uint,
}

#[derive(Debug)]
pub struct AMX {
    /// 0 if tiles are not configured
    palette_id: c_uint,
    /// used to restart tile operations
    start_row: c_uint,

    tilecfg: [TILECFG; 8],
}

impl TILECFG {
    fn clear(&mut self) {
        self.rows = 0;
        self.bytes_per_row = 0;
    }
}
