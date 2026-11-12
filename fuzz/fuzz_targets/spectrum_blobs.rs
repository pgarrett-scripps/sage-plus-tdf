#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| sage_plus_tdf_fuzz::spectrum_blobs(data));
