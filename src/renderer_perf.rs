use super::*;
use std::{hint::black_box, time::Instant};

#[test]
fn simd_rejection_never_discards_visible_or_explicit_surface_sprites() {
    let texture = Arc::new(Texture::from_rgba(1, 1, vec![255; 4]).unwrap());
    let mut sprites: Vec<_> = (0..4).map(|_| Sprite::new(texture.clone())).collect();
    let bounds = vec2(1280., 720.);
    for sprite in &mut sprites {
        sprite.position = vec2(2000., 0.);
    }
    assert!(reject_default_chunk(&sprites, bounds));
    for lane in 0..4 {
        for position in [vec2(0., 0.), vec2(-0.5, 0.), vec2(f32::NAN, 0.)] {
            sprites[lane].position = position;
            assert!(!reject_default_chunk(&sprites, bounds));
        }
        sprites[lane].position = vec2(2000., 0.);
        sprites[lane].surface = Some(Surface {
            renderer: 1,
            index: 0,
        });
        assert!(!reject_default_chunk(&sprites, bounds));
        sprites[lane].surface = None;
    }
    for len in 0..4 {
        assert!(!reject_default_chunk(&sprites[..len], bounds));
    }
}

#[test]
#[ignore = "release performance benchmark; requires Vulkan"]
fn preparation_percentiles() {
    let window = Window::with_visibility("Preparation benchmark", 960, 640, false).unwrap();
    let mut renderer = Renderer::with_vsync(&window, false).unwrap();
    let texture = Arc::new(Texture::from_rgba(1, 1, vec![255; 4]).unwrap());
    for (name, visible, culled) in [
        ("mixed", 10_000, 30_000),
        ("offscreen", 0, 100_000),
        ("visible", 10_000, 0),
    ] {
        renderer.sprites.clear();
        for i in 0..visible + culled {
            let mut sprite = Sprite::new(texture.clone());
            sprite.size = Some(vec2(8., 8.));
            sprite.position = if i < visible {
                vec2((i % 150 * 8) as f32, (i / 150 % 85 * 8) as f32)
            } else {
                vec2(2000., 0.)
            };
            renderer.sprites.push(sprite);
        }
        let mut samples = Vec::with_capacity(2000);
        for iteration in 0..2200 {
            let start = Instant::now();
            let stats = black_box(renderer.prepare(black_box(vec2(960., 640.))).unwrap());
            let elapsed = start.elapsed().as_secs_f64() * 1000.;
            assert_eq!((stats.drawn, stats.culled), (visible, culled));
            assert_eq!(stats.draw_calls, if visible == 0 { 0 } else { 2 });
            if iteration >= 200 {
                samples.push(elapsed);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "{name}: CPU prepare median={:.6} p95={:.6} p99={:.6} ms",
            samples[1000], samples[1900], samples[1980]
        );
    }
}
