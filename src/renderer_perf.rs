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
    assert!(reject_default_chunk(
        &sprites.iter().collect::<Vec<_>>(),
        bounds
    ));
    for lane in 0..4 {
        for position in [vec2(0., 0.), vec2(-0.5, 0.), vec2(f32::NAN, 0.)] {
            sprites[lane].position = position;
            assert!(!reject_default_chunk(
                &sprites.iter().collect::<Vec<_>>(),
                bounds
            ));
        }
        sprites[lane].position = vec2(2000., 0.);
        sprites[lane].surface = Some(Surface {
            renderer: 1,
            index: 0,
            logical_size: [1280, 720],
        });
        assert!(!reject_default_chunk(
            &sprites.iter().collect::<Vec<_>>(),
            bounds
        ));
        sprites[lane].surface = None;
    }
    for len in 0..4 {
        assert!(!reject_default_chunk(
            &sprites[..len].iter().collect::<Vec<_>>(),
            bounds
        ));
    }
}

#[test]
#[ignore = "release performance benchmark; requires Vulkan"]
fn preparation_percentiles() {
    let window = Window::with_visibility("Preparation benchmark", 960, 640, false).unwrap();
    let mut renderer = Renderer::with_vsync(&window, false).unwrap();
    let mut scene = Scene::new();
    let texture = Arc::new(Texture::from_rgba(1, 1, vec![255; 4]).unwrap());
    for (name, visible, culled) in [
        ("mixed", 10_000, 30_000),
        ("offscreen", 0, 100_000),
        ("visible", 10_000, 0),
    ] {
        scene.clear();
        for i in 0..visible + culled {
            let mut sprite = Sprite::new(texture.clone());
            sprite.size = Some(vec2(8., 8.));
            sprite.position = if i < visible {
                vec2((i % 150 * 8) as f32, (i / 150 % 85 * 8) as f32)
            } else {
                vec2(2000., 0.)
            };
            scene.add(sprite);
        }
        let mut samples = Vec::with_capacity(2000);
        for iteration in 0..2200 {
            let start = Instant::now();
            let stats = black_box(
                renderer
                    .prepare(&mut scene, black_box(vec2(960., 640.)))
                    .unwrap(),
            );
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
// Test-only allocation instrumentation; enabled only around the switching workload.
mod animation_allocations {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    pub static ENABLED: AtomicBool = AtomicBool::new(false);
    pub static COUNT: AtomicUsize = AtomicUsize::new(0);
    pub struct Counting;
    fn record() {
        if ENABLED.load(Ordering::Relaxed) {
            COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            record();
            unsafe { System.alloc(layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            record();
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn realloc(&self, p: *mut u8, layout: Layout, n: usize) -> *mut u8 {
            record();
            unsafe { System.realloc(p, layout, n) }
        }
        unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
            unsafe { System.dealloc(p, layout) }
        }
    }
}
#[global_allocator]
static ANIMATION_BENCH_ALLOCATOR: animation_allocations::Counting = animation_allocations::Counting;

#[test]
#[ignore = "A-G release animation benchmark; requires Vulkan; run with --test-threads=1"]
#[allow(clippy::assertions_on_constants)]
fn animation_workloads() {
    // Fail when explicitly invoked in debug, but still compile other debug tests.
    assert!(!cfg!(debug_assertions), "run this benchmark with --release");
    use std::sync::atomic::Ordering;
    let texture = Arc::new(Texture::from_rgba(128, 96, vec![255; 128 * 96 * 4]).unwrap());
    let mut atlas = SpriteFrames::new(texture);
    // Valid nonempty frames keep F/G visibility counts exact. All workloads use
    // the same 515-frame atlas, alternating packed rotation with nonzero trimming.
    for i in 0..515 {
        let mut frame =
            crate::Frame::new([((i % 12) * 8) as f32, ((i / 12 % 10) * 8) as f32, 8., 6.]);
        frame.rotated = i % 2 != 0;
        frame.source_size = vec2(16., 16.);
        frame.offset = vec2(2., 3.);
        atlas.add_frame(&format!("frame{i:04}"), frame).unwrap();
    }
    atlas.shrink_to_fit();
    let atlas = Arc::new(atlas);
    let sequence: Vec<u32> = (0..515).collect();
    let make_sprite = || {
        let mut sprite = AnimatedSprite::new(atlas.clone());
        sprite.add_animation("first", &sequence, 24., true).unwrap();
        sprite.play("first").unwrap();
        sprite
    };
    let mut csv = String::from("workload,sample,total_ms,allocations\n");
    let mut measure = |name: &str, warmup: usize, count: usize, work: &mut dyn FnMut()| {
        for _ in 0..warmup {
            work();
        }
        let mut times = Vec::with_capacity(count);
        let mut allocations = 0;
        for sample in 0..count {
            animation_allocations::COUNT.store(0, Ordering::Relaxed);
            animation_allocations::ENABLED.store(name == "D", Ordering::Relaxed);
            let start = Instant::now();
            work();
            let ms = start.elapsed().as_secs_f64() * 1000.;
            animation_allocations::ENABLED.store(false, Ordering::Relaxed);
            let allocated = animation_allocations::COUNT.load(Ordering::Relaxed);
            allocations += allocated;
            times.push(ms);
            use std::fmt::Write;
            let allocation_text = if name == "D" {
                allocated.to_string()
            } else {
                String::new()
            };
            writeln!(csv, "{name},{sample},{ms:.9},{allocation_text}").unwrap();
        }
        times.sort_by(f64::total_cmp);
        let percentile = |p: f64| times[((count as f64 * p).ceil() as usize).saturating_sub(1)];
        let allocation_text = if name == "D" {
            allocations.to_string()
        } else {
            "not_measured".into()
        };
        println!(
            "{name}: median={:.6} p95={:.6} p99={:.6} ms; samples={count}; allocations_total={allocation_text}",
            percentile(0.5),
            percentile(0.95),
            percentile(0.99)
        );
        if name == "D" {
            assert_eq!(allocations, 0);
        }
    };
    let mut sprite = make_sprite();
    measure("A", 10, 101, &mut || {
        for _ in 0..1_000_000 {
            black_box(&mut sprite).update(black_box(1. / 60.));
        }
    });
    measure("B", 3, 31, &mut || {
        for _ in 0..10_000_000 {
            black_box(&mut sprite).update(black_box(1. / 60.));
        }
    });
    // Binary-exact rate/dt ensure exactly one frame, including wrap, per update.
    sprite.set_framerate("first", 32.).unwrap();
    let before = sprite.frame();
    sprite.update(1. / 32.);
    assert_eq!(sprite.frame(), (before + 1) % 515);
    measure("C", 10, 101, &mut || {
        for _ in 0..1_000_000 {
            black_box(&mut sprite).update(black_box(1. / 32.));
        }
    });
    sprite
        .add_animation("second", &sequence, 24., true)
        .unwrap();
    measure("D", 10, 101, &mut || {
        for i in 0..1_000_000 {
            black_box(&mut sprite)
                .play(black_box(if i & 1 == 0 { "first" } else { "second" }))
                .unwrap();
        }
    });
    let mut sprites: Vec<_> = (0..10_000).map(|_| make_sprite()).collect();
    measure("E", 20, 201, &mut || {
        for sprite in black_box(&mut sprites) {
            sprite.update(black_box(1. / 60.));
        }
    });
    drop(sprites);
    let window = Window::with_visibility("Animation CPU benchmark", 960, 640, false).unwrap();
    let mut renderer = Renderer::with_vsync(&window, false).unwrap();
    let mut scene = Scene::new();
    println!(
        "GPU: {}; atlas_frames={}; atlas_metadata_bytes={}",
        renderer.gpu_name(),
        atlas.len(),
        atlas.metadata_bytes()
    );
    for (name, total) in [("F", 10_000), ("G", 100_000)] {
        scene.clear();
        for i in 0..total {
            let mut sprite = make_sprite();
            sprite.position = if i < 10_000 {
                vec2((i % 100 * 12) as f32, (i / 100 % 50 * 12) as f32)
            } else {
                vec2(2000., 0.)
            };
            // Stagger phases, so E's synchronized cohort is not assumed by F/G.
            sprite.set_frame(i % 515).unwrap();
            scene.add(sprite);
        }
        // First prepare uploads the atlas and grows buffers; deliberately untimed.
        let stats = renderer.prepare(&mut scene, vec2(960., 640.)).unwrap();
        assert_eq!((stats.drawn, stats.culled), (10_000, total - 10_000));
        measure(name, 20, 201, &mut || {
            scene.update_animations(black_box(1. / 60.));
            let stats = black_box(
                renderer
                    .prepare(&mut scene, black_box(vec2(960., 640.)))
                    .unwrap(),
            );
            assert_eq!((stats.drawn, stats.culled), (10_000, total - 10_000));
            assert_eq!(stats.texture_uploads, 0);
            assert_eq!(stats.draw_calls, 2); // Atlas batch + surface composite.
        });
    }
    if let Ok(path) = std::env::var("VELOCITY_ANIMATION_BENCH_OUTPUT") {
        std::fs::write(path, csv).unwrap();
    }
}
#[test]
#[ignore = "requires Vulkan"]
fn animation_culling_respects_trim_and_replacement() {
    let window = Window::with_visibility("Trim rejection verification", 960, 640, false).unwrap();
    let mut renderer = Renderer::with_vsync(&window, false).unwrap();
    let mut scene = Scene::new();
    let texture = Arc::new(Texture::from_rgba(16, 16, vec![255; 16 * 16 * 4]).unwrap());
    for (offset, scale) in [(-4., 1.), (11., -1.)] {
        let mut atlas = SpriteFrames::new(texture.clone());
        let mut frame = crate::Frame::new([0., 0., 2., 3.]);
        frame.source_size = vec2(10., 10.);
        frame.offset = vec2(offset, 0.);
        atlas.add_frame("a", frame).unwrap();
        assert!(!atlas.contained);
        let mut sprite = AnimatedSprite::new(Arc::new(atlas));
        sprite.position = vec2(1280., 0.);
        sprite.scale = vec2(scale, 1.);
        scene.clear();
        let animated_handle = scene.add(sprite);
        assert_eq!(
            renderer
                .prepare(&mut scene, vec2(960., 640.))
                .unwrap()
                .drawn,
            1
        );
        let mut inside = SpriteFrames::new(texture.clone());
        inside
            .add_frame("inside", crate::Frame::new([0., 0., 2., 3.]))
            .unwrap();
        assert!(inside.contained);
        scene[animated_handle].frames = Arc::new(inside);
        let stats = renderer.prepare(&mut scene, vec2(960., 640.)).unwrap();
        assert_eq!((stats.drawn, stats.culled), (0, 1));
    }
}

#[test]
#[ignore = "requires Vulkan"]
fn mixed_sprite_order_including_interleaved_surfaces() {
    let window = Window::with_visibility("Scene ordering", 64, 64, false).unwrap();
    let mut renderer = Renderer::with_vsync(&window, false).unwrap();
    let a = renderer.create_surface(64, 64, Vec2::ZERO).unwrap();
    let b = renderer.create_surface(64, 64, Vec2::ZERO).unwrap();
    let texture = Arc::new(Texture::from_rgba(1, 1, vec![255; 4]).unwrap());
    let mut atlas = SpriteFrames::new(texture.clone());
    atlas
        .add_frame("a", crate::Frame::new([0., 0., 1., 1.]))
        .unwrap();
    let atlas = Arc::new(atlas);
    for middle_surface in [a, b] {
        for animated_first in [false, true] {
            let mut scene = Scene::new();
            for (i, color) in [
                Color::new(1., 0., 0., 0.5),
                Color::new(0., 1., 0., 0.5),
                Color::new(0., 0., 1., 0.5),
            ]
            .into_iter()
            .enumerate()
            {
                let surface = if i == 1 { middle_surface } else { a };
                if (i == 1) != animated_first {
                    let handle = scene.add(AnimatedSprite::new(atlas.clone()));
                    scene[handle].surface = Some(surface);
                    scene[handle].size = Some(vec2(64., 64.));
                    scene[handle].color = color;
                } else {
                    let handle = scene.add(Sprite::new(texture.clone()));
                    scene[handle].surface = Some(surface);
                    scene[handle].size = Some(vec2(64., 64.));
                    scene[handle].color = color;
                }
            }
            let (stats, pixels) = renderer.render_capture(&mut scene).unwrap();
            assert_eq!(stats.drawn, 3);
            let pixel = &pixels[(32 * 64 + 32) * 4..][..4];
            for (&actual, expected) in pixel.iter().zip([32u8, 64, 128, 255]) {
                assert!(actual.abs_diff(expected) <= 2, "{pixel:?}");
            }
        }
    }
}
