use std::sync::Arc;
use velocity::{
    Key, Renderer, Scene, SceneBehavior, Sprite, Texture, get_key_just_pressed, quit, vec2,
};

struct Level;
impl SceneBehavior for Level {
    fn setup(&mut self, scene: &mut Scene, _renderer: &mut Renderer) -> Result<(), String> {
        let texture = Arc::new(Texture::from_rgba(1, 1, vec![255; 4])?);
        let sprite = scene.add(Sprite::new(texture));
        scene[sprite].position = vec2(100., 100.);
        scene[sprite].size = Some(vec2(64., 64.));
        Ok(())
    }
    fn process(&mut self, _scene: &mut Scene, _dt: f32) {
        if get_key_just_pressed(Key::Escape) {
            quit();
        }
    }
}
fn main() -> Result<(), String> {
    Level.run()
}
