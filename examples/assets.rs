use velocity::{Renderer, Scene, SceneBehavior, Sprite, Texture, get_assets};

struct Level;
impl SceneBehavior for Level {
    fn setup(&mut self, scene: &mut Scene, _renderer: &mut Renderer) -> Result<(), String> {
        // Put an image at assets/player.png before running.
        let texture = get_assets()?.load::<Texture>("player.png")?;
        scene.add(Sprite::new(texture));
        Ok(())
    }
}
fn main() -> Result<(), String> {
    Level.run()
}
