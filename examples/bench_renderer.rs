use std::{sync::Arc, time::Instant};
use velocity::{Renderer, Sprite, Texture, Window, vec2};
fn main() -> Result<(), String> {
    let count: usize = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
    let window = Window::new("Velocity throughput benchmark", 960, 640)?;
    let mut renderer = Renderer::with_vsync(&window, false)?;
    let texture = Arc::new(Texture::from_rgba(1, 1, vec![255; 4])?);
    for index in 0..count * 4 {
        let mut sprite = Sprite::new(texture.clone());
        sprite.position = vec2(
            ((index % count) % 120 * 8) as f32,
            ((index % count) / 120 % 80 * 8) as f32,
        );
        sprite.size = Some(vec2(8., 8.));
        match index / count {
            1 => sprite.visible = false,
            2 => sprite.color.a = 0.,
            3 => sprite.position.x = 2000.,
            _ => {}
        }
        renderer.sprites.push(sprite);
    }
    let mut samples = Vec::with_capacity(600);
    for frame in 0..720 {
        if !window.poll_events() {
            return Err("benchmark window closed".into());
        }
        let start = Instant::now();
        let stats = renderer.render()?;
        let elapsed = start.elapsed();
        assert_eq!(
            (stats.drawn, stats.culled, stats.draw_calls),
            (count, count * 3, 2)
        );
        if frame > 0 {
            assert_eq!(stats.texture_uploads, 0);
        }
        if frame >= 120 {
            samples.push(elapsed.as_secs_f64() * 1000.);
        }
    }
    samples.sort_by(f64::total_cmp);
    //MESSY AS FUCK
    println!(
        "{{\"gpu\":\"{}\",\"visible\":{count},\"culled\":{},\"draw_calls\":2,\"median_ms\":{},\"p95_ms\":{},\"instance_bytes\":{}}}",
        renderer.gpu_name(),
        count * 3,
        samples[300],
        samples[570],
        (count + 1) * 48
    );
    Ok(())
}
