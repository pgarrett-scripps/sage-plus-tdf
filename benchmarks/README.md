# Reader performance measurements

Build once, then measure opening, complete sequential decoding and a reproducible random-access workload:

```sh
cargo build --locked --release --example benchmark
target/release/examples/benchmark /path/to/acquisition.d 3 64 20260926
```

Arguments are the acquisition directory, repeat count, number of random frame reads per repeat and random seed. Output is a single JSON object containing package version, operating system, architecture, build mode, frame and peak counts, and nanosecond measurements for each pass. The random sequence restarts with the recorded seed on every repeat. It samples with replacement.

Opening includes SQLite metadata loading and opening the binary file. Sequential and random timing includes seek, read, decode and disposal of each complete frame. The random workload follows sequential decoding, so those reads are generally warm. No cache flush is attempted. Compare results on the same machine, build profile, source acquisition and seed. Record the source Git revision and source-file hashes alongside any baseline.

Peak resident memory is intentionally measured outside the process. On Linux, run `/usr/bin/time -v` around the executable and retain its maximum resident-set-size value. The benchmark adds no allocator, runtime service or benchmark framework dependency.

Timing is informational. CI does not fail based on a hardware-dependent throughput threshold. Correct frame and peak counts, typed failures, decoding parity and bounded fuzz checks remain correctness gates.

## Candidate baseline

The committed `dda.json`, `public-dia.json` and `scp.json` record the initial 0.1.1-rc.2 release-build measurements on the machine described in `environment.json`. These runs shared the machine with builds and qualification tasks. Cache state was uncontrolled. They are reproducible workload records, not isolated performance claims or regression thresholds.

| Source fixture in qualification.json | Frames | Peaks | Median sequential seconds | Peak RSS KiB |
| --- | ---: | ---: | ---: | ---: |
| example-dda.d | 710 | 26,086,520 | 4.975 | 28,244 |
| example_dia.d | 500 | 3,183,326 | 1.124 | 21,480 |
| timstof-scp-idleflow-dia.d | 131 | 4,470,167 | 0.869 | 25,092 |

Each run uses three repeats, 64 random reads per repeat and seed 42. RSS covers the complete process, including all repeats. Source-file hashes are recorded in `qualification.json`. The matching benchmark source is the rc.2 commit containing these reports.
