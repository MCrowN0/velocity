# Benchmarks

Measured on 2026-09-23 with Windows 11, an Intel i5-11400H, an RTX 3050 Laptop GPU,
and Rust 1.94.1. Release builds use thin LTO, one codegen unit, and stripped symbols.
Background activity was not controlled, so treat these as local measurements.

## Window and input

Both implementations use a hidden 320×240 window. Each run measures startup,
10,000 empty event pumps, and 1,000 batches of 128 queued mouse events after
100 warmup batches. Event posting is outside the timed section.

Medians of seven runs ([raw results](window-input.jsonl)):

| Metric | windows-sys 0.61.2 | Winit 0.30.13 |
| --- | ---: | ---: |
| Startup | 26.58 ms | 40.42 ms |
| Empty pump | 6.45 ns | 184.27 ns |
| Median dispatch per event | 1.285 µs | 2.585 µs |
| p95 dispatch per event | 1.545 µs | 3.317 µs |
| Executable size | 180,224 bytes | 506,880 bytes |

This measures queued message processing, not hardware input latency. The native
benchmark uses `src/window.rs`; Winit uses `ApplicationHandler` and zero-timeout
`pump_app_events`. Winit is only a benchmark dependency.

```powershell
cargo build --manifest-path tools/window-bench/Cargo.toml --release --features winit
1..7 | ForEach-Object {
    & tools/window-bench/target/release/native.exe
    & tools/window-bench/target/release/winit.exe
}
```

## Renderer

A 960×640 window with vsync disabled, using 8×8 sprites sharing one texture.
For each visible sprite, three more are culled: hidden, transparent, and offscreen.
Each run warms up for 120 frames and measures 600.

Medians of three runs ([raw results](renderer.jsonl)):

| Workload | Median render time | p95 render time | Draw calls | Instance bytes/frame |
| --- | ---: | ---: | ---: | ---: |
| 1,000 visible + 3,000 culled | 0.3814 ms | 1.8159 ms | 1 | 48,000 |
| 10,000 visible + 30,000 culled | 0.6324 ms | 1.2677 ms | 1 | 480,000 |

Times measure the CPU `render()` call, including GPU and presentation waits,
not isolated GPU time. The lower p95 in the larger workload reflects noise.
Sharing one texture gives ideal batching; alternating textures adds draw calls.

```powershell
cargo build --release --example bench_renderer
1..3 | ForEach-Object {
    & target/release/examples/bench_renderer.exe 1000
    & target/release/examples/bench_renderer.exe 10000
}
```

The demo executable was 463 KiB, including its PNG and shaders, excluding the
system Vulkan loader and graphics driver.
