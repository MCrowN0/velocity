use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};
use velocity::{Game, Handle, Renderer, Scene, SceneBehavior, Sprite, Texture, quit, switch_scene};

struct TestScene {
    second: bool,
    sprite: Option<Handle<Sprite>>,
    retired: Option<std::sync::Weak<Texture>>,
    log: Arc<Mutex<Vec<&'static str>>>,
    main: ThreadId,
    rendered: bool,
    deadline: Instant,
}
impl SceneBehavior for TestScene {
    fn enter(&mut self, _scene: &mut Scene) {
        assert_eq!(thread::current().id(), self.main);
        self.log
            .lock()
            .unwrap()
            .push(if self.second { "enter2" } else { "enter1" });
    }
    fn exit(&mut self, _scene: &mut Scene) {
        assert_eq!(thread::current().id(), self.main);
        self.log
            .lock()
            .unwrap()
            .push(if self.second { "exit2" } else { "exit1" });
    }
    fn process(&mut self, _scene: &mut Scene, dt: f32) {
        assert_eq!(thread::current().id(), self.main);
        assert!(dt >= 0.);
        assert!(
            Instant::now() < self.deadline,
            "tick worker or transition stalled"
        );
    }
    fn setup(&mut self, scene: &mut Scene, _renderer: &mut Renderer) -> Result<(), String> {
        assert!(!self.rendered, "setup must run only once");
        if !self.rendered {
            assert!(scene.is_empty(), "transition must clear previous sprites");
            let texture = Arc::new(Texture::from_rgba(1, 1, vec![255; 4])?);
            let handle = scene.add(Sprite::new(texture.clone()));
            if let Some(old) = self.sprite {
                assert!(scene.get(old).is_none());
                assert!(self.retired.as_ref().unwrap().upgrade().is_none());
            }
            self.sprite = Some(handle);
            self.retired = Some(Arc::downgrade(&texture));
            self.rendered = true;
        }
        Ok(())
    }
    fn tick(&mut self, _scene: &mut Scene) {
        assert_ne!(thread::current().id(), self.main);
        if !self.rendered {
            return;
        }
        self.log
            .lock()
            .unwrap()
            .push(if self.second { "tick2" } else { "tick1" });
        if self.second {
            quit();
        } else {
            switch_scene(TestScene {
                second: true,
                sprite: self.sprite,
                retired: self.retired.clone(),
                log: self.log.clone(),
                main: self.main,
                rendered: false,
                deadline: self.deadline,
            });
        }
    }
}
fn main() -> Result<(), String> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut game = Game::new(TestScene {
        second: false,
        sprite: None,
        retired: None,
        log: log.clone(),
        main: thread::current().id(),
        rendered: false,
        deadline: Instant::now() + Duration::from_secs(10),
    });
    game.canvas_size = [128, 128];
    game.resizable = false;
    game.max_framerate = 120;
    game.run()?;
    assert_eq!(
        *log.lock().unwrap(),
        ["enter1", "tick1", "exit1", "enter2", "tick2", "exit2"]
    );
    struct FailedSetup(Arc<Mutex<Vec<&'static str>>>);
    impl SceneBehavior for FailedSetup {
        fn setup(&mut self, _scene: &mut Scene, _: &mut Renderer) -> Result<(), String> {
            Err("setup failure".into())
        }
        fn exit(&mut self, _scene: &mut Scene) {
            self.0.lock().unwrap().push("exit");
        }
    }
    impl Drop for FailedSetup {
        fn drop(&mut self) {
            self.0.lock().unwrap().push("drop");
        }
    }
    log.lock().unwrap().clear();
    assert_eq!(FailedSetup(log.clone()).run(), Err("setup failure".into()));
    assert_eq!(*log.lock().unwrap(), ["exit", "drop"]);
    println!("PASS: worker thread, lifecycle, transitions, scene cleanup, shutdown");
    Ok(())
}
