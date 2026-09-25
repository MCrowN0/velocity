use super::*;
#[test]
fn texture_handles_do_not_keep_retirement_queue_alive() {
    let retired = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut texture = Texture::from_rgba(1, 1, vec![255; 4]).unwrap();
    texture.gpu = Some(assets::GpuHandle {
        owner: 1,
        key: 42,
        retired: Arc::downgrade(&retired),
    });
    assert_eq!(Arc::strong_count(&retired), 1);
    drop(texture);
    assert_eq!(*retired.lock().unwrap(), [42]);
    let mut texture = Texture::from_rgba(1, 1, vec![255; 4]).unwrap();
    texture.gpu = Some(assets::GpuHandle {
        owner: 1,
        key: 43,
        retired: Arc::downgrade(&retired),
    });
    let weak = Arc::downgrade(&retired);
    drop(retired);
    assert!(weak.upgrade().is_none());
    drop(texture);
}
fn sprite() -> Sprite {
    Sprite::new(Arc::new(Texture::from_rgba(2, 2, vec![255; 16]).unwrap()))
}
#[test]
fn rejects_invalid_images() {
    assert!(Texture::from_rgba(0, 1, vec![]).is_err());
    assert!(Texture::from_rgba(2, 2, vec![255; 15]).is_err());
}
#[test]
fn culls_hidden_transparent_and_outside_but_keeps_partial_and_black() {
    let mut s = sprite();
    let bounds = vec2(100., 100.);
    s.visible = false;
    assert!(drawable_size(&s, bounds).is_none());
    s.visible = true;
    s.color.a = 0.;
    assert!(drawable_size(&s, bounds).is_none());
    s.color = Color::BLACK;
    assert!(drawable_size(&s, bounds).is_some());
    for position in [vec2(-2., 0.), vec2(100., 0.), vec2(0., -2.), vec2(0., 100.)] {
        s.position = position;
        assert!(drawable_size(&s, bounds).is_none());
    }
    s.position = vec2(-1., -1.);
    assert!(drawable_size(&s, bounds).is_some());
}
#[test]
fn size_overrides_scale_and_flips_keep_top_left_anchor() {
    let mut s = sprite();
    s.scale = vec2(0., 0.);
    assert!(drawable_size(&s, vec2(100., 100.)).is_none());
    s.size = Some(vec2(-20., 10.));
    assert_eq!(drawable_size(&s, vec2(100., 100.)), Some(vec2(20., 10.)));
    s.position.x = f32::NAN;
    assert!(drawable_size(&s, vec2(100., 100.)).is_none());
}

#[test]
fn surface_fit_preserves_aspect_and_centers_with_logical_offset() {
    use crate::renderer::surface_layout;
    let logical = vec2(1280., 720.);
    assert_eq!(
        surface_layout(logical, vec2(2560., 1440.), Vec2::ZERO),
        (Vec2::ZERO, vec2(2560., 1440.))
    );
    assert_eq!(
        surface_layout(logical, vec2(1920., 1200.), Vec2::ZERO),
        (vec2(0., 60.), vec2(1920., 1080.))
    );
    assert_eq!(
        surface_layout(logical, vec2(2560., 1440.), vec2(10., -20.)),
        (vec2(20., -40.), vec2(2560., 1440.))
    );
}
#[test]
fn scene_data_can_cross_threads() {
    fn send<T: Send>() {}
    send::<Sprite>();
    send::<Texture>();
    send::<Surface>();
}

#[test]
fn canvas_bounds_round_inward_for_arbitrary_window_sizes() {
    for canvas in [[1280, 720], [720, 1280], [960, 640]] {
        for (w, h) in [
            (1920, 1080),
            (1920, 1009),
            (1001, 777),
            (1, 1),
            (1, 100),
            (100, 1),
        ] {
            let rect = crate::renderer::canvas_rect(
                canvas,
                ash::vk::Extent2D {
                    width: w,
                    height: h,
                },
            );
            assert!(rect.extent.width > 0 && rect.extent.height > 0);
            assert!(rect.offset.x >= 0 && rect.offset.y >= 0);
            assert!(rect.offset.x as u32 + rect.extent.width <= w);
            assert!(rect.offset.y as u32 + rect.extent.height <= h);
            let scale = (w as f64 / canvas[0] as f64).min(h as f64 / canvas[1] as f64);
            assert!((rect.extent.width as f64 - canvas[0] as f64 * scale).abs() <= 1.);
            assert!((rect.extent.height as f64 - canvas[1] as f64 * scale).abs() <= 1.);
        }
    }
}
