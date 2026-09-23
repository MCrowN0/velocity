mod common;
#[allow(dead_code)]
#[path = "../../src/window.rs"]
mod window;
fn main() {
    let start = std::time::Instant::now();
    let window =
        window::Window::with_visibility("Native input benchmark", 320, 240, false).unwrap();
    let startup = start.elapsed();
    common::measure("windows-sys", window.hwnd(), startup, || {
        window.poll_events();
        window.input_event_count()
    });
}
