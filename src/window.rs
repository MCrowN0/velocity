use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    Win32WindowHandle, WindowHandle,
};
use std::{cell::Cell, num::NonZeroIsize, ptr::null_mut, rc::Rc, sync::OnceLock};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::*,
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture},
        WindowsAndMessaging::*,
    },
};

#[derive(Clone, Copy, Default, Debug)]
pub struct Input {
    pub mouse: [f32; 2],
    pub mouse_buttons: u8,
    pub wheel: f32,
    pub focused: bool,
}

struct State {
    width: Cell<u32>,
    height: Cell<u32>,
    close: Cell<bool>,
    input: Cell<Input>,
    keys: [Cell<bool>; 256],
    just_pressed: [Cell<bool>; 256],
    events: Cell<u64>,
}

struct NativeWindow {
    hwnd: HWND,
    state: Box<State>,
}
impl Drop for NativeWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

#[derive(Clone)]
pub struct Window(Rc<NativeWindow>);

impl Window {
    pub fn new(title: &str, width: u32, height: u32) -> Result<Self, String> {
        Self::with_visibility(title, width, height, true)
    }

    pub fn with_visibility(
        title: &str,
        width: u32,
        height: u32,
        visible: bool,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 || width > 32767 || height > 32767 {
            return Err("invalid window dimensions".into());
        }
        static CLASS: OnceLock<Result<u16, String>> = OnceLock::new();
        let class = CLASS
            .get_or_init(|| unsafe {
                SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
                let class = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(window_proc),
                    hInstance: GetModuleHandleW(std::ptr::null()),
                    lpszClassName: windows_sys::w!("VelocityNative"),
                    hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                    hbrBackground: windows_sys::Win32::Graphics::Gdi::GetStockObject(
                        windows_sys::Win32::Graphics::Gdi::BLACK_BRUSH,
                    ) as _,
                    ..std::mem::zeroed()
                };
                let atom = RegisterClassW(&class);
                if atom == 0 {
                    Err(std::io::Error::last_os_error().to_string())
                } else {
                    Ok(atom)
                }
            })
            .as_ref()
            .map_err(Clone::clone)?;
        let state = Box::new(State {
            width: Cell::new(width),
            height: Cell::new(height),
            close: Cell::new(false),
            input: Cell::new(Input::default()),
            keys: std::array::from_fn(|_| Cell::new(false)),
            just_pressed: std::array::from_fn(|_| Cell::new(false)),
            events: Cell::new(0),
        });
        let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            };
            AdjustWindowRectEx(&mut rect, WS_OVERLAPPEDWINDOW, 0, 0);
            let hwnd = CreateWindowExW(
                0,
                *class as usize as *const u16,
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                null_mut(),
                null_mut(),
                GetModuleHandleW(std::ptr::null()),
                &*state as *const State as *const _,
            );
            if hwnd.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let result = Self(Rc::new(NativeWindow { hwnd, state }));
            if visible {
                ShowWindow(hwnd, SW_SHOW);
            }
            Ok(result)
        }
    }

    pub fn poll_events(&self) -> bool {
        for key in &self.0.state.just_pressed {
            key.set(false);
        }
        let mut input = self.0.state.input.get();
        input.wheel = 0.;
        self.0.state.input.set(input);
        unsafe {
            let mut message = std::mem::zeroed();
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.message == WM_QUIT {
                    self.0.state.close.set(true);
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        !self.0.state.close.get()
    }
    pub fn wait_events(&self) {
        unsafe {
            WaitMessage();
        }
    }
    pub fn size(&self) -> [u32; 2] {
        [self.0.state.width.get(), self.0.state.height.get()]
    }
    pub fn input(&self) -> Input {
        self.0.state.input.get()
    }
    pub fn key_down(&self, key: impl Into<u8>) -> bool {
        let key = key.into();
        self.0.state.keys[key as usize].get()
    }
    pub fn key_just_pressed(&self, key: impl Into<u8>) -> bool {
        let key = key.into();
        self.0.state.just_pressed[key as usize].get()
    }
    pub fn set_resizable(&self, resizable: bool) {
        unsafe {
            let original = GetWindowLongPtrW(self.hwnd(), GWL_STYLE) as u32;
            let mut style = original;
            if resizable {
                style |= WS_THICKFRAME | WS_MAXIMIZEBOX;
            } else {
                style &= !(WS_THICKFRAME | WS_MAXIMIZEBOX);
            }
            if style == original {
                return;
            }
            let [width, height] = self.size();
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            };
            AdjustWindowRectExForDpi(
                &mut rect,
                style,
                0,
                GetWindowLongPtrW(self.hwnd(), GWL_EXSTYLE) as u32,
                GetDpiForWindow(self.hwnd()),
            );
            SetWindowLongPtrW(self.hwnd(), GWL_STYLE, style as isize);
            SetWindowPos(
                self.hwnd(),
                null_mut(),
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
    pub fn input_event_count(&self) -> u64 {
        self.0.state.events.get()
    }
    pub fn hwnd(&self) -> HWND {
        self.0.hwnd
    }
}

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let mut handle = Win32WindowHandle::new(NonZeroIsize::new(self.0.hwnd as isize).unwrap());
        handle.hinstance =
            NonZeroIsize::new(unsafe { GetModuleHandleW(std::ptr::null()) } as isize);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
    }
}
impl HasDisplayHandle for Window {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::windows())
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // The boxed State outlives HWND. Cell permits synchronous Win32 reentrancy
    // without constructing aliased mutable references.
    unsafe {
        if message == WM_GETMINMAXINFO {
            let info = &mut *(l as *mut MINMAXINFO);
            info.ptMinTrackSize = POINT { x: 64, y: 64 };
            return 0;
        }
        if message == WM_NCCREATE {
            let create = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
        if !state.is_null() {
            let state = &*state;
            let mut input = state.input.get();
            match message {
                WM_CLOSE => {
                    state.close.set(true);
                    return 0;
                }
                WM_SIZE => {
                    state.width.set((l as u32) & 0xffff);
                    state.height.set(((l as u32) >> 16) & 0xffff);
                }
                WM_DPICHANGED => {
                    let rect = &*(l as *const RECT);
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                WM_SETFOCUS => input.focused = true,
                WM_KILLFOCUS => {
                    input.focused = false;
                    input.mouse_buttons = 0;
                    for key in &state.just_pressed {
                        key.set(false);
                    }
                    for key in &state.keys {
                        key.set(false);
                    }
                }
                WM_MOUSEMOVE => {
                    input.mouse = [(l as u16 as i16) as f32, ((l >> 16) as u16 as i16) as f32];
                    state.events.set(state.events.get() + 1);
                }
                WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
                    if w < 256 {
                        let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
                        if down && !state.keys[w].get() {
                            state.just_pressed[w].set(true);
                        }
                        state.keys[w].set(down);
                    }
                    state.events.set(state.events.get() + 1);
                }
                WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
                    input.mouse_buttons |= match message {
                        WM_LBUTTONDOWN => 1,
                        WM_RBUTTONDOWN => 2,
                        _ => 4,
                    };
                    SetCapture(hwnd);
                }
                WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP => {
                    input.mouse_buttons &= !match message {
                        WM_LBUTTONUP => 1,
                        WM_RBUTTONUP => 2,
                        _ => 4,
                    };
                    if input.mouse_buttons == 0 {
                        ReleaseCapture();
                    }
                }
                WM_CAPTURECHANGED => input.mouse_buttons = 0,
                WM_MOUSEWHEEL => input.wheel += ((w >> 16) as u16 as i16) as f32 / 120.,
                WM_ERASEBKGND => return 1,
                _ => {}
            }
            state.input.set(input);
        }
        DefWindowProcW(hwnd, message, w, l)
    }
}
