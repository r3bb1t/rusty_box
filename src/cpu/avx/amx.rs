const BX_TILE_REGISTERS: usize = 8;

#[derive(Debug, Default)]
pub struct TILECFG {
    rows: u32,
    bytes_per_row: u32,
}

#[derive(Debug)]
pub struct AMX {
    /// 0 if tiles are not configured
    palette_id: u32,
    /// used to restart tile operations
    start_row: u32,

    tilecfg: [TILECFG; 8],
}

impl TILECFG {
    fn clear(&mut self) {
        self.rows = 0;
        self.bytes_per_row = 0;
    }
}
