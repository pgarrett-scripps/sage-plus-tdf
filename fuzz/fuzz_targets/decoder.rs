#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| sage_plus_tdf_fuzz::decoder(data));
