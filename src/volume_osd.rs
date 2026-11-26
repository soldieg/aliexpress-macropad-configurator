use std::{
    ptr::{null, null_mut},
    sync::OnceLock,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};

const TIMER: usize = 1;
const DISPLAY_MS: u32 = 1400;
const WIDTH: i32 = 272;
const HEIGHT: i32 = 64;

#[derive(Default)]
pub struct VolumeOsd(HWND);

impl VolumeOsd {
    pub fn show(&mut self, level: u8) {
        unsafe {
            if self.0.is_null() {
                self.0 = open();
            }
            let window = self.0;
            if window.is_null() {
                return;
            }
            SetWindowLongPtrW(window, GWLP_USERDATA, isize::from(level.min(100)));
            let foreground = GetForegroundWindow();
            let monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let positioned = GetMonitorInfoW(monitor, &mut info) != 0;
            let work = info.rcWork;
            let (x, y) = if positioned {
                (
                    (work.left + work.right - WIDTH) / 2,
                    (work.bottom - HEIGHT - 92).max(work.top),
                )
            } else {
                (
                    (GetSystemMetrics(SM_CXSCREEN) - WIDTH) / 2,
                    GetSystemMetrics(SM_CYSCREEN) - HEIGHT - 92,
                )
            };
            if SetTimer(window, TIMER, DISPLAY_MS, None) == 0 {
                return;
            }
            SetWindowPos(
                window,
                HWND_TOPMOST,
                x,
                y,
                WIDTH,
                HEIGHT,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(window, null(), 0);
        }
    }
}

impl Drop for VolumeOsd {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
}

unsafe fn open() -> HWND {
    static CLASS: OnceLock<(Vec<u16>, bool)> = OnceLock::new();
    let title: Vec<u16> = "MacroMiniVolumeOsd".encode_utf16().chain([0]).collect();
    let module = GetModuleHandleW(null());
    let (class, registered) = CLASS.get_or_init(|| {
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: module,
            lpszClassName: title.as_ptr(),
            ..Default::default()
        };
        let registered = RegisterClassW(&class) != 0;
        (title, registered)
    });
    if !registered {
        return null_mut();
    }
    let window = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED,
        class.as_ptr(),
        class.as_ptr(),
        WS_POPUP,
        0,
        0,
        WIDTH,
        HEIGHT,
        null_mut(),
        null_mut(),
        module,
        null(),
    );
    if !window.is_null() {
        SetLayeredWindowAttributes(window, 0, 245, LWA_ALPHA);
        let rounded = CreateRoundRectRgn(0, 0, WIDTH + 1, HEIGHT + 1, 20, 20);
        if !rounded.is_null() && SetWindowRgn(window, rounded, 0) == 0 {
            DeleteObject(rounded);
        }
    }
    window
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT {
        left,
        top,
        right,
        bottom,
    }
}

unsafe fn fill(dc: HDC, bounds: RECT, color: u32) {
    let brush = CreateSolidBrush(color);
    if !brush.is_null() {
        FillRect(dc, &bounds, brush);
        DeleteObject(brush);
    }
}

unsafe fn paint(window: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let dc = BeginPaint(window, &mut paint);
    let level = GetWindowLongPtrW(window, GWLP_USERDATA).clamp(0, 100) as i32;
    fill(dc, rect(0, 0, WIDTH, HEIGHT), 0x00282019);
    fill(dc, rect(23, 29, 177, 35), 0x00524436);
    if level > 0 {
        fill(dc, rect(23, 29, 23 + 154 * level / 100, 35), 0x00F9D379);
    }
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, 0x00FFFFFF);
    let face: Vec<u16> = "Segoe UI".encode_utf16().chain([0]).collect();
    let font = CreateFontW(
        -23,
        0,
        0,
        0,
        600,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        0,
        face.as_ptr(),
    );
    let previous = if font.is_null() {
        null_mut()
    } else {
        SelectObject(dc, font)
    };
    let text: Vec<u16> = format!("{level}%").encode_utf16().collect();
    let mut text_rect = rect(183, 0, 252, HEIGHT);
    DrawTextW(
        dc,
        text.as_ptr(),
        text.len() as i32,
        &mut text_rect,
        DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
    );
    if !font.is_null() {
        SelectObject(dc, previous);
        DeleteObject(font);
    }
    EndPaint(window, &paint);
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            paint(window);
            0
        }
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_TIMER if wparam == TIMER => {
            KillTimer(window, TIMER);
            ShowWindow(window, SW_HIDE);
            0
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}
