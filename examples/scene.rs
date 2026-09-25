use std::sync::Arc;
use velocity::{Renderer, Scene, Sprite, Texture, get_key_just_pressed, quit, vec2};

struct Level;
impl Scene for Level {
    fn setup(&mut self, renderer: &mut Renderer) -> Result<(), String> {
        let texture = Arc::new(Texture::from_rgba(1, 1, vec![255; 4])?);
        let mut sprite = Sprite::new(texture);
        sprite.position = vec2(100., 100.);
        sprite.size = Some(vec2(64., 64.));
        renderer.sprites.push(sprite);
        Ok(())
    }
    fn process(&mut self, _dt: f32) {
        if get_key_just_pressed(0x1B) {
            quit();
        }
    }
}
fn main() -> Result<(), String> {
    Level.run()
}
