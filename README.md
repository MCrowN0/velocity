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

Use `get_key_pressed(velocity::Key::Down)` to check whether a key is held, or
`get_key_just_pressed(velocity::Key::Space)` for a press in the current frame.
`Key` includes arrows, letters, digits (`Num0`–`Num9`), numpad keys, modifiers,
and function keys (`F1`–`F12`). Both helpers and the `Window` key methods also
accept raw `u8` Windows virtual-key codes.

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

Both `Sprite` and `AnimatedSprite` support `center("X")`, `center("Y")`, and
`center("XY")` to set their position on the selected axes. Centering uses the
assigned surface's logical size, or 1280×720 when `surface` is `None`, and
accounts for scale or an explicit size override. Animated sprites use the
current frame's untrimmed size. Call again after changing size or frame if
you want to recenter. Unsupported axis strings panic.

## Animated sprites

```rust
use velocity::{AnimatedSprite, SpriteFrames, get_assets};
fn add_bf(renderer: &mut velocity::Renderer) -> Result<(), String> {
    let frames = get_assets()?.load::<SpriteFrames>("bf.xml")?;
    let mut bf = AnimatedSprite::new(frames);
    bf.add_by_prefix("idle", "BF idle dance", 24., true)?;
    bf.play("idle")?;
    bf.position.x = 100.;
    renderer.add_animated_sprite(bf);
    Ok(())
}
```

## Checks

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo test gpu_resources -- --ignored
cargo run --release --example verify_animation
cargo run --example verify_assets
cargo run --example verify_render
cargo run --example verify_scene
```

Requires Windows x64 and the Rust/MSVC toolchain. AVIF decoder and license details:
[vendor/avif](vendor/avif/README.md).

Copyright 2026 MCrowN. [Apache-2.0](LICENSE).
