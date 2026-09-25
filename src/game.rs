use crate::{Assets, AudioEngine, Renderer, Scene, Vec2, Window, vec2};
use std::{
    cell::RefCell,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub trait SceneBehavior: Send + 'static {
    /// Run this scene with the default game settings.
    fn run(self) -> Result<(), String>
    where
        Self: Sized,
    {
        Game::new(self).run()
    }
    fn enter(&mut self, _scene: &mut Scene) {}
    /// Called once after enter, before updates. Load assets and add sprites here.
    fn setup(&mut self, _scene: &mut Scene, _renderer: &mut Renderer) -> Result<(), String> {
        Ok(())
    }
    fn exit(&mut self, _scene: &mut Scene) {}
    fn process(&mut self, _scene: &mut Scene, _dt: f32) {}
    fn tick(&mut self, _scene: &mut Scene) {}
    fn render(&mut self, _scene: &mut Scene, _renderer: &mut Renderer) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
struct Snapshot {
    fps: f32,
    mouse: Vec2,
    buttons: u8,
    keys: [u64; 4],
    pressed: [u64; 4],
}
#[derive(Default)]
struct Context {
    assets: Option<Assets>,
    audio: Option<Arc<AudioEngine>>,
    input: Snapshot,
    next: Option<Box<dyn SceneBehavior>>,
    quit: bool,
    active: bool,
}
thread_local! { static CONTEXT: RefCell<Context> = RefCell::new(Context::default()); }
pub(crate) fn assets() -> Result<Assets, String> {
    CONTEXT.with(|c| {
        c.borrow()
            .assets
            .clone()
            .ok_or_else(|| "get_assets requires a scene callback".into())
    })
}

pub(crate) fn with_audio<T>(
    f: impl FnOnce(&AudioEngine) -> Result<T, String>,
) -> Result<T, String> {
    CONTEXT.with(|c| {
        let engine = c
            .borrow()
            .audio
            .clone()
            .ok_or("play_audio requires a scene callback")?;
        f(&engine)
    })
}

pub fn get_fps() -> f32 {
    CONTEXT.with(|c| c.borrow().input.fps)
}
pub fn get_mouse_position() -> Vec2 {
    CONTEXT.with(|c| c.borrow().input.mouse)
}
pub fn get_mouse_button_pressed(index: u8) -> bool {
    CONTEXT.with(|c| index < 3 && c.borrow().input.buttons & (1 << index) != 0)
}
/// Whether a named key or raw Windows virtual-key code is held.
pub fn get_key_pressed(key: impl Into<u8>) -> bool {
    let key = key.into();
    CONTEXT.with(|c| c.borrow().input.keys[key as usize / 64] & (1 << (key % 64)) != 0)
}
/// Whether a named key or raw Windows virtual-key code was pressed this frame.
pub fn get_key_just_pressed(key: impl Into<u8>) -> bool {
    let key = key.into();
    CONTEXT.with(|c| c.borrow().input.pressed[key as usize / 64] & (1 << (key % 64)) != 0)
}
pub fn switch_scene(scene: impl SceneBehavior) {
    CONTEXT.with(|c| {
        let mut c = c.borrow_mut();
        assert!(c.active, "switch_scene requires a scene callback");
        c.next = Some(Box::new(scene));
    });
}
pub fn quit() {
    CONTEXT.with(|c| {
        let mut c = c.borrow_mut();
        assert!(c.active, "quit requires a scene callback");
        c.quit = true;
    });
}

struct Runtime {
    assets: Option<Assets>,
    audio: Option<Arc<AudioEngine>>,
    scene: Box<dyn SceneBehavior>,
    objects: Scene,
    input: Snapshot,
    next: Option<Box<dyn SceneBehavior>>,
    quit: bool,
}
impl Runtime {
    fn call<T>(
        &mut self,
        tick: bool,
        callback: impl FnOnce(&mut dyn SceneBehavior, &mut Scene) -> T,
    ) -> T {
        let mut input = self.input;
        if tick {
            input.pressed = [0; 4];
        }
        CONTEXT.with(|c| {
            *c.borrow_mut() = Context {
                assets: self.assets.clone(),
                audio: self.audio.clone(),
                input,
                active: true,
                ..Context::default()
            }
        });
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                CONTEXT.with(|c| *c.borrow_mut() = Context::default());
            }
        }
        let _reset = Reset;
        let result = callback(self.scene.as_mut(), &mut self.objects);
        CONTEXT.with(|c| {
            let mut c = c.borrow_mut();
            if let Some(next) = c.next.take() {
                self.next = Some(next);
            }
            self.quit |= c.quit;
        });
        result
    }
}

struct Worker {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Worker {
    fn start(runtime: Arc<Mutex<Runtime>>) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let handle = thread::Builder::new()
            .name("velocity-tick".into())
            .spawn(move || {
                let period = Duration::from_secs_f64(1. / 60.);
                let mut deadline = Instant::now() + period;
                while !signal.load(Ordering::Acquire) {
                    let now = Instant::now();
                    if now < deadline {
                        thread::park_timeout(deadline - now);
                        continue;
                    }
                    {
                        let Ok(mut state) = runtime.lock() else {
                            break;
                        };
                        if state.quit {
                            break;
                        }
                        if state.next.is_none() {
                            state.call(true, |scene, objects| scene.tick(objects));
                        }
                    }
                    deadline += period;
                    if deadline <= Instant::now() {
                        deadline = Instant::now() + period;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
    fn finish(&mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            handle
                .join()
                .map_err(|_| "scene tick panicked".to_string())?;
        }
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

pub struct Game {
    pub assets: Assets,
    pub audio: AudioEngine,
    pub title: String,
    /// Initial client size and fixed canvas aspect ratio, preserved on resize.
    pub canvas_size: [u32; 2],
    pub resizable: bool,
    pub max_framerate: i32,
    pub initial_scene: Box<dyn SceneBehavior>,
}
impl Game {
    pub fn new(initial_scene: impl SceneBehavior) -> Self {
        Self {
            assets: Assets::new(),
            audio: AudioEngine::new(),
            title: "Velocity".into(),
            canvas_size: [1280, 720],
            resizable: true,
            max_framerate: -1,
            initial_scene: Box::new(initial_scene),
        }
    }
    pub fn run(mut self) -> Result<(), String> {
        if self.max_framerate != -1 && self.max_framerate <= 0 {
            return Err("max_framerate must be -1 or positive".into());
        }
        let window = Window::new(&self.title, self.canvas_size[0], self.canvas_size[1])?;
        window.set_resizable(self.resizable);
        let mut renderer = Renderer::with_vsync(&window, false)?;
        renderer.attach_assets(&self.assets)?;
        self.audio.start()?;
        let runtime = Arc::new(Mutex::new(Runtime {
            assets: Some(self.assets),
            audio: Some(Arc::new(self.audio)),
            scene: self.initial_scene,
            objects: Scene::new(),
            input: Snapshot::default(),
            next: None,
            quit: false,
        }));
        {
            let mut state = runtime.lock().unwrap();
            state.call(false, |scene, objects| scene.enter(objects));
            if let Err(error) =
                state.call(false, |scene, objects| scene.setup(objects, &mut renderer))
            {
                state.call(false, |scene, objects| scene.exit(objects));
                return Err(error);
            }
        }
        let mut worker = match Worker::start(runtime.clone()) {
            Ok(worker) => worker,
            Err(error) => {
                runtime
                    .lock()
                    .unwrap()
                    .call(false, |scene, objects| scene.exit(objects));
                return Err(error);
            }
        };
        let frame_period = (self.max_framerate > 0)
            .then(|| Duration::from_secs_f64(1. / self.max_framerate as f64));
        let result = (|| {
            let mut last = Instant::now();
            while window.poll_events() {
                let start = Instant::now();
                let dt = start.duration_since(last).as_secs_f32();
                last = start;
                let native = window.input();
                let mut input = Snapshot {
                    fps: if dt > 0. { 1. / dt } else { 0. },
                    mouse: vec2(native.mouse[0], native.mouse[1]),
                    buttons: native.mouse_buttons,
                    ..Snapshot::default()
                };
                for key in 0..=255u8 {
                    if window.key_down(key) {
                        input.keys[key as usize / 64] |= 1 << (key % 64);
                    }
                    if window.key_just_pressed(key) {
                        input.pressed[key as usize / 64] |= 1 << (key % 64);
                    }
                }
                {
                    let mut state = runtime.lock().map_err(|_| "scene callback panicked")?;
                    if let Some(error) = state.audio.as_ref().and_then(|audio| audio.error()) {
                        return Err(format!("audio output failed: {error}"));
                    }
                    state.input = input;
                    if state.quit {
                        break;
                    }
                    if let Some(next) = state.next.take() {
                        state.call(false, |scene, objects| scene.exit(objects));
                        state.objects = Scene::new();
                        state.scene = next;
                        renderer.clear_scene()?;
                        state.call(false, |scene, objects| scene.enter(objects));
                        state.call(false, |scene, objects| scene.setup(objects, &mut renderer))?;
                    }
                    if state.quit {
                        break;
                    }
                    state.call(false, |scene, objects| scene.process(objects, dt));
                    if state.quit {
                        break;
                    }
                    state.objects.update_animations(dt);
                    state.call(false, |scene, objects| scene.render(objects, &mut renderer))?;
                    renderer.render(&mut state.objects)?;
                }
                let period = if window.size().contains(&0) {
                    Some(Duration::from_millis(16))
                } else {
                    frame_period
                };
                if let Some(period) = period {
                    thread::sleep(period.saturating_sub(start.elapsed()));
                }
            }
            Ok(())
        })();
        let joined = worker.finish();
        if let Ok(mut state) = runtime.lock() {
            state.call(false, |scene, objects| scene.exit(objects));
        }
        result.and(joined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Empty;
    impl SceneBehavior for Empty {}
    fn runtime() -> Runtime {
        Runtime {
            assets: None,
            audio: None,
            scene: Box::new(Empty),
            objects: Scene::new(),
            input: Snapshot::default(),
            next: None,
            quit: false,
        }
    }
    #[test]
    fn named_keys_share_raw_key_state() {
        use crate::Key;

        let mut state = runtime();
        // WM_KEYDOWN supplies 0x28 for the Down arrow.
        state.input.keys[0] = 1 << 0x28;
        state.input.pressed[0] = 1 << 0x28;
        state.call(false, |_, _| {
            assert!(get_key_pressed(Key::Down));
            assert!(get_key_pressed(0x28));
            assert!(get_key_just_pressed(Key::Down));
            assert!(get_key_just_pressed(0x28));
            assert!(!get_key_pressed(Key::Up));
            assert!(!get_key_just_pressed(Key::Up));
        });
        state.call(true, |_, _| {
            assert!(get_key_pressed(Key::Down));
            assert!(!get_key_just_pressed(Key::Down));
        });
        assert!(!get_key_pressed(Key::Down));
        assert!(!get_key_just_pressed(Key::Down));
    }
    #[test]
    fn helpers_read_callback_snapshot_and_tick_has_no_frame_edges() {
        let mut state = runtime();
        state.input.fps = 120.;
        state.input.mouse = vec2(12., 34.);
        state.input.buttons = 5;
        state.input.keys[1] = 2; // A = 65
        state.input.pressed[1] = 2;
        state.call(false, |_, _| {
            assert_eq!(get_fps(), 120.);
            assert_eq!(get_mouse_position(), vec2(12., 34.));
            assert!(get_mouse_button_pressed(0));
            assert!(!get_mouse_button_pressed(1));
            assert!(get_mouse_button_pressed(2));
            assert!(!get_mouse_button_pressed(255));
            assert!(get_key_pressed(b'A'));
            assert!(get_key_just_pressed(b'A'));
            switch_scene(Empty);
        });
        assert!(state.next.is_some());
        state.call(true, |_, _| {
            assert!(get_key_pressed(b'A'));
            assert!(!get_key_just_pressed(b'A'));
            quit();
        });
        assert!(state.quit);
        assert!(!get_key_pressed(b'A'));
    }
    #[test]
    fn assets_are_available_in_callbacks_and_reset_afterward() {
        let mut state = runtime();
        state.assets = Some(Assets::new());
        assert!(assets().is_err());
        state.call(false, |_, _| assert!(assets().is_ok()));
        state.call(true, |_, _| assert!(assets().is_ok()));
        assert!(assets().is_err());
    }
    #[test]
    fn panicking_callback_releases_pending_scene_and_context() {
        struct Tracked(Arc<()>);
        impl SceneBehavior for Tracked {}
        let tracked = Arc::new(());
        let mut state = runtime();
        state.assets = Some(Assets::new());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            state.call(false, |_, _| {
                let scene = Tracked(tracked.clone());
                assert_eq!(Arc::strong_count(&scene.0), 2);
                switch_scene(scene);
                panic!("callback failure");
            });
        }));
        assert!(result.is_err());
        assert_eq!(Arc::strong_count(&tracked), 1);
        assert!(assets().is_err());
        CONTEXT.with(|c| assert!(!c.borrow().active));
    }
    #[test]
    fn worker_runs_elsewhere_and_stops_promptly() {
        struct Tick(std::sync::mpsc::Sender<thread::ThreadId>);
        impl SceneBehavior for Tick {
            fn tick(&mut self, _scene: &mut Scene) {
                self.0.send(thread::current().id()).unwrap();
                quit();
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let mut state = runtime();
        state.scene = Box::new(Tick(tx));
        let runtime = Arc::new(Mutex::new(state));
        let mut worker = Worker::start(runtime).unwrap();
        assert_ne!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            thread::current().id()
        );
        worker.finish().unwrap();
    }
    #[test]
    fn worker_panic_is_reported_and_joined() {
        struct Panics;
        impl SceneBehavior for Panics {
            fn tick(&mut self, _scene: &mut Scene) {
                panic!("test panic");
            }
        }
        let mut state = runtime();
        state.scene = Box::new(Panics);
        let runtime = Arc::new(Mutex::new(state));
        let mut worker = Worker::start(runtime.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !runtime.is_poisoned() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(worker.finish().is_err());
    }
}
