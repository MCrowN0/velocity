# Velocity

A game framework in development, written in Rust and built on Vulkan. Currently
supports Windows, with scenes, input handling, fixed updates, and sprite rendering.
Requires a Vulkan 1.3 driver with dynamic rendering.

The framework lives in `src/`; the demo and its assets live in `examples/demo/`.
Run the demo with:

```powershell
cargo run --release --example demo
```

Pass a frame count for a short run: `cargo run --release --example demo -- 120`.

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
  The default is 1280�720. Resizing or maximizing fits the canvas inside the
  window with opaque black bars and GPU clipping on every draw. The canvas
  background defaults to black; `renderer.clear_color` changes only the canvas.
- Surfaces provide logical coordinates and fit the canvas while keeping their
  aspect ratio. They draw beneath canvas sprites, in creation order. Direct
  sprites use physical pixels relative to the fitted canvas�s top-left corner.
  A standalone `Renderer` captures its canvas ratio from the window at creation.
- Mouse positions use physical window pixels. Use `renderer.surface_position`
  to convert them to surface coordinates.

## Checks

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo run --release --example verify_render
cargo run --release --example verify_scene
cargo run --release --example bench_renderer -- 10000
```

The verification examples open a window and exit when finished. Diagnostic
readbacks (`render_capture` and `read_surface`) block until complete and return
top-left RGBA; surface RGB is premultiplied.
