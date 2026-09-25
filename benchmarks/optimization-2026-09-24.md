# Rendering and audio optimization measurements

Windows x64, release profile, NVIDIA GeForce RTX 3050 Laptop GPU. These are
wall-clock measurements on the development machine, not isolated lab runs or
latency guarantees. Other activity, scheduling and clock changes were not
controlled. Repeated runs showed substantial variance, especially at p95/p99.

## CPU preparation

Actual `Renderer::prepare`, including culling, texture lookup, instance generation
and batching. Excludes GPU buffer copies, command recording, fence waits,
acquisition and presentation. 200 warmup iterations, 2,000 measured iterations.
One shared texture, explicit 8x8 sprite sizes, default surface. Offscreen sprites
are positioned at x=2000; these numbers do not cover every culling distribution.
The scene is processed each iteration; no unchanged-scene result cache is used.

| Scene | Baseline median/p95/p99 (ms) | Final median/p95/p99 (ms) |
| --- | --- | --- |
| 10k visible + 30k offscreen | 0.7713 / 1.5845 / 2.9292 | 0.2039 / 0.7107 / 1.0519 |
| 100k offscreen | 0.9420 / 1.2552 / 1.5926 | 0.2301 / 0.7924 / 1.1380 |
| 10k visible | 0.2621 / 0.3433 / 0.4168 | 0.1556 / 0.2414 / 0.2878 |

The mixed-scene median meets 0.5 ms for preparation only. Its tails do not.
The 100k offscreen case does not meet 0.2 ms. During development, the SIMD
offscreen benchmark also produced medians from 0.32 to 0.79 ms; the final run
must not be treated as a guaranteed ceiling.

## Full render call

Existing benchmark: 10k visible, 10k hidden, 10k transparent, 10k offscreen.
120 warmup frames, 600 samples. Includes driver calls and synchronization.

| | Median (ms) | p95 (ms) | p99 (ms) |
| --- | --- | --- | --- |
| Baseline | 1.3185 | 2.2446 | Not recorded |
| Final | 1.1685 | 3.1536 | 4.7940 |

Full-frame 0.5 ms is not achieved. Tail improvement is not established; p95 in
the final sample is worse. GPU execution time was not independently measured.
Sprite batches already used instancing: 10k consecutive sprites sharing a
texture/surface use one sprite draw, plus one compositing draw (two total).
Transparent painter order is preserved; separate texture runs are not reordered.

## Audio CPU mixing

32 memory voices, 480 stereo output frames at 48 kHz, 200 warmups and 2,000
samples. Excludes decoding, output-device waits, and control-lock contention.

| Source | Median/p95/p99 (ms) |
| --- | --- |
| 44.1 kHz mono, resampled | 0.0928 / 0.1206 / 0.1453 |
| 44.1 kHz stereo, resampled | 0.0701 / 0.0965 / 0.1150 |
| 48 kHz mono | 0.0049 / 0.0050 / 0.0050 |
| 48 kHz stereo | 0.0039 / 0.0040 / 0.0048 |

These are final measurements, not before/after comparisons between equal workloads.

## Changes

- SSE2 rejection of groups of four offscreen default-surface sprites, with scalar
  handling for mixed groups, explicit surfaces, and remaining sprites.
- Early rejection before texture access; lazy texture dimensions for size overrides.
- Surface clipping computed once per surface instead of transforming every sprite
  twice for intersection tests. Nonfinite transformed bounds still get rejected.
- Consecutive visible sprites reuse their validated texture descriptor within a
  frame, avoiding repeated hash-map lookups without caching mutable scene state.
- Four-vertex triangle strips replace six-vertex triangle lists: one third fewer
  vertex invocations per sprite. Pixel tests confirm orientation and blending.
- Matching-rate, integer-position audio uses contiguous mixing loops. Streaming
  processes both ring-buffer slices and drains once per block. Fractional positions
  and differing rates retain interpolation, lookahead, and underrun behavior.
- Added reproducible percentile benchmarks and SIMD/audio regression tests.

No extra worker dispatch or GPU culling was introduced. Scanning the publicly
mutable array of sprites still costs memory bandwidth; a compact bounds layout
or explicitly tracked scene updates would be a larger API/design change.

## Reproduce

```powershell
cargo test --release preparation_percentiles -- --ignored --nocapture
cargo test --release mixer_percentiles -- --ignored --nocapture
cargo run --release --example bench_renderer -- 10000
cargo test --release --lib -- --test-threads=1
cargo run --release --example verify_render
cargo clippy --lib --tests -- -D warnings
```

Validation: 26 tests passed, five opt-in tests ignored by the ordinary suite;
the two new performance tests were run explicitly. Strict Clippy passed.
GPU verification passed alpha, orientation, flips, culling-before-upload,
clearing, texture retirement, 10k batching, resize, minimize/restore and input.
