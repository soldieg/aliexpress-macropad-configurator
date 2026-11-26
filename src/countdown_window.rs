//! Independent modeless power-countdown window. The gesture engine owns timing
//! and power actions; this window only displays progress and requests cancel.

use std::{
    cell::Cell,
    io,
    mem::size_of,
    ptr::{null, null_mut},
    sync::OnceLock,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::VK_ESCAPE, WindowsAndMessaging::*},
};

const CLIENT_W: i32 = 348;
const CLIENT_H: i32 = 150;
const CANCEL_ID: usize = 1;
const CLOCK_ID: usize = 1;
const TOTAL_MS: u32 = 5000;

pub struct CountdownWindow {
    hwnd: HWND,
    state: Box<State>,
}

struct State {
    millis: Cell<u32>,
    dry_run: Cell<bool>,
    cancelled: Cell<bool>,
    wake: Box<dyn Fn()>,
    body_font: HFONT,
    number_font: HFONT,
}

impl CountdownWindow {
    pub fn open(dry_run: bool, wake: impl Fn() + 'static) -> Result<Self, String> {
        unsafe {
            let class = window_class()?;
            let state = Box::new(State {
                millis: Cell::new(TOTAL_MS),
                dry_run: Cell::new(dry_run),
                cancelled: Cell::new(false),
                wake: Box::new(wake),
                body_font: font(16, 500),
                number_font: font(44, 600),
            });
            const STYLE: u32 = WS_POPUP | WS_CAPTION | WS_SYSMENU;
            const EX_STYLE: u32 = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
            let mut bounds = rect(0, 0, CLIENT_W, CLIENT_H);
            AdjustWindowRectEx(&mut bounds, STYLE, 0, EX_STYLE);
            let width = bounds.right - bounds.left;
            let height = bounds.bottom - bounds.top;
            let work_area = monitor_work_area();
            let x = (work_area.right - width - 24).max(work_area.left);
            let y = work_area.top + 24;
            let hwnd = CreateWindowExW(
                EX_STYLE,
                class.as_ptr(),
                wide("Mini Teclado USB").as_ptr(),
                STYLE,
                x,
                y,
                width,
                height,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                (&*state as *const State).cast(),
            );
            if hwnd.is_null() {
                return Err(error("Não foi possível abrir a contagem"));
            }
            let window = Self { hwnd, state };
            let button = CreateWindowExW(
                0,
                wide("BUTTON").as_ptr(),
                wide(crate::i18n::native("Cancelar")).as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                CLIENT_W - 130,
                91,
                106,
                34,
                hwnd,
                CANCEL_ID as HMENU,
                GetModuleHandleW(null()),
                null(),
            );
            if button.is_null() {
                return Err(error("Não foi possível criar o botão Cancelar"));
            }
            if !window.state.body_font.is_null() {
                SendMessageW(button, WM_SETFONT, window.state.body_font as WPARAM, 0);
            }
            if SetTimer(hwnd, CLOCK_ID, 80, None) == 0 {
                return Err(error("Não foi possível iniciar o relógio da contagem"));
            }
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            Ok(window)
        }
    }

    pub fn update(&mut self, remaining: Duration, dry_run: bool) {
        let millis = remaining.as_millis().min(u128::from(TOTAL_MS)) as u32;
        let before = self.state.millis.replace(millis);
        let previous_mode = self.state.dry_run.replace(dry_run);
        if before != millis || previous_mode != dry_run {
            unsafe { InvalidateRect(self.hwnd, null(), 0) };
        }
    }

    pub fn take_cancelled(&self) -> bool {
        self.state.cancelled.replace(false)
    }
}

impl Drop for CountdownWindow {
    fn drop(&mut self) {
        // The window procedure only borrows state while this box is alive.
        unsafe { DestroyWindow(self.hwnd) };
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for font in [self.body_font, self.number_font] {
            if !font.is_null() {
                unsafe { DeleteObject(font) };
            }
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn error(label: &str) -> String {
    format!("{label}: {}", io::Error::last_os_error())
}

unsafe fn window_class() -> Result<&'static [u16], String> {
    static CLASS: OnceLock<Result<Vec<u16>, String>> = OnceLock::new();
    match CLASS.get_or_init(|| {
        let name = wide("MiniTecladoUsbCountdown");
        let class = WNDCLASSW {
            hInstance: GetModuleHandleW(null()),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpfnWndProc: Some(window_proc),
            lpszClassName: name.as_ptr(),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            Err(error("Não foi possível registrar a janela da contagem"))
        } else {
            Ok(name)
        }
    }) {
        Ok(name) => Ok(name),
        Err(error) => Err(error.clone()),
    }
}

unsafe fn monitor_work_area() -> RECT {
    let mut point = POINT::default();
    GetCursorPos(&mut point);
    let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTOPRIMARY);
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(monitor, &mut info) != 0 {
        return info.rcWork;
    }
    rect(
        0,
        0,
        GetSystemMetrics(SM_CXSCREEN),
        GetSystemMetrics(SM_CYSCREEN),
    )
}

unsafe fn font(height: i32, weight: i32) -> HFONT {
    CreateFontW(
        -height,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET.into(),
        0,
        0,
        CLEARTYPE_QUALITY.into(),
        0,
        wide("Segoe UI").as_ptr(),
    )
}

const fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT {
        left,
        top,
        right,
        bottom,
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

unsafe fn fill(dc: HDC, bounds: &RECT, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    if !brush.is_null() {
        FillRect(dc, bounds, brush);
        DeleteObject(brush);
    }
}

unsafe fn text(dc: HDC, value: &str, mut area: RECT, font: HFONT, color: COLORREF) {
    SetTextColor(dc, color);
    let old_font = (!font.is_null()).then(|| SelectObject(dc, font));
    DrawTextW(
        dc,
        wide(value).as_ptr(),
        -1,
        &mut area,
        DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    if let Some(old_font) = old_font {
        SelectObject(dc, old_font);
    }
}

unsafe fn paint(hwnd: HWND, state: &State) {
    let mut ps = PAINTSTRUCT::default();
    let dc = BeginPaint(hwnd, &mut ps);
    if dc.is_null() {
        return;
    }
    let millis = state.millis.get();
    let seconds = millis.div_ceil(1000);
    let dry_run = state.dry_run.get();
    let accent = if dry_run {
        rgb(121, 211, 249)
    } else {
        rgb(255, 146, 142)
    };
    fill(dc, &rect(0, 0, CLIENT_W, CLIENT_H), rgb(13, 20, 32));
    SetBkMode(dc, TRANSPARENT as i32);
    text(
        dc,
        if dry_run {
            crate::i18n::native("Simulação de desligamento")
        } else {
            crate::i18n::native("Desligamento do computador")
        },
        rect(23, 13, CLIENT_W - 23, 39),
        state.body_font,
        rgb(229, 238, 251),
    );
    text(
        dc,
        &format!("{seconds} s"),
        rect(22, 38, 185, 91),
        state.number_font,
        accent,
    );
    text(
        dc,
        crate::i18n::native("Tecladinho ou botão"),
        rect(24, 92, 199, 122),
        state.body_font,
        rgb(167, 183, 203),
    );
    let track = rect(24, 132, CLIENT_W - 24, 137);
    fill(dc, &track, rgb(45, 63, 88));
    let right = track.left + (track.right - track.left) * millis as i32 / TOTAL_MS as i32;
    fill(
        dc,
        &rect(track.left, track.top, right, track.bottom),
        accent,
    );
    EndPaint(hwnd, &ps);
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let creation = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, creation.lpCreateParams as isize);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let state = &*ptr;
    let cancel = match message {
        WM_CLOSE => true,
        WM_COMMAND => wparam & 0xffff == CANCEL_ID && lparam != 0,
        WM_KEYDOWN => wparam == VK_ESCAPE as usize,
        _ => false,
    };
    if cancel {
        state.cancelled.set(true);
        KillTimer(hwnd, CLOCK_ID);
        ShowWindow(hwnd, SW_HIDE);
        (state.wake)();
        return 0;
    }
    match message {
        WM_TIMER if wparam == CLOCK_ID => {
            (state.wake)();
            0
        }
        WM_PAINT => {
            paint(hwnd, state);
            0
        }
        WM_ERASEBKGND => 1,
        WM_NCDESTROY => {
            KillTimer(hwnd, CLOCK_ID);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
