use std::{sync::Arc, time::Instant};
use velocity::{
    Assets, Renderer, Sprite, Texture, TextureCompression::*, TextureLoadOptions, Window, vec2,
};

fn main() -> Result<(), String> {
    let directory = std::env::temp_dir().join(format!("velocity-assets-{}", std::process::id()));
    let root = directory.join("assets");
    std::fs::create_dir_all(root.join("nested")).map_err(|e| e.to_string())?;
    let original = std::env::current_dir().map_err(|e| e.to_string())?;
    let image = image::RgbaImage::from_pixel(3, 5, image::Rgba([255, 128, 64, 255]));
    image
        .save(root.join("nested/test.png"))
        .map_err(|e| e.to_string())?;
    image
        .save(root.join("test.avif"))
        .map_err(|e| e.to_string())?;
    image::RgbaImage::from_pixel(7, 1, image::Rgba([255, 128, 64, 128]))
        .save(root.join("alpha.png"))
        .map_err(|e| e.to_string())?;
    std::env::set_current_dir(&directory).map_err(|e| e.to_string())?;
    let assets = Assets::new();
    std::env::set_current_dir(original).map_err(|e| e.to_string())?;
    assert_eq!(assets.len(), 3);
    let window = Window::with_visibility("Asset verification", 128, 128, false)?;
    let mut renderer = Renderer::with_vsync(&window, false)?;
    renderer.attach_assets(&assets)?;
    let surface = renderer.create_surface(128, 128, vec2(0., 0.))?;
    println!("GPU: {}", renderer.gpu_name());
    for compression in [BC1, BC2, BC3, BC4, BC5, BC6H, BC7, RGBA8] {
        let start = Instant::now();
        let texture =
            assets.load_with::<Texture>("nested/test.png", TextureLoadOptions { compression })?;
        assert!(texture.pixels().is_empty());
        assert_eq!((texture.width(), texture.height()), (3, 5));
        let mut sprite = Sprite::new(texture);
        sprite.size = Some(vec2(128., 128.));
        sprite.surface = Some(surface);
        renderer.sprites = vec![sprite];
        let (stats, pixels) = renderer.render_capture()?;
        assert_eq!(stats.texture_uploads, 0);
        let expected = match compression {
            BC4 => [255, 0, 0, 255],
            BC5 => [255, 128, 0, 255],
            _ => [255, 128, 64, 255],
        };
        for (actual, expected) in pixels[(64 * 128 + 64) * 4..][..4].iter().zip(expected) {
            assert!(
                actual.abs_diff(expected) <= 5,
                "{compression:?}: {actual} != {expected}"
            );
        }
        println!("{compression:?}: {:?}", start.elapsed());
    }
    let cached = assets.preload::<Texture>("test.avif")?;
    assert!(Arc::ptr_eq(&cached, &assets.load::<Texture>("test.avif")?));
    let shared = assets.clone();
    let worker = std::thread::spawn(move || shared.load::<Texture>("test.avif"))
        .join()
        .unwrap()?;
    assert!(Arc::ptr_eq(&cached, &worker));
    drop(worker);
    let shared = assets.clone();
    assert!(
        std::thread::spawn(move || shared.load::<Texture>("nested/test.png"))
            .join()
            .unwrap()
            .is_err()
    );
    assets.deload("test.avif");
    let mut sprite = Sprite::new(cached);
    sprite.size = Some(vec2(128., 128.));
    sprite.surface = Some(surface);
    renderer.sprites = vec![sprite];
    let (_, pixels) = renderer.render_capture()?;
    for (actual, expected) in pixels[(64 * 128 + 64) * 4..][..4]
        .iter()
        .zip([255, 128, 64, 255])
    {
        assert!(
            actual.abs_diff(expected) <= 8,
            "AVIF: {actual} != {expected}"
        );
    }
    renderer.sprites.clear();
    for compression in [BC2, BC3, BC7, RGBA8] {
        let texture =
            assets.load_with::<Texture>("alpha.png", TextureLoadOptions { compression })?;
        let mut sprite = Sprite::new(texture);
        sprite.size = Some(vec2(128., 128.));
        sprite.surface = Some(surface);
        renderer.sprites = vec![sprite];
        let (_, pixels) = renderer.render_capture()?;
        let alpha = if compression == BC2 { 136 } else { 128 };
        for (actual, expected) in
            pixels[(64 * 128 + 64) * 4..][..4]
                .iter()
                .zip([alpha, alpha / 2, alpha / 4, 255])
        {
            assert!(
                actual.abs_diff(expected) <= 5,
                "alpha {compression:?}: {actual} != {expected}"
            );
        }
    }
    renderer.sprites.clear();
    let fresh = assets.load::<Texture>("nested/test.png")?;
    let another = assets.load::<Texture>("nested/test.png")?;
    assert!(!Arc::ptr_eq(&fresh, &another));
    std::thread::spawn(move || drop((fresh, another)))
        .join()
        .unwrap();
    for _ in 0..3 {
        renderer.render()?;
    }
    assert_eq!(renderer.cached_texture_count(), 0);
    for _ in 0..8 {
        let mut handles = Vec::new();
        for _ in 0..32 {
            let texture = assets.load::<Texture>("nested/test.png")?;
            handles.push(Arc::downgrade(&texture));
            renderer.sprites.push(Sprite::new(texture));
        }
        renderer.render()?;
        renderer.sprites.clear();
        renderer.collect_unused()?;
        assert_eq!(renderer.cached_texture_count(), 0);
        assert!(handles.iter().all(|handle| handle.upgrade().is_none()));
    }
    // Handles may outlive the renderer without retaining its Vulkan device.
    let survivor = assets.load::<Texture>("test.avif")?;
    drop(renderer);
    drop(survivor);
    std::fs::remove_file(root.join("nested/test.png")).map_err(|e| e.to_string())?;
    std::fs::remove_file(root.join("test.avif")).map_err(|e| e.to_string())?;
    std::fs::remove_file(root.join("alpha.png")).map_err(|e| e.to_string())?;
    std::fs::remove_dir(root.join("nested")).map_err(|e| e.to_string())?;
    std::fs::remove_dir(root).map_err(|e| e.to_string())?;
    std::fs::remove_dir(directory).map_err(|e| e.to_string())?;
    println!("Asset verification passed");
    Ok(())
}
