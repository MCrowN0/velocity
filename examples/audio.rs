use velocity::{AudioPlayer, AudioSource, Renderer, Scene, SceneBehavior, play_audio};

struct Level(Option<AudioPlayer>);
impl SceneBehavior for Level {
    fn setup(&mut self, _scene: &mut Scene, _renderer: &mut Renderer) -> Result<(), String> {
        // Put a sound at assets/hit.wav before running.
        let player = AudioPlayer::new(AudioSource::load("assets/hit.wav")?)?;
        play_audio(&player)?;
        self.0 = Some(player);
        Ok(())
    }
}
fn main() -> Result<(), String> {
    Level(None).run()
}
