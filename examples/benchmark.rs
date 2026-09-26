//! Reproducible workloads with measured, hardware-dependent timing.
use std::{hint::black_box, time::Instant};
use timsrust_reader::TdfReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: benchmark acquisition.d [repeats] [random-samples] [seed]")?;
    let repeats: usize = args.next().unwrap_or_else(|| "3".into()).parse()?;
    let samples: usize = args.next().unwrap_or_else(|| "64".into()).parse()?;
    let seed: u64 = args.next().unwrap_or_else(|| "20260926".into()).parse()?;
    if args.next().is_some() || !(1..=100).contains(&repeats) || !(1..=1_000_000).contains(&samples)
    {
        return Err("repeats must be 1..=100 and random-samples 1..=1000000".into());
    }
    let mut opening_ns = Vec::new();
    let mut sequential_ns = Vec::new();
    let mut random_ns = Vec::new();
    let mut frame_count = 0usize;
    let mut peak_count = None;
    let mut random_peak_count = None;
    for _ in 0..repeats {
        let started = Instant::now();
        let mut reader = TdfReader::open(&path)?;
        opening_ns.push(started.elapsed().as_nanos());
        let ids: Vec<_> = reader.frames().map(|frame| frame.id).collect();
        frame_count = ids.len();
        if ids.is_empty() {
            return Err("benchmark requires at least one frame".into());
        }
        let started = Instant::now();
        let mut peaks = 0u64;
        for id in &ids {
            let frame = reader.read_frame(*id)?;
            peaks += frame.tof_indices.len() as u64;
            black_box(frame);
        }
        sequential_ns.push(started.elapsed().as_nanos());
        if peak_count.is_some_and(|previous| previous != peaks) {
            return Err("acquisition changed between passes".into());
        }
        peak_count = Some(peaks);
        let mut state = seed;
        let started = Instant::now();
        let mut sampled_peaks = 0u64;
        for _ in 0..samples {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let frame = reader.read_frame(ids[(state % ids.len() as u64) as usize])?;
            sampled_peaks += frame.tof_indices.len() as u64;
            black_box(frame);
        }
        random_ns.push(started.elapsed().as_nanos());
        if random_peak_count.is_some_and(|previous| previous != sampled_peaks) {
            return Err("random-access peak count changed between passes".into());
        }
        random_peak_count = Some(sampled_peaks);
    }
    println!(
        "{}",
        serde_json::json!({
            "schema_version": 1, "package_version": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS, "architecture": std::env::consts::ARCH,
            "debug_assertions": cfg!(debug_assertions), "repeats": repeats,
            "random_samples_per_repeat": samples, "seed": seed,
            "frames_per_sequential_pass": frame_count, "peaks_per_sequential_pass": peak_count,
            "peaks_per_random_pass": random_peak_count,
            "opening_ns": opening_ns, "sequential_ns": sequential_ns, "random_ns": random_ns,
            "cache_state": "uncontrolled, repeated filesystem reads may be cached"
        })
    );
    Ok(())
}
