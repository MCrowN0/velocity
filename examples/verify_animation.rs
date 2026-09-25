//! Hidden-window pixel, asset lifecycle, and allocation verification.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};
use velocity::Scene;
use velocity::{
    AnimatedSprite, Assets, Color, Renderer, SpriteFrames, Texture, TextureCompression,
    TextureLoadOptions, Window, vec2,
};
struct Count;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static ALLOCATOR: Count = Count;
const COLORS: [[u8; 4]; 6] = [
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [0, 0, 255, 255],
    [255, 255, 0, 255],
    [255, 0, 255, 255],
    [0, 255, 255, 255],
];
fn main() -> Result<(), String> {
    let directory = std::env::temp_dir().join(format!("velocity-animation-{}", std::process::id()));
    let root = directory.join("assets/nested");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let mut image = image::RgbaImage::new(16, 8);
    for y in 0..2 {
        for x in 0..3 {
            image.put_pixel(1 + x, 1 + y, image::Rgba(COLORS[(y * 3 + x) as usize]));
            image.put_pixel(8 + 1 - y, 1 + x, image::Rgba(COLORS[(y * 3 + x) as usize]));
        }
    }
    image
        .save(root.join("atlas.png"))
        .map_err(|e| e.to_string())?;
    let mut xml = String::from("<TextureAtlas imagePath='atlas.png'>");
    for rotated in [false, true] {
        for trimmed in [false, true] {
            for fx in [false, true] {
                for fy in [false, true] {
                    let (x, w, h) = if rotated { (8, 2, 3) } else { (1, 3, 2) };
                    xml.push_str(&format!("<SubTexture name='f{}' x='{x}' y='1' width='{w}' height='{h}' rotated='{rotated}' flipX='{fx}' flipY='{fy}' {} />",usize::from(rotated)*8+usize::from(trimmed)*4+usize::from(fx)*2+usize::from(fy),if trimmed {"frameX='-2' frameY='-1' frameWidth='7' frameHeight='6'"}else{""}));
                }
            }
        }
    }
    xml.push_str("</TextureAtlas>");
    std::fs::write(root.join("atlas.xml"), xml).map_err(|e| e.to_string())?;
    let original = std::env::current_dir().map_err(|e| e.to_string())?;
    std::env::set_current_dir(&directory).map_err(|e| e.to_string())?;
    let assets = Assets::new();
    std::env::set_current_dir(original).map_err(|e| e.to_string())?;
    let window = Window::with_visibility("AnimatedSprite verification", 64, 64, false)?;
    let mut renderer = Renderer::with_vsync(&window, false)?;
    let mut scene = Scene::new();
    renderer.attach_assets(&assets)?;
    let surface = renderer.create_surface(64, 64, vec2(0., 0.))?;
    renderer.clear_color = Color::BLACK;
    let options = TextureLoadOptions {
        compression: TextureCompression::RGBA8,
        filter: velocity::TextureFilter::Nearest,
    };
    let texture = assets.preload_with::<Texture>("nested/atlas.png", options)?;
    let frames = assets.preload_with::<SpriteFrames>("nested/atlas.xml", options)?;
    assert!(Arc::ptr_eq(&texture, &frames.texture));
    assert!(frames.texture.pixels().is_empty());
    assert!(Arc::ptr_eq(
        &frames,
        &assets.load_with::<SpriteFrames>("nested/atlas.xml", options)?
    ));
    let cloned = assets.clone();
    let worker =
        std::thread::spawn(move || cloned.load_with::<SpriteFrames>("nested/atlas.xml", options))
            .join()
            .unwrap()?;
    assert!(Arc::ptr_eq(&frames, &worker));
    drop(worker);
    let compressed = assets.preload::<SpriteFrames>("nested/atlas.xml")?;
    assert!(!Arc::ptr_eq(&compressed, &frames));
    assert!(compressed.texture.pixels().is_empty());
    drop(compressed);
    assets.deload("nested/atlas.xml");
    assets.deload("nested/atlas.png");
    let mut sprite = AnimatedSprite::new(frames.clone());
    sprite.surface = Some(surface);
    sprite.position = vec2(8., 8.);
    sprite.add_animation("all", &(0..16).collect::<Vec<_>>(), 24., true)?;
    sprite.play("all")?;
    let animated_handle = scene.add(sprite);
    for index in 0..16 {
        for sx in [-1., 1.] {
            for sy in [-1., 1.] {
                let sprite = &mut scene[animated_handle];
                sprite.set_frame(index)?;
                sprite.scale = vec2(sx * 4., sy * 4.);
                let f = frames.get(index).unwrap();
                let fx = f.flip_x ^ (sx < 0.);
                let fy = f.flip_y ^ (sy < 0.);
                let ox = if fx {
                    f.source_size.x - f.offset.x - 3.
                } else {
                    f.offset.x
                };
                let oy = if fy {
                    f.source_size.y - f.offset.y - 2.
                } else {
                    f.offset.y
                };
                let (stats, pixels) = renderer.render_capture(&mut scene)?;
                assert_eq!(stats.texture_uploads, 0);
                assert_eq!(stats.drawn, 1);
                for y in 0..2 {
                    for x in 0..3 {
                        let px = 8
                            + ((ox + if fx { 2. - x as f32 } else { x as f32 }) * 4.) as usize
                            + 2;
                        let py = 8
                            + ((oy + if fy { 1. - y as f32 } else { y as f32 }) * 4.) as usize
                            + 2;
                        assert_eq!(
                            &pixels[(py * 64 + px) * 4..][..4],
                            &COLORS[y * 3 + x],
                            "frame {index} scale {sx},{sy}, pixel {x},{y}"
                        );
                    }
                }
                assert_eq!(&pixels[(8 * 64 + 7) * 4..][..4], &[0, 0, 0, 255]);
            }
        }
    }
    // Stop outside the viewport while positive trim offsets bring visible pixels back in.
    scene[animated_handle].set_frame(4)?;
    scene[animated_handle].position = vec2(-8., 8.);
    scene[animated_handle].scale = vec2(4., 4.);
    assert_eq!(renderer.render_capture(&mut scene)?.0.drawn, 1);
    println!(
        "GPU {}: 64 rotation/trim/flip/scale combinations passed; compressed loading/cache/deload passed",
        renderer.gpu_name()
    );
    let sprite = &mut scene[animated_handle];
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    let start = Instant::now();
    for _ in 0..1_000_000 {
        sprite.update(std::hint::black_box(1. / 60.));
        std::hint::black_box(sprite.frame());
    }
    let elapsed = start.elapsed();
    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), before);
    println!(
        "1,000,000 updates: {elapsed:?}; zero allocations; metadata {} bytes",
        frames.metadata_bytes()
    );
    println!(
        "Object sizes: Sprite {} B, AnimatedSprite {} B, SpriteFrames {} B",
        std::mem::size_of::<velocity::Sprite>(),
        std::mem::size_of::<AnimatedSprite>(),
        std::mem::size_of::<SpriteFrames>()
    );
    let weak = Arc::downgrade(&frames);
    scene.clear();
    drop(frames);
    drop(texture);
    drop(assets);
    assert!(weak.upgrade().is_none());
    renderer.collect_unused()?;
    assert_eq!(renderer.cached_texture_count(), 0);
    drop(renderer);
    for name in ["atlas.png", "atlas.xml"] {
        std::fs::remove_file(root.join(name)).map_err(|e| e.to_string())?;
    }
    std::fs::remove_dir(&root).map_err(|e| e.to_string())?;
    std::fs::remove_dir(directory.join("assets")).map_err(|e| e.to_string())?;
    std::fs::remove_dir(directory).map_err(|e| e.to_string())?;
    Ok(())
}
