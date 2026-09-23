# Project: Velocity

A game framework in development, written in Rust and built on Vulkan. Currently
supports Windows, with scenes, input handling, fixed updates, and sprite rendering.
Requires a Vulkan 1.3 driver with dynamic rendering.

The framework lives in `src/`.

## Scenes

Implement `Scene` and pass it to `Game`:

```rust
use velocity::{Game, Scene, get_key_just_pressed, quit};

struct Menu;

impl Scene for Menu {
    fn process(&mut self, _dt: f32) {
        if get_key_just_pressed(0x1B) {
            quit();
        }
    }
}

fn main() -> Result<(), String> {
    let mut game = Game::new(Menu);
    game.title = "My game".into();
    game.canvas_size = [1280, 720];
    game.run()
}
```

All callbacks are optional. `enter` and `exit` handle setup and cleanup,
`process(dt)` runs each frame with elapsed seconds, and `render` prepares sprites.
`tick` runs at 60 Hz on a worker. Callbacks never overlap, so keep them short.

`switch_scene(next)` switches at the next frame boundary and clears sprites and
surfaces. Keep shared textures in your own data to reuse them. Set
`game.max_framerate` to a positive limit or `-1` for uncapped rendering.

## Input and rendering

- Input helpers work inside scene callbacks. Keys use Windows virtual-key codes;
  mouse buttons are 0 left, 1 right, and 2 middle.
- `get_key_pressed` reports held keys. `get_key_just_pressed` lasts one frame
  and is always false in `tick`.
- Add sprites to `renderer.sprites`. Textures use `Arc<Texture>` and can load PNG
  or RGBA8 data. A sprite's `size` overrides `scale`; negative sizes flip it.
- `game.canvas_size` sets the initial client size and fixed canvas aspect ratio.
  The default is 1280ï¿½720. Resizing or maximizing fits the canvas inside the
  window with opaque black bars and GPU clipping on every draw. The canvas
  background defaults to black; `renderer.clear_color` changes only the canvas.
- Surfaces provide logical coordinates and fit the canvas while keeping their
  aspect ratio and draw in creation order. Sprites without an explicit surface
  use `renderer.default_surface`, created at 1280×720 with position `Vec2::ZERO`.
  Scene switches recreate this default surface.
  A standalone `Renderer` captures its canvas ratio from the window at creation.
- Mouse positions use physical window pixels. Use `renderer.surface_position`
  to convert them to surface coordinates.

## Audio

`Game::new` creates `game.audio`; `run` starts its WASAPI backend before the first
scene callback and stops it on exit. In a callback, call `play_audio(&player)?`
(or handle the error in callbacks that return `()`). Keep the player in your scene;
dropping it stops playback. Outside Game, use an engine directly:

```rust,no_run
use velocity::{AudioEngine, AudioLoadMode, AudioPlayer, AudioSource};
use std::time::Duration;

# fn main() -> Result<(), String> {
let hit = AudioSource::load("hit.wav")?;
let music = AudioSource::load_with_mode("song.flac", AudioLoadMode::Stream)?;
let hit_player = AudioPlayer::new(hit.clone())?;
let music_player = AudioPlayer::new(music)?;

let mut engine = AudioEngine::new();
engine.start()?;
engine.play(&hit_player)?;
engine.play(&music_player)?;
music_player.set_volume(0.5)?;
music_player.seek(Duration::from_secs(30));
music_player.pause();
music_player.play(); // resume an attached player; replay from zero after EOF
// Keep the engine and players alive while your application runs.
# Ok(())
# }
```

- `load(path)` defaults to `Auto`; Rust's explicit-mode equivalent is
  `load_with_mode(path, mode)`. Auto streams when decoded **32-bit float PCM at the
  original sample rate and channel count exceeds 32 MiB**. Exactly 32 MiB stays in
  memory. Unknown or underestimated lengths are checked while decoding, stopping
  once the limit is crossed. Explicit `Memory` always decodes the whole file.
- Symphonia's `all` feature enables its supported codecs and containers, including
  WAV, FLAC, MP3, AAC/MP4, Vorbis/Ogg, AIFF, ALAC, CAF and Matroska. This does not
  imply support for every codec those containers can carry (for example Opus).
- Cloned sources share memory PCM. Mono stays mono in memory; surround is downmixed
  to stereo (LFE omitted). Each streaming player owns an independent decoder and a
  three-second stereo float buffer, about **1.1 MiB at 48 kHz**, plus codec/file
  buffers and one pending packet. A decoder worker sleeps when full or at EOF;
  playback wakes it at half capacity. Paused players fill once, then sleep.
- WASAPI uses shared, event-driven output at the device sample rate. The engine
  reuses its mix/output buffers. File access and decoding happen away from the
  output thread; that thread uses nonblocking buffer access. Underruns or lock
  contention produce silence without advancing the affected player.
- Memory seeking is immediate. Stream seeking clears read-ahead immediately, then
  uses the format's seek index and decodes to the requested sample on the worker.
  Seek cost depends on the format and available indexes. `position()` tracks mixed
  source audio, ahead of the speakers by the device's output latency.
- Sample-rate conversion uses inexpensive linear interpolation; it is not a
  band-limited mastering resampler. Mixed output is clamped to avoid clipping
  overflow. Up to 128 live players can be attached to one engine.
- Check `player.error()` for asynchronous decoding/seek errors and `engine.error()`
  for output failures. Game propagates output failures from `run`. Device loss
  currently requires creating a new engine; automatic device switching is not
  implemented. `AudioBackend::Asio` is an extension point and returns an explicit
  unsupported error when started.

Backend and seek API references: [WASAPI stream modes](https://docs.rs/wasapi/0.24.0/wasapi/enum.StreamMode.html)
and [Symphonia seeking](https://docs.rs/symphonia-core/0.5.5/symphonia_core/formats/enum.SeekMode.html).

## Checks

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo test wasapi_device_smoke -- --ignored
cargo test --release mixer_benchmark -- --ignored --nocapture
cargo run --release --example verify_render
cargo run --release --example verify_scene
cargo run --release --example bench_renderer -- 10000
```

The verification examples open a window and exit when finished. Diagnostic
readbacks (`render_capture` and `read_surface`) block until complete and return
top-left RGBA; surface RGB is premultiplied.

## License

Copyright 2026 MCrowN

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.
