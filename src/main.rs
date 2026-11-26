#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg_attr(target_os = "linux", path = "linux_audio.rs")]
#[cfg_attr(target_os = "macos", path = "macos_audio.rs")]
mod audio;
#[cfg(windows)]
mod countdown_window;
mod custom;
#[cfg(not(windows))]
mod desktop_tray;
#[cfg(not(windows))]
mod desktop_windows;
mod device;
mod gestures;
mod i18n;
mod input;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(windows))]
use desktop_windows::{countdown_window, volume_osd};
#[cfg_attr(not(windows), path = "desktop.rs")]
mod platform;
mod ui;
#[cfg(windows)]
mod volume_osd;

use device::{Binding, ControlKind, Device, Layout};
use eframe::egui;
use gestures::{GestureAction, GestureConfig, GestureEngine, ShutdownTrial};
use platform::{InputEvent, Listener};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Profile {
    bindings: BTreeMap<String, Binding>,
    delays: BTreeMap<String, u16>,
    #[serde(skip_serializing)]
    volume_steps: BTreeMap<String, u8>,
    volume_step: u8,
    volume_mode: bool,
    report_id: Option<u8>,
    volume_managed: BTreeSet<u8>,
    gestures: GestureConfig,
    simulation: bool,
    host: u8,
    layout: String,
    #[serde(skip_serializing)]
    custom: Option<custom::CustomLayout>,
    visual_slots: [Option<custom::CustomLayout>; 2],
    visual_slot: u8,
    hide_default_layout: bool,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            bindings: BTreeMap::new(),
            delays: BTreeMap::new(),
            volume_steps: BTreeMap::new(),
            volume_step: 2,
            volume_mode: false,
            report_id: None,
            volume_managed: BTreeSet::new(),
            gestures: GestureConfig {
                display: 1,
                shutdown: 4,
                cancel: 2,
                enabled: false,
                shutdown_armed: false,
            },
            host: 0,
            simulation: true,
            layout: String::new(),
            custom: None,
            visual_slots: [None, None],
            visual_slot: 0,
            hide_default_layout: false,
        }
    }
}

impl Profile {
    fn migrate_volume(&mut self) {
        self.volume_step = self.volume_step.clamp(1, 10);
        let mut shared_step = None;
        for slot in (0..6).filter_map(platform::volume_slot) {
            let key = address(1, slot);
            if let Some(step) = self
                .volume_steps
                .get(&key)
                .filter(|s| (1..=10).contains(*s))
            {
                if self.bindings.get(&key).and_then(volume_direction).is_some() {
                    shared_step.get_or_insert(*step);
                    self.volume_managed.insert(slot);
                }
            }
        }
        if let Some(step) = shared_step {
            self.volume_step = step;
            self.volume_mode = true;
        }
        self.volume_steps.clear();
    }

    fn rotary_adjustment(&self, slot: u8) -> Option<i8> {
        if !self.volume_managed.contains(&slot) {
            return None;
        }
        Some(volume_direction(self.bindings.get(&address(1, slot))?)? * self.volume_step as i8)
    }

    fn marker_conflict(&self, slot: u8, marker: u8) -> bool {
        let current = address(1, slot);
        self.bindings
            .iter()
            .any(|(address, action)| address != &current && uses_key(action, marker))
    }

    fn observed_slot(
        &self,
        input: platform::DirectInput,
        available: &[u8],
        host_slots: &[u8],
    ) -> Option<u8> {
        let mut selected = None;
        for (address, binding) in &self.bindings {
            let Some((layer, slot)) = address.split_once(':') else {
                continue;
            };
            let Ok(slot) = slot.parse::<u8>() else {
                continue;
            };
            if !available.contains(&slot)
                || (layer == "1"
                    && (host_slots.contains(&slot) || self.volume_managed.contains(&slot)))
            {
                continue;
            }
            if input.matches_later_step(binding) {
                return None;
            }
            if input.matches_binding(binding) {
                if selected.is_some_and(|previous| previous != slot) {
                    return None;
                }
                selected = Some(slot);
            }
        }
        selected
    }

    fn migrate_layouts(&mut self) {
        if let Some(old) = self.custom.take().filter(|layout| layout.is_valid()) {
            if self.visual_slots[0].is_none() {
                self.visual_slots[0] = Some(old);
                if self.visual_slot == 0 {
                    self.visual_slot = 1;
                }
            }
        }
        if self.visual_slot != 0 && self.selected_layout().is_none() {
            self.visual_slot = 0;
        }
        if self.hide_default_layout && self.visual_slot == 0 {
            if let Some(slot) = self.first_saved_layout() {
                self.visual_slot = slot;
            } else {
                self.hide_default_layout = false;
            }
        }
    }

    fn first_saved_layout(&self) -> Option<u8> {
        self.visual_slots
            .iter()
            .position(|layout| layout.as_ref().is_some_and(custom::CustomLayout::is_valid))
            .map(|index| index as u8 + 1)
    }

    fn selected_layout(&self) -> Option<&custom::CustomLayout> {
        self.visual_slot
            .checked_sub(1)
            .and_then(|index| self.visual_slots.get(index as usize))
            .and_then(Option::as_ref)
            .filter(|layout| layout.is_valid())
    }
}

fn location() -> PathBuf {
    device::settings_dir().join("perfis.json")
}

fn device_key(d: &Device) -> String {
    format!("{:04X}:{:04X}", d.vid, d.pid)
}
fn address(layer: u8, slot: u8) -> String {
    format!("{layer}:{slot}")
}

fn volume_direction(binding: &Binding) -> Option<i8> {
    match binding {
        Binding::Media(0xe9) => Some(1),
        Binding::Media(0xea) => Some(-1),
        _ => None,
    }
}

fn uses_key(binding: &Binding, code: u8) -> bool {
    matches!(binding, Binding::Keyboard(keys) if keys.iter().any(|(_, usage)| *usage == code))
}

fn log_marker_event(event: impl std::fmt::Display) {
    use std::io::Write;
    if let Some(path) = std::env::var_os("MINI_TECLADO_EVENTOS") {
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(log, "{event}");
        }
    }
}

const DIAL_STEP: f32 = std::f32::consts::TAU / 24.;
const DIAL_EASE: Duration = Duration::from_millis(90);

#[derive(Clone, Copy, Default)]
struct DialTurn {
    from: f32,
    target: f32,
    changed: Option<Instant>,
}

impl DialTurn {
    fn running(self, now: Instant) -> bool {
        self.changed
            .is_some_and(|changed| now.saturating_duration_since(changed) < DIAL_EASE)
    }

    fn at(self, now: Instant) -> f32 {
        let Some(changed) = self.changed else {
            return 0.;
        };
        let progress = (now.saturating_duration_since(changed).as_secs_f32()
            / DIAL_EASE.as_secs_f32())
        .min(1.);
        self.from + (self.target - self.from) * (1. - (1. - progress).powi(3))
    }

    fn advance(&mut self, direction: f32, now: Instant) {
        self.from = self.at(now);
        self.target += direction * DIAL_STEP;
        self.changed = Some(now);
    }
}
fn seed() -> Binding {
    Binding::Keyboard(vec![(0, 4)])
}

fn slots(layout: &Layout) -> Vec<u8> {
    let mut out = layout
        .controls
        .iter()
        .filter(|c| c.kind == ControlKind::Button)
        .map(|c| c.id)
        .collect::<Vec<_>>();
    out.sort_unstable();
    if out.len() < 4 {
        out.extend(
            layout
                .controls
                .iter()
                .filter(|c| c.kind == ControlKind::Knob)
                .filter_map(|c| c.slots().get(1).copied()),
        );
    }
    out.truncate(4);
    out
}

const PREVIEW_PROFILE: &str = "leiaute-local";
const APP_TITLE: &str = "Macro para Mini Teclados";

#[derive(PartialEq, Copy, Clone)]
enum Page {
    Keys,
    Gestures,
    Lights,
}
#[derive(PartialEq, Copy, Clone)]
enum Confirm {
    Activate,
    Restore,
}

fn gesture_targets(settings: GestureConfig, available: u8) -> Result<u8, String> {
    if !settings.enabled {
        return Ok(0);
    }
    if settings.display != 0 && settings.display == settings.shutdown {
        return Err("Escolha gestos diferentes para a tela e para desligar o PC.".into());
    }
    let gestures = settings.display | settings.shutdown;
    if gestures == 0 {
        return Err("Escolha pelo menos uma ação para ativar as combinações.".into());
    }
    let required = gestures
        | if settings.shutdown != 0 {
            settings.cancel
        } else {
            0
        };
    if required & !available != 0 {
        return Err("Uma combinação usa um controle indisponível neste leiaute.".into());
    }
    Ok(required)
}

fn missing_individual_actions(profile: &Profile, keys: &[u8], targets: u8) -> Vec<u8> {
    keys.iter()
        .copied()
        .enumerate()
        .filter_map(|(index, slot)| {
            (targets & (1 << index) != 0 && !profile.bindings.contains_key(&address(1, slot)))
                .then_some(slot)
        })
        .collect()
}

struct App {
    hwnd: Option<usize>,
    devices: Vec<Device>,
    current: usize,
    layout: Option<Layout>,
    key: Option<String>,
    profiles: BTreeMap<String, Profile>,
    load_error: Option<String>,
    next_scan: Instant,
    status: String,
    page: Page,
    slot: u8,
    layer: u8,
    draft: Binding,
    recording: bool,
    delay: u16,
    volume_pressed: u8,
    volume_osd: volume_osd::VolumeOsd,
    dial_turns: BTreeMap<u8, DialTurn>,
    led: u8,
    led_color: u8,
    rgb: device::RgbLighting,
    listener: Option<Listener>,
    engine: GestureEngine,
    screen: platform::ScreenToggle,
    gesture_draft: GestureConfig,
    simulate_draft: bool,
    simulate: bool,
    paused: bool,
    exiting: bool,
    startup_enabled: bool,
    language: Option<usize>,
    start_minimized: bool,
    hide_startup_at: Option<Instant>,
    shutdown_trial: ShutdownTrial,
    preview_until: Option<Instant>,
    countdown: Option<countdown_window::CountdownWindow>,
    last_input: Option<String>,
    raw_pressed: u8,
    physical_flash: [Option<Instant>; 22],
    confirm: Option<Confirm>,
    custom_dialog: Option<custom::CustomLayout>,
    edit_visual_slot: u8,
    usb_dialog: Option<Result<Vec<device::UsbCandidate>, String>>,
    logo: egui::TextureHandle,
}

impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        logo: &image::RgbaImage,
        start_minimized: bool,
    ) -> Self {
        let ctx = &cc.egui_ctx;
        let hwnd = cc
            .window_handle()
            .ok()
            .and_then(|handle| match handle.as_raw() {
                RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as usize),
                _ => None,
            });
        ui::theme(ctx);
        let language = i18n::load(ctx);
        let (profiles, load_error) = match std::fs::read(location()) {
            Ok(data) => match serde_json::from_slice(&data) {
                Ok(mut value) => {
                    let profiles: &mut BTreeMap<String, Profile> = &mut value;
                    for profile in profiles.values_mut() {
                        profile.migrate_layouts();
                        profile.migrate_volume();
                    }
                    (value, None)
                }
                Err(error) => (
                    BTreeMap::new(),
                    Some(format!("Arquivo de perfis inválido: {error}")),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (BTreeMap::new(), None),
            Err(e) => (
                BTreeMap::new(),
                Some(format!("Não foi possível abrir os perfis: {e}")),
            ),
        };
        let mut app = Self {
            hwnd,
            devices: Vec::new(),
            current: 0,
            layout: None,
            key: None,
            profiles,
            load_error,
            next_scan: Instant::now(),
            status: String::new(),
            page: Page::Keys,
            slot: 1,
            layer: 1,
            draft: seed(),
            recording: false,
            delay: 0,
            volume_pressed: 0,
            volume_osd: volume_osd::VolumeOsd::default(),
            dial_turns: BTreeMap::new(),
            led: 0,
            led_color: 0,
            rgb: device::RgbLighting::default(),
            listener: None,
            engine: GestureEngine::default(),
            screen: platform::ScreenToggle::default(),
            gesture_draft: Profile::default().gestures,
            simulate_draft: true,
            simulate: true,
            paused: false,
            exiting: false,
            startup_enabled: platform::startup_enabled().unwrap_or(false),
            language,
            start_minimized,
            hide_startup_at: None,
            shutdown_trial: ShutdownTrial::load(
                device::settings_dir().join("desligamento-testado.v1"),
            ),
            preview_until: None,
            countdown: None,
            last_input: None,
            raw_pressed: 0,
            physical_flash: [None; 22],
            confirm: None,
            custom_dialog: None,
            edit_visual_slot: 1,
            usb_dialog: None,
            logo: ctx.load_texture(
                "Mini Teclado USB",
                egui::ColorImage::from_rgba_unmultiplied(
                    [logo.width() as usize, logo.height() as usize],
                    logo.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            ),
        };
        app.scan(ctx);
        if app.layout.is_none() {
            app.show_offline_layout();
        }
        if let Some(e) = &app.load_error {
            app.status = e.clone();
        }
        app
    }

    fn profile(&self) -> Option<&Profile> {
        self.key.as_ref().and_then(|k| self.profiles.get(k))
    }
    fn profile_mut(&mut self) -> &mut Profile {
        self.profiles
            .get_mut(self.key.as_ref().expect("dispositivo selecionado"))
            .expect("perfil selecionado")
    }

    fn show_offline_layout(&mut self) {
        self.key = Some(PREVIEW_PROFILE.into());
        let profile = self.profiles.entry(PREVIEW_PROFILE.into()).or_default();
        let stock = device::default_layout();
        self.layout = Some(
            profile
                .selected_layout()
                .map_or_else(|| stock.clone(), |custom| custom.layout(&stock)),
        );
    }

    fn restore_visual_layout(&mut self) {
        let Some(device) = self.devices.get(self.current) else {
            self.layout = None;
            self.key = None;
            self.show_offline_layout();
            return;
        };
        let mut layout = self.hardware_layout(device);
        if let Some(custom) = self.profile().and_then(Profile::selected_layout) {
            layout = custom.layout(&layout);
        }
        self.layout = Some(layout);
    }

    fn hardware_layout(&self, device: &Device) -> Layout {
        let name = self.profile().map_or("", |p| p.layout.as_str());
        device::layouts_for(device)
            .into_iter()
            .find(|l| l.name == name)
            .unwrap_or_else(|| device.profile.clone())
    }

    fn allows_visual_slot(&self, slot: u8) -> bool {
        self.profile()
            .and_then(Profile::selected_layout)
            .is_none_or(|c| c.allows(slot))
    }

    fn first_visual_slot(&self) -> u8 {
        self.layout
            .as_ref()
            .and_then(|layout| {
                layout
                    .controls
                    .iter()
                    .flat_map(|control| control.slots())
                    .find(|slot| self.allows_visual_slot(*slot))
            })
            .unwrap_or(1)
    }

    fn activate_visual_slot(&mut self, index: u8) {
        self.profile_mut().visual_slot = index;
        self.restore_visual_layout();
        self.slot = self.first_visual_slot();
        self.layer = 1;
        self.reload_binding();
        let result = self.persist().map(|_| "Leiaute selecionado.".into());
        self.result(result);
    }

    fn set_default_layout_hidden(&mut self, hide: bool) {
        let Some(profile) = self.profile() else {
            return;
        };
        let Some(saved_slot) = profile.first_saved_layout() else {
            return;
        };
        let (old_hide, old_slot) = (profile.hide_default_layout, profile.visual_slot);
        let profile = self.profile_mut();
        profile.hide_default_layout = hide;
        if hide && profile.visual_slot == 0 {
            profile.visual_slot = saved_slot;
        }
        if let Err(error) = self.persist() {
            let profile = self.profile_mut();
            profile.hide_default_layout = old_hide;
            profile.visual_slot = old_slot;
            self.result(Err(error));
            return;
        }
        self.restore_visual_layout();
        if !self.allows_visual_slot(self.slot) {
            self.slot = self.first_visual_slot();
            self.reload_binding();
        }
    }

    fn delete_visual_layout(&mut self, index: u8) -> Result<String, String> {
        let position = index
            .checked_sub(1)
            .filter(|slot| *slot < 2)
            .ok_or("Escolha um layout personalizado salvo.")? as usize;
        let profile = self.profile_mut();
        let old = profile.visual_slots[position]
            .take()
            .ok_or("Escolha um layout personalizado salvo.")?;
        let (old_slot, old_hidden) = (profile.visual_slot, profile.hide_default_layout);
        let next = profile.first_saved_layout();
        if next.is_none() {
            profile.hide_default_layout = false;
        }
        if old_slot == index {
            profile.visual_slot = if profile.hide_default_layout {
                next.unwrap_or(0)
            } else {
                0
            };
        }
        if let Err(error) = self.persist() {
            let profile = self.profile_mut();
            profile.visual_slots[position] = Some(old);
            profile.visual_slot = old_slot;
            profile.hide_default_layout = old_hidden;
            return Err(error);
        }
        if old_slot == index {
            self.restore_visual_layout();
            self.slot = self.first_visual_slot();
            self.layer = 1;
            self.reload_binding();
        }
        Ok("Layout excluído.".into())
    }

    fn dial_angle(&self, index: u8) -> f32 {
        self.dial_turns
            .get(&index)
            .map_or(0., |turn| turn.at(Instant::now()))
    }

    fn physical_slot_down(&self, slot: u8) -> bool {
        let flashed = self
            .physical_flash
            .get(usize::from(slot))
            .is_some_and(|until| until.is_some_and(|end| end > Instant::now()));
        flashed
            || (self.raw_pressed != 0
                && self
                    .gesture_slots()
                    .iter()
                    .position(|s| *s == slot)
                    .is_some_and(|index| self.raw_pressed & (1 << index) != 0))
    }

    fn flash_control(&mut self, slot: u8) {
        if let Some(end) = self.physical_flash.get_mut(usize::from(slot)) {
            *end = Some(Instant::now() + Duration::from_millis(200));
        }
        if let Some(offset) = slot.checked_sub(13) {
            let direction = match offset % 3 {
                0 => -1.,
                2 => 1.,
                _ => return,
            };
            self.dial_turns
                .entry(offset / 3)
                .or_default()
                .advance(direction, Instant::now());
        }
    }

    fn observe_input(&mut self, input: platform::DirectInput, label: String) {
        let slot = self.profile().and_then(|profile| {
            let device = self.devices.get(self.current)?;
            let available: Vec<_> = self
                .hardware_layout(device)
                .controls
                .iter()
                .flat_map(|control| control.slots())
                .collect();
            let host_slots: Vec<_> = self
                .gesture_slots()
                .into_iter()
                .enumerate()
                .filter_map(|(index, slot)| (profile.host & (1 << index) != 0).then_some(slot))
                .collect();
            profile.observed_slot(input, &available, &host_slots)
        });
        if let Some(slot) = slot {
            self.flash_control(slot);
            log_marker_event(format_args!("controle observado {slot}: {label}"));
            self.last_input = Some(format!("{} · {}", ui::slot_name(slot), i18n::tr(&label)));
        } else {
            self.last_input = Some(i18n::tr(&label).to_owned());
        }
    }

    fn gesture_slots(&self) -> Vec<u8> {
        let Some(device) = self.devices.get(self.current) else {
            return Vec::new();
        };
        slots(&self.hardware_layout(device))
    }

    fn persist(&self) -> Result<(), String> {
        if let Some(e) = &self.load_error {
            return Err(e.clone());
        }
        let path = location();
        let tmp = path.with_extension("novo");
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(&self.profiles).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(tmp, path).map_err(|e| e.to_string())
    }

    fn result(&mut self, result: Result<String, String>) {
        self.status = result.unwrap_or_else(|e| format!("Erro: {e}"));
    }

    fn scan(&mut self, ctx: &egui::Context) {
        self.next_scan = Instant::now() + Duration::from_secs(2);
        match device::discover() {
            Ok(devices) => {
                let before = self.devices.get(self.current).map(|d| d.path.clone());
                self.current = before
                    .as_ref()
                    .and_then(|p| devices.iter().position(|d| &d.path == p))
                    .unwrap_or(0);
                let changed = before.as_ref() != devices.get(self.current).map(|d| &d.path);
                self.devices = devices;
                if changed {
                    self.select(ctx);
                }
                if self.devices.is_empty() {
                    self.status = "Conecte um Mini Teclado USB compatível.".into();
                }
                if let Some(d) = self.devices.get(self.current) {
                    if self.listener.is_some()
                        && self
                            .devices
                            .iter()
                            .filter(|o| o.vid == d.vid && o.pid == d.pid)
                            .count()
                            != 1
                    {
                        self.listener = None;
                        self.engine.reset();
                        self.simulate = true;
                        show_window(ctx);
                        self.status =
                            "Dois aparelhos iguais detectados. A escuta foi interrompida.".into();
                    }
                }
            }
            Err(e) => {
                self.engine.reset();
                self.simulate = true;
                self.status = e;
            }
        }
    }

    fn select(&mut self, ctx: &egui::Context) {
        let was_listening = self.listener.take().is_some();
        self.engine.reset();
        self.raw_pressed = 0;
        self.physical_flash.fill(None);
        self.dial_turns.clear();
        self.volume_pressed = 0;
        self.last_input = None;
        self.rgb = device::RgbLighting::default();
        self.simulate = true;
        self.key = self.devices.get(self.current).map(device_key);
        self.layout = None;
        let Some(d) = self.devices.get(self.current) else {
            self.show_offline_layout();
            if was_listening {
                show_window(ctx);
            }
            return;
        };
        let p = self.profiles.entry(device_key(d)).or_default();
        self.gesture_draft = p.gestures;
        self.simulate_draft = p.simulation;
        self.simulate = p.simulation;
        self.restore_visual_layout();
        self.layer = 1;
        self.slot = self.first_visual_slot();
        self.reload_binding();
        self.status = "Dispositivo reconhecido.".into();
        let result = self
            .listen(ctx)
            .map(|_| "Tecladinho conectado. Entradas monitoradas.".into());
        self.result(result);
    }

    fn reload_binding(&mut self) {
        self.recording = false;
        let key = address(self.layer, self.slot);
        self.draft = self
            .profile()
            .and_then(|p| p.bindings.get(&key))
            .cloned()
            .unwrap_or_else(seed);
        self.delay = self
            .profile()
            .and_then(|p| p.delays.get(&key))
            .copied()
            .unwrap_or(0);
    }

    fn listen(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let d = self
            .devices
            .get(self.current)
            .ok_or("Dispositivo desconectado.")?;
        if self
            .devices
            .iter()
            .filter(|o| o.vid == d.vid && o.pid == d.pid)
            .count()
            != 1
        {
            return Err("Deixe conectado um único aparelho deste modelo.".into());
        }
        let wake = ctx.clone();
        let hwnd = self.hwnd;
        self.listener = Some(Listener::start_with_wake(d.vid, d.pid, move || {
            if let Some(hwnd) = hwnd {
                platform::wake_gui(hwnd);
            }
            wake.request_repaint()
        })?);
        Ok(())
    }

    fn target(&self) -> Result<Device, String> {
        let mut d = self
            .devices
            .get(self.current)
            .ok_or("Dispositivo desconectado.")?
            .clone();
        d.profile = self.hardware_layout(&d);
        Ok(d)
    }

    fn probe_target(&self, device: &Device) -> Result<u8, String> {
        device::probe_report(device, self.profile().and_then(|profile| profile.report_id))
    }

    fn test_usb_report(&mut self, requested: Option<u8>) -> Result<String, String> {
        let device = self.target()?;
        let report = device::probe_report(&device, requested)?;
        let previous = self.profile_mut().report_id;
        if previous != requested {
            self.profile_mut().report_id = requested;
            if let Err(error) = self.persist() {
                self.profile_mut().report_id = previous;
                return Err(error);
            }
        }
        Ok(i18n::formatted(
            "A interface USB aceitou o relatório {}.",
            &[&report.to_string()],
        ))
    }

    fn manages_binding(&self, layer: u8, slot: u8) -> bool {
        let host = self.profile().map_or(0, |p| p.host);
        layer == 1
            && self
                .gesture_slots()
                .iter()
                .position(|s| *s == slot)
                .is_some_and(|i| host & (1 << i) != 0)
    }

    fn set_volume_step(&mut self, step: u8) -> Result<String, String> {
        let old = self.profile().ok_or("Selecione um perfil.")?.volume_step;
        self.profile_mut().volume_step = step.clamp(1, 10);
        if let Err(error) = self.persist() {
            self.profile_mut().volume_step = old;
            return Err(error);
        }
        Ok(format!(
            "Passo do giratório salvo em {step}% para os dois sentidos."
        ))
    }

    fn set_volume_mode(&mut self, enabled: bool) -> Result<String, String> {
        let device = self.target()?;
        if enabled && self.listener.is_none() {
            return Err("A escuta do tecladinho precisa estar ativa.".into());
        }
        let profile = self.profile().ok_or("Selecione um perfil.")?;
        let mut changes = Vec::new();
        for slot in (0..6).filter_map(platform::volume_slot) {
            let marked = profile.volume_managed.contains(&slot);
            let Some(action) = profile.bindings.get(&address(1, slot)) else {
                continue;
            };
            if volume_direction(action).is_none() || (!enabled && !marked) {
                continue;
            }
            let binding = if enabled {
                let marker = platform::volume_key(slot).expect("giro conhecido");
                if profile.marker_conflict(slot, marker) {
                    return Err(format!(
                        "Outro controle já usa o sinal de {}.",
                        ui::slot_name(slot)
                    ));
                }
                Binding::Keyboard(vec![(0, marker)])
            } else {
                action.clone()
            };
            changes.push((slot, binding));
        }
        if enabled && changes.is_empty() {
            return Err(
                "Salve Aumentar volume ou Diminuir volume em Mídia para o giratório.".into(),
            );
        }
        let mut reports = Vec::new();
        if !changes.is_empty() {
            let id = self.probe_target(&device)?;
            for (slot, binding) in &changes {
                reports.extend(device::binding_reports(&device, id, *slot, 1, 0, binding)?);
            }
        }
        let old_mode = profile.volume_mode;
        let old_routes = profile.volume_managed.clone();
        if !enabled && !reports.is_empty() {
            device::send(&device, &reports)?;
        }
        self.profile_mut().volume_mode = enabled;
        if enabled {
            self.profile_mut()
                .volume_managed
                .extend(changes.iter().map(|(slot, _)| *slot));
        } else {
            self.profile_mut().volume_managed.clear();
        }
        if let Err(error) = self.persist() {
            self.profile_mut().volume_mode = old_mode;
            self.profile_mut().volume_managed = old_routes;
            return Err(error);
        }
        if enabled && !reports.is_empty() {
            device::send(&device, &reports)?;
        }
        Ok(if enabled {
            "Passo do aplicativo ativo para os giros configurados como volume."
        } else {
            "Volume direto gravado no teclado. Funciona com o aplicativo fechado."
        }
        .into())
    }

    fn apply_binding(&mut self) -> Result<String, String> {
        if let Some(e) = &self.load_error {
            return Err(e.clone());
        }
        let d = self.target()?;
        let binding = self.draft.clone();
        if matches!(&binding,Binding::Keyboard(keys) if keys.is_empty()) {
            return Err("Adicione pelo menos um passo.".into());
        }
        let key = address(self.layer, self.slot);
        let marker = platform::volume_key(self.slot).filter(|_| self.layer == 1);
        if let Some(marker) = marker.filter(|_| {
            volume_direction(&binding).is_some() && self.profile().is_some_and(|p| p.volume_mode)
        }) {
            if self.listener.is_none() {
                return Err("Ative a escuta do tecladinho antes de salvar o volume.".into());
            }
            if self
                .profile()
                .is_some_and(|p| p.marker_conflict(self.slot, marker))
            {
                return Err("Outro controle já usa a tecla reservada para este ajuste. Altere aquele atalho primeiro.".into());
            }
            let id = self.probe_target(&d)?;
            let reports = device::binding_reports(
                &d,
                id,
                self.slot,
                self.layer,
                0,
                &Binding::Keyboard(vec![(0, marker)]),
            )?;
            let slot = self.slot;
            let profile = self.profile_mut();
            let had_marker = !profile.volume_managed.insert(slot);
            let old_binding = profile.bindings.insert(key.clone(), binding);
            if let Err(e) = self.persist() {
                let profile = self.profile_mut();
                if !had_marker {
                    profile.volume_managed.remove(&slot);
                }
                if let Some(old) = old_binding {
                    profile.bindings.insert(key, old);
                } else {
                    profile.bindings.remove(&key);
                }
                return Err(e);
            }
            device::send(&d, &reports)?;
            return Ok("Ação de volume enviada. O passo é compartilhado pelos giros.".into());
        }
        let managed = self.manages_binding(self.layer, self.slot);
        #[cfg(target_os = "macos")]
        if managed {
            macos::validate_replay(&binding)?;
        }
        if !managed {
            let conflict = self.profile().is_some_and(|p| {
                p.volume_managed
                    .iter()
                    .copied()
                    .filter_map(platform::volume_key)
                    .filter(|code| Some(*code) != marker)
                    .any(|code| uses_key(&binding, code))
            });
            if conflict {
                return Err("Esse atalho usa uma tecla reservada pelo ajuste de volume.".into());
            }
            let id = self.probe_target(&d)?;
            let reports =
                device::binding_reports(&d, id, self.slot, self.layer, self.delay, &binding)?;
            device::send(&d, &reports)?;
        }
        let (delay, slot) = (self.delay, self.slot);
        let profile = self.profile_mut();
        let old_delay = profile.delays.insert(key.clone(), delay);
        let old_binding = profile.bindings.insert(key.clone(), binding);
        let old_volume = marker.is_some() && profile.volume_managed.remove(&slot);
        if let Err(error) = self.persist() {
            let profile = self.profile_mut();
            if old_volume {
                profile.volume_managed.insert(slot);
            }
            if let Some(old) = old_delay {
                profile.delays.insert(key.clone(), old);
            } else {
                profile.delays.remove(&key);
            }
            if let Some(old) = old_binding {
                profile.bindings.insert(key, old);
            } else {
                profile.bindings.remove(&key);
            }
            return Err(error);
        }
        Ok(if managed {
            "Ação individual salva. As combinações da tecla continuam ativas."
        } else {
            "Ação enviada. Teste no tecladinho."
        }
        .into())
    }

    fn alter_host(&mut self, ctx: &egui::Context, op: Confirm) -> Result<String, String> {
        if let Some(e) = &self.load_error {
            return Err(e.clone());
        }
        let d = self.target()?;
        let keys = self.gesture_slots();
        let restoring = op == Confirm::Restore;
        let host = self.profile().map_or(0, |p| p.host);
        let allowed = ((1u16 << keys.len()) - 1) as u8;
        if restoring && host & !allowed != 0 {
            return Err("Leiaute incompatível com as teclas do perfil.".into());
        }
        let mut cfg = self.gesture_draft;
        let targets = if restoring {
            host
        } else {
            gesture_targets(cfg, allowed)?
        };
        let missing = missing_individual_actions(self.profile().unwrap(), &keys, targets);
        if !missing.is_empty() {
            let names = missing
                .into_iter()
                .map(ui::slot_name)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "Salve as ações individuais em Teclado antes de continuar. Faltam {names}."
            ));
        }
        #[cfg(target_os = "macos")]
        if !restoring {
            for (index, &slot) in keys.iter().enumerate() {
                if targets & (1 << index) != 0 {
                    macos::validate_replay(&self.profile().unwrap().bindings[&address(1, slot)])
                        .map_err(|error| format!("{}: {error}", ui::slot_name(slot)))?;
                }
            }
        }
        self.engine.reset();
        if !restoring && targets != 0 && self.listener.is_none() {
            self.listen(ctx)?;
        }
        let id = if targets != 0 {
            Some(self.probe_target(&d)?)
        } else {
            None
        };
        for (i, slot) in keys.into_iter().enumerate() {
            let bit = 1 << i;
            if targets & bit == 0 {
                continue;
            }
            let binding = if restoring {
                self.profile().unwrap().bindings[&address(1, slot)].clone()
            } else {
                Binding::Keyboard(vec![(0, 104 + i as u8)])
            };
            let delay = if restoring {
                self.profile()
                    .unwrap()
                    .delays
                    .get(&address(1, slot))
                    .copied()
                    .unwrap_or(0)
            } else {
                0
            };
            let reports = device::binding_reports(
                &d,
                id.expect("relatório para alvo de escrita"),
                slot,
                1,
                delay,
                &binding,
            )?;
            if !restoring {
                let previous = self.profile().unwrap().host;
                self.profile_mut().host |= bit;
                if let Err(e) = self.persist() {
                    self.profile_mut().host = previous;
                    return Err(e);
                }
            }
            device::send(&d, &reports)?;
            if restoring {
                self.profile_mut().host &= !bit;
                self.persist()?;
            }
        }
        if restoring {
            cfg = self.profile().unwrap().gestures;
            cfg.enabled = false;
            cfg.shutdown_armed = false;
            self.gesture_draft = cfg;
        } else {
            cfg.shutdown_armed &= cfg.enabled && cfg.shutdown != 0;
        }
        let simulation = restoring || self.simulate_draft;
        let (previous, previous_simulation) =
            self.profile().map(|p| (p.gestures, p.simulation)).unwrap();
        self.profile_mut().gestures = cfg;
        self.profile_mut().simulation = simulation;
        if let Err(error) = self.persist() {
            self.profile_mut().gestures = previous;
            self.profile_mut().simulation = previous_simulation;
            return Err(error);
        }
        self.gesture_draft = cfg;
        self.simulate_draft = simulation;
        self.simulate = simulation;
        log_marker_event(format_args!(
            "aplicado: sinais {targets:04b}; teste {simulation}"
        ));
        Ok(if restoring {
            "Modo direto restaurado."
        } else if targets == 0 {
            "Combinações desativadas."
        } else if simulation {
            "Sinais enviados. As combinações estão em simulação."
        } else {
            "Sinais enviados. As ações do sistema estão ativas."
        }
        .into())
    }

    fn apply_led(&mut self) -> Result<String, String> {
        let d = self.target()?;
        if device::supports_rgb(&d) {
            self.rgb = device::apply_lighting(&d, self.rgb)?;
            return Ok("Iluminação RGB aplicada e confirmada pela leitura do aparelho.".into());
        }
        let id = self.probe_target(&d)?;
        let reports = device::led_reports(&d, id, self.layer, self.led, self.led_color)?;
        device::send(&d, &reports)?;
        Ok("Iluminação enviada ao aparelho.".into())
    }

    fn read_rgb(&mut self) -> Result<String, String> {
        let d = self.target()?;
        self.rgb = device::read_lighting(&d)?;
        Ok("Iluminação RGB lida do aparelho.".into())
    }

    fn read_device_bindings(&mut self) -> Result<String, String> {
        let device = self.target()?;
        let active_layer = self.layer;
        let mut saved = 0;
        for (slot, binding) in device::read_bindings(&device, active_layer)? {
            let key = address(active_layer, slot);
            if self.manages_binding(active_layer, slot)
                || (active_layer == 1
                    && self
                        .profile()
                        .is_some_and(|p| p.volume_managed.contains(&slot)))
            {
                continue;
            }
            let profile = self.profile_mut();
            if let Some(binding) = binding {
                profile.bindings.insert(key, binding);
            } else {
                profile.bindings.remove(&key);
            }
            saved += 1;
        }
        self.persist()?;
        self.reload_binding();
        Ok(format!(
            "{saved} ações reconhecidas foram lidas do aparelho."
        ))
    }

    fn handle(&mut self, ctx: &egui::Context, action: GestureAction) {
        log_marker_event(format_args!("ação: {action:?}; teste {}", self.simulate));
        match action {
            GestureAction::Individual(i) => {
                let binding = self
                    .gesture_slots()
                    .get(i as usize)
                    .copied()
                    .and_then(|slot| self.profile()?.bindings.get(&address(1, slot)));
                if let Some(b) = binding {
                    match platform::replay(b) {
                        Ok(()) => log_marker_event(format_args!(
                            "ação individual {} enviada: {b:?}",
                            i + 1
                        )),
                        Err(error) => self.status = error,
                    }
                }
            }
            GestureAction::DisplayOff => {
                if self.simulate {
                    self.status = "Teste: combinação de alternar a tela reconhecida.".into();
                } else {
                    let result = self.screen.toggle().map(|off| {
                        if off {
                            "Pedido para apagar a tela enviado."
                        } else {
                            "Pedido para ligar a tela enviado."
                        }
                        .into()
                    });
                    self.result(result);
                }
            }
            GestureAction::CountdownStarted => {
                self.engine.present_countdown(Instant::now());
                self.preview_until = None;
                self.status = if self.shutdown_trial.is_complete() {
                    "Contagem iniciada. O gesto de cancelar está disponível."
                } else {
                    "Primeiro teste de desligamento. Você pode deixar terminar ou cancelar."
                }
                .into();
            }
            GestureAction::Cancelled => {
                self.status = "Desligamento cancelado.".into();
                self.finish_shutdown_trial(ctx, "Teste inicial cancelado.");
            }
            GestureAction::CountdownAborted => {
                self.status = "Desligamento cancelado.".into();
            }
            GestureAction::ShutdownSimulation => {
                self.status = "Teste concluído. O computador continua ligado.".into();
                self.finish_shutdown_trial(ctx, "Teste inicial concluído.");
            }
            GestureAction::Shutdown => {
                let armed = self.profile().is_some_and(|p| p.gestures.shutdown_armed);
                if self.shutdown_trial.allow_real(armed, self.simulate) {
                    self.result(platform::shutdown().map(|_| "Desligamento solicitado.".into()));
                }
            }
        }
    }

    fn finish_shutdown_trial(&mut self, ctx: &egui::Context, outcome: &str) {
        match self.shutdown_trial.complete() {
            Ok(false) => return,
            Ok(true) => {
                let armed = self.profile().is_some_and(|p| p.gestures.shutdown_armed);
                let next = if self.shutdown_trial.allow_real(armed, self.simulate) {
                    "O próximo gesto pode desligar o computador."
                } else {
                    "O gesto poderá desligar quando você aplicar o modo real e permitir o desligamento."
                };
                self.status = format!("{outcome} {next}");
            }
            Err(error) => {
                self.status = format!("{outcome} Não foi possível salvar o teste: {error}");
            }
        }
        self.page = Page::Gestures;
        show_window(ctx);
    }

    fn update_countdown(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let remaining = self.engine.remaining(now).or_else(|| {
            self.preview_until
                .map(|until| until.saturating_duration_since(now))
        });
        if let Some(remaining) = remaining {
            let dry_run = !self.engine.will_shutdown();
            if self.countdown.is_none() {
                let wake = ctx.clone();
                let hwnd = self.hwnd;
                match countdown_window::CountdownWindow::open(dry_run, move || {
                    if let Some(hwnd) = hwnd {
                        platform::wake_gui(hwnd);
                    }
                    wake.request_repaint();
                }) {
                    Ok(window) => self.countdown = Some(window),
                    Err(error) => {
                        self.engine.cancel();
                        self.preview_until = None;
                        self.status = format!("Contagem cancelada. {error}");
                    }
                }
            }
            if let Some(window) = &mut self.countdown {
                window.update(remaining, dry_run);
            }
        } else {
            self.countdown = None;
        }
    }

    fn process_events(&mut self, ctx: &egui::Context) {
        let (mut cfg, host) = self
            .profile()
            .map(|p| (p.gestures, p.host))
            .unwrap_or_default();
        let host = host & 0x0f;
        cfg.enabled &= !self.paused;
        if cfg.display & !host != 0 {
            cfg.display = 0;
        }
        if cfg.shutdown & !host != 0 {
            cfg.shutdown = 0;
        }
        if cfg.cancel & !host != 0 {
            cfg.cancel = 0;
        }
        if cfg.display != 0 && cfg.display == cfg.shutdown {
            cfg.shutdown = 0;
        }
        cfg.shutdown_armed = self
            .shutdown_trial
            .allow_real(cfg.shutdown_armed, self.simulate);
        let events: Vec<_> = self
            .listener
            .as_ref()
            .map(|l| l.events.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                InputEvent::Key { index, down, at } if index < 4 => {
                    let bit = 1 << index;
                    if host & bit == 0 {
                        if down {
                            self.observe_input(
                                platform::DirectInput::function_key(0x7c + u16::from(index)),
                                format!("F{}", index + 13),
                            );
                        }
                        continue;
                    }
                    if down {
                        if self.raw_pressed & bit == 0 {
                            if let Some(slot) = self.gesture_slots().get(index as usize).copied() {
                                self.flash_control(slot);
                            }
                            self.last_input =
                                Some(self.gesture_slots().get(index as usize).map_or_else(
                                    || format!("F{}", index + 13),
                                    |slot| ui::slot_name(*slot),
                                ));
                        }
                        self.raw_pressed |= bit;
                    } else {
                        self.raw_pressed &= !bit;
                    }
                    if host & bit != 0 {
                        for output in self.engine.event(index, down, at, &cfg) {
                            self.handle(ctx, output);
                        }
                    }
                    let pressed = self.engine.pressed_mask();

                    log_marker_event(format_args!(
                        "controle {} {} — simultâneas {pressed:04b}",
                        index + 1,
                        if down { "pressionado" } else { "solto" }
                    ));
                }
                InputEvent::Key { .. } => {}
                InputEvent::VolumeKey { index, down } => {
                    let bit = 1 << index;
                    if !down {
                        self.volume_pressed &= !bit;
                        continue;
                    }
                    if self.volume_pressed & bit != 0 {
                        continue;
                    }
                    self.volume_pressed |= bit;
                    if let Some(slot) = platform::volume_slot(index) {
                        let adjustment = self.profile().and_then(|p| p.rotary_adjustment(slot));
                        if let Some(step) = adjustment {
                            self.flash_control(slot);
                            match audio::adjust(step) {
                                Ok(level) => {
                                    self.volume_osd.show(level);
                                    self.last_input = Some(format!(
                                        "{} · {} {level}%",
                                        ui::slot_name(slot),
                                        i18n::system_text("Volume do Windows")
                                    ));
                                    log_marker_event(format_args!(
                                        "volume {step:+} → {level}% ({slot})"
                                    ));
                                }
                                Err(error) => self.status = error,
                            }
                        } else {
                            self.observe_input(
                                platform::DirectInput::function_key(0x80 + u16::from(index)),
                                format!("F{}", index + 17),
                            );
                        }
                    }
                }
                InputEvent::Observed(input, label) => {
                    log_marker_event(format_args!("entrada direta: {label}"));
                    self.observe_input(input, label);
                }
                InputEvent::Disconnected => {
                    self.engine.reset();
                    self.raw_pressed = 0;
                    self.physical_flash.fill(None);
                    self.dial_turns.clear();
                    self.volume_pressed = 0;
                    self.simulate = true;
                    self.scan(ctx);
                    return;
                }
                InputEvent::ShowWindow => show_window(ctx),
                InputEvent::Quit => {
                    self.exiting = true;
                    self.engine.reset();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if let Some(action) = self.engine.tick(Instant::now(), &cfg) {
            self.handle(ctx, action);
        }
    }
}

fn show_window(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(not(windows))]
        if platform::take_show_request() {
            show_window(ctx);
        }
        if std::mem::take(&mut self.start_minimized) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            if self.listener.as_ref().is_some_and(Listener::tray_available) {
                // eframe displays the root window after the first paint.
                self.hide_startup_at = Some(Instant::now() + Duration::from_millis(150));
            }
        }
        if let Some(deadline) = self.hide_startup_at {
            let now = Instant::now();
            if now >= deadline {
                self.hide_startup_at = None;
                if self.listener.as_ref().is_some_and(Listener::tray_available) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
            } else {
                ctx.request_repaint_after(deadline - now);
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            if !self.exiting && self.listener.as_ref().is_some_and(Listener::tray_available) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            } else {
                self.engine.reset();
            }
        }
        if Instant::now() >= self.next_scan {
            self.scan(ctx);
        }
        if self
            .countdown
            .as_ref()
            .is_some_and(|window| window.take_cancelled())
        {
            if let Some(action) = self.engine.cancel() {
                self.handle(ctx, action);
            } else {
                self.preview_until = None;
                self.status = "Demonstração cancelada.".into();
            }
        }
        self.process_events(ctx);
        let now = Instant::now();
        if self.dial_turns.values().any(|turn| turn.running(now)) {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        if let Some(end) = self
            .physical_flash
            .iter()
            .flatten()
            .filter(|end| **end > now)
            .min()
        {
            ctx.request_repaint_after(*end - now);
        }
        if self.preview_until.is_some_and(|deadline| now >= deadline) {
            self.preview_until = None;
            self.status = "Demonstração concluída. O computador continua ligado.".into();
        }
        self.update_countdown(ctx);
        #[cfg(not(windows))]
        {
            if let Some(window) = &mut self.countdown {
                window.render(ctx);
            }
            self.volume_osd.render(ctx);
        }
        ctx.request_repaint_after(self.next_scan.saturating_duration_since(now));
        self.keyboard_shortcuts(ctx);
        self.render(ctx);
    }
}

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--diagnostico") {
        let result = device::discover().and_then(|devices| {
            if devices.is_empty() {
                return Err("Nenhum dispositivo reconhecido.".into());
            }
            devices
                .into_iter()
                .map(|d| {
                    device::probe(&d).map(|report| {
                        format!("{} | {} | Relatório ID {report}", device_key(&d), d.model)
                    })
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|rows| rows.join("\n"))
        });
        let message = result.as_deref().unwrap_or_else(|e| e);
        if let Some(path) = std::env::var_os("MINI_TECLADO_DIAGNOSTICO") {
            if let Err(e) = std::fs::write(path, message) {
                eprintln!("{e}");
                std::process::exit(2);
            }
        }
        println!("{message}");
        if result.is_err() {
            std::process::exit(1);
        }
        return Ok(());
    }
    let start_minimized = std::env::args().any(|argument| argument == "--bandeja");
    let _instance = if std::env::var_os("MINI_TECLADO_CONFIG").is_none() {
        match platform::claim_instance() {
            Ok(Some(guard)) => Some(guard),
            Ok(None) => {
                if !start_minimized {
                    platform::restore_existing_app(APP_TITLE);
                }
                return Ok(());
            }
            Err(error) => {
                eprintln!("{error}");
                None
            }
        }
    } else {
        None
    };
    let logo = image::load_from_memory(include_bytes!("../assets/logo.png"))
        .expect("logo integrado")
        .to_rgba8();
    let small = image::imageops::resize(&logo, 64, 64, image::imageops::FilterType::Lanczos3);
    let icon = egui::IconData {
        rgba: small.into_raw(),
        width: 64,
        height: 64,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1050.0, 700.0])
            .with_min_inner_size([550.0, 450.0])
            .with_active(!start_minimized)
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc, &logo, start_minimized)))),
    )
}

#[cfg(test)]
mod apply_tests {
    use super::*;

    #[test]
    fn fast_detents_accumulate_and_reverse_without_restarting_angle() {
        let now = Instant::now();
        let mut turn = DialTurn::default();
        turn.advance(1., now);
        turn.advance(1., now + Duration::from_millis(10));
        turn.advance(1., now + Duration::from_millis(20));
        let settled = now + Duration::from_secs(1);
        assert!((turn.at(settled) - 3. * DIAL_STEP).abs() < 0.0001);
        turn.advance(-1., settled);
        assert!((turn.at(settled + Duration::from_secs(1)) - 2. * DIAL_STEP).abs() < 0.0001);
    }

    #[test]
    fn physical_feedback_identifies_unique_configured_outputs_only() {
        let profile: Profile = serde_json::from_str(
            r#"{"bindings":{"1:1":{"Media":205},"1:2":{"Media":181},"2:4":{"Media":181},"1:14":{"Keyboard":[[1,4]]}}}"#,
        ).unwrap();
        let allowed = [1, 2, 4, 14];
        assert_eq!(
            profile.observed_slot(platform::DirectInput::Media(0xcd), &allowed, &[]),
            Some(1)
        );
        assert_eq!(
            profile.observed_slot(platform::DirectInput::Media(0xb5), &allowed, &[]),
            None
        );
        assert_eq!(
            profile.observed_slot(platform::DirectInput::Media(0xcd), &allowed, &[1]),
            None
        );
        assert_eq!(
            profile.observed_slot(
                platform::DirectInput::Keyboard {
                    scan: 0x1e,
                    vk: 0x41,
                    modifiers: 1
                },
                &allowed,
                &[]
            ),
            Some(14)
        );
        assert_eq!(
            profile.observed_slot(
                platform::DirectInput::Keyboard {
                    scan: 0x1e,
                    vk: 0x41,
                    modifiers: 0
                },
                &allowed,
                &[]
            ),
            None
        );
    }

    #[test]
    fn hiding_default_layout_switches_to_a_saved_layout_and_recovers_without_one() {
        let mut profile: Profile = serde_json::from_str(r#"{"hide_default_layout":true}"#).unwrap();
        profile.migrate_layouts();
        assert_eq!(profile.visual_slot, 0);
        assert!(!profile.hide_default_layout);
        profile.hide_default_layout = true;
        profile.visual_slots[1] = Some(custom::CustomLayout::default());
        profile.migrate_layouts();
        assert_eq!(profile.visual_slot, 2);
        assert!(profile.hide_default_layout);
    }

    #[test]
    fn legacy_rotary_markers_share_one_step_and_keep_their_directions() {
        let mut profile: Profile = serde_json::from_str(
            r#"{"bindings":{"1:13":{"Media":234},"1:15":{"Media":233}},"volume_steps":{"1:13":5,"1:15":7}}"#,
        ).unwrap();
        profile.migrate_volume();
        assert!(profile.volume_mode);
        assert_eq!(profile.volume_step, 5);
        assert_eq!(profile.rotary_adjustment(13), Some(-5));
        assert_eq!(profile.rotary_adjustment(15), Some(5));
        profile.volume_step = 1;
        assert_eq!(profile.rotary_adjustment(13), Some(-1));
        assert_eq!(profile.rotary_adjustment(15), Some(1));
        assert!(!serde_json::to_string(&profile)
            .unwrap()
            .contains("volume_steps"));
    }

    #[test]
    fn native_media_does_not_change_the_windows_volume_twice() {
        let mut profile: Profile = serde_json::from_str(
            r#"{"bindings":{"1:13":{"Media":234},"1:15":{"Media":233}},"volume_step":10}"#,
        )
        .unwrap();
        profile.migrate_volume();
        assert!(!profile.volume_mode);
        assert_eq!(profile.rotary_adjustment(13), None);
        assert_eq!(profile.rotary_adjustment(15), None);
        assert_eq!(profile.bindings.get("1:15"), Some(&Binding::Media(233)));
    }

    #[test]
    fn legacy_custom_layout_moves_to_slot_two_without_touching_actions() {
        let mut profile: Profile = serde_json::from_str(
            r#"{"custom":{"buttons":4,"knobs":[]},"bindings":{"1:1":{"Media":182}}}"#,
        )
        .unwrap();
        profile.migrate_layouts();
        assert_eq!(profile.visual_slot, 1);
        assert_eq!(profile.selected_layout().unwrap().buttons, 4);
        assert!(profile.visual_slots[1].is_none());
        assert_eq!(profile.bindings["1:1"], Binding::Media(182));
        let saved = serde_json::to_string(&profile).unwrap();
        let round_trip: Profile = serde_json::from_str(&saved).unwrap();
        assert_eq!(round_trip.selected_layout().unwrap().buttons, 4);
    }

    #[test]
    fn chords_require_known_individual_actions_for_every_reserved_key() {
        let mut profile = Profile::default();
        let keys = [1, 2, 3, 14];
        profile.bindings.insert(address(1, 1), Binding::Media(0xb6));
        assert_eq!(missing_individual_actions(&profile, &keys, 7), [2, 3]);
        profile.bindings.insert(address(1, 2), Binding::Media(0xcd));
        profile.bindings.insert(address(1, 3), Binding::Media(0xb5));
        assert!(missing_individual_actions(&profile, &keys, 7).is_empty());
        assert_eq!(missing_individual_actions(&profile, &keys, 15), [14]);
        assert_eq!(profile.bindings[&address(1, 1)], Binding::Media(0xb6));
    }

    #[test]
    fn apply_selects_all_keys_of_a_three_key_shutdown_and_optional_fourth() {
        let settings = GestureConfig {
            enabled: true,
            display: 8,
            shutdown: 7,
            cancel: 2,
            shutdown_armed: false,
        };
        assert_eq!(gesture_targets(settings, 15).unwrap(), 15);
        assert!(gesture_targets(settings, 7).is_err());
    }

    #[test]
    fn disabled_settings_do_not_write_markers_and_duplicates_are_rejected_when_active() {
        let mut settings = GestureConfig {
            enabled: false,
            display: 7,
            shutdown: 7,
            cancel: 2,
            shutdown_armed: false,
        };
        assert_eq!(gesture_targets(settings, 15).unwrap(), 0);
        settings.enabled = true;
        assert!(gesture_targets(settings, 15).is_err());
    }
}
