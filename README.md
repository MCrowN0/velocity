# Velocity

Small Rust game framework for Windows, using Vulkan 1.3.

Implement `Scene`, then call `MyScene.run()`. All callbacks are optional:

- `setup(renderer)` — load assets and create sprites once; returns `Result`.
- `process(dt)` — update each frame, with elapsed seconds.
- `render(renderer)` — change sprites each frame; returns `Result`.
- `tick()` — fixed 60 Hz worker update; callbacks never overlap.
- `enter()` / `exit()` — lifecycle hooks.

Use `Game::new(scene)` to change the title, canvas size or frame limit.
`switch_scene(next)` clears sprites and surfaces at the next frame boundary.

## Examples

- [Scene](examples/scene.rs): run a scene and draw a sprite.
- [Assets](examples/assets.rs): load textures from `assets/`.
- [Audio](examples/audio.rs): play a sound.

Run one with `cargo run --example scene`.

Textures default to SIMD-compressed BC7 and live on the GPU. `preload` keeps a
texture cached until `deload`; ordinary `load` leaves ownership to you.
Drop unused sprites/handles to release textures. GPU cleanup follows rendered
frames; `renderer.collect_unused()?` forces cleanup while rendering is paused.
Load new assets on the main thread; `tick` can only retrieve preloaded assets.

Audio supports memory and streaming playback. Keep players alive while playing;
dropping a player stops it. `Game` starts and stops the audio engine.

## Checks

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo test gpu_resources -- --ignored
cargo run --example verify_assets
cargo run --example verify_render
cargo run --example verify_scene
```

Requires Windows x64 and the Rust/MSVC toolchain. AVIF decoder and license details:
[vendor/avif](vendor/avif/README.md).

Copyright 2026 MCrowN. [Apache-2.0](LICENSE).
