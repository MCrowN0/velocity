use std::time::{Duration, Instant};
use windows_sys::Win32::{Foundation::HWND, UI::WindowsAndMessaging::*};

pub fn measure(name: &str, hwnd: HWND, startup: Duration, mut pump: impl FnMut() -> u64) {
    for _ in 0..100 {
        pump();
    }
    let start = Instant::now();
    for _ in 0..10_000 {
        std::hint::black_box(pump());
    }
    let empty_ns = start.elapsed().as_nanos() as f64 / 10_000.;
    let mut samples = Vec::new();
    for batch in 0..1100 {
        let before = pump();
        for index in 0..128 {
            let pos = 10 + index % 64;
            assert_ne!(
                unsafe { PostMessageW(hwnd, WM_MOUSEMOVE, 0, pos | (pos << 16)) },
                0
            );
        }
        let start = Instant::now();
        let deadline = start + Duration::from_secs(2);
        let count = loop {
            let count = pump() - before;
            if count >= 128 {
                break count;
            }
            assert!(
                Instant::now() < deadline,
                "only {count}/128 events dispatched"
            );
        };
        assert_eq!(count, 128);
        if batch >= 100 {
            samples.push(start.elapsed().as_nanos() as f64 / 128.);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{{\"backend\":\"{name}\",\"startup_ms\":{},\"empty_pump_ns\":{empty_ns},\"input_median_ns\":{},\"input_p95_ns\":{},\"events\":128000}}",
        startup.as_secs_f64() * 1000.,
        samples[500],
        samples[950]
    );
}
