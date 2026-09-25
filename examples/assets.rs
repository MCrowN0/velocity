use velocity::{Renderer, Scene, Sprite, Texture, get_assets};

struct Level;
impl Scene for Level {
    fn setup(&mut self, renderer: &mut Renderer) -> Result<(), String> {
        // Put an image at assets/player.png before running.
        let texture = get_assets()?.load::<Texture>("player.png")?;
        renderer.sprites.push(Sprite::new(texture));
        Ok(())
    }
}
fn main() -> Result<(), String> {
    Level.run()
}
