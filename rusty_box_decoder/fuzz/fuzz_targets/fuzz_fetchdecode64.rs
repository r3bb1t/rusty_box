#![no_main]

use rusty_box_decoder::fetch_decode64;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = fetch_decode64(data, false);
});
