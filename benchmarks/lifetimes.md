# Lifetime and performance checks

2026-09-24, RTX 3050 Laptop GPU, Windows. Local measurements, not guarantees.

- `cargo test`: 24 passed. Includes callback panic cleanup and 64 audio-player lifetimes, checking decoder buffers are released.
- `cargo test gpu_resources -- --ignored`: 2,048 uploads across four renderer lifetimes; texture caches drain and device/instance owners drop.
- `cargo run --example verify_assets`: formats, AVIF, cache ownership and 256 repeated asset uploads; caches return to zero.
- `cargo run --example verify_scene`: transitions, one-time setup, failed setup cleanup and worker shutdown.
- `cargo run --example verify_render`: pixel checks, resize, minimize and texture retirement.

These assert ownership and cleanup. They do not instrument every native allocation or prove that third-party codecs/drivers cannot leak.

## GPU / SIMD

- Rendering already uses GPU instancing and compositing. BC encoding already uses Intel's SIMD compressor.
- `cargo run --release --example bench_renderer -- 10000`: 10k visible + 30k culled, two draw calls, 0.9854 ms median / 1.6262 ms p95. Includes GPU/presentation waits.
- `cargo test --release mixer_benchmark -- --ignored --nocapture`: mixing ten seconds of audio took 3.79 ms for one voice, 99.24 ms for 32 voices (about 1% of one core).
- Next SIMD candidate: a contiguous, same-sample-rate audio mixing path. GPU audio would add transfers and synchronization to a small workload.
- GPU culling would need compaction and order-preserving batching for transparency. The current timings do not justify that complexity. Profile larger real scenes first.

No new GPU compute or SIMD intrinsics were added.
