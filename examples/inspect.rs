use sha2::{Digest, Sha256};
use timsrust_reader::TdfReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: inspect acquisition.d")?;
    let mut reader = TdfReader::open(path)?;
    let ids: Vec<_> = reader.frames().map(|f| f.id).collect();
    let mut peaks = 0u64;
    let mut hash = Sha256::new();
    for id in &ids {
        let frame = reader.read_frame(*id)?;
        hash.update(id.to_le_bytes());
        for offset in frame.scan_offsets {
            hash.update((offset as u64).to_le_bytes());
        }
        for (tof, intensity) in frame.tof_indices.iter().zip(&frame.intensities) {
            hash.update(tof.to_le_bytes());
            hash.update(intensity.to_le_bytes());
        }
        peaks += frame.tof_indices.len() as u64;
    }
    println!(
        "{}",
        serde_json::json!({"frames":ids.len(),"peaks":peaks,"raw_digest":format!("{:x}",hash.finalize())})
    );
    Ok(())
}
