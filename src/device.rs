//! Report formats and stock layout data adapted from rOzzy1987/MacroPad, GPL-3.0.
//! Reference commit c51d4d39bc94ec4c0cdfc6a2b8203551a852dca5.

use hidapi::{BusType, DeviceInfo, HidApi, HidDevice};
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

#[path = "fd.rs"]
mod fd;
#[path = "webhub.rs"]
mod webhub;
pub use webhub::RgbLighting;

const CONFIG: &str = concat!(
    include_str!("../assets/rsoft-config.txt"),
    "\n28027:56570,mi_02,2\n"
);
const LAYOUTS: &str = concat!(
    include_str!("../assets/rsoft-layouts.txt"),
    "\nLayout: SDINNOVATION SIDE-KEYBOARD (3 teclas e 1 botão giratório)\n28027:56570\n1:1:0:1:5\nB1,5,5\nB2,25,5\nB3,45,5\nK1,70,5\n"
);
const REPORT_LEN: usize = 65;
const KNOWN_LEGACY_DEVICE: (u16, u16) = (0x1189, 0x8890);
const KNOWN_WEBHUB_DEVICE: (u16, u16) = (0x6d7b, 0xdcfa);
const KNOWN_FD_DEVICE: (u16, u16) = (0x1189, 0x8840);
const FAMILY_VIDS: [u16; 2] = [0x1189, 0x514c];
static HID_API: Mutex<Option<HidApi>> = Mutex::new(None);
static WEBHUB_CHANNEL: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    Legacy,
    Extended,
    WebHub,
    Fd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlKind {
    Button,
    Knob,
}

#[derive(Clone, Debug)]
pub struct Control {
    pub kind: ControlKind,
    pub id: u8,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Control {
    pub fn slots(&self) -> Vec<u8> {
        match self.kind {
            ControlKind::Button => vec![self.id],
            ControlKind::Knob => {
                let start = 13 + 3 * (self.id - 1);
                vec![start, start + 1, start + 2]
            }
        }
    }

    fn contains_slot(&self, slot: u8) -> bool {
        match self.kind {
            ControlKind::Button => slot == self.id,
            ControlKind::Knob => {
                let start = 13 + 3 * (self.id - 1);
                (start..=start + 2).contains(&slot)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub name: String,
    pub layers: u8,
    pub max_sequence: u8,
    pub supports_delay: bool,
    pub supports_led_color: bool,
    pub led_modes: u8,
    pub controls: Vec<Control>,
}

#[derive(Clone, Debug)]
pub struct Device {
    pub path: CString,
    pub vid: u16,
    pub pid: u16,
    pub report_len: usize,
    pub model: String,
    pub profile: Layout,
    pub protocol: Protocol,
    layouts: Arc<[Layout]>,
}

#[derive(Clone, Debug)]
pub struct UsbCandidate {
    pub vid: u16,
    pub pid: u16,
    pub model: String,
    pub interface: String,
    pub usage_page: u16,
    pub usage: u16,
    /// An approximate layout is only a visual suggestion.
    pub likely_layout: Option<String>,
    /// This interface matches the regular configuration, including USB usage.
    pub supported: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Binding {
    Keyboard(Vec<(u8, u8)>),
    Media(u16),
    Mouse {
        buttons: u8,
        scroll: i8,
        modifiers: u8,
    },
}

fn content_lines(input: &str) -> impl Iterator<Item = &str> {
    input
        .trim_start_matches('\u{feff}')
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty())
}

fn parse_id(pair: &str) -> Option<(u16, u16)> {
    let (vendor, product) = pair.split_once(':')?;
    Some((vendor.trim().parse().ok()?, product.trim().parse().ok()?))
}

pub fn settings_dir() -> PathBuf {
    std::env::var_os("MINI_TECLADO_CONFIG")
        .map(PathBuf::from)
        .or_else(settings_base)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(windows)]
fn settings_base() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|root| PathBuf::from(root).join("MiniTecladoUSB"))
}

#[cfg(target_os = "macos")]
fn settings_base() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/MiniTecladoUSB"))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn settings_base() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|path| path.join("MiniTecladoUSB"))
}

fn custom_file(name: &str) -> String {
    std::fs::read_to_string(settings_dir().join(name)).unwrap_or_default()
}

fn configured_devices(custom: &str) -> impl Iterator<Item = ((u16, u16), &str, Protocol)> {
    content_lines(custom)
        .chain(content_lines(CONFIG))
        .filter_map(|line| {
            let mut fields = line.split(',').map(str::trim);
            let id = parse_id(fields.next()?)?;
            let interface = fields.next()?;
            let protocol = match fields.next()? {
                "0" => Protocol::Legacy,
                "1" => Protocol::Extended,
                "2" => Protocol::WebHub,
                _ => return None,
            };
            if interface.len() < 3
                || interface.len() > 96
                || !interface
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_&#-".contains(&byte))
                || fields.next().is_some()
            {
                return None;
            }
            Some((id, interface, protocol))
        })
}

fn parse_control(line: &str) -> Option<Control> {
    let mut fields = line.split(',').map(str::trim);
    let key = fields.next()?;
    let (kind, id, limit) = if let Some(id) = key.strip_prefix('B') {
        (ControlKind::Button, id, 12)
    } else {
        (ControlKind::Knob, key.strip_prefix('K')?, 3)
    };
    let id = id.parse::<u8>().ok()?;
    if !(1..=limit).contains(&id) {
        return None;
    }
    let x = fields.next()?.parse::<f32>().ok()?;
    let y = fields.next()?.parse::<f32>().ok()?;
    let width = fields
        .next()
        .map_or(Some(20.0), |field| field.parse().ok())?;
    let height = fields
        .next()
        .map_or(Some(20.0), |field| field.parse().ok())?;
    if ![x, y, width, height]
        .iter()
        .all(|n| n.is_finite() && n.abs() <= 4096.0)
        || width <= 0.0
        || height <= 0.0
        || fields.next().is_some()
    {
        return None;
    }
    Some(Control {
        kind,
        id,
        x,
        y,
        width,
        height,
    })
}

fn parse_layout(block: &[&str]) -> Option<(Vec<(u16, u16)>, Layout)> {
    let name = block.first()?.strip_prefix("Layout:")?.trim();
    let pairs = block
        .get(1)?
        .split(',')
        .map(parse_id)
        .collect::<Option<Vec<_>>>()?;
    let capabilities: Vec<u8> = block
        .get(2)?
        .split(':')
        .map(|element| element.trim().parse::<u8>())
        .collect::<Result<_, _>>()
        .ok()?;
    let &[layers, max_sequence, delay, led_color, led_modes] = capabilities.as_slice() else {
        return None;
    };
    if name.is_empty()
        || name.len() > 100
        || !(1..=15).contains(&layers)
        || !(1..=27).contains(&max_sequence)
        || delay > 1
        || led_color > 1
        || led_modes > 6
    {
        return None;
    }
    let controls = block
        .get(3..)?
        .iter()
        .map(|line| parse_control(line))
        .collect::<Option<Vec<_>>>()?;
    if controls.is_empty()
        || controls.len() > 15
        || controls.iter().enumerate().any(|(index, control)| {
            controls[..index]
                .iter()
                .any(|previous| previous.kind == control.kind && previous.id == control.id)
        })
    {
        return None;
    }
    Some((
        pairs,
        Layout {
            name: if name == "3 buttons 1 knob" {
                "3 teclas e 1 botão giratório".into()
            } else {
                name.into()
            },
            layers,
            max_sequence,
            supports_delay: delay == 1,
            supports_led_color: led_color == 1,
            led_modes,
            controls,
        },
    ))
}

fn layout_matches(defined: (u16, u16), actual: (u16, u16)) -> bool {
    // RSoft's USB config lists decimal VID 4489 for its extended models, while
    // the corresponding stock geometry entries spell the same vendor 4498.
    defined == actual || (defined.0 == 4498 && actual.0 == 4489 && defined.1 == actual.1)
}

type LayoutEntry = (Vec<(u16, u16)>, Layout);

fn parse_layout_file(input: &str) -> Vec<LayoutEntry> {
    let lines: Vec<_> = content_lines(input).collect();
    let mut starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.starts_with("Layout:").then_some(index))
        .collect();
    starts.push(lines.len());
    starts
        .windows(2)
        .filter_map(|window| parse_layout(&lines[window[0]..window[1]]))
        .collect()
}

fn matching_layouts(entries: &[LayoutEntry], vid: u16, pid: u16) -> impl Iterator<Item = &Layout> {
    entries.iter().filter_map(move |(ids, layout)| {
        ids.iter()
            .any(|&id| layout_matches(id, (vid, pid)))
            .then_some(layout)
    })
}

fn stock_layouts() -> &'static [LayoutEntry] {
    static STOCK: OnceLock<Vec<LayoutEntry>> = OnceLock::new();
    STOCK.get_or_init(|| parse_layout_file(LAYOUTS))
}

#[cfg(test)]
fn layouts(vid: u16, pid: u16) -> Vec<Layout> {
    layouts_with_custom(vid, pid, &[])
}

fn layouts_with_custom(vid: u16, pid: u16, custom: &[LayoutEntry]) -> Vec<Layout> {
    let mut matches: Vec<Layout> = matching_layouts(stock_layouts(), vid, pid)
        .cloned()
        .collect();
    for layout in matching_layouts(custom, vid, pid) {
        if matches!((vid, pid), KNOWN_LEGACY_DEVICE | KNOWN_WEBHUB_DEVICE)
            && matches.first().is_some_and(|stock| {
                layout.layers > stock.layers
                    || layout.max_sequence > stock.max_sequence
                    || (layout.supports_delay && !stock.supports_delay)
                    || (layout.supports_led_color && !stock.supports_led_color)
                    || layout.led_modes > stock.led_modes
                    || layout.controls.iter().any(|control| {
                        !stock
                            .controls
                            .iter()
                            .any(|native| native.kind == control.kind && native.id == control.id)
                    })
            })
        {
            continue;
        }
        if !matches.iter().any(|existing| existing.name == layout.name) {
            matches.push(layout.clone());
        }
    }
    if !matches.is_empty() {
        return matches;
    }
    let mut controls = Vec::new();
    for id in 1..=12u8 {
        controls.push(Control {
            kind: ControlKind::Button,
            id,
            x: f32::from((id - 1) % 4) * 20.0,
            y: f32::from((id - 1) / 4) * 20.0,
            width: 20.0,
            height: 20.0,
        });
    }
    for id in 1..=3u8 {
        controls.push(Control {
            kind: ControlKind::Knob,
            id,
            x: 87.0,
            y: f32::from(id - 1) * 20.0,
            width: 20.0,
            height: 20.0,
        });
    }
    vec![Layout {
        name: "Layout genérico de 12 teclas e 3 botões giratórios".into(),
        layers: 3,
        max_sequence: 18,
        supports_delay: true,
        supports_led_color: true,
        led_modes: 6,
        controls,
    }]
}

pub fn layouts_for(device: &Device) -> Vec<Layout> {
    device.layouts.to_vec()
}

pub fn default_layout() -> Layout {
    let (vid, pid) = KNOWN_LEGACY_DEVICE;
    matching_layouts(stock_layouts(), vid, pid)
        .next()
        .expect("perfil embutido do tecladinho 1189:8890")
        .clone()
}

type ConfigEntry<'a> = ((u16, u16), &'a str, Protocol);

fn configured_protocol(info: &DeviceInfo, config: &[ConfigEntry<'_>]) -> Option<Protocol> {
    let id = (info.vendor_id(), info.product_id());
    // Non-Windows HIDAPI can enumerate several top-level collections with an
    // identical OS path. Require a vendor collection before selecting one by MI.
    #[cfg(not(windows))]
    if !is_usb_vendor_collection(info.bus_type(), info.usage_page()) {
        return None;
    }
    if fd_interface(info) {
        return Some(Protocol::Fd);
    }
    if id == KNOWN_FD_DEVICE && !is_usb_vendor_collection(info.bus_type(), info.usage_page()) {
        return None;
    }
    if id == KNOWN_LEGACY_DEVICE && (info.usage_page() != 0xff00 || info.usage() != 1) {
        return None;
    }
    if id == KNOWN_WEBHUB_DEVICE
        && (!matches!(info.bus_type(), BusType::Usb)
            || info.usage_page() != 0xff00
            || !device_interface_matches(info, b"mi_02"))
    {
        return None;
    }
    config
        .iter()
        .find(|(configured, interface, protocol)| {
            *configured == id
                && (id != KNOWN_LEGACY_DEVICE || *protocol == Protocol::Legacy)
                && ((id == KNOWN_WEBHUB_DEVICE) == (*protocol == Protocol::WebHub))
                && device_interface_matches(info, interface.as_bytes())
        })
        .map(|(_, _, protocol)| *protocol)
}

fn fd_interface(info: &DeviceInfo) -> bool {
    is_fd_collection(
        (info.vendor_id(), info.product_id()),
        info.bus_type(),
        info.usage_page(),
        device_interface_matches(info, b"mi_00"),
    )
}

fn is_fd_collection(id: (u16, u16), bus: BusType, page: u16, is_interface_zero: bool) -> bool {
    id == KNOWN_FD_DEVICE && matches!(bus, BusType::Usb) && page == 0xff00 && is_interface_zero
}

fn fd_layout() -> Layout {
    let source = [
        "Layout: Mini Teclado FD (6 teclas e 1 botão giratório)",
        "4489:34880",
        "1:18:0:0:0",
        "B1,5,5",
        "B2,25,5",
        "B3,45,5",
        "B4,5,25",
        "B5,25,25",
        "B6,45,25",
        "K1,70,15",
    ];
    parse_layout(&source).expect("perfil FD embutido").1
}

#[cfg(windows)]
fn device_interface_matches(info: &DeviceInfo, interface: &[u8]) -> bool {
    path_matches_interface(info.path().to_bytes(), interface)
}

#[cfg(not(windows))]
fn device_interface_matches(info: &DeviceInfo, interface: &[u8]) -> bool {
    number_matches_interface(info.interface_number(), interface)
}

#[cfg(any(not(windows), test))]
fn number_matches_interface(number: i32, interface: &[u8]) -> bool {
    if interface.len() != 5
        || !interface[..3].eq_ignore_ascii_case(b"mi_")
        || !interface[3..].iter().all(u8::is_ascii_hexdigit)
    {
        return false;
    }
    std::str::from_utf8(&interface[3..])
        .ok()
        .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        .is_some_and(|interface_number| i32::from(interface_number) == number)
}

#[cfg(any(windows, test))]
fn path_matches_interface(path: &[u8], interface: &[u8]) -> bool {
    path.iter()
        .enumerate()
        .filter(|(_, byte)| **byte == b'&')
        .map(|(index, _)| &path[index + 1..])
        .any(|tail| {
            tail.get(..interface.len())
                .is_some_and(|label| label.eq_ignore_ascii_case(interface))
                && matches!(tail.get(interface.len()), Some(b'#' | b'&'))
        })
}

fn is_usb_vendor_collection(bus: BusType, page: u16) -> bool {
    matches!(bus, BusType::Usb) && page >= 0xff00
}

fn keypad_name(text: &str) -> bool {
    let text = text.to_ascii_lowercase().replace(['-', '_'], " ");
    [
        "macro pad",
        "macropad",
        "macro keypad",
        "macro keyboard",
        "mini keyboard",
        "mini keypad",
        "mini typewriter",
    ]
    .iter()
    .any(|term| text.contains(term))
}

fn suggested_layout(id: (u16, u16), text: &str, page: u16, usage: u16) -> Option<String> {
    if !FAMILY_VIDS.contains(&id.0) || (page, usage) != (0xff00, 1) {
        return None;
    }
    // For a different VID, require the matching product ID, collection shape,
    // and USB text. A VID alone says nothing about the physical control count.
    if id.0 != FAMILY_VIDS[0] && !keypad_name(text) {
        return None;
    }
    matching_layouts(stock_layouts(), FAMILY_VIDS[0], id.1)
        .next()
        .map(|layout| layout.name.clone())
}

/// Enumerate USB vendor-defined collections without opening the command channel.
pub fn usb_candidates() -> Result<Vec<UsbCandidate>, String> {
    with_hid(true, candidates).map_err(|err| format!("Falha ao consultar USB HID. {err}"))
}

fn candidates(api: &HidApi) -> Vec<UsbCandidate> {
    let overrides = custom_file("config.txt");
    let config: Vec<_> = configured_devices(&overrides).collect();
    let layouts = parse_layout_file(&custom_file("layouts.txt"));
    let mut found = Vec::new();
    let has_fd_endpoint = api.device_list().any(fd_interface);
    for info in api.device_list() {
        let (usage_page, usage) = (info.usage_page(), info.usage());
        if !is_usb_vendor_collection(info.bus_type(), usage_page) {
            continue;
        }
        let id = (info.vendor_id(), info.product_id());
        let protocol = configured_protocol(info, &config);
        let supported = protocol.is_some()
            && (cfg!(windows) || protocol != Some(Protocol::Fd))
            && !(has_fd_endpoint && id == KNOWN_FD_DEVICE && protocol != Some(Protocol::Fd));
        let manufacturer = info.manufacturer_string().unwrap_or_default().trim();
        let product = info.product_string().unwrap_or_default().trim();
        let text = format!("{manufacturer} {product}");
        let likely_layout = if protocol == Some(Protocol::Fd) {
            Some(fd_layout().name)
        } else if supported {
            matching_layouts(stock_layouts(), id.0, id.1)
                .next()
                .or_else(|| matching_layouts(&layouts, id.0, id.1).next())
                .map(|layout| layout.name.clone())
        } else {
            suggested_layout(id, &text, usage_page, usage)
        };
        let interface = match info.interface_number() {
            n if n >= 0 => format!("MI_{n:02X}"),
            _ => "Interface sem número USB".into(),
        };
        let model = if supported && id == KNOWN_LEGACY_DEVICE {
            likely_layout.clone().unwrap_or_default()
        } else if !product.is_empty()
            && !manufacturer.is_empty()
            && !product.starts_with(manufacturer)
        {
            format!("{manufacturer} {product}")
        } else if !product.is_empty() {
            product.into()
        } else if !manufacturer.is_empty() {
            manufacturer.into()
        } else {
            "HID USB sem nome".into()
        };
        let score = u16::from(supported) * 100
            + u16::from(FAMILY_VIDS.contains(&id.0)) * 30
            + u16::from(keypad_name(&text)) * 20
            + u16::from(likely_layout.is_some()) * 10
            + u16::from((usage_page, usage) == (0xff00, 1)) * 4;
        found.push((
            score,
            UsbCandidate {
                vid: id.0,
                pid: id.1,
                model,
                interface,
                usage_page,
                usage,
                likely_layout,
                supported,
            },
        ));
    }
    found.sort_by_key(|(score, item)| (std::cmp::Reverse(*score), item.vid, item.pid));
    found.into_iter().map(|(_, candidate)| candidate).collect()
}

pub fn discover() -> Result<Vec<Device>, String> {
    with_hid(true, discover_devices).map_err(|err| format!("Falha ao consultar USB HID. {err}"))
}

fn discover_devices(api: &HidApi) -> Vec<Device> {
    let overrides = custom_file("config.txt");
    let config: Vec<_> = configured_devices(&overrides).collect();
    let layouts = parse_layout_file(&custom_file("layouts.txt"));
    let mut devices = Vec::new();
    let has_fd_endpoint = api.device_list().any(fd_interface);
    for info in api.device_list() {
        let (vid, pid) = (info.vendor_id(), info.product_id());
        let Some(protocol) = configured_protocol(info, &config) else {
            continue;
        };
        if protocol == Protocol::Fd && !cfg!(windows) {
            continue;
        }
        // A unit with the FD command endpoint exposes normal input under MI_01.
        // Don't list a legacy configuration channel for a matching product in
        // the same session when its FD endpoint has been identified.
        if (vid, pid) == KNOWN_FD_DEVICE && has_fd_endpoint && protocol != Protocol::Fd {
            continue;
        }
        let options: Arc<[Layout]> = if protocol == Protocol::Fd {
            vec![fd_layout()].into()
        } else {
            layouts_with_custom(vid, pid, &layouts).into()
        };
        let profile = options[0].clone();
        let product = info.product_string().filter(|name| !name.trim().is_empty());
        let model = if (vid, pid) == KNOWN_LEGACY_DEVICE {
            profile.name.clone()
        } else {
            product.unwrap_or(&profile.name).to_owned()
        };
        let device = Device {
            path: info.path().to_owned(),
            vid,
            pid,
            report_len: REPORT_LEN,
            model,
            profile,
            protocol,
            layouts: options,
        };
        devices.push(((info.usage_page() < 0xff00, vid, pid), device));
    }
    // Prefer manufacturer configuration collections over standard keyboard or
    // media collections on variants whose MI also exposes multiple collections.
    devices.sort_by_key(|(key, _)| *key);
    #[cfg(not(windows))]
    {
        let mut seen = std::collections::HashSet::new();
        devices.retain(|(_, device)| seen.insert(device.path.clone()));
    }
    devices.into_iter().map(|(_, device)| device).collect()
}

fn with_hid<T>(refresh: bool, operation: impl FnOnce(&HidApi) -> T) -> Result<T, String> {
    let mut state = HID_API.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(api) = state.as_mut() {
        if refresh {
            api.refresh_devices().map_err(|err| err.to_string())?;
        }
        Ok(operation(api))
    } else {
        let api = HidApi::new().map_err(|err| err.to_string())?;
        let result = operation(&api);
        *state = Some(api);
        Ok(result)
    }
}

fn open(device: &Device) -> Result<HidDevice, String> {
    with_hid(false, |api| api.open_path(&device.path))?
        .map_err(|err| format!("Não foi possível abrir o canal USB HID. {err}"))
}

fn write_report(handle: &HidDevice, report: &[u8; REPORT_LEN]) -> Result<(), String> {
    let written = handle.write(report).map_err(|err| err.to_string())?;
    if written != report.len() {
        return Err(format!("{written} de {} bytes enviados.", report.len()));
    }
    Ok(())
}

pub fn probe(device: &Device) -> Result<u8, String> {
    probe_report(device, None)
}

pub fn report_ids(device: &Device) -> &'static [u8] {
    match device.protocol {
        Protocol::Fd => &[3],
        Protocol::WebHub => &[0],
        Protocol::Legacy if (device.vid, device.pid) == KNOWN_LEGACY_DEVICE => &[3],
        _ => &[0, 2, 3],
    }
}

pub fn probe_report(device: &Device, requested: Option<u8>) -> Result<u8, String> {
    let allowed = report_ids(device);
    if requested.is_some_and(|id| !allowed.contains(&id)) {
        return Err("Relatório HID incompatível.".into());
    }
    if device.protocol == Protocol::Fd {
        return fd::probe(device);
    }
    if supports_rgb(device) {
        return with_webhub(device, |handle| {
            webhub::transfer(handle, &webhub::info_request())?;
            Ok(0)
        });
    }
    let handle = open(device)?;
    // Upstream uses all-zero command payloads for report-ID tests.
    let ids = requested.as_ref().map_or(allowed, std::slice::from_ref);
    for &id in ids {
        if write_report(&handle, &packet(id, &[])).is_ok() {
            return Ok(id);
        }
    }
    Err("A interface não aceitou os relatórios de teste USB.".into())
}

pub fn send(device: &Device, reports: &[[u8; REPORT_LEN]]) -> Result<(), String> {
    if reports.is_empty() {
        return Err("Nenhum relatório para enviar.".into());
    }
    if device.protocol == Protocol::Fd {
        return fd::send(device, reports);
    }
    if supports_rgb(device) {
        return with_webhub(device, |handle| {
            for report in reports {
                if !matches!(report[2], webhub::WRITE_KEY | webhub::WRITE_LIGHT) {
                    return Err("Quadro de gravação incompatível com WebHub.".into());
                }
            }
            for report in reports {
                webhub::transfer(handle, report)?;
            }
            Ok(())
        });
    }
    let handle = open(device)?;
    for (index, report) in reports.iter().enumerate() {
        write_report(&handle, report).map_err(|err| {
            format!(
                "Falha no relatório {} de {}. {err}",
                index + 1,
                reports.len()
            )
        })?;
    }
    Ok(())
}

pub fn supports_rgb(device: &Device) -> bool {
    device.protocol == Protocol::WebHub && (device.vid, device.pid) == KNOWN_WEBHUB_DEVICE
}

fn with_webhub<T>(
    device: &Device,
    operation: impl FnOnce(&HidDevice) -> Result<T, String>,
) -> Result<T, String> {
    if !supports_rgb(device) {
        return Err("Aparelho sem leitura de configuração WebHub confirmada.".into());
    }
    let _lock = WEBHUB_CHANNEL
        .lock()
        .map_err(|_| "Canal WebHub indisponível.")?;
    operation(&open(device)?)
}

pub fn read_lighting(device: &Device) -> Result<RgbLighting, String> {
    with_webhub(device, webhub::read_light)
}

pub fn apply_lighting(device: &Device, light: RgbLighting) -> Result<RgbLighting, String> {
    with_webhub(device, |handle| webhub::apply_light(handle, light))
}

pub fn read_bindings(device: &Device, layer: u8) -> Result<Vec<(u8, Option<Binding>)>, String> {
    validate_report(device, 0, layer)?;
    with_webhub(device, |handle| {
        let slots = device.profile.controls.iter().flat_map(Control::slots);
        webhub::read_keys(handle, layer, slots)
    })
}

fn packet(report_id: u8, data: &[u8]) -> [u8; REPORT_LEN] {
    let mut report = [0; REPORT_LEN];
    report[0] = report_id;
    report[1..data.len() + 1].copy_from_slice(data);
    report
}

fn extended(
    report_id: u8,
    slot: u8,
    layer: u8,
    kind: u8,
    delay: u16,
    data: &[u8],
) -> [u8; REPORT_LEN] {
    let mut report = packet(report_id, &[254, slot, layer, kind]);
    report[5..7].copy_from_slice(&delay.to_le_bytes());
    report[10] = data
        .iter()
        .rposition(|&byte| byte != 0)
        .map_or(0, |index| (index / 2 + 1) as u8);
    report[11..11 + data.len()].copy_from_slice(data);
    report
}

fn media_data(code: u16, report_id: u8) -> Result<[u8; 2], String> {
    let (legacy, alternate, standard) = match code {
        0xcd => ([64, 0], [0, 4], [205, 0]),
        0xb5 => ([0, 1], [0, 10], [181, 0]),
        0xb6 => ([128, 0], [0, 11], [182, 0]),
        0xe2 => ([4, 0], [0, 1], [226, 0]),
        0xe9 => ([2, 0], [64, 0], [233, 0]),
        0xea => ([1, 0], [128, 0], [234, 0]),
        _ => return Err("Função de mídia não reconhecida pelo protocolo RSoft.".into()),
    };
    Ok(match report_id {
        0 => legacy,
        2 => alternate,
        _ => standard,
    })
}

fn validate_report(device: &Device, id: u8, layer: u8) -> Result<(), String> {
    if device.report_len != REPORT_LEN || !matches!(id, 0 | 2 | 3) {
        return Err("Relatório HID incompatível.".into());
    }
    if !(1..=device.profile.layers).contains(&layer) || layer > 15 {
        return Err("Camada fora do intervalo do aparelho.".into());
    }
    if device.protocol == Protocol::WebHub && (!supports_rgb(device) || id != 0 || layer != 1) {
        return Err("O perfil WebHub exige relatório 0 e uma camada.".into());
    }
    if device.protocol == Protocol::Fd && (id != 3 || layer != 1) {
        return Err("O perfil FD exige relatório 3 e uma camada.".into());
    }
    if device.protocol == Protocol::Legacy && id == 0 && layer != 1 {
        return Err("O relatório Legacy 0 tem uma única camada.".into());
    }
    Ok(())
}

pub fn wheel_steps(device: &Device, slot: u8) -> i8 {
    if supports_rgb(device) {
        return 10;
    }
    if (device.vid, device.pid) == KNOWN_LEGACY_DEVICE
        && device.protocol == Protocol::Legacy
        && matches!(slot, 13 | 15)
    {
        10
    } else {
        1
    }
}

pub fn binding_reports(
    device: &Device,
    report_id: u8,
    slot: u8,
    layer: u8,
    delay: u16,
    binding: &Binding,
) -> Result<Vec<[u8; REPORT_LEN]>, String> {
    validate_report(device, report_id, layer)?;
    if !device
        .profile
        .controls
        .iter()
        .any(|control| control.contains_slot(slot))
    {
        return Err("Controle fora do layout selecionado.".into());
    }
    if supports_rgb(device) {
        if delay != 0 {
            return Err("WebHub não oferece atraso salvo no aparelho.".into());
        }
        return Ok(vec![webhub::key_packet(slot, layer, binding)?]);
    }
    if delay != 0 && (!device.profile.supports_delay || device.protocol == Protocol::Legacy) {
        return Err("Este aparelho não oferece atraso gravado para essa ação.".into());
    }
    if let Binding::Keyboard(keys) = binding {
        if keys.len() > usize::from(device.profile.max_sequence.min(27)) {
            return Err(format!(
                "O perfil aceita até {} passos.",
                device.profile.max_sequence
            ));
        }
    }
    if device.protocol == Protocol::Fd {
        return fd::reports(slot, layer, binding);
    }
    if let Binding::Mouse {
        buttons, scroll, ..
    } = binding
    {
        let limit = if report_id == 3 {
            wheel_steps(device, slot)
        } else {
            1
        };
        if buttons & !7 != 0 || !(-limit..=limit).contains(scroll) {
            return Err("Ação de mouse fora do intervalo permitido.".into());
        }
    }

    if device.protocol == Protocol::Extended {
        let report = match binding {
            Binding::Keyboard(keys) => {
                let data: Vec<_> = keys
                    .iter()
                    .flat_map(|&(modifiers, key)| [modifiers, key])
                    .collect();
                extended(report_id, slot, layer, 1, delay, &data)
            }
            Binding::Media(code) => {
                let [b1, b2] = media_data(*code, report_id)?;
                extended(report_id, slot, layer, 2, 0, &[0, b1, b2, 0])
            }
            Binding::Mouse {
                buttons,
                scroll,
                modifiers,
            } => {
                // Extended mouse is type 2 in upstream. Legacy uses type 3.
                extended(
                    report_id,
                    slot,
                    layer,
                    2,
                    0,
                    &[*buttons, 0, 0, *scroll as u8, *modifiers, 0],
                )
            }
        };
        return Ok(vec![report]);
    }

    let mut reports = Vec::new();
    let layer_bits = if report_id == 0 {
        0
    } else {
        reports.push(packet(report_id, &[161, layer]));
        layer << 4
    };
    match binding {
        Binding::Keyboard(keys) => {
            let keys = if keys.is_empty() {
                &[(0, 0)][..]
            } else {
                keys.as_slice()
            };
            let count = keys.len() as u8;
            reports.push(packet(
                report_id,
                &[slot, layer_bits | 1, count, 0, keys[0].0, 0],
            ));
            for (index, &(modifiers, key)) in keys.iter().enumerate() {
                reports.push(packet(
                    report_id,
                    &[
                        slot,
                        layer_bits | 1,
                        count,
                        (index + 1) as u8,
                        modifiers,
                        key,
                    ],
                ));
            }
        }
        Binding::Media(code) => {
            let [b1, b2] = media_data(*code, report_id)?;
            reports.push(packet(report_id, &[slot, layer_bits | 2, b1, b2]));
        }
        Binding::Mouse {
            buttons,
            scroll,
            modifiers,
        } => {
            reports.push(packet(
                report_id,
                &[
                    slot,
                    layer_bits | 3,
                    *buttons,
                    0,
                    0,
                    *scroll as u8,
                    *modifiers,
                ],
            ));
        }
    }
    reports.push(packet(report_id, &[170, 170]));
    Ok(reports)
}

pub fn led_reports(
    device: &Device,
    report_id: u8,
    layer: u8,
    mode: u8,
    color: u8,
) -> Result<Vec<[u8; REPORT_LEN]>, String> {
    validate_report(device, report_id, layer)?;
    if mode >= device.profile.led_modes
        || (device.protocol == Protocol::Legacy && report_id == 0 && mode > 2)
    {
        return Err("Modo de luz indisponível neste modelo.".into());
    }
    if (color != 0 && !device.profile.supports_led_color) || color > 0x70 || color & 0x0f != 0 {
        return Err("Cor de luz indisponível neste modelo.".into());
    }
    if supports_rgb(device) {
        let hues = [255, 0, 21, 43, 85, 128, 170, 213];
        return Ok(vec![webhub::light_packet(RgbLighting {
            mode,
            palette: color == 0,
            hue: hues[usize::from(color >> 4)],
            ..RgbLighting::default()
        })?]);
    }
    if device.protocol == Protocol::Extended {
        // RSoft's mapper emits zero for the LED pseudo-action in extended packets.
        return Ok(vec![extended(
            report_id,
            0,
            layer,
            8,
            0,
            &[layer, mode | color, 0, 0, 0, 0],
        )]);
    }
    Ok(vec![
        packet(report_id, &[176, 8, mode | color]),
        packet(report_id, &[170, 161]),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_identification_accepts_only_vendor_defined_usb_collections() {
        assert!(is_usb_vendor_collection(BusType::Usb, 0xff00));
        assert!(is_usb_vendor_collection(BusType::Usb, 0xffff));
        assert!(!is_usb_vendor_collection(BusType::Usb, 0x01));
        assert!(!is_usb_vendor_collection(BusType::Bluetooth, 0xff00));
    }

    #[test]
    fn interface_match_is_case_insensitive_and_requires_full_boundaries() {
        let path = b"VID_1189&PID_8890&MI_01&COL01#8&FF";
        assert!(path_matches_interface(path, b"mi_01"));
        assert!(path_matches_interface(path, b"mi_01&col01"));
        assert!(!path_matches_interface(path, b"mi_0"));
        assert!(!path_matches_interface(path, b"mi_02"));
        assert!(!path_matches_interface(path, b"col"));
    }

    #[test]
    fn native_usb_interface_numbers_match_only_complete_selectors() {
        assert!(number_matches_interface(1, b"mi_01"));
        assert!(number_matches_interface(10, b"MI_0a"));
        assert!(!number_matches_interface(0, b"mi_01"));
        assert!(!number_matches_interface(-1, b"mi_ff"));
        assert!(!number_matches_interface(256, b"mi_00"));
        assert!(!number_matches_interface(1, b"mi_01&col01"));
        assert!(!number_matches_interface(1, b"mi_+1"));
        assert!(!number_matches_interface(1, b"mi_001"));
    }

    #[test]
    fn fd_protocol_is_limited_to_its_confirmed_usb_collection() {
        let correct = path_matches_interface(b"hid#vid_1189&pid_8840&mi_00#abc", b"mi_00");
        let input = path_matches_interface(b"hid#vid_1189&pid_8840&mi_01&col04#abc", b"mi_00");
        assert!(is_fd_collection(
            KNOWN_FD_DEVICE,
            BusType::Usb,
            0xff00,
            correct
        ));
        assert!(!is_fd_collection(
            KNOWN_FD_DEVICE,
            BusType::Usb,
            0xff00,
            input
        ));
        assert!(!is_fd_collection(
            KNOWN_LEGACY_DEVICE,
            BusType::Usb,
            0xff00,
            correct
        ));
        assert!(!is_fd_collection(KNOWN_FD_DEVICE, BusType::Usb, 1, correct));
        assert!(is_fd_collection(
            KNOWN_FD_DEVICE,
            BusType::Usb,
            0xff00,
            number_matches_interface(0, b"mi_00")
        ));
        assert_eq!(fd_layout().controls.len(), 7);
    }

    #[test]
    fn unknown_candidate_geometry_requires_specific_evidence() {
        assert_eq!(
            suggested_layout((0x514c, 0x8890), "mini keyboard", 0xff00, 1).as_deref(),
            Some("3 teclas e 1 botão giratório")
        );
        assert!(suggested_layout((0x1189, 0x99ff), "usb keyboard", 0xff00, 1).is_none());
        assert!(suggested_layout((0x046d, 0x8890), "receiver", 0xff00, 1).is_none());
        assert!(suggested_layout((0x514c, 0x8890), "mini keyboard", 0xff43, 0x100).is_none());
    }

    #[test]
    #[ignore = "requer o Mini Teclado USB 1189:8890 conectado ao Windows"]
    fn live_usb_candidates_include_registered_configuration_interface() {
        let candidates = usb_candidates().unwrap();
        let candidate = candidates
            .iter()
            .find(|candidate| {
                (candidate.vid, candidate.pid) == KNOWN_LEGACY_DEVICE && candidate.supported
            })
            .expect("1189:8890 precisa aparecer no inventário USB");
        assert_eq!((candidate.usage_page, candidate.usage), (0xff00, 1));
        assert_eq!(candidate.interface, "MI_01");
        assert!(!candidate.model.is_empty());
        assert_eq!(
            candidate.likely_layout.as_deref(),
            Some("3 teclas e 1 botão giratório")
        );
    }

    fn fixture(protocol: Protocol, model: Layout) -> Device {
        Device {
            path: CString::new("test").unwrap(),
            vid: 4489,
            pid: 34960,
            report_len: 65,
            model: model.name.clone(),
            layouts: Arc::from([model.clone()]),
            profile: model,
            protocol,
        }
    }

    #[test]
    fn user_hardware_layout_matches_rsoft() {
        let device = fixture(Protocol::Legacy, layouts(4489, 34960).remove(0));
        assert_eq!(device.profile.name, default_layout().name);
        assert_eq!(device.profile.layers, 1);
        assert_eq!(device.profile.max_sequence, 5);
        assert_eq!(device.profile.led_modes, 3);
        assert!(!device.profile.supports_delay);
        assert_eq!(device.profile.controls.len(), 4);
        assert_eq!(device.profile.controls[3].slots(), vec![13, 14, 15]);
    }

    #[test]
    fn confirmed_rotary_wheel_range_uses_signed_legacy_byte() {
        let device = fixture(Protocol::Legacy, default_layout());
        for (slot, scroll) in [(13, -10), (15, 10)] {
            let reports = binding_reports(
                &device,
                3,
                slot,
                1,
                0,
                &Binding::Mouse {
                    buttons: 0,
                    scroll,
                    modifiers: 0,
                },
            )
            .unwrap();
            assert_eq!(&reports[1][..8], &[3, slot, 0x13, 0, 0, 0, scroll as u8, 0]);
        }
        for (slot, scroll) in [(1, 10), (15, 11), (13, -11)] {
            assert!(binding_reports(
                &device,
                3,
                slot,
                1,
                0,
                &Binding::Mouse {
                    buttons: 0,
                    scroll,
                    modifiers: 0
                }
            )
            .is_err());
        }
    }

    #[test]
    fn legacy_shortcut_matches_reference_bytes() {
        let device = fixture(Protocol::Legacy, layouts(4489, 34960).remove(0));
        let packets = binding_reports(
            &device,
            3,
            1,
            1,
            0,
            &Binding::Keyboard(vec![(1, 4), (0, 40)]),
        )
        .unwrap();
        let expected = [
            packet(3, &[161, 1]),
            packet(3, &[1, 17, 2, 0, 1, 0]),
            packet(3, &[1, 17, 2, 1, 1, 4]),
            packet(3, &[1, 17, 2, 2, 0, 40]),
            packet(3, &[170, 170]),
        ];
        assert_eq!(packets, expected);
        assert!(binding_reports(&device, 3, 4, 1, 0, &Binding::Media(0xe9)).is_err());
        assert_eq!(
            led_reports(&device, 3, 1, 2, 0).unwrap(),
            [packet(3, &[176, 8, 2]), packet(3, &[170, 161])]
        );
    }

    #[test]
    fn extended_shortcut_matches_reference_bytes() {
        let mut device = fixture(Protocol::Extended, layouts(4489, 34866).remove(0));
        device.pid = 34866;
        let packets = binding_reports(
            &device,
            3,
            14,
            2,
            300,
            &Binding::Keyboard(vec![(16, 58), (0, 40)]),
        )
        .unwrap();
        assert_eq!(
            packets,
            [packet(
                3,
                &[254, 14, 2, 1, 44, 1, 0, 0, 0, 2, 16, 58, 0, 40]
            )]
        );
        assert_eq!(
            led_reports(&device, 3, 2, 4, 0x30).unwrap(),
            [packet(3, &[254, 0, 2, 8, 0, 0, 0, 0, 0, 1, 2, 0x34])]
        );
    }

    #[test]
    fn legacy_media_report_id_mappings() {
        let device = fixture(Protocol::Legacy, layouts(4489, 34960).remove(0));
        assert_eq!(
            binding_reports(&device, 0, 1, 1, 0, &Binding::Media(0xe9)).unwrap()[0],
            packet(0, &[1, 2, 2, 0])
        );
        assert_eq!(
            binding_reports(&device, 2, 1, 1, 0, &Binding::Media(0xea)).unwrap()[1],
            packet(2, &[1, 18, 128, 0])
        );
    }

    #[test]
    fn manual_usb_report_respects_fixed_protocol_ids_before_opening_hardware() {
        let mut pad = fixture(Protocol::Legacy, default_layout());
        (pad.vid, pad.pid) = KNOWN_LEGACY_DEVICE;
        assert_eq!(report_ids(&pad), &[3]);
        assert_eq!(
            probe_report(&pad, Some(0)),
            Err("Relatório HID incompatível.".into())
        );
        pad.protocol = Protocol::WebHub;
        assert_eq!(report_ids(&pad), &[0]);
        assert!(probe_report(&pad, Some(3)).is_err());
        pad.protocol = Protocol::Extended;
        assert_eq!(report_ids(&pad), &[0, 2, 3]);
    }

    #[test]
    fn upstream_vendor_typo_is_resolved_for_supported_devices() {
        assert_eq!(layouts(4489, 34865).len(), 2);
        assert_eq!(layouts(4498, 34865).len(), 2);
        assert_eq!(layouts(4489, 34866).len(), 2);
        assert_eq!(layouts(4489, 34832)[0].controls.len(), 15);
    }

    #[test]
    fn custom_config_can_add_a_device_without_removing_stock() {
        let cfg = "\u{feff}9999:1234,mi_02,1\n9999:1235,*,1\n4489:34960,mi_01,9";
        let devices = configured_devices(cfg).collect::<Vec<_>>();
        assert!(devices.contains(&((9999, 1234), "mi_02", Protocol::Extended)));
        assert!(!devices.iter().any(|(id, _, _)| *id == (9999, 1235)));
        assert!(devices.contains(&((4489, 34960), "mi_01", Protocol::Legacy)));
    }

    #[test]
    fn visual_overrides_are_alternatives_and_do_not_expand_known_legacy() {
        let text = "Layout: Invertido\n4489:34960\n1:5:0:0:3\nK1,5,5\nB3,25,5\nB2,45,5\nB1,65,5\n\nLayout: Inválido\n4489:34960\n3:18:1:1:6\nB12,10,10\n";
        let available = layouts_with_custom(4489, 34960, &parse_layout_file(text));
        assert_eq!(available[0].name, "3 teclas e 1 botão giratório");
        let inverted = available
            .iter()
            .find(|layout| layout.name == "Invertido")
            .unwrap();
        assert_eq!(inverted.controls[0].kind, ControlKind::Knob);
        assert!(!available.iter().any(|layout| layout.name == "Inválido"));
    }
}
