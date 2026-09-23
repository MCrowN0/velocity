mod common;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    platform::pump_events::EventLoopExtPumpEvents,
    window::{Window, WindowId},
};
#[derive(Default)]
struct App {
    window: Option<Window>,
    count: u64,
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.window = Some(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title("Winit input benchmark")
                            .with_inner_size(winit::dpi::PhysicalSize::new(320, 240))
                            .with_visible(false),
                    )
                    .unwrap(),
            );
        }
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::CursorMoved { .. }) {
            self.count += 1;
        }
    }
}
fn main() {
    let start = std::time::Instant::now();
    let mut events = EventLoop::new().unwrap();
    let mut app = App::default();
    events.pump_app_events(Some(std::time::Duration::ZERO), &mut app);
    let startup = start.elapsed();
    let RawWindowHandle::Win32(handle) = app
        .window
        .as_ref()
        .unwrap()
        .window_handle()
        .unwrap()
        .as_raw()
    else {
        unreachable!()
    };
    common::measure("winit", handle.hwnd.get() as _, startup, || {
        events.pump_app_events(Some(std::time::Duration::ZERO), &mut app);
        app.count
    });
}
