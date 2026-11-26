//! Linux Input subsystem adapter. Capture observes the selected USB device
//! without grabbing it, and replay uses a separately identified virtual device.

use crate::{
    device::Binding,
    platform::{DirectInput, InputEvent},
};
use std::{
    env,
    ffi::{c_int, c_long, c_ulong},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    mem::size_of,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_MSC: u16 = 4;
const SYN_REPORT: u16 = 0;
const SYN_DROPPED: u16 = 3;
const MSC_SCAN: u16 = 4;
const REL_WHEEL: u16 = 8;
const O_NONBLOCK: c_int = 0x800;
const EVENT_BYTES: usize = size_of::<c_long>() * 2 + 8;
const SCAN_DELAY: Duration = Duration::from_millis(400);
const POLL_DELAY: Duration = Duration::from_millis(5);

const MODIFIERS: [u16; 8] = [29, 42, 56, 125, 97, 54, 100, 126];
const MOUSE_KEYS: [u16; 5] = [0x110, 0x111, 0x112, 0x113, 0x114];
const MEDIA: [(u16, u16, &str); 7] = [
    (0xb5, 163, "Próxima faixa"),
    (0xb6, 165, "Faixa anterior"),
    (0xb7, 166, "Parar mídia"),
    (0xcd, 164, "Reproduzir / pausar"),
    (0xe2, 113, "Mudo"),
    (0xe9, 115, "Aumentar volume"),
    (0xea, 114, "Diminuir volume"),
];

#[derive(Clone, Debug, Eq, PartialEq)]
struct Node {
    path: PathBuf,
    usb_device: PathBuf,
}

struct InputFile {
    file: File,
    scan: Option<u32>,
    sync_lost: bool,
}

fn read_hex(path: &Path) -> Option<u16> {
    u16::from_str_radix(fs::read_to_string(path).ok()?.trim(), 16).ok()
}

fn matching_nodes(vid: u16, pid: u16) -> io::Result<Vec<Node>> {
    let mut nodes = Vec::new();
    let entries = match fs::read_dir("/sys/class/input") {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(nodes),
        Err(error) => return Err(error),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(number) = name.to_str().and_then(|name| name.strip_prefix("event")) else {
            continue;
        };
        if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let device = entry.path().join("device");
        let id = device.join("id");
        if read_hex(&id.join("bustype")) != Some(3)
            || read_hex(&id.join("vendor")) != Some(vid)
            || read_hex(&id.join("product")) != Some(pid)
        {
            continue;
        }
        if let Ok(real) = fs::canonicalize(&device) {
            if let Some(usb_device) = real.ancestors().find(|parent| {
                read_hex(&parent.join("idVendor")) == Some(vid)
                    && read_hex(&parent.join("idProduct")) == Some(pid)
            }) {
                nodes.push(Node {
                    path: Path::new("/dev/input").join(&name),
                    usb_device: usb_device.to_owned(),
                });
            }
        }
    }
    nodes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(nodes)
}

fn open_nodes(nodes: &[Node]) -> Result<Vec<InputFile>, String> {
    let first = nodes
        .first()
        .ok_or("O tecladinho não tem entradas evdev disponíveis.")?;
    if nodes.iter().any(|node| node.usb_device != first.usb_device) {
        return Err("Deixe conectado um único aparelho deste modelo.".into());
    }
    nodes.iter().map(|node| {
        OpenOptions::new().read(true).custom_flags(O_NONBLOCK).open(&node.path)
            .map(|file| InputFile { file, scan: None, sync_lost: false })
            .map_err(|error| format!("Não foi possível ler {}: {error}. Verifique a permissão de entrada do tecladinho.", node.path.display()))
    }).collect()
}

pub fn start(
    vid: u16,
    pid: u16,
    events: Sender<InputEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    stop: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, String> {
    let mut known = matching_nodes(vid, pid).map_err(|error| error.to_string())?;
    let mut inputs = open_nodes(&known)?;
    thread::Builder::new()
        .name("mini-teclado-evdev".into())
        .spawn(move || {
            let mut state = Keys::default();
            let mut poll_at = Instant::now() + SCAN_DELAY;
            let emit = |event| -> bool {
                let sent = events.send(event).is_ok();
                if sent {
                    wake();
                }
                sent
            };
            while !stop.load(Ordering::Acquire) {
                if Instant::now() >= poll_at {
                    let next = matching_nodes(vid, pid).unwrap_or_default();
                    if next != known {
                        state = Keys::default();
                        inputs = open_nodes(&next).unwrap_or_default();
                        known = next;
                        if !emit(InputEvent::Disconnected) {
                            break;
                        }
                    } else if inputs.is_empty() {
                        inputs = open_nodes(&known).unwrap_or_default();
                    }
                    poll_at = Instant::now() + SCAN_DELAY;
                }
                if inputs
                    .iter_mut()
                    .any(|input| !drain(input, &mut state, &emit))
                {
                    state = Keys::default();
                    inputs.clear();
                    if !emit(InputEvent::Disconnected) {
                        break;
                    }
                }
                thread::sleep(POLL_DELAY);
            }
        })
        .map_err(|error| format!("Não foi possível iniciar a escuta: {error}"))
}

fn drain(input: &mut InputFile, keys: &mut Keys, emit: &impl Fn(InputEvent) -> bool) -> bool {
    let mut bytes = [0u8; 32 * EVENT_BYTES];
    // A burst from a single collection cannot delay reading the other ones.
    for _ in 0..4 {
        match input.file.read(&mut bytes) {
            Ok(0) => return false,
            Ok(read) => {
                if read % EVENT_BYTES != 0 {
                    return false;
                }
                for raw in bytes[..read].as_chunks::<EVENT_BYTES>().0 {
                    let tail = &raw[EVENT_BYTES - 8..];
                    let kind = u16::from_ne_bytes([tail[0], tail[1]]);
                    let code = u16::from_ne_bytes([tail[2], tail[3]]);
                    let value = i32::from_ne_bytes([tail[4], tail[5], tail[6], tail[7]]);
                    let event = match (kind, code) {
                        (EV_SYN, SYN_DROPPED) => {
                            input.scan = None;
                            input.sync_lost = true;
                            *keys = Keys::default();
                            Some(InputEvent::Disconnected)
                        }
                        (EV_SYN, SYN_REPORT) => {
                            input.sync_lost = false;
                            input.scan = None;
                            None
                        }
                        _ if input.sync_lost => None,
                        (EV_MSC, MSC_SCAN) => {
                            input.scan = Some(value as u32);
                            None
                        }
                        (EV_KEY, _) => keys.key(code, value, input.scan.take(), Instant::now()),
                        (EV_REL, REL_WHEEL) if value != 0 => {
                            let delta = value
                                .saturating_mul(120)
                                .clamp(i16::MIN as i32, i16::MAX as i32)
                                as i16;
                            Some(InputEvent::Observed(
                                DirectInput::Mouse {
                                    buttons: 0,
                                    delta,
                                    modifiers: keys.modifiers,
                                },
                                if value > 0 {
                                    "Rolagem para cima"
                                } else {
                                    "Rolagem para baixo"
                                }
                                .into(),
                            ))
                        }
                        _ => None,
                    };
                    if event.is_some_and(|event| !emit(event)) {
                        return false;
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        }
    }
    true
}

#[derive(Default)]
struct Keys {
    modifiers: u8,
}

impl Keys {
    fn key(&mut self, code: u16, value: i32, scan: Option<u32>, at: Instant) -> Option<InputEvent> {
        if !(0..=1).contains(&value) {
            return None;
        }
        let down = value == 1;
        if (183..=186).contains(&code) {
            return Some(InputEvent::Key {
                index: (code - 183) as u8,
                down,
                at,
            });
        }
        if (187..=192).contains(&code) {
            return Some(InputEvent::VolumeKey {
                index: (code - 187) as u8,
                down,
            });
        }
        if let Some(index) = MODIFIERS.iter().position(|&key| key == code) {
            let bit = 1 << index;
            if down {
                self.modifiers |= bit;
            } else {
                self.modifiers &= !bit;
            }
        }
        if !down {
            return None;
        }
        if let Some(&(usage, _, label)) = MEDIA.iter().find(|&&(_, key, _)| key == code) {
            return Some(InputEvent::Observed(
                DirectInput::Media(usage),
                label.into(),
            ));
        }
        if let Some(scan) = scan.filter(|scan| scan >> 16 == 0x0c) {
            let usage = scan as u16;
            return Some(InputEvent::Observed(
                DirectInput::Media(usage),
                format!("Mídia HID {usage:04X}"),
            ));
        }
        if let Some(index) = MOUSE_KEYS.iter().position(|&key| key == code) {
            let label = [
                "Clique esquerdo",
                "Clique direito",
                "Clique central",
                "Botão lateral 1",
                "Botão lateral 2",
            ][index];
            return Some(InputEvent::Observed(
                DirectInput::Mouse {
                    buttons: 1 << index,
                    delta: 0,
                    modifiers: self.modifiers,
                },
                label.into(),
            ));
        }
        let usage = scan
            .filter(|scan| scan >> 16 == 7)
            .and_then(|scan| u8::try_from(scan & 0xffff).ok())
            .or_else(|| linux_hid_key(code))?;
        let input = DirectInput::keyboard_usage(usage, self.modifiers)?;
        Some(InputEvent::Observed(input, key_label(usage)))
    }
}

fn key_label(usage: u8) -> String {
    match usage {
        0x04..=0x1d => char::from(b'A' + usage - 4).to_string(),
        0x1e..=0x27 => ((usage - 0x1d) % 10).to_string(),
        0x3a..=0x45 => format!("F{}", usage - 0x39),
        0x68..=0x73 => format!("F{}", usage - 0x68 + 13),
        _ => format!("Tecla HID {usage:02X}"),
    }
}

// Linux's KEY_* codes corresponding to standard Keyboard/Keypad HID usages.
// The first 0x28–0x65 entries include the ABNT non-US and numeric keypad keys.
const LETTERS: [u16; 26] = [
    30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45,
    21, 44,
];
const STANDARD: [u16; 62] = [
    28, 1, 14, 15, 57, 12, 13, 26, 27, 43, 43, 39, 40, 41, 51, 52, 53, 58, 59, 60, 61, 62, 63, 64,
    65, 66, 67, 68, 87, 88, 99, 70, 119, 110, 102, 104, 111, 107, 109, 106, 105, 108, 103, 69, 98,
    55, 74, 78, 96, 79, 80, 81, 75, 76, 77, 71, 72, 73, 82, 83, 86, 127,
];

fn hid_key(usage: u8) -> Option<u16> {
    Some(match usage {
        0x04..=0x1d => LETTERS[(usage - 4) as usize],
        0x1e..=0x27 => (usage - 0x1c).into(),
        0x28..=0x65 => STANDARD[(usage - 0x28) as usize],
        0x67 => 117,
        0x68..=0x73 => 183 + u16::from(usage - 0x68),
        0x85 => 121,
        0x87 => 89,
        0x89 => 124,
        0xe0..=0xe7 => MODIFIERS[(usage - 0xe0) as usize],
        _ => return None,
    })
}

fn linux_hid_key(code: u16) -> Option<u8> {
    (0x04..=0xe7).find(|&usage| hid_key(usage) == Some(code))
}

struct VirtualInput(File);

impl VirtualInput {
    fn create() -> Result<Self, String> {
        // Asm-generic ioctl layout, including Linux builds targeted by this app.
        if !cfg!(any(
            target_arch = "x86",
            target_arch = "x86_64",
            target_arch = "aarch64"
        )) {
            return Err("A reprodução uinput não está habilitada nesta arquitetura Linux.".into());
        }
        let mut open = OpenOptions::new();
        open.write(true).custom_flags(O_NONBLOCK);
        let file = open.open("/dev/uinput").or_else(|error| {
            if error.kind() == io::ErrorKind::NotFound { open.open("/dev/input/uinput") } else { Err(error) }
        }).map_err(|error| format!(
            "Reprodução Linux indisponível: {error}. Verifique se /dev/uinput existe e se o usuário pode gravar nele."
        ))?;
        let fd = file.as_raw_fd();
        let configure = || -> io::Result<()> {
            set_uinput_bit(fd, 100, u32::from(EV_KEY))?;
            set_uinput_bit(fd, 100, u32::from(EV_REL))?;
            let codes = (0x04..=0xe7)
                .filter_map(hid_key)
                .chain(MEDIA.iter().map(|&(_, key, _)| key))
                .chain(MOUSE_KEYS);
            for code in codes.collect::<std::collections::BTreeSet<_>>() {
                set_uinput_bit(fd, 101, u32::from(code))?;
            }
            for axis in [0, 1, REL_WHEEL] {
                set_uinput_bit(fd, 102, u32::from(axis))?;
            }
            let mut identity = UinputSetup {
                id: [6, 0, 0, 1], // BUS_VIRTUAL, not a hardware device watched by evdev capture.
                name: [0; 80],
                effects: 0,
            };
            let name = b"Mini Teclado USB - virtual";
            identity.name[..name.len()].copy_from_slice(name);
            let setup = uinput_ioctl(3, size_of::<UinputSetup>(), true);
            let create = uinput_ioctl(1, 0, false);
            // Kernel copies the repr(C) structure synchronously.
            check_ioctl(unsafe { ioctl(fd, setup, &identity as *const UinputSetup) })?;
            check_ioctl(unsafe { ioctl(fd, create, 0) })
        };
        configure()
            .map_err(|error| format!("Não foi possível criar o dispositivo uinput: {error}"))?;
        // Give udev and the running compositor time to attach to the new input node.
        thread::sleep(Duration::from_millis(350));
        Ok(Self(file))
    }

    fn emit(&mut self, kind: u16, code: u16, value: i32) -> io::Result<()> {
        let mut bytes = [0u8; EVENT_BYTES];
        let tail = &mut bytes[EVENT_BYTES - 8..];
        tail[..2].copy_from_slice(&kind.to_ne_bytes());
        tail[2..4].copy_from_slice(&code.to_ne_bytes());
        tail[4..].copy_from_slice(&value.to_ne_bytes());
        self.0.write_all(&bytes)
    }
}

#[repr(C)]
struct UinputSetup {
    id: [u16; 4],
    name: [u8; 80],
    effects: u32,
}

unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

fn uinput_ioctl(number: u8, len: usize, write: bool) -> c_ulong {
    ((u64::from(write) << 30) | (len as u64) << 16 | (u64::from(b'U') << 8) | u64::from(number))
        as c_ulong
}

fn set_uinput_bit(fd: c_int, request: u8, code: u32) -> io::Result<()> {
    check_ioctl(unsafe { ioctl(fd, uinput_ioctl(request, size_of::<c_int>(), true), code) })
}

fn check_ioctl(result: c_int) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

type Event = (u16, u16, i32);

fn modifiers(events: &mut Vec<Event>, mask: u8, down: bool) {
    let items = MODIFIERS.iter().enumerate();
    let push = |(index, key): (usize, &u16)| {
        (mask & (1 << index) != 0).then_some((EV_KEY, *key, i32::from(down)))
    };
    if down {
        events.extend(items.filter_map(push));
    } else {
        events.extend(items.rev().filter_map(push));
    }
}

fn key_events(events: &mut Vec<Event>, code: Option<u16>, mask: u8) {
    modifiers(events, mask, true);
    if let Some(code) = code {
        events.push((EV_KEY, code, 1));
    }
    events.push((EV_SYN, SYN_REPORT, 0));
    if let Some(code) = code {
        events.push((EV_KEY, code, 0));
    }
    modifiers(events, mask, false);
    events.push((EV_SYN, SYN_REPORT, 0));
}

fn replay_events(binding: &Binding) -> Result<Vec<Event>, String> {
    let mut events = Vec::new();
    match binding {
        Binding::Keyboard(keys) => {
            for &(mask, usage) in keys {
                let code = if usage == 0 {
                    None
                } else {
                    Some(hid_key(usage).ok_or_else(|| {
                        format!("Código HID sem reprodução no Linux: {usage:#04x}.")
                    })?)
                };
                key_events(&mut events, code, mask);
            }
        }
        Binding::Media(usage) => {
            let (_, code, _) = MEDIA
                .iter()
                .find(|(known, _, _)| known == usage)
                .ok_or_else(|| format!("Ação de mídia sem reprodução no Linux: {usage:#06x}."))?;
            key_events(&mut events, Some(*code), 0);
        }
        Binding::Mouse {
            buttons,
            scroll,
            modifiers: mask,
        } => {
            if buttons & !0x1f != 0 {
                return Err("Botão de mouse sem reprodução no Linux.".into());
            }
            modifiers(&mut events, *mask, true);
            for (index, &code) in MOUSE_KEYS.iter().enumerate() {
                if buttons & (1 << index) != 0 {
                    events.push((EV_KEY, code, 1));
                }
            }
            if *scroll != 0 {
                events.push((EV_REL, REL_WHEEL, i32::from(*scroll)));
            }
            events.push((EV_SYN, SYN_REPORT, 0));
            for (index, &code) in MOUSE_KEYS.iter().enumerate().rev() {
                if buttons & (1 << index) != 0 {
                    events.push((EV_KEY, code, 0));
                }
            }
            modifiers(&mut events, *mask, false);
            events.push((EV_SYN, SYN_REPORT, 0));
        }
    }
    Ok(events)
}

pub fn replay(binding: &Binding) -> Result<(), String> {
    let events = replay_events(binding)?;
    if events.is_empty() {
        return Ok(());
    }
    static OUTPUT: Mutex<Option<VirtualInput>> = Mutex::new(None);
    let mut output = OUTPUT
        .lock()
        .map_err(|_| "O dispositivo virtual Linux está indisponível.")?;
    if output.is_none() {
        *output = Some(VirtualInput::create()?);
    }
    let result = events
        .into_iter()
        .try_for_each(|(kind, code, value)| output.as_mut().unwrap().emit(kind, code, value));
    if let Err(error) = result {
        // Closing the failed virtual device also releases any half-sent presses.
        *output = None;
        return Err(format!("O Linux não recebeu toda a ação: {error}."));
    }
    Ok(())
}

fn command(program: &str, args: &[&str]) -> Result<Output, String> {
    let mut exec = Command::new(program);
    exec.args(args).stdin(Stdio::null()).env("LC_ALL", "C");
    let output = exec
        .output()
        .map_err(|error| format!("Não foi possível executar {program}: {error}."))?;
    if output.status.success() {
        Ok(output)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.trim();
        if reason.is_empty() {
            Err(format!("{program} respondeu com {}.", output.status))
        } else {
            Err(format!("{program}: {reason}"))
        }
    }
}

pub fn toggle_display(toggle: &mut bool) -> Result<bool, String> {
    let next_off = !*toggle;
    let power = if next_off { "off" } else { "on" };
    let command_power = if next_off { "--off" } else { "--on" };
    let session = env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let wayland =
        session.eq_ignore_ascii_case("wayland") || env::var_os("WAYLAND_DISPLAY").is_some();
    if wayland {
        let desktop = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        if desktop
            .split(':')
            .any(|name| name.eq_ignore_ascii_case("kde"))
        {
            let out = command("kscreen-doctor", &["--dpms", power])?;
            // libkscreen may report an unsupported DPMS extension with exit code 0.
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.to_ascii_lowercase().contains("dpms not supported") {
                return Err("O KDE informou que esta sessão não oferece DPMS.".into());
            }
        } else {
            command("wlopm", &[command_power, "*"]).map_err(|error| {
                format!(
                    "O compositor Wayland precisa aceitar o controle de energia de telas: {error}"
                )
            })?;
        }
    } else if session.eq_ignore_ascii_case("x11") || env::var_os("DISPLAY").is_some() {
        command("xset", &["dpms", "force", power])?;
    } else {
        return Err("Não há uma sessão gráfica Linux disponível para controlar a tela.".into());
    }
    *toggle = next_off;
    Ok(next_off)
}

pub fn shutdown() -> Result<(), String> {
    command("systemctl", &["--no-ask-password", "poweroff"]).map(|_| ())
}

const STARTUP_FILE: &str = "aliexpress-macropad-configurator.desktop";
const STARTUP_MARKER: &str = "X-Mini-Teclado-USB-Autostart=true";

fn config_dir() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return Ok(path);
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|home| home.join(".config"))
        .ok_or_else(|| "Diretório de configuração Linux indisponível.".into())
}

fn desktop_entry(exe: &Path) -> Result<String, String> {
    let path = exe
        .to_str()
        .filter(|str| !str.is_empty() && !str.contains('=') && !str.chars().any(char::is_control))
        .ok_or("O caminho do aplicativo não pode ser usado na inicialização automática Linux.")?;
    let mut quoted = String::with_capacity(path.len() + 2);
    quoted.push('"');
    for ch in path.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\\\\\"),
            '"' | '$' | '\x60' => {
                quoted.push_str("\\\\");
                quoted.push(ch);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    Ok(format!("[Desktop Entry]\nType=Application\nName=Mini Teclado USB\nExec={quoted} --bandeja\nTerminal=false\n{STARTUP_MARKER}\n"))
}

fn read_startup(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) if text.lines().any(|line| line == STARTUP_MARKER) => Ok(Some(text)),
        Ok(_) => Err(
            "Já existe uma configuração de inicialização que não pertence ao aplicativo.".into(),
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Não foi possível ler a inicialização automática: {error}."
        )),
    }
}

pub fn startup_enabled() -> Result<bool, String> {
    let path = config_dir()?.join("autostart").join(STARTUP_FILE);
    Ok(read_startup(&path)?.is_some_and(|entry| !entry.lines().any(|line| line == "Hidden=true")))
}

pub fn set_startup(enabled: bool) -> Result<(), String> {
    let dir = config_dir()?.join("autostart");
    let path = dir.join(STARTUP_FILE);
    let existing = read_startup(&path)?;
    if !enabled {
        if existing.is_none() {
            return Ok(());
        }
        return fs::remove_file(&path).map_err(|error| {
            format!("Não foi possível remover a inicialização automática: {error}.")
        });
    }
    let exe = env::current_exe()
        .map_err(|error| format!("Não foi possível localizar o executável: {error}."))?;
    let entry = desktop_entry(&exe)?;
    if existing.as_deref() == Some(&entry) {
        return Ok(());
    }
    fs::create_dir_all(&dir)
        .map_err(|error| format!("Não foi possível criar a pasta de inicialização: {error}."))?;
    let temporary = dir.join(format!(".{STARTUP_FILE}.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| {
            format!("Não foi possível preparar a inicialização automática: {error}.")
        })?;
    let result = file
        .write_all(entry.as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&temporary, &path));
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Não foi possível gravar a inicialização automática: {error}."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_keeps_press_release_and_ignores_repeat() {
        let mut state = Keys::default();
        let at = Instant::now();
        assert!(matches!(
            state.key(183, 1, None, at),
            Some(InputEvent::Key {
                index: 0,
                down: true,
                ..
            })
        ));
        assert!(state.key(183, 2, None, at).is_none());
        assert!(matches!(
            state.key(192, 0, None, at),
            Some(InputEvent::VolumeKey {
                index: 5,
                down: false
            })
        ));
    }

    #[test]
    fn keeps_linux_hid_mapping_and_modifier_state() {
        let mut state = Keys::default();
        let at = Instant::now();
        state.key(29, 1, None, at);
        assert_eq!(state.modifiers, 1);
        assert!(matches!(
            state.key(30, 1, Some(0x0007_0004), at),
            Some(InputEvent::Observed(_, _))
        ));
        state.key(29, 0, None, at);
        assert_eq!(state.modifiers, 0);
        assert_eq!(hid_key(0x64), Some(86));
        assert_eq!(hid_key(0x87), Some(89));
        assert_eq!(linux_hid_key(186), Some(0x6b));
    }

    #[test]
    fn desktop_entry_escapes_path_and_blocks_injected_fields() {
        let entry = desktop_entry(Path::new("/opt/Mini \\$teclado%/mini")).unwrap();
        assert!(entry.contains("Exec=\"/opt/Mini \\\\\\\\\\\\$teclado%%/mini\" --bandeja"));
        assert!(desktop_entry(Path::new("/opt/mini\nExec=other")).is_err());
    }

    #[test]
    fn replay_packs_ordered_key_up_and_linux_relative_wheel_units() {
        assert_eq!(
            replay_events(&Binding::Keyboard(vec![(1, 0x04)])).unwrap(),
            [
                (EV_KEY, 29, 1),
                (EV_KEY, 30, 1),
                (EV_SYN, SYN_REPORT, 0),
                (EV_KEY, 30, 0),
                (EV_KEY, 29, 0),
                (EV_SYN, SYN_REPORT, 0),
            ]
        );
        assert!(replay_events(&Binding::Mouse {
            buttons: 0,
            scroll: -1,
            modifiers: 0
        })
        .unwrap()
        .contains(&(EV_REL, REL_WHEEL, -1)));
    }
}
