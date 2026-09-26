#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| timsrust_reader_fuzz::decoder(data));
