use std::sync::Arc;
use velocity::Scene;
use velocity::{Color, Renderer, Sprite, Texture, Window, vec2};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

fn pixel(bytes: &[u8], width: usize, x: usize, y: usize, expected: [u8; 4]) {
    let actual = &bytes[(y * width + x) * 4..][..4];
    for (&actual, expected) in actual.iter().zip(expected) {
        assert!(
            actual.abs_diff(expected) <= 2,
            "pixel ({x},{y}): {actual} != {expected}"
        );
    }
}
fn main() -> Result<(), String> {
    let window = Window::new("Velocity Vulkan verification", 128, 128)?;
    let mut renderer = Renderer::with_vsync(&window, false)?;
    let mut scene = Scene::new();
    let mut handles = Vec::new();
    println!("GPU: {}", renderer.gpu_name());
    renderer.clear_color = Color::new(0., 0., 1., 1.);
    let mut texture = Texture::from_rgba(1, 2, vec![255, 0, 0, 255, 0, 255, 0, 255])?;
    texture.filter = velocity::TextureFilter::Nearest;
    let texture = Arc::new(texture);
    assert_eq!(
        renderer.surface_size(renderer.default_surface)?,
        [1280, 720]
    );
    assert_eq!(
        renderer.surface_position(renderer.default_surface, vec2(64., 64.))?,
        vec2(640., 360.)
    );
    let mut fallback = Sprite::new(texture.clone());
    fallback.position = vec2(640., 0.);
    fallback.size = Some(vec2(320., 320.));
    scene.add(fallback);
    let (_, frame) = renderer.render_capture(&mut scene)?;
    pixel(&frame, 128, 72, 36, [255, 0, 0, 255]);
    pixel(&frame, 128, 72, 52, [0, 255, 0, 255]);
    let fallback_pixels = renderer.read_surface(renderer.default_surface)?;
    pixel(&fallback_pixels, 128, 72, 8, [255, 0, 0, 255]);
    scene.clear();
    handles.clear();
    renderer.render(&mut scene)?;
    let target = renderer.create_surface(128, 128, vec2(16., 16.))?;
    let canvas = renderer.create_surface(128, 128, vec2(0., 0.))?;
    for (position, surface) in [(vec2(0., 0.), Some(target)), (vec2(64., 16.), Some(canvas))] {
        let mut sprite = Sprite::new(texture.clone());
        sprite.position = position;
        sprite.size = Some(vec2(32., 32.));
        sprite.surface = surface;
        sprite.color.a = 0.5;
        if sprite.surface.is_none() {
            sprite.surface = Some(canvas);
        }
        handles.push(scene.add(sprite));
    }
    for kind in 0..3 {
        let mut sprite = Sprite::new(Arc::new(Texture::from_rgba(1, 1, vec![255; 4])?));
        match kind {
            0 => sprite.visible = false,
            1 => sprite.color.a = 0.,
            _ => sprite.position.x = 128.,
        }
        if sprite.surface.is_none() {
            sprite.surface = Some(canvas);
        }
        handles.push(scene.add(sprite));
    }
    window.poll_events();
    println!("Window size: {:?}", window.size());
    let (stats, frame) = renderer.render_capture(&mut scene)?;
    assert_eq!(
        (
            stats.drawn,
            stats.culled,
            stats.texture_uploads,
            stats.draw_calls
        ),
        (2, 3, 0, 4)
    );
    assert_eq!(renderer.cached_texture_count(), 1);
    for x in [24, 72] {
        pixel(&frame, 128, x, 24, [128, 0, 127, 255]);
        pixel(&frame, 128, x, 40, [0, 128, 127, 255]);
    }
    let offscreen = renderer.read_surface(target)?;
    pixel(&offscreen, 128, 8, 8, [128, 0, 0, 128]);
    pixel(&offscreen, 128, 8, 24, [0, 128, 0, 128]);
    scene[handles[1]].size = Some(vec2(32., -32.));
    let (_, frame) = renderer.render_capture(&mut scene)?;
    pixel(&frame, 128, 72, 24, [0, 128, 127, 255]);
    pixel(&frame, 128, 72, 40, [128, 0, 127, 255]);
    renderer.set_surface_position(target, vec2(-24., 16.))?;
    scene[handles[0]].size = Some(vec2(8., 8.));
    let stats = renderer.render(&mut scene)?;
    assert_eq!((stats.drawn, stats.culled), (1, 4));
    scene.clear();
    handles.clear();
    drop(texture);
    let (_, frame) = renderer.render_capture(&mut scene)?;
    pixel(&frame, 128, 24, 24, [0, 0, 255, 255]);
    assert_eq!(renderer.cached_texture_count(), 0);
    let white = Arc::new(Texture::from_rgba(1, 1, vec![255; 4])?);
    for color in [Color::new(1., 0., 0., 0.5), Color::new(0., 1., 0., 0.5)] {
        let mut sprite = Sprite::new(white.clone());
        sprite.position = vec2(64., 64.);
        sprite.size = Some(vec2(16., 16.));
        sprite.color = color;
        if sprite.surface.is_none() {
            sprite.surface = Some(canvas);
        }
        handles.push(scene.add(sprite));
    }
    let (stats, frame) = renderer.render_capture(&mut scene)?;
    assert_eq!(stats.draw_calls, 2);
    pixel(&frame, 128, 68, 68, [64, 128, 64, 255]);
    scene.clear();
    handles.clear();
    for index in 0..10_000 {
        let mut sprite = Sprite::new(white.clone());
        sprite.position = vec2((index % 100) as f32, (index / 100) as f32);
        if sprite.surface.is_none() {
            sprite.surface = Some(canvas);
        }
        handles.push(scene.add(sprite));
    }
    for frame in 0..8 {
        let stats = renderer.render(&mut scene)?;
        assert_eq!((stats.drawn, stats.draw_calls), (10_000, 2));
        assert_eq!(stats.texture_uploads, 0, "frame {frame}");
    }
    unsafe {
        SetWindowPos(
            window.hwnd(),
            std::ptr::null_mut(),
            0,
            0,
            240,
            220,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    window.poll_events();
    let size = window.size();
    let (_, frame) = renderer.render_capture(&mut scene)?;
    assert_eq!(frame.len(), size[0] as usize * size[1] as usize * 4);
    unsafe {
        ShowWindow(window.hwnd(), SW_MINIMIZE);
    }
    window.poll_events();
    assert!(window.size().contains(&0));
    assert_eq!(renderer.render(&mut scene)?.draw_calls, 0);
    unsafe {
        ShowWindow(window.hwnd(), SW_RESTORE);
    }
    window.poll_events();
    renderer.render(&mut scene)?;
    unsafe {
        PostMessageW(window.hwnd(), WM_KEYDOWN, 0x41, 1);
        PostMessageW(window.hwnd(), WM_MOUSEMOVE, 0, 23 | (37 << 16));
    }
    window.poll_events();
    assert!(window.key_down(0x41));
    assert!(window.key_just_pressed(0x41));
    window.poll_events();
    assert!(!window.key_just_pressed(0x41));
    unsafe {
        PostMessageW(window.hwnd(), WM_KEYDOWN, 0x41, 1);
    }
    window.poll_events();
    assert!(!window.key_just_pressed(0x41));
    assert_eq!(window.input().mouse, [23., 37.]);
    unsafe {
        PostMessageW(window.hwnd(), WM_KEYUP, 0x41, 1);
    }
    window.poll_events();
    assert!(!window.key_down(0x41));
    unsafe {
        PostMessageW(window.hwnd(), WM_KEYDOWN, 0x41, 1);
        PostMessageW(window.hwnd(), WM_KEYUP, 0x41, 1);
    }
    window.poll_events();
    assert!(window.key_just_pressed(0x41));
    assert!(!window.key_down(0x41));
    scene.clear();
    handles.clear();
    drop(white);
    for index in 0..300 {
        let mut sprite = Sprite::new(Arc::new(Texture::from_rgba(1, 1, vec![255; 4])?));
        sprite.position = vec2((index % 100) as f32, (index / 100) as f32);
        if sprite.surface.is_none() {
            sprite.surface = Some(canvas);
        }
        handles.push(scene.add(sprite));
    }
    assert_eq!(renderer.render(&mut scene)?.texture_uploads, 300);
    scene.clear();
    handles.clear();
    for _ in 0..4 {
        renderer.render(&mut scene)?;
    }
    assert_eq!(renderer.cached_texture_count(), 0);
    scene.clear();
    handles.clear();
    let surface = renderer.create_surface(128, 64, vec2(0., 0.))?;
    let mut sprite = Sprite::new(Arc::new(Texture::from_rgba(1, 1, vec![255; 4])?));
    sprite.surface = Some(surface);
    sprite.position = vec2(120., 0.);
    sprite.size = Some(vec2(32., 64.));
    handles.push(scene.add(sprite));
    for (width, height) in [(256, 192), (512, 384)] {
        unsafe {
            let mut rect = windows_sys::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            AdjustWindowRectEx(&mut rect, WS_OVERLAPPEDWINDOW, 0, 0);
            SetWindowPos(
                window.hwnd(),
                std::ptr::null_mut(),
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        window.poll_events();
        assert_eq!(window.size(), [width as u32, height as u32]);
        let (_, frame) = renderer.render_capture(&mut scene)?;
        assert_eq!(
            renderer.surface_size(surface)?,
            [height as u32, height as u32 / 2]
        );
        let top = height / 4;
        let right = (width + height) / 2 - 1;
        pixel(
            &frame,
            width as usize,
            right as usize,
            (top + 8) as usize,
            [255; 4],
        );
        pixel(
            &frame,
            width as usize,
            right as usize,
            (top - 1) as usize,
            [0, 0, 255, 255],
        );
        pixel(
            &frame,
            width as usize,
            ((width - height) / 2 + 1) as usize,
            (top + 8) as usize,
            [0, 0, 255, 255],
        );
        assert_eq!(
            renderer.surface_position(surface, vec2(width as f32 / 2., height as f32 / 2.))?,
            vec2(64., 32.)
        );
    }
    // Deliberately oversized geometry must never touch any bar pixel.
    scene.clear();
    handles.clear();
    for surface in [None, Some(surface)] {
        let mut sprite = Sprite::new(Arc::new(Texture::from_rgba(1, 1, vec![255; 4])?));
        sprite.surface = surface;
        sprite.position = vec2(-10000., -10000.);
        sprite.size = Some(vec2(30000., 30000.));
        if sprite.surface.is_none() {
            sprite.surface = Some(canvas);
        }
        handles.push(scene.add(sprite));
    }
    for (width, height) in [(301, 200), (200, 301), (319, 180), (180, 319)] {
        unsafe {
            let mut rect = windows_sys::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            AdjustWindowRectEx(&mut rect, WS_OVERLAPPEDWINDOW, 0, 0);
            SetWindowPos(
                window.hwnd(),
                std::ptr::null_mut(),
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        window.poll_events();
        verify_bars(&mut renderer, &mut scene, &window, [128, 128])?;
    }
    unsafe {
        ShowWindow(window.hwnd(), SW_MAXIMIZE);
    }
    window.poll_events();
    verify_bars(&mut renderer, &mut scene, &window, [128, 128])?;
    unsafe {
        ShowWindow(window.hwnd(), SW_RESTORE);
    }
    window.poll_events();
    let client = window.size();
    window.set_resizable(false);
    window.poll_events();
    assert_eq!(window.size(), client);
    unsafe {
        assert_eq!(
            GetWindowLongPtrW(window.hwnd(), GWL_STYLE) as u32 & (WS_THICKFRAME | WS_MAXIMIZEBOX),
            0
        );
    }
    window.set_resizable(true);
    window.poll_events();
    assert_eq!(window.size(), client);
    let wide_window = Window::new("16:9 canvas verification", 320, 180)?;
    let mut wide_renderer = Renderer::with_vsync(&wide_window, false)?;
    let mut wide_scene = Scene::new();
    assert_eq!(wide_renderer.clear_color, Color::BLACK);
    wide_renderer.clear_color = Color::new(1., 0., 1., 1.);
    let mut oversized = Sprite::new(Arc::new(Texture::from_rgba(1, 1, vec![255; 4])?));
    oversized.position = vec2(-10000., -10000.);
    oversized.size = Some(vec2(30000., 30000.));
    wide_scene.add(oversized);
    for mode in [SW_MAXIMIZE, SW_RESTORE, SW_MAXIMIZE] {
        unsafe {
            ShowWindow(wide_window.hwnd(), mode);
        }
        wide_window.poll_events();
        verify_bars(
            &mut wide_renderer,
            &mut wide_scene,
            &wide_window,
            [320, 180],
        )?;
    }
    println!(
        "PASS: alpha, orientation, flips, culling-before-upload, clearing, texture retirement, 10k batching, resize, minimize/restore, input"
    );
    Ok(())
}

fn verify_bars(
    renderer: &mut Renderer,
    scene: &mut Scene,
    window: &Window,
    canvas: [u32; 2],
) -> Result<(), String> {
    let [w, h] = window.size();
    let scale = (w as f64 / canvas[0] as f64).min(h as f64 / canvas[1] as f64);
    let cw = (canvas[0] as f64 * scale).floor() as u32;
    let ch = (canvas[1] as f64 * scale).floor() as u32;
    let (left, top) = ((w - cw) / 2, (h - ch) / 2);
    let (_, frame) = renderer.render_capture(scene)?;
    assert_eq!(frame.len(), (w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let outside = x < left || x >= left + cw || y < top || y >= top + ch;
            let expected = if outside { [0, 0, 0, 255] } else { [255; 4] };
            let actual = &frame[((y * w + x) * 4) as usize..][..4];
            assert_eq!(actual, expected, "canvas pixel ({x},{y}) in {w}x{h}");
        }
    }
    Ok(())
}
