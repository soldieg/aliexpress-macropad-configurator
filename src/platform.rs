use crate::device::Binding;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    io,
    mem::{offset_of, size_of},
    os::windows::process::CommandExt,
    os::windows::{
        ffi::OsStrExt,
        io::{FromRawHandle, OwnedHandle},
    },
    path::PathBuf,
    process::Command,
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{channel, sync_channel, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::Instant,
};
use windows_sys::Win32::{
    Devices::HumanInterfaceDevice::{
        HidP_GetUsagesEx, HidP_Input, HIDP_STATUS_SUCCESS, USAGE_AND_PAGE,
    },
    Foundation::{
        GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HWND, LPARAM,
        LRESULT, POINT, WPARAM,
    },
    System::{LibraryLoader::GetModuleHandleW, Registry::*, Threading::CreateMutexW},
    UI::{
        Input::{KeyboardAndMouse::*, *},
        Shell::*,
        WindowsAndMessaging::*,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Key { index: u8, down: bool, at: Instant },
    VolumeKey { index: u8, down: bool },
    Observed(DirectInput, String),
    Disconnected,
    ShowWindow,
    Quit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectInput {
    Keyboard {
        scan: u16,
        vk: u16,
        modifiers: u8,
    },
    Media(u16),
    Mouse {
        buttons: u8,
        delta: i16,
        modifiers: u8,
    },
}

impl DirectInput {
    pub fn function_key(vk: u16) -> Self {
        Self::Keyboard {
            scan: 0,
            vk,
            modifiers: 0,
        }
    }

    fn matches_key(self, modifiers: u8, usage: u8) -> bool {
        let Self::Keyboard {
            scan,
            vk,
            modifiers: observed,
        } = self
        else {
            return false;
        };
        observed == modifiers
            && match usage {
                0x48 => vk == VK_PAUSE,
                0x68..=0x73 => vk == VK_F13 + u16::from(usage - 0x68),
                _ => scan_code(usage) == Some(scan),
            }
    }

    pub fn matches_binding(self, binding: &Binding) -> bool {
        match binding {
            Binding::Keyboard(keys) => keys
                .first()
                .is_some_and(|(modifiers, usage)| self.matches_key(*modifiers, *usage)),
            Binding::Media(usage) => self == Self::Media(*usage),
            Binding::Mouse {
                buttons,
                scroll,
                modifiers,
            } => {
                let Self::Mouse {
                    buttons: down,
                    delta,
                    modifiers: observed,
                } = self
                else {
                    return false;
                };
                *buttons == down
                    && *modifiers == observed
                    && (delta == 120 * i16::from(*scroll)
                        || (delta != 0
                            && delta.abs_diff(0) < 120
                            && i16::from(*scroll) == delta.signum()))
            }
        }
    }

    pub fn matches_later_step(self, binding: &Binding) -> bool {
        matches!(binding, Binding::Keyboard(keys) if keys.iter().skip(1).any(|(mods, key)| self.matches_key(*mods, *key)))
    }
}

pub struct Listener {
    pub events: Receiver<InputEvent>,
    pub tray_available: bool,
    window: HWND,
    worker: Option<JoinHandle<()>>,
}

impl Listener {
    pub fn start_with_wake(
        vid: u16,
        pid: u16,
        wake: impl Fn() + Send + 'static,
    ) -> Result<Self, String> {
        let (events, event_rx) = channel();
        let (ready, ready_rx) = sync_channel(1);
        let worker = thread::Builder::new()
            .name("mini-teclado-raw-input".into())
            .spawn(move || {
                let mut state = ListenerState {
                    vid,
                    pid,
                    // Refreshed again after Raw Input registration, to close the arrival race.
                    devices: InputTopology::default(),
                    events,
                    wake: Box::new(wake),
                    taskbar_created: 0,
                    icon: null_mut(),
                    owns_icon: false,
                    previous_inputs: Vec::new(),
                    consumers: HashMap::new(),
                    modifiers: 0,
                    owns_registration: false,
                };
                match unsafe { open_listener(&mut state) } {
                    Ok((window, class, tray_available)) => unsafe {
                        if ready.send(Ok((window as usize, tray_available))).is_ok() {
                            let mut message = MSG::default();
                            while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                                DispatchMessageW(&message);
                            }
                        }
                        if IsWindow(window) != 0 {
                            DestroyWindow(window);
                        }
                        UnregisterClassW(class.as_ptr(), GetModuleHandleW(null()));
                    },
                    Err(error) => {
                        let _ = ready.send(Err(error));
                    }
                }
            })
            .map_err(|err| format!("Não foi possível iniciar a escuta: {err}"))?;
        match ready_rx.recv() {
            Ok(Ok((window, tray_available))) => Ok(Self {
                events: event_rx,
                tray_available,
                window: window as HWND,
                worker: Some(worker),
            }),
            other => {
                let _ = worker.join();
                Err(other
                    .ok()
                    .and_then(Result::err)
                    .unwrap_or_else(|| "A escuta de teclado encerrou antes de iniciar.".into()))
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        unsafe {
            PostMessageW(self.window, WM_CANCELMODE, 0, 0);
            PostMessageW(self.window, WM_CLOSE, 0, 0);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

const WM_TRAY: u32 = WM_APP + 1;
const TRAY_KEY_SELECT: u32 = NIN_SELECT | 1;

struct ListenerState {
    vid: u16,
    pid: u16,
    devices: InputTopology,
    events: Sender<InputEvent>,
    wake: Box<dyn Fn() + Send>,
    taskbar_created: u32,
    icon: HICON,
    owns_icon: bool,
    previous_inputs: Vec<RAWINPUTDEVICE>,
    consumers: HashMap<usize, ConsumerReports>,
    modifiers: u8,
    owns_registration: bool,
}

impl ListenerState {
    fn emit(&self, event: InputEvent) {
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

pub fn claim_instance() -> Result<Option<OwnedHandle>, String> {
    let name = wide(r"Local\MacroParaMiniTeclados");
    let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(win32_error("Reserva da instância do aplicativo"));
    }
    let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let guard = unsafe { OwnedHandle::from_raw_handle(handle) };
    Ok((!exists).then_some(guard))
}

pub fn restore_existing_app(title: &str) {
    let title = wide(title);
    let window = unsafe { FindWindowW(null(), title.as_ptr()) };
    if !window.is_null() {
        unsafe {
            ShowWindowAsync(window, SW_RESTORE);
            SetForegroundWindow(window);
        }
    }
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "MacroParaMiniTeclados";

fn startup_command(path: &OsStr) -> Result<Vec<u16>, String> {
    let mut command = vec![u16::from(b'"')];
    command.extend(path.encode_wide());
    command.extend(wide(r#"" --bandeja"#));
    if command.len() > 260 {
        return Err(
            "O caminho do aplicativo é longo demais para a inicialização do Windows.".into(),
        );
    }
    Ok(command)
}

pub fn startup_enabled() -> Result<bool, String> {
    let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
    let mut length = 0;
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            &mut length,
        )
    };
    match code {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        _ => Err(io::Error::from_raw_os_error(code as i32).to_string()),
    }
}

pub fn set_startup(enabled: bool) -> Result<(), String> {
    let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
    let code = if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let command = startup_command(exe.as_os_str())?;
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        }
    } else {
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) }
    };
    if code == ERROR_SUCCESS || (!enabled && code == ERROR_FILE_NOT_FOUND) {
        Ok(())
    } else {
        Err(format!(
            "Inicialização do Windows: {}",
            io::Error::from_raw_os_error(code as i32)
        ))
    }
}

fn keyboard_interface(path: &str, vid: u16, pid: u16) -> Option<String> {
    let mut parts = path.split('#');
    let bus = parts.next()?;
    if !bus.rsplit('\\').next()?.eq_ignore_ascii_case("hid") {
        return None;
    }
    let function = parts.next()?.to_ascii_lowercase();
    let mut identifiers = function.split('&');
    if identifiers.next()? == format!("vid_{vid:04x}")
        && identifiers.next()? == format!("pid_{pid:04x}")
    {
        Some(function)
    } else {
        None
    }
}

fn marker_event(vkey: u16, flags: u16, at: Instant) -> Option<InputEvent> {
    if (VK_F17..=VK_F22).contains(&vkey) {
        return Some(InputEvent::VolumeKey {
            index: (vkey - VK_F17) as u8,
            down: flags & RI_KEY_BREAK as u16 == 0,
        });
    }
    (VK_F13..=VK_F16).contains(&vkey).then(|| InputEvent::Key {
        index: (vkey - VK_F13) as u8,
        down: flags & RI_KEY_BREAK as u16 == 0,
        at,
    })
}

pub fn volume_slot(index: u8) -> Option<u8> {
    (index < 6).then(|| 13 + (index / 2) * 3 + (index % 2) * 2)
}

pub fn volume_key(slot: u8) -> Option<u8> {
    (0..6)
        .find(|index| volume_slot(*index) == Some(slot))
        .map(|index| 108 + index)
}

unsafe fn device_name(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<String> {
    let mut chars = 0;
    if GetRawInputDeviceInfoW(handle, RIDI_DEVICENAME, null_mut(), &mut chars) == u32::MAX
        || chars == 0
    {
        return None;
    }
    let mut buf = vec![0u16; chars as usize];
    let actual =
        GetRawInputDeviceInfoW(handle, RIDI_DEVICENAME, buf.as_mut_ptr().cast(), &mut chars);
    if actual == u32::MAX {
        return None;
    }
    let end = buf.iter().position(|&ch| ch == 0).unwrap_or(buf.len());
    String::from_utf16(&buf[..end]).ok()
}

#[derive(Debug, Default, PartialEq, Eq)]
struct InputTopology {
    handles: HashSet<usize>,
    duplicated: bool,
}

impl InputTopology {
    fn accepts(&self, handle: usize) -> bool {
        !self.duplicated && self.handles.contains(&handle)
    }
}

unsafe fn selected_inputs(vid: u16, pid: u16, removing: Option<usize>) -> InputTopology {
    let mut count = 0;
    let entry_size = size_of::<RAWINPUTDEVICELIST>() as u32;
    for _ in 0..2 {
        if GetRawInputDeviceList(null_mut(), &mut count, entry_size) == u32::MAX || count == 0 {
            break;
        }
        let mut list = vec![RAWINPUTDEVICELIST::default(); count as usize];
        let actual = GetRawInputDeviceList(list.as_mut_ptr(), &mut count, entry_size);
        if actual == u32::MAX {
            continue;
        }
        let mut interfaces = HashSet::new();
        let mut topology = InputTopology::default();
        for entry in list.iter().take(actual as usize) {
            let handle = entry.hDevice as usize;
            if Some(handle) != removing {
                if let Some(interface) =
                    device_name(entry.hDevice).and_then(|name| keyboard_interface(&name, vid, pid))
                {
                    // Two entries for the same collection imply the VID/PID is ambiguous.
                    // Refuse all matching input before it could combine keys from two devices.
                    topology.duplicated |= !interfaces.insert(interface);
                    topology.handles.insert(handle);
                }
            }
        }
        return topology;
    }
    InputTopology::default()
}

const INPUT_COLLECTIONS: [(u16, u16); 3] = [(1, 6), (1, 2), (0x0c, 1)];

unsafe fn registered_inputs() -> Vec<RAWINPUTDEVICE> {
    let mut count = 0;
    let size = size_of::<RAWINPUTDEVICE>() as u32;
    // The size query can return UINT_MAX with ERROR_INSUFFICIENT_BUFFER.
    GetRegisteredRawInputDevices(null_mut(), &mut count, size);
    if count == 0 {
        return Vec::new();
    }
    let mut entries = vec![RAWINPUTDEVICE::default(); count as usize];
    let read = GetRegisteredRawInputDevices(entries.as_mut_ptr(), &mut count, size);
    if read == u32::MAX {
        return Vec::new();
    }
    entries
        .into_iter()
        .take(read as usize)
        .filter(|entry| INPUT_COLLECTIONS.contains(&(entry.usUsagePage, entry.usUsage)))
        .collect()
}

unsafe fn open_listener(state: &mut ListenerState) -> Result<(HWND, Vec<u16>, bool), String> {
    static NUMBER: AtomicUsize = AtomicUsize::new(0);
    let class = wide(&format!(
        "MiniTecladoUsbRawInput{}",
        NUMBER.fetch_add(1, Ordering::Relaxed)
    ));
    let instance = GetModuleHandleW(null());
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        ..Default::default()
    };
    if RegisterClassW(&window_class) == 0 {
        return Err(win32_error("Registro da janela de escuta"));
    }
    let window = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        class.as_ptr(),
        class.as_ptr(),
        WS_POPUP,
        0,
        0,
        0,
        0,
        null_mut(),
        null_mut(),
        instance,
        (state as *mut ListenerState).cast(),
    );
    if window.is_null() {
        let error = win32_error("Criação da janela de escuta");
        UnregisterClassW(class.as_ptr(), instance);
        return Err(error);
    }

    let devices = INPUT_COLLECTIONS.map(|(page, usage)| RAWINPUTDEVICE {
        usUsagePage: page,
        usUsage: usage,
        dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY,
        hwndTarget: window,
    });
    state.previous_inputs = registered_inputs();
    if RegisterRawInputDevices(
        devices.as_ptr(),
        devices.len() as u32,
        size_of::<RAWINPUTDEVICE>() as u32,
    ) == 0
    {
        let error = win32_error("Registro de entrada USB");
        DestroyWindow(window);
        UnregisterClassW(class.as_ptr(), instance);
        return Err(error);
    }
    state.owns_registration = true;
    state.devices = selected_inputs(state.vid, state.pid, None);
    state.taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
    state.icon = app_icon();
    state.owns_icon = !state.icon.is_null();
    if !state.owns_icon {
        state.icon = LoadIconW(null_mut(), IDI_APPLICATION);
    }
    Ok((window, class, install_tray(window, state.icon)))
}

unsafe fn app_icon() -> HICON {
    let Ok(image) = image::load_from_memory(include_bytes!("../assets/logo.png")) else {
        return null_mut();
    };
    const SIZE: u32 = 32;
    let image = image.thumbnail(SIZE, SIZE).to_rgba8();
    let left = (SIZE - image.width()) / 2;
    let top = (SIZE - image.height()) / 2;

    // RT_ICON with a BITMAPINFOHEADER, bottom-up BGRA pixels, and a one-bit
    // mask. Using u32 storage keeps the buffer aligned for User32.
    let pixels = (SIZE * SIZE) as usize;
    let mut icon: Vec<u32> = vec![0; 10 + pixels + SIZE as usize];
    icon[0] = 40; // BITMAPINFOHEADER size
    icon[1] = SIZE;
    icon[2] = SIZE * 2; // image and mask
    icon[3] = 1 | (32 << 16); // one color plane, 32 bits per pixel
    icon[5] = (pixels * 4) as u32;
    for (x, y, pixel) in image.enumerate_pixels() {
        let [r, g, b, a] = pixel.0;
        let index = 10 + ((SIZE - 1 - top - y) * SIZE + left + x) as usize;
        icon[index] = u32::from_le_bytes([b, g, r, a]);
    }
    CreateIconFromResourceEx(
        icon.as_ptr().cast(),
        (icon.len() * size_of::<u32>()) as u32,
        1,
        0x0003_0000,
        SIZE as i32,
        SIZE as i32,
        LR_DEFAULTCOLOR,
    )
}

struct ConsumerReports {
    preparsed: Vec<usize>,
    held: HashMap<u8, Vec<u16>>,
}

impl ConsumerReports {
    unsafe fn open(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<Self> {
        let mut length = 0;
        if GetRawInputDeviceInfoW(handle, RIDI_PREPARSEDDATA, null_mut(), &mut length) == u32::MAX
            || length == 0
            || length > 65536
        {
            return None;
        }
        let mut preparsed = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
        if GetRawInputDeviceInfoW(
            handle,
            RIDI_PREPARSEDDATA,
            preparsed.as_mut_ptr().cast(),
            &mut length,
        ) == u32::MAX
        {
            return None;
        }
        Some(Self {
            preparsed,
            held: HashMap::new(),
        })
    }

    unsafe fn observe(&mut self, report: &[u8], compact_pad: bool) -> Vec<u16> {
        if compact_pad {
            if let Some(usage) = compact_consumer_usage(report) {
                let active = (usage != 0).then_some(usage).into_iter().collect();
                return self.presses(report[0], active);
            }
        }
        let mut usages = [USAGE_AND_PAGE::default(); 48];
        let mut count = usages.len() as u32;
        let status = HidP_GetUsagesEx(
            HidP_Input,
            0,
            usages.as_mut_ptr(),
            &mut count,
            self.preparsed.as_ptr() as isize,
            report.as_ptr(),
            report.len() as u32,
        );
        if status != HIDP_STATUS_SUCCESS {
            return Vec::new();
        }
        let active: Vec<_> = usages[..(count as usize).min(usages.len())]
            .iter()
            .filter_map(|usage| (usage.UsagePage == 0x0c).then_some(usage.Usage))
            .collect();
        self.presses(report[0], active)
    }

    fn presses(&mut self, report_id: u8, active: Vec<u16>) -> Vec<u16> {
        let held = self.held.entry(report_id).or_default();
        let presses = active
            .iter()
            .copied()
            .filter(|value| !held.contains(value))
            .collect();
        *held = active;
        presses
    }
}

fn compact_consumer_usage(report: &[u8]) -> Option<u16> {
    // HID descriptor for 1189:8890 MI_02/Col02: ID 2 and one 16-bit usage array.
    let [2, low, high] = *report else {
        return None;
    };
    let usage = u16::from_le_bytes([low, high]);
    (usage <= 0x02ff).then_some(usage)
}

fn consumer_label(usage: u16) -> String {
    match usage {
        0xb5 => "Próxima faixa",
        0xb6 => "Faixa anterior",
        0xb7 => "Parar reprodução",
        0xcd => "Reproduzir / pausar",
        0xe2 => "Silenciar",
        0xe9 => "Aumentar volume",
        0xea => "Diminuir volume",
        _ => return format!("Mídia HID {usage:04X}"),
    }
    .into()
}

fn keyboard_label(key: RAWKEYBOARD) -> String {
    let mut buffer = [0u16; 64];
    let flags =
        (i32::from(key.MakeCode) << 16) | (i32::from(key.Flags & RI_KEY_E0 as u16 != 0) << 24);
    let size = unsafe { GetKeyNameTextW(flags, buffer.as_mut_ptr(), buffer.len() as i32) };
    if size > 0 {
        return String::from_utf16_lossy(&buffer[..size as usize]);
    }
    format!("Tecla VK {:02X}", key.VKey)
}

fn mouse_label(flags: u16, data: u16) -> Option<String> {
    for (code, name) in [
        (RI_MOUSE_LEFT_BUTTON_DOWN, "Clique esquerdo"),
        (RI_MOUSE_RIGHT_BUTTON_DOWN, "Clique direito"),
        (RI_MOUSE_MIDDLE_BUTTON_DOWN, "Clique central"),
        (RI_MOUSE_BUTTON_4_DOWN, "Botão 4 do mouse"),
        (RI_MOUSE_BUTTON_5_DOWN, "Botão 5 do mouse"),
    ] {
        if flags & code as u16 != 0 {
            return Some(name.into());
        }
    }
    if flags & RI_MOUSE_WHEEL as u16 != 0 {
        let delta = data as i16;
        let label = if delta > 0 {
            "Rolar para cima"
        } else {
            "Rolar para baixo"
        };
        return Some(format!("{label} · delta {delta:+}"));
    }
    None
}

unsafe fn observe_consumer(state: &mut ListenerState, lparam: LPARAM, header: RAWINPUTHEADER) {
    const OFFSET: usize = offset_of!(RAWINPUT, data) + offset_of!(RAWHID, bRawData);
    if (header.dwSize as usize) < OFFSET || header.dwSize > 4096 {
        return;
    }
    let mut buffer = vec![0usize; (header.dwSize as usize).div_ceil(size_of::<usize>())];
    let mut capacity = header.dwSize;
    let read = GetRawInputData(
        lparam as HRAWINPUT,
        RID_INPUT,
        buffer.as_mut_ptr().cast(),
        &mut capacity,
        size_of::<RAWINPUTHEADER>() as u32,
    );
    if read == u32::MAX || (read as usize) < OFFSET || read > header.dwSize {
        return;
    }
    let raw = std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), read as usize);
    let size = u32::from_ne_bytes(raw[OFFSET - 8..OFFSET - 4].try_into().unwrap()) as usize;
    let count = u32::from_ne_bytes(raw[OFFSET - 4..OFFSET].try_into().unwrap()) as usize;
    let Some(full) = size
        .checked_mul(count)
        .filter(|n| size > 0 && *n <= raw.len() - OFFSET)
    else {
        return;
    };
    let handle = header.hDevice as usize;
    if let std::collections::hash_map::Entry::Vacant(slot) = state.consumers.entry(handle) {
        let Some(collector) = ConsumerReports::open(header.hDevice) else {
            return;
        };
        slot.insert(collector);
    }
    let compact_pad = (state.vid, state.pid) == (0x1189, 0x8890);
    for report in raw[OFFSET..OFFSET + full].chunks_exact(size) {
        for usage in state
            .consumers
            .get_mut(&handle)
            .unwrap()
            .observe(report, compact_pad)
        {
            state.emit(InputEvent::Observed(
                DirectInput::Media(usage),
                consumer_label(usage),
            ));
        }
    }
}

unsafe fn read_raw_input(state: &mut ListenerState, lparam: LPARAM) {
    let at = Instant::now();
    let mut header = RAWINPUTHEADER::default();
    let size_header = size_of::<RAWINPUTHEADER>() as u32;
    let mut header_size = size_header;
    if GetRawInputData(
        lparam as HRAWINPUT,
        RID_HEADER,
        (&mut header as *mut RAWINPUTHEADER).cast(),
        &mut header_size,
        size_header,
    ) == u32::MAX
        || !state.devices.accepts(header.hDevice as usize)
    {
        return;
    }
    if header.dwType == RIM_TYPEHID {
        observe_consumer(state, lparam, header);
        return;
    }
    let mut input = RAWINPUT::default();
    let mut size = size_of::<RAWINPUT>() as u32;
    let read = GetRawInputData(
        lparam as HRAWINPUT,
        RID_INPUT,
        (&mut input as *mut RAWINPUT).cast(),
        &mut size,
        size_header,
    );
    if read == u32::MAX {
        return;
    }
    match header.dwType {
        RIM_TYPEKEYBOARD if read >= size_header + size_of::<RAWKEYBOARD>() as u32 => {
            let key = input.data.keyboard;
            let scan = key.MakeCode
                | if key.Flags & RI_KEY_E0 as u16 != 0 {
                    0xe000
                } else {
                    0
                };
            if let Some(index) = MODIFIERS.iter().position(|(_, code)| *code == scan) {
                let bit = 1 << index;
                if key.Flags & RI_KEY_BREAK as u16 == 0 {
                    state.modifiers |= bit;
                } else {
                    state.modifiers &= !bit;
                }
            }
            if let Some(event) = marker_event(key.VKey, key.Flags, at) {
                state.emit(event);
            } else if key.VKey != 0xff && key.Flags & RI_KEY_BREAK as u16 == 0 {
                state.emit(InputEvent::Observed(
                    DirectInput::Keyboard {
                        scan,
                        vk: key.VKey,
                        modifiers: state.modifiers,
                    },
                    keyboard_label(key),
                ));
            }
        }
        RIM_TYPEMOUSE if read >= size_header + size_of::<RAWMOUSE>() as u32 => {
            let buttons = input.data.mouse.Anonymous.Anonymous;
            if let Some(label) = mouse_label(buttons.usButtonFlags, buttons.usButtonData) {
                let flags = u32::from(buttons.usButtonFlags);
                let down = [
                    RI_MOUSE_LEFT_BUTTON_DOWN,
                    RI_MOUSE_RIGHT_BUTTON_DOWN,
                    RI_MOUSE_MIDDLE_BUTTON_DOWN,
                    RI_MOUSE_BUTTON_4_DOWN,
                    RI_MOUSE_BUTTON_5_DOWN,
                ]
                .into_iter()
                .enumerate()
                .fold(0, |mask, (index, flag)| {
                    mask | (u8::from(flags & flag != 0) << index)
                });
                let delta = if flags & RI_MOUSE_WHEEL != 0 {
                    buttons.usButtonData as i16
                } else {
                    0
                };
                state.emit(InputEvent::Observed(
                    DirectInput::Mouse {
                        buttons: down,
                        delta,
                        modifiers: state.modifiers,
                    },
                    label,
                ));
            }
        }
        _ => {}
    }
}

unsafe fn install_tray(window: HWND, icon: HICON) -> bool {
    let mut info = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    let label = wide("Mini Teclado USB");
    info.szTip[..label.len()].copy_from_slice(&label);
    if Shell_NotifyIconW(NIM_ADD, &info) == 0 {
        return false;
    }
    info.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    // When Explorer supports version 4, context activation also works from the keyboard.
    Shell_NotifyIconW(NIM_SETVERSION, &info);
    true
}

unsafe fn tray_menu(window: HWND) -> Option<InputEvent> {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return None;
    }
    AppendMenuW(
        menu,
        MF_STRING,
        1,
        wide(crate::i18n::native("Abrir")).as_ptr(),
    );
    AppendMenuW(
        menu,
        MF_STRING,
        2,
        wide(crate::i18n::native("Sair")).as_ptr(),
    );
    let mut position = POINT::default();
    GetCursorPos(&mut position);
    SetForegroundWindow(window);
    let choice = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        position.x,
        position.y,
        0,
        window,
        null(),
    );
    PostMessageW(window, WM_NULL, 0, 0);
    DestroyMenu(menu);
    match choice {
        1 => Some(InputEvent::ShowWindow),
        2 => Some(InputEvent::Quit),
        _ => None,
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let creation = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(window, GWLP_USERDATA, creation.lpCreateParams as isize);
    }
    let state_ptr = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut ListenerState;
    if state_ptr.is_null() {
        return DefWindowProcW(window, message, wparam, lparam);
    }

    if message == WM_TRAY {
        // A menu pumps messages. Do not borrow ListenerState across that nested dispatch.
        let event = match (lparam as u32) & 0xffff {
            NIN_SELECT | TRAY_KEY_SELECT | WM_LBUTTONUP => Some(InputEvent::ShowWindow),
            WM_CONTEXTMENU | WM_RBUTTONUP => tray_menu(window),
            _ => None,
        };
        if let Some(event) = event {
            (&*state_ptr).emit(event);
        }
        return 0;
    }

    let state = &mut *state_ptr;
    match message {
        WM_INPUT => read_raw_input(state, lparam),
        WM_INPUT_DEVICE_CHANGE => {
            // Enumeration can still include a just-removed handle when the notification arrives.
            let removing = (wparam as u32 == GIDC_REMOVAL).then_some(lparam as usize);
            let next = selected_inputs(state.vid, state.pid, removing);
            if state.devices != next {
                state.modifiers = 0;
                state.emit(InputEvent::Disconnected);
            }
            state
                .consumers
                .retain(|handle, _| next.handles.contains(handle));
            state.devices = next;
        }
        WM_DESTROY => {
            let info = NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: window,
                uID: 1,
                ..Default::default()
            };
            Shell_NotifyIconW(NIM_DELETE, &info);
            if state.owns_icon {
                DestroyIcon(state.icon);
            }
            if state.owns_registration {
                let mut restore = Vec::new();
                for current in registered_inputs()
                    .into_iter()
                    .filter(|entry| entry.hwndTarget == window)
                {
                    restore.push(
                        state
                            .previous_inputs
                            .iter()
                            .copied()
                            .find(|entry| {
                                entry.usUsagePage == current.usUsagePage
                                    && entry.usUsage == current.usUsage
                                    && (entry.hwndTarget.is_null()
                                        || IsWindow(entry.hwndTarget) != 0)
                            })
                            .unwrap_or(RAWINPUTDEVICE {
                                dwFlags: RIDEV_REMOVE,
                                hwndTarget: null_mut(),
                                ..current
                            }),
                    );
                }
                if !restore.is_empty() {
                    RegisterRawInputDevices(
                        restore.as_ptr(),
                        restore.len() as u32,
                        size_of::<RAWINPUTDEVICE>() as u32,
                    );
                }
            }
            PostQuitMessage(0);
            return 0;
        }
        _ if message == state.taskbar_created && message != 0 => {
            install_tray(window, state.icon);
            return 0;
        }
        _ => {}
    }
    // Foreground WM_INPUT must reach DefWindowProcW for User32 cleanup.
    DefWindowProcW(window, message, wparam, lparam)
}

fn win32_error(action: &str) -> String {
    format!("{action}: {}", io::Error::last_os_error())
}

pub fn wake_gui(window: usize) {
    if window != 0 {
        // Winit's invalidation does not produce a paint while the window is
        // hidden. Explicitly queue a frame so tray events and gestures still
        // reach the UI state machine.
        unsafe { PostMessageW(window as HWND, WM_PAINT, 0, 0) };
    }
}

#[derive(Default)]
pub struct ScreenToggle {
    requested_off: bool,
}

impl ScreenToggle {
    pub fn toggle(&mut self) -> Result<bool, String> {
        self.toggle_with(|command| {
            if unsafe {
                PostMessageW(
                    HWND_BROADCAST,
                    WM_SYSCOMMAND,
                    SC_MONITORPOWER as usize,
                    command,
                )
            } == 0
            {
                let action = if command == 2 { "apagar" } else { "ligar" };
                Err(win32_error(&format!("Não foi possível {action} a tela")))
            } else {
                Ok(())
            }
        })
    }

    fn toggle_with(
        &mut self,
        request: impl FnOnce(isize) -> Result<(), String>,
    ) -> Result<bool, String> {
        let next_off = !self.requested_off;
        request(if next_off { 2 } else { -1 })?;
        // Key-down may also wake the monitor before the next gesture is released.
        // The next operation must still follow the application's previous request.
        self.requested_off = next_off;
        Ok(next_off)
    }
}

pub fn shutdown() -> Result<(), String> {
    let system_root = std::env::var_os("SystemRoot").ok_or("Pasta do Windows indisponível.")?;
    let shutdown_exe = PathBuf::from(system_root)
        .join("System32")
        .join("shutdown.exe");
    let status = Command::new(shutdown_exe)
        .args(["/s", "/t", "0"])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .status()
        .map_err(|err| format!("Não foi possível iniciar o desligamento: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Windows recusou o desligamento ({status})."))
    }
}

fn scan_code(usage: u8) -> Option<u16> {
    // USB keyboard usage to PC/AT Set 1 make code. The high byte, when E0,
    // is passed separately to SendInput. Letter keys are physical positions.
    const LETTERS: [u16; 26] = [
        0x1e, 0x30, 0x2e, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18,
        0x19, 0x10, 0x13, 0x1f, 0x14, 0x16, 0x2f, 0x11, 0x2d, 0x15, 0x2c,
    ];
    const STANDARD_AND_PAD: [u16; 62] = [
        0x1c, 0x01, 0x0e, 0x0f, 0x39, 0x0c, 0x0d, 0x1a, 0x1b, 0x2b, 0x2b, 0x27, 0x28, 0x29, 0x33,
        0x34, 0x35, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f, 0x40, 0x41, 0x42, 0x43, 0x44, 0x57, 0x58,
        0xe037, 0x46, 0, 0xe052, 0xe047, 0xe049, 0xe053, 0xe04f, 0xe051, 0xe04d, 0xe04b, 0xe050,
        0xe048, 0x45, 0xe035, 0x37, 0x4a, 0x4e, 0xe01c, 0x4f, 0x50, 0x51, 0x4b, 0x4c, 0x4d, 0x47,
        0x48, 0x49, 0x52, 0x53, 0x56, 0xe05d,
    ];
    let code = match usage {
        0x04..=0x1d => LETTERS[(usage - 4) as usize],
        0x1e..=0x27 => u16::from(usage - 0x1c),
        0x28..=0x65 => STANDARD_AND_PAD[(usage - 0x28) as usize],
        0x87 => 0x73,
        0x89 => 0x7d,
        0xe0..=0xe7 => MODIFIERS[(usage - 0xe0) as usize].1,
        _ => return None,
    };
    (code != 0).then_some(code)
}

const MODIFIERS: [(u16, u16); 8] = [
    (VK_LCONTROL, 0x1d),
    (VK_LSHIFT, 0x2a),
    (VK_LMENU, 0x38),
    (VK_LWIN, 0xe05b),
    (VK_RCONTROL, 0xe01d),
    (VK_RSHIFT, 0x36),
    (VK_RMENU, 0xe038),
    (VK_RWIN, 0xe05c),
];

fn key_record(vk: u16, scan: u16, mut flags: KEYBD_EVENT_FLAGS, up: bool) -> INPUT {
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

fn scan_input(scan: u16, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_SCANCODE;
    if scan & 0xff00 == 0xe000 {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    key_record(0, scan & 0xff, flags, up)
}

fn inject(inputs: &[INPUT]) -> Result<(), String> {
    if inputs.is_empty() {
        return Ok(());
    }
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent as usize == inputs.len() {
        return Ok(());
    }

    // A partial SendInput can leave synthetic modifiers or buttons held down.
    // Re-send only corresponding releases for downs that were actually inserted.
    let mut releases = Vec::new();
    for input in inputs[..(sent as usize).min(inputs.len())].iter().rev() {
        unsafe {
            match input.r#type {
                INPUT_KEYBOARD if input.Anonymous.ki.dwFlags & KEYEVENTF_KEYUP == 0 => {
                    let mut release = *input;
                    release.Anonymous.ki.dwFlags |= KEYEVENTF_KEYUP;
                    releases.push(release);
                }
                INPUT_MOUSE => {
                    let up = match input.Anonymous.mi.dwFlags {
                        MOUSEEVENTF_LEFTDOWN => MOUSEEVENTF_LEFTUP,
                        MOUSEEVENTF_RIGHTDOWN => MOUSEEVENTF_RIGHTUP,
                        MOUSEEVENTF_MIDDLEDOWN => MOUSEEVENTF_MIDDLEUP,
                        _ => continue,
                    };
                    releases.push(mouse_record(up, 0));
                }
                _ => {}
            }
        }
    }
    if !releases.is_empty() {
        unsafe {
            SendInput(
                releases.len() as u32,
                releases.as_ptr(),
                size_of::<INPUT>() as i32,
            )
        };
    }
    Err(
        "O Windows não enviou toda a ação. Verifique se a janela ativa tem permissões elevadas."
            .into(),
    )
}

fn with_modifiers(modifiers: u8, inputs: &mut Vec<INPUT>, action: impl FnOnce(&mut Vec<INPUT>)) {
    // Preserve modifiers already held down on the user's main keyboard.
    let held: Vec<_> = MODIFIERS
        .iter()
        .enumerate()
        .filter(|(i, (vk, _))| {
            modifiers & (1 << i) != 0 && unsafe { GetAsyncKeyState(i32::from(*vk)) } >= 0
        })
        .map(|(_, (_, scan))| *scan)
        .collect();
    inputs.extend(held.iter().map(|&scan| scan_input(scan, false)));
    action(inputs);
    inputs.extend(held.into_iter().rev().map(|scan| scan_input(scan, true)));
}

fn keyboard(sequence: &[(u8, u8)]) -> Result<(), String> {
    let mut inputs = Vec::new();
    for &(modifiers, usage) in sequence {
        let key = if usage == 0 {
            None
        } else if (0x68..=0x73).contains(&usage) {
            let vk = VK_F13 + u16::from(usage - 0x68);
            Some([key_record(vk, 0, 0, false), key_record(vk, 0, 0, true)])
        } else if usage == 0x48 {
            Some([
                key_record(VK_PAUSE, 0, 0, false),
                key_record(VK_PAUSE, 0, 0, true),
            ])
        } else {
            let scan = scan_code(usage).ok_or_else(|| {
                format!("Código de teclado sem reprodução no Windows: {usage:#04X}.")
            })?;
            Some([scan_input(scan, false), scan_input(scan, true)])
        };
        with_modifiers(modifiers, &mut inputs, |list| {
            list.extend(key.into_iter().flatten())
        });
    }
    inject(&inputs)
}

fn media(usage: u16) -> Result<(), String> {
    let key = match usage {
        0xb5 => VK_MEDIA_NEXT_TRACK,
        0xb6 => VK_MEDIA_PREV_TRACK,
        0xb7 => VK_MEDIA_STOP,
        0xcd => VK_MEDIA_PLAY_PAUSE,
        0xe2 => VK_VOLUME_MUTE,
        0xe9 => VK_VOLUME_UP,
        0xea => VK_VOLUME_DOWN,
        _ => {
            return Err(format!(
                "Ação de mídia sem reprodução no Windows: {usage:#06X}."
            ))
        }
    };
    inject(&[key_record(key, 0, 0, false), key_record(key, 0, 0, true)])
}

fn mouse_record(flags: MOUSE_EVENT_FLAGS, data: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dwFlags: flags,
                mouseData: data,
                ..Default::default()
            },
        },
    }
}

fn mouse(buttons: u8, scroll: i8, modifiers: u8) -> Result<(), String> {
    if buttons & !7 != 0 {
        return Err("Botão de mouse sem reprodução no Windows.".into());
    }
    let mut inputs = Vec::new();
    with_modifiers(modifiers, &mut inputs, |list| {
        for (button, down, up) in [
            (1, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            (2, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            (4, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        ] {
            if buttons & button != 0 {
                list.extend([mouse_record(down, 0), mouse_record(up, 0)]);
            }
        }
        if scroll != 0 {
            let delta = i32::from(scroll) * 120;
            list.push(mouse_record(MOUSEEVENTF_WHEEL, delta as u32));
        }
    });
    inject(&inputs)
}

pub fn replay(binding: &Binding) -> Result<(), String> {
    match binding {
        Binding::Keyboard(sequence) => keyboard(sequence),
        Binding::Media(usage) => media(*usage),
        Binding::Mouse {
            buttons,
            scroll,
            modifiers,
        } => mouse(*buttons, *scroll, *modifiers),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_command_quotes_the_executable_and_uses_the_tray_flag() {
        use std::ffi::OsStr;
        let command = startup_command(OsStr::new(r"C:\Pasta com espaço\Macro.exe")).unwrap();
        assert_eq!(
            command,
            wide(r#""C:\Pasta com espaço\Macro.exe" --bandeja"#)
        );
        assert!(startup_command(OsStr::new(&"a".repeat(260))).is_err());
    }

    #[test]
    fn side_markers_preserve_all_knob_clicks_and_gesture_markers() {
        assert_eq!(
            (0..6)
                .map(|index| volume_slot(index).unwrap())
                .collect::<Vec<_>>(),
            [13, 15, 16, 18, 19, 21]
        );
        assert_eq!(volume_slot(255), None);
        assert_eq!(volume_key(14), None);
        assert_eq!(volume_key(15), Some(109));
        assert_eq!(
            marker_event(VK_F17, 0, Instant::now()),
            Some(InputEvent::VolumeKey {
                index: 0,
                down: true
            })
        );
    }

    #[test]
    fn compact_consumer_reports_show_the_usage_again_after_release() {
        let mut monitor = ConsumerReports {
            preparsed: Vec::new(),
            held: HashMap::new(),
        };
        assert_eq!(unsafe { monitor.observe(&[2, 0xea, 0], true) }, [0xea]);
        assert!(unsafe { monitor.observe(&[2, 0xea, 0], true) }.is_empty());
        assert!(unsafe { monitor.observe(&[2, 0, 0], true) }.is_empty());
        assert_eq!(unsafe { monitor.observe(&[2, 0xea, 0], true) }, [0xea]);
        assert_eq!(compact_consumer_usage(&[3, 0xea, 0]), None);
    }

    #[test]
    fn display_toggle_sends_off_on_off_in_order() {
        let mut toggle = ScreenToggle::default();
        let mut commands = Vec::new();
        for requested_off in [true, false, true] {
            assert_eq!(
                toggle.toggle_with(|command| {
                    commands.push(command);
                    Ok(())
                }),
                Ok(requested_off)
            );
        }
        assert_eq!(commands, [2, -1, 2]);
    }

    #[test]
    fn failed_display_request_preserves_the_next_command() {
        let mut toggle = ScreenToggle::default();
        assert!(toggle.toggle_with(|_| Err("não enviado".into())).is_err());
        assert_eq!(
            toggle.toggle_with(|command| {
                assert_eq!(command, 2);
                Ok(())
            }),
            Ok(true)
        );
    }

    #[test]
    fn raw_names_match_the_selected_usb_device_exactly() {
        assert_eq!(
            keyboard_interface(
                r"\\?\HID#VID_1189&PID_8890&MI_02&COL01#first#{id}\KBD",
                0x1189,
                0x8890
            ),
            Some("vid_1189&pid_8890&mi_02&col01".into())
        );
        assert!(
            keyboard_interface(r"\\?\HID#VID_1189F&PID_8890&MI_00#first", 0x1189, 0x8890).is_none()
        );
        assert!(
            keyboard_interface(r"\\?\HID#VID_1189&PID_8891&MI_02#first", 0x1189, 0x8890).is_none()
        );
        assert!(keyboard_interface(r"\??\ROOT#RDP_KBD#first", 0x1189, 0x8890).is_none());
    }

    #[test]
    fn raw_gesture_markers_preserve_key_up_and_unrelated_keys_are_ignored() {
        let at = Instant::now();
        assert_eq!(marker_event(0x7b, 0, at), None);
        assert_eq!(
            marker_event(0x7c, 0, at),
            Some(InputEvent::Key {
                index: 0,
                down: true,
                at,
            })
        );
        assert_eq!(
            marker_event(0x7f, 1, at),
            Some(InputEvent::Key {
                index: 3,
                down: false,
                at,
            })
        );
        assert_eq!(marker_event(0x86, 0, at), None);
    }

    #[test]
    fn replay_keeps_extended_and_locale_dependent_keys_physical() {
        assert_eq!(scan_code(0x04), Some(0x001e)); // a position
        assert_eq!(scan_code(0x4f), Some(0xe04d)); // right arrow
        assert_eq!(scan_code(0x64), Some(0x0056)); // non-US backslash
        assert_eq!(scan_code(0x87), Some(0x0073)); // ABNT slash
        assert_eq!(scan_code(0xff), None);
    }

    #[test]
    #[ignore = "requer o Mini Teclado USB 1189:8890 conectado ao Windows"]
    fn live_registers_selected_inputs_and_removes_registration() {
        let devices = unsafe { selected_inputs(0x1189, 0x8890, None) };
        assert!(
            !devices.duplicated && !devices.handles.is_empty(),
            "nenhum teclado 1189:8890 isolado em Raw Input"
        );
        let previous = unsafe {
            registered_inputs()
                .into_iter()
                .find(|entry| entry.usUsagePage == 1 && entry.usUsage == 6)
                .map(|device| device.hwndTarget)
        };
        let listener = Listener::start_with_wake(0x1189, 0x8890, || {}).unwrap();
        assert!(unsafe {
            registered_inputs()
                .into_iter()
                .find(|entry| entry.usUsagePage == 1 && entry.usUsage == 6)
        }
        .is_some_and(|device| device.hwndTarget == listener.window));
        let tray = listener.tray_available;
        drop(listener);
        assert_eq!(
            unsafe {
                registered_inputs()
                    .into_iter()
                    .find(|entry| entry.usUsagePage == 1 && entry.usUsage == 6)
                    .map(|device| device.hwndTarget)
            },
            previous
        );
        eprintln!(
            "coleções de entrada do dispositivo: {}; bandeja: {tray}",
            devices.handles.len()
        );
    }

    #[test]
    fn generated_logo_creates_a_windows_icon() {
        let icon = unsafe { app_icon() };
        assert!(!icon.is_null());
        assert_ne!(unsafe { DestroyIcon(icon) }, 0);
    }
}
