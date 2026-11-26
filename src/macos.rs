//! macOS USB HID input and system bindings. Listening does not seize the device.
use crate::device::Binding;
use crate::platform::{DirectInput, InputEvent};
use std::{
    collections::HashMap,
    ffi::{c_char, c_void, CStr},
    fs,
    path::{Path, PathBuf},
    process::Command,
    ptr::{null, null_mut, NonNull},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender, SyncSender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

type CfRef = *const c_void;
type DeviceRef = *mut c_void;
type Callback = unsafe extern "C" fn(*mut c_void, i32, *mut c_void, DeviceRef);

struct OwnedCf(NonNull<c_void>);
impl OwnedCf {
    fn new(ptr: CfRef, message: &str) -> Result<Self, String> {
        NonNull::new(ptr.cast_mut())
            .map(Self)
            .ok_or_else(|| message.to_owned())
    }

    fn as_ref(&self) -> CfRef {
        self.0.as_ptr()
    }
}

impl Drop for OwnedCf {
    fn drop(&mut self) {
        unsafe { CFRelease(self.as_ref()) };
    }
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopDefaultMode: CfRef;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    fn CFRelease(value: CfRef);
    fn CFGetTypeID(value: CfRef) -> usize;
    fn CFStringCreateWithCString(allocator: CfRef, text: *const c_char, encoding: u32) -> CfRef;
    fn CFNumberCreate(allocator: CfRef, kind: isize, value: *const c_void) -> CfRef;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: CfRef, kind: isize, output: *mut c_void) -> u8;
    fn CFDictionaryCreate(
        allocator: CfRef,
        keys: *const CfRef,
        values: *const CfRef,
        count: isize,
        key_callbacks: CfRef,
        value_callbacks: CfRef,
    ) -> CfRef;
    fn CFRunLoopGetCurrent() -> CfRef;
    fn CFRunLoopRunInMode(mode: CfRef, seconds: f64, return_after_source: u8) -> i32;
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOHIDManagerCreate(allocator: CfRef, options: u32) -> CfRef;
    fn IOHIDManagerSetDeviceMatching(manager: CfRef, matching: CfRef);
    fn IOHIDManagerRegisterDeviceMatchingCallback(manager: CfRef, cb: Callback, ctx: *mut c_void);
    fn IOHIDManagerRegisterDeviceRemovalCallback(manager: CfRef, cb: Callback, ctx: *mut c_void);
    fn IOHIDManagerRegisterInputValueCallback(manager: CfRef, cb: Callback, ctx: *mut c_void);
    fn IOHIDManagerScheduleWithRunLoop(manager: CfRef, run_loop: CfRef, mode: CfRef);
    fn IOHIDManagerUnscheduleFromRunLoop(manager: CfRef, run_loop: CfRef, mode: CfRef);
    fn IOHIDManagerOpen(manager: CfRef, options: u32) -> i32;
    fn IOHIDManagerClose(manager: CfRef, options: u32) -> i32;
    fn IOHIDDeviceGetProperty(device: DeviceRef, key: CfRef) -> CfRef;
    fn IOHIDValueGetElement(value: DeviceRef) -> DeviceRef;
    fn IOHIDValueGetIntegerValue(value: DeviceRef) -> isize;
    fn IOHIDElementGetDevice(element: DeviceRef) -> DeviceRef;
    fn IOHIDElementGetUsagePage(element: DeviceRef) -> u32;
    fn IOHIDElementGetUsage(element: DeviceRef) -> u32;
    fn IOPMAssertionDeclareUserActivity(name: CfRef, activity: u32, assertion: *mut u32) -> i32;
    fn IOPMAssertionRelease(assertion: u32) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
    fn CGEventCreate(source: CfRef) -> CfRef;
    fn CGEventCreateKeyboardEvent(source: CfRef, key: u16, down: bool) -> CfRef;
    fn CGEventCreateMouseEvent(source: CfRef, kind: u32, point: Point, button: u32) -> CfRef;
    fn CGEventCreateScrollWheelEvent2(
        source: CfRef,
        units: u32,
        count: u32,
        first: i32,
        second: i32,
        third: i32,
    ) -> CfRef;
    fn CGEventGetLocation(event: CfRef) -> Point;
    fn CGEventSetFlags(event: CfRef, flags: u64);
    fn CGEventSetIntegerValueField(event: CfRef, field: u32, value: i64);
    fn CGEventPost(tap: u32, event: CfRef);
    fn CGMainDisplayID() -> u32;
    fn CGDisplayIsAsleep(display: u32) -> u32;
}

const CF_UTF8: u32 = 0x0800_0100;
const CF_INT_32: isize = 3;
const CF_INT_64: isize = 4;
const NO_OPTIONS: u32 = 0;
const HID_ACCESS: &str =
    "Permita Monitoramento de Entrada para o aplicativo nos Ajustes do Sistema do macOS.";
const OUTPUT_ACCESS: &str =
    "Permita Acessibilidade para o aplicativo nos Ajustes do Sistema do macOS.";
const APP_ID: &str = "br.minitecladousb.startup";

fn cf_string(value: &CStr) -> Result<OwnedCf, String> {
    OwnedCf::new(
        unsafe { CFStringCreateWithCString(null(), value.as_ptr(), CF_UTF8) },
        "Não foi possível criar texto para Core Foundation.",
    )
}

fn cf_number(value: u16) -> Result<OwnedCf, String> {
    let value = i32::from(value);
    OwnedCf::new(
        unsafe { CFNumberCreate(null(), CF_INT_32, (&value as *const i32).cast()) },
        "Não foi possível criar número para Core Foundation.",
    )
}

fn matching(vid: u16, pid: u16) -> Result<OwnedCf, String> {
    let keys = [
        cf_string(c"VendorID")?,
        cf_string(c"ProductID")?,
        cf_string(c"Transport")?,
    ];
    let values = [cf_number(vid)?, cf_number(pid)?, cf_string(c"USB")?];
    let key_refs = keys.each_ref().map(OwnedCf::as_ref);
    let value_refs = values.each_ref().map(OwnedCf::as_ref);
    OwnedCf::new(
        unsafe {
            CFDictionaryCreate(
                null(),
                key_refs.as_ptr(),
                value_refs.as_ptr(),
                key_refs.len() as isize,
                (&raw const kCFTypeDictionaryKeyCallBacks).cast(),
                (&raw const kCFTypeDictionaryValueCallBacks).cast(),
            )
        },
        "Não foi possível criar o filtro USB para IOKit.",
    )
}

#[derive(Default)]
struct Source {
    // Identical VID/PID on a different physical USB port is a different unit.
    port: Option<u32>,
    modifiers: u8,
}

struct InputState {
    events: Sender<InputEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    location_key: OwnedCf,
    sources: HashMap<usize, Source>,
}

impl InputState {
    fn emit(&self, event: InputEvent) {
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }

    fn unique(&self) -> bool {
        if self.sources.len() <= 1 {
            return true;
        }
        let mut sources = self.sources.values();
        sources
            .next()
            .and_then(|s| s.port)
            .is_some_and(|port| sources.all(|s| s.port == Some(port)))
    }

    fn insert(&mut self, device: DeviceRef) {
        let old = self.unique();
        let key = device as usize;
        if self.sources.contains_key(&key) {
            return;
        }
        let port = unsafe {
            let value = IOHIDDeviceGetProperty(device, self.location_key.as_ref());
            let mut port = 0_i64;
            if !value.is_null()
                && CFGetTypeID(value) == CFNumberGetTypeID()
                && CFNumberGetValue(value, CF_INT_64, (&mut port as *mut i64).cast()) != 0
                && port != 0
            {
                u32::try_from(port)
                    .ok()
                    .or_else(|| i32::try_from(port).ok().map(|id| id as u32))
            } else {
                None
            }
        };
        self.sources.insert(
            key,
            Source {
                port,
                ..Default::default()
            },
        );
        if old && !self.unique() {
            self.sources
                .values_mut()
                .for_each(|source| source.modifiers = 0);
            self.emit(InputEvent::Disconnected);
        }
    }

    fn keyboard(&mut self, device: usize, usage: u8, down: bool) {
        if (0xe0..=0xe7).contains(&usage) {
            if let Some(source) = self.sources.get_mut(&device) {
                let bit = 1 << (usage - 0xe0);
                source.modifiers = if down {
                    source.modifiers | bit
                } else {
                    source.modifiers & !bit
                };
            }
            return;
        }
        if let Some(marker) = marker_event(usage, down, Instant::now()) {
            self.emit(marker);
        } else if down {
            if let Some(input) = DirectInput::keyboard_usage(usage, self.modifiers()) {
                self.emit(InputEvent::Observed(input, format!("HID {usage:02X}")));
            }
        }
    }

    fn modifiers(&self) -> u8 {
        self.sources
            .values()
            .fold(0, |all, source| all | source.modifiers)
    }

    fn mouse(&self, buttons: u8, delta: i16, label: &'static str) {
        self.emit(InputEvent::Observed(
            DirectInput::Mouse {
                buttons,
                delta,
                modifiers: self.modifiers(),
            },
            label.into(),
        ));
    }
}

fn marker_event(usage: u8, down: bool, at: Instant) -> Option<InputEvent> {
    match usage {
        0x68..=0x6b => Some(InputEvent::Key {
            index: usage - 0x68,
            down,
            at,
        }),
        0x6c..=0x71 => Some(InputEvent::VolumeKey {
            index: usage - 0x6c,
            down,
        }),
        _ => None,
    }
}

unsafe extern "C" fn arrived(context: *mut c_void, result: i32, _: *mut c_void, device: DeviceRef) {
    if result == 0 && !device.is_null() {
        (&mut *context.cast::<InputState>()).insert(device);
    }
}

unsafe extern "C" fn removed(context: *mut c_void, _: i32, _: *mut c_void, device: DeviceRef) {
    let state = &mut *context.cast::<InputState>();
    if state.sources.remove(&(device as usize)).is_some() {
        state
            .sources
            .values_mut()
            .for_each(|source| source.modifiers = 0);
        state.emit(InputEvent::Disconnected);
    }
}

unsafe extern "C" fn input(context: *mut c_void, result: i32, _: *mut c_void, value: DeviceRef) {
    if result != 0 || value.is_null() {
        return;
    }
    let element = IOHIDValueGetElement(value);
    if element.is_null() {
        return;
    }
    let device = IOHIDElementGetDevice(element);
    if device.is_null() {
        return;
    }
    let state = &mut *context.cast::<InputState>();
    state.insert(device);
    if !state.unique() {
        return;
    }
    let page = IOHIDElementGetUsagePage(element);
    let usage = IOHIDElementGetUsage(element);
    let amount = IOHIDValueGetIntegerValue(value);
    match (page, usage) {
        (0x07, 4..=0xe7) => state.keyboard(device as usize, usage as u8, amount != 0),
        (0x0c, 1..=0xffff) if amount > 0 => {
            let label = match usage {
                0xb5 => "Próxima faixa",
                0xb6 => "Faixa anterior",
                0xb7 => "Parar reprodução",
                0xcd => "Reproduzir / pausar",
                0xe2 => "Silenciar",
                0xe9 => "Aumentar volume",
                0xea => "Diminuir volume",
                _ => "",
            };
            state.emit(InputEvent::Observed(
                DirectInput::Media(usage as u16),
                if label.is_empty() {
                    format!("Mídia HID {usage:04X}")
                } else {
                    label.into()
                },
            ));
        }
        (0x09, 1..=5) if amount != 0 => {
            let label = [
                "Clique esquerdo",
                "Clique direito",
                "Clique central",
                "Botão 4 do mouse",
                "Botão 5 do mouse",
            ];
            state.mouse(1 << (usage - 1), 0, label[(usage - 1) as usize]);
        }
        (0x01, 0x38) if amount != 0 => {
            let delta = amount
                .saturating_mul(120)
                .clamp(i16::MIN as isize, i16::MAX as isize) as i16;
            state.mouse(
                0,
                delta,
                if delta > 0 {
                    "Rolar para cima"
                } else {
                    "Rolar para baixo"
                },
            );
        }
        _ => {}
    }
}

fn monitor(
    mut state: InputState,
    vid: u16,
    pid: u16,
    stop: &AtomicBool,
    ready: &SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let matching = matching(vid, pid)?;
    let manager = OwnedCf::new(
        unsafe { IOHIDManagerCreate(null(), NO_OPTIONS) },
        "Não foi possível criar o gerenciador HID do macOS.",
    )?;
    let manager_ref = manager.as_ref();
    let context = (&mut state as *mut InputState).cast();
    unsafe {
        IOHIDManagerSetDeviceMatching(manager_ref, matching.as_ref());
        IOHIDManagerRegisterDeviceMatchingCallback(manager_ref, arrived, context);
        IOHIDManagerRegisterDeviceRemovalCallback(manager_ref, removed, context);
        IOHIDManagerRegisterInputValueCallback(manager_ref, input, context);
        let status = IOHIDManagerOpen(manager_ref, NO_OPTIONS);
        if status != 0 {
            return Err(format!(
                "Não foi possível abrir a escuta USB no macOS ({status:#010x}). {HID_ACCESS}"
            ));
        }
        let mode = kCFRunLoopDefaultMode;
        let run_loop = CFRunLoopGetCurrent();
        IOHIDManagerScheduleWithRunLoop(manager_ref, run_loop, mode);
        if ready.send(Ok(())).is_ok() {
            while !stop.load(Ordering::Acquire) {
                let outcome = CFRunLoopRunInMode(mode, 0.1, 0);
                if matches!(outcome, 1 | 2) {
                    // Finished/stopped runs can return immediately without a matching source.
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
        IOHIDManagerUnscheduleFromRunLoop(manager_ref, run_loop, mode);
        IOHIDManagerClose(manager_ref, NO_OPTIONS);
    }
    Ok(())
}

pub fn start(
    vid: u16,
    pid: u16,
    events: Sender<InputEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    stop: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, String> {
    static REQUESTED: AtomicBool = AtomicBool::new(false);
    if unsafe { !CGPreflightListenEventAccess() }
        && (REQUESTED.swap(true, Ordering::Relaxed) || unsafe { !CGRequestListenEventAccess() })
    {
        return Err(HID_ACCESS.into());
    }

    let (ready, started) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("macos-usb-input".into())
        .spawn(move || {
            let result = cf_string(c"LocationID").and_then(|location_key| {
                let state = InputState {
                    events,
                    wake,
                    location_key,
                    sources: HashMap::new(),
                };
                monitor(state, vid, pid, &stop, &ready)
            });
            if let Err(error) = result {
                let _ = ready.send(Err(error));
            }
        })
        .map_err(|e| format!("Não foi possível iniciar a escuta: {e}"))?;
    match started.recv() {
        Ok(Ok(())) => Ok(worker),
        other => {
            let _ = worker.join();
            Err(other
                .ok()
                .and_then(Result::err)
                .unwrap_or_else(|| "A escuta de teclado encerrou antes de iniciar.".into()))
        }
    }
}
fn require_output_access() -> Result<(), String> {
    static REQUESTED: AtomicBool = AtomicBool::new(false);
    if unsafe { CGPreflightPostEventAccess() }
        || (!REQUESTED.swap(true, Ordering::Relaxed) && unsafe { CGRequestPostEventAccess() })
    {
        Ok(())
    } else {
        Err(OUTPUT_ACCESS.into())
    }
}

fn virtual_key(usage: u8) -> Option<u16> {
    // HID keyboard usage to Carbon Events.h virtual key positions.
    const LETTERS: [u16; 26] = [
        0x00, 0x0b, 0x08, 0x02, 0x0e, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2e, 0x2d, 0x1f,
        0x23, 0x0c, 0x0f, 0x01, 0x11, 0x20, 0x09, 0x0d, 0x07, 0x10, 0x06,
    ];
    const NUMBERS: [u16; 10] = [0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1a, 0x1c, 0x19, 0x1d];
    const F1_TO_F12: [u16; 12] = [
        0x7a, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6d, 0x67, 0x6f,
    ];
    const F13_TO_F20: [u16; 8] = [0x69, 0x6b, 0x71, 0x6a, 0x40, 0x4f, 0x50, 0x5a];
    const PAD_1_TO_0: [u16; 10] = [0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5b, 0x5c, 0x52];
    Some(match usage {
        0x04..=0x1d => LETTERS[(usage - 4) as usize],
        0x1e..=0x27 => NUMBERS[(usage - 0x1e) as usize],
        0x28 => 0x24,
        0x29 => 0x35,
        0x2a => 0x33,
        0x2b => 0x30,
        0x2c => 0x31,
        0x2d => 0x1b,
        0x2e => 0x18,
        0x2f => 0x21,
        0x30 => 0x1e,
        0x31 | 0x32 => 0x2a,
        0x33 => 0x29,
        0x34 => 0x27,
        0x35 => 0x32,
        0x36 => 0x2b,
        0x37 => 0x2f,
        0x38 => 0x2c,
        0x39 => 0x39,
        0x3a..=0x45 => F1_TO_F12[(usage - 0x3a) as usize],
        0x49 => 0x72, // Help / Insert position
        0x4a => 0x73,
        0x4b => 0x74,
        0x4c => 0x75,
        0x4d => 0x77,
        0x4e => 0x79,
        0x4f => 0x7c,
        0x50 => 0x7b,
        0x51 => 0x7d,
        0x52 => 0x7e,
        0x53 => 0x47,
        0x54 => 0x4b,
        0x55 => 0x43,
        0x56 => 0x4e,
        0x57 => 0x45,
        0x58 => 0x4c,
        0x59..=0x62 => PAD_1_TO_0[(usage - 0x59) as usize],
        0x63 => 0x41,
        0x64 => 0x0a,
        0x67 => 0x51,
        0x68..=0x6f => F13_TO_F20[(usage - 0x68) as usize],
        0x87 => 0x5e, // JIS underscore position
        0x89 => 0x5d, // JIS Yen position
        0xe0..=0xe7 => [0x3b, 0x38, 0x3a, 0x37, 0x3e, 0x3c, 0x3d, 0x36][(usage - 0xe0) as usize],
        _ => return None,
    })
}

fn event_flags(modifiers: u8) -> u64 {
    const CONTROL: u64 = 1 << 18;
    const SHIFT: u64 = 1 << 17;
    const ALT: u64 = 1 << 19;
    const COMMAND: u64 = 1 << 20;
    // Quartz's generic modifiers, with the corresponding device-side NX bit.
    const MASKS: [u64; 8] = [
        CONTROL | 0x0001,
        SHIFT | 0x0002,
        ALT | 0x0020,
        COMMAND | 0x0008,
        CONTROL | 0x2000,
        SHIFT | 0x0004,
        ALT | 0x0040,
        COMMAND | 0x0010,
    ];
    MASKS
        .iter()
        .enumerate()
        .filter_map(|(index, &flag)| (modifiers & (1 << index) != 0).then_some(flag))
        .fold(0, |all, flag| all | flag)
}

fn post_events(events: Vec<(OwnedCf, u64)>) {
    const HID_EVENT_TAP: u32 = 0;
    for (event, flags) in events {
        unsafe {
            CGEventSetFlags(event.as_ref(), flags);
            CGEventPost(HID_EVENT_TAP, event.as_ref());
        }
    }
}

fn replay_keyboard(steps: &[(u8, u8)]) -> Result<(), String> {
    require_output_access()?;
    let mut events = Vec::with_capacity(steps.len() * 2);
    for &(modifiers, usage) in steps {
        let key = virtual_key(usage)
            .ok_or_else(|| format!("Código de teclado sem reprodução no macOS: {usage:#04X}."))?;
        for down in [true, false] {
            let event = OwnedCf::new(
                unsafe { CGEventCreateKeyboardEvent(null(), key, down) },
                "Não foi possível criar o evento de teclado do macOS.",
            )?;
            events.push((event, event_flags(modifiers)));
        }
    }
    post_events(events);
    Ok(())
}

fn replay_mouse(buttons: u8, scroll: i8, modifiers: u8) -> Result<(), String> {
    if buttons & !0x1f != 0 {
        return Err("Botão de mouse sem reprodução no macOS.".into());
    }
    require_output_access()?;
    let flags = event_flags(modifiers);
    let mut events = Vec::new();
    if buttons != 0 {
        let cursor_event = OwnedCf::new(
            unsafe { CGEventCreate(null()) },
            "Não foi possível localizar o ponteiro do macOS.",
        )?;
        let point = unsafe { CGEventGetLocation(cursor_event.as_ref()) };
        let button_events = [(1, 2), (3, 4), (25, 26), (25, 26), (25, 26)];
        for (button, (down, up)) in button_events.into_iter().enumerate() {
            if buttons & (1 << button) == 0 {
                continue;
            }
            for kind in [down, up] {
                let event = OwnedCf::new(
                    unsafe { CGEventCreateMouseEvent(null(), kind, point, button as u32) },
                    "Não foi possível criar o clique no macOS.",
                )?;
                const CLICK_COUNT: u32 = 1;
                unsafe { CGEventSetIntegerValueField(event.as_ref(), CLICK_COUNT, 1) };
                events.push((event, flags));
            }
        }
    }
    if scroll != 0 {
        const LINE_UNITS: u32 = 1;
        let event = OwnedCf::new(
            unsafe {
                CGEventCreateScrollWheelEvent2(null(), LINE_UNITS, 1, i32::from(scroll), 0, 0)
            },
            "Não foi possível criar a rolagem no macOS.",
        )?;
        events.push((event, flags));
    }
    post_events(events);
    Ok(())
}

pub(crate) fn validate_replay(binding: &Binding) -> Result<(), String> {
    match binding {
        Binding::Keyboard(steps) => {
            if let Some(&(_, usage)) = steps.iter().find(|&&(_, usage)| virtual_key(usage).is_none()) {
                return Err(format!("Código de teclado sem reprodução no macOS: {usage:#04X}."));
            }
            require_output_access()
        }
        Binding::Mouse { buttons, .. } if buttons & !0x1f != 0 => {
            Err("Botão de mouse sem reprodução no macOS.".into())
        }
        Binding::Mouse { .. } => require_output_access(),
        Binding::Media(0xe9 | 0xea | 0xe2) => crate::audio::current().map(|_| ()),
        Binding::Media(usage) => Err(format!(
            "Ação de mídia sem reprodução global no macOS: {usage:#06X}. Mantenha essa tecla em modo direto."
        )),
    }
}

pub fn replay(binding: &Binding) -> Result<(), String> {
    match binding {
        Binding::Keyboard(steps) => replay_keyboard(steps),
        Binding::Mouse {
            buttons,
            scroll,
            modifiers,
        } => replay_mouse(*buttons, *scroll, *modifiers),
        Binding::Media(0xe9) => crate::audio::adjust(5).map(|_| ()),
        Binding::Media(0xea) => crate::audio::adjust(-5).map(|_| ()),
        Binding::Media(0xe2) => crate::audio::toggle_mute(),
        Binding::Media(usage) => Err(format!(
            "Ação de mídia sem reprodução global no macOS: {usage:#06X}."
        )),
    }
}

pub fn toggle_display(off: &mut bool) -> Result<bool, String> {
    // A physical keypad press can already awaken a sleeping display. Preserve
    // the requested toggle state in addition to checking the actual display.
    if *off || unsafe { CGDisplayIsAsleep(CGMainDisplayID()) } != 0 {
        let name = cf_string(c"Mini Teclado USB display toggle")?;
        let mut assertion = 0;
        let result = unsafe { IOPMAssertionDeclareUserActivity(name.as_ref(), 0, &mut assertion) };
        if result != 0 {
            return Err(format!(
                "Não foi possível ligar a tela no macOS ({result:#010x})."
            ));
        }
        unsafe { IOPMAssertionRelease(assertion) };
        *off = false;
    } else {
        let result = Command::new("/usr/bin/pmset")
            .arg("displaysleepnow")
            .output()
            .map_err(|e| format!("Não foi possível solicitar repouso da tela: {e}"))?;
        if !result.status.success() {
            return Err(format!(
                "O macOS recusou o repouso da tela. {}",
                String::from_utf8_lossy(&result.stderr).trim()
            ));
        }
        *off = true;
    }
    Ok(*off)
}

#[repr(C)]
struct AppleEvent {
    descriptor_type: u32,
    handle: *mut c_void,
}

impl Default for AppleEvent {
    fn default() -> Self {
        Self {
            descriptor_type: u32::from_be_bytes(*b"null"),
            handle: null_mut(),
        }
    }
}

impl Drop for AppleEvent {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { AEDisposeDesc(self) };
        }
    }
}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn AECreateDesc(kind: u32, data: *const c_void, size: isize, out: *mut AppleEvent) -> i16;
    fn AECreateAppleEvent(
        class: u32,
        id: u32,
        target: *const AppleEvent,
        return_id: i16,
        transaction: i32,
        out: *mut AppleEvent,
    ) -> i16;
    fn AESendMessage(
        event: *const AppleEvent,
        reply: *mut AppleEvent,
        mode: i32,
        timeout: isize,
    ) -> i32;
    fn AEGetParamPtr(
        event: *const AppleEvent,
        key: u32,
        desired_type: u32,
        actual_type: *mut u32,
        data: *mut c_void,
        max_size: isize,
        actual_size: *mut isize,
    ) -> i16;
    fn AEDisposeDesc(desc: *mut AppleEvent) -> i16;
}

fn shutdown_error(status: i32) -> Result<(), String> {
    match status {
        0 => Ok(()),
        -1743 => {
            Err("O macOS negou a automação. Confira a permissão em Privacidade e Segurança.".into())
        }
        _ => Err(format!(
            "O macOS recusou o pedido de desligamento ({status})."
        )),
    }
}

pub fn shutdown() -> Result<(), String> {
    const LOGINWINDOW: &[u8] = b"com.apple.loginwindow";
    let (mut target, mut event, mut reply) = (
        AppleEvent::default(),
        AppleEvent::default(),
        AppleEvent::default(),
    );
    unsafe {
        shutdown_error(i32::from(AECreateDesc(
            u32::from_be_bytes(*b"bund"),
            LOGINWINDOW.as_ptr().cast(),
            LOGINWINDOW.len() as isize,
            &mut target,
        )))?;
        // The OS-native shutdown dialog keeps macOS's normal handling of unsaved documents.
        shutdown_error(i32::from(AECreateAppleEvent(
            u32::from_be_bytes(*b"aevt"),
            u32::from_be_bytes(*b"rsdn"),
            &target,
            -1,
            0,
            &mut event,
        )))?;
        const WAIT_REPLY: i32 = 3;
        const CAN_INTERACT: i32 = 0x20;
        shutdown_error(AESendMessage(
            &event,
            &mut reply,
            WAIT_REPLY | CAN_INTERACT,
            300,
        ))?;
        let mut result = 0_i32;
        let status = AEGetParamPtr(
            &reply,
            u32::from_be_bytes(*b"errn"),
            u32::from_be_bytes(*b"long"),
            null_mut(),
            (&mut result as *mut i32).cast(),
            4,
            null_mut(),
        );
        if status != -1701 {
            shutdown_error(i32::from(status))?;
            shutdown_error(result)?;
        }
    }
    Ok(())
}

const PLIST_MARKER: &str = "<!-- Gerado por Mini Teclado USB -->";

fn startup_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("Pasta pessoal indisponível no macOS.")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{APP_ID}.plist")))
}

fn owned_startup_file(path: &Path) -> Result<bool, String> {
    match fs::read_to_string(path) {
        Ok(xml) if xml.contains(PLIST_MARKER) => Ok(true),
        Ok(_) => Err(
            "Já existe um LaunchAgent com o mesmo nome que não foi criado pelo aplicativo.".into(),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Não foi possível ler o LaunchAgent: {error}")),
    }
}

pub fn startup_enabled() -> Result<bool, String> {
    owned_startup_file(&startup_path()?)
}

fn startup_plist(program: &str) -> String {
    let program = program
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
{PLIST_MARKER}
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{APP_ID}</string>
  <key>ProgramArguments</key>
  <array><string>{program}</string><string>--bandeja</string></array>
  <key>RunAtLoad</key><true/>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
</dict>
</plist>
"#
    )
}

pub fn set_startup(enabled: bool) -> Result<(), String> {
    let path = startup_path()?;
    let exists = owned_startup_file(&path)?;
    if !enabled {
        if exists {
            fs::remove_file(&path)
                .map_err(|e| format!("Não foi possível remover o LaunchAgent: {e}"))?;
        }
        return Ok(());
    }
    let program = std::env::current_exe().map_err(|e| e.to_string())?;
    let program = program
        .to_str()
        .ok_or("O caminho do aplicativo não está em UTF-8.")?;
    let directory = path.parent().ok_or("Pasta de LaunchAgents indisponível.")?;
    let temp = path.with_extension("novo");
    fs::create_dir_all(directory)
        .and_then(|()| fs::write(&temp, startup_plist(program)))
        .and_then(|()| fs::rename(temp, path))
        .map_err(|e| format!("Não foi possível salvar o LaunchAgent: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_values_and_physical_virtual_keys_are_distinct() {
        let now = Instant::now();
        assert_eq!(
            marker_event(0x6b, false, now),
            Some(InputEvent::Key {
                index: 3,
                down: false,
                at: now
            })
        );
        assert_eq!(
            marker_event(0x71, true, now),
            Some(InputEvent::VolumeKey {
                index: 5,
                down: true
            })
        );
        assert_eq!(marker_event(0x72, true, now), None);
        assert_eq!(virtual_key(0x04), Some(0));
        assert_eq!(virtual_key(0x4f), Some(0x7c));
        assert_eq!(virtual_key(0x64), Some(0x0a));
        assert_eq!(virtual_key(0x6f), Some(0x5a));
    }

    #[test]
    fn modifiers_keep_quartz_generic_and_device_side_bits() {
        assert_eq!(event_flags(0), 0);
        assert_eq!(event_flags(1), (1 << 18) | 1);
        assert_eq!(event_flags(1 << 5), (1 << 17) | 4);
        assert_eq!(event_flags(1 << 7), (1 << 20) | 0x10);
    }

    #[test]
    fn launch_agent_keeps_the_program_path_as_one_xml_argument() {
        let plist = startup_plist("/Applications/A & B <v2>.app/Contents/MacOS/mini");
        assert!(plist.contains(
            "<string>/Applications/A &amp; B &lt;v2&gt;.app/Contents/MacOS/mini</string>"
        ));
        assert!(plist.contains("<string>--bandeja</string>"));
        assert!(plist.contains(PLIST_MARKER));
    }
}
