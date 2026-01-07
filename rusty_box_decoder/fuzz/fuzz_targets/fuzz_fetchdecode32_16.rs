#![no_main]

use rusty_box_decoder::fetchdecode32::fetch_decode32;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = fetch_decode32(data, false);
});
